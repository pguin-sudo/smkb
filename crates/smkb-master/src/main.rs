mod config;
mod discovery;
mod pointer;
mod safety;
mod state;

use anyhow::Context;
use config::Config;
use state::App;
use std::path::PathBuf;

fn default_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
        PathBuf::from(home).join(".config")
    });
    base.join("smkb").join("master.yaml")
}

fn parse_args() -> PathBuf {
    let mut config = default_config_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--config requires a path argument");
                std::process::exit(2);
            }),
            "-h" | "--help" => {
                println!("smkb-master [--config PATH]\n\n--config PATH   config file (default: $XDG_CONFIG_HOME/smkb/master.yaml)");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
        }
    }
    config
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to install SIGTERM handler, Ctrl+C still works");
            std::future::pending().await
        }
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = sigterm.recv() => {}
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = parse_args();
    let config = Config::load(&config_path).with_context(|| format!("loading config from {}", config_path.display()))?;
    let psk = config.load_psk().context("loading psk_file")?;

    let backend = smkb_platform::detect_master().await.context("detecting a supported compositor")?;
    let mut capture = backend.capture;
    let mut cursor = backend.cursor;

    let explicit_devices = match &config.capture.devices {
        config::AutoOr::Fixed(paths) => Some(paths.clone()),
        config::AutoOr::Auto => None,
    };
    capture.apply_filters(explicit_devices.as_deref(), &config.capture.exclude, config.capture.include_consumer_control);
    if capture.devices().is_empty() {
        anyhow::bail!("no input devices left to capture after filtering — check capture.exclude and capture.devices in {}", config_path.display());
    }
    for d in capture.devices() {
        tracing::info!(name = %d.name, kind = %d.kind, path = %d.path.display(), "will capture");
    }

    if let Ok(monitors) = cursor.real_monitors().await {
        tracing::debug!(count = monitors.len(), "real monitors visible at startup");
    }

    let socket = tokio::net::UdpSocket::bind(&config.listen).await.with_context(|| format!("binding UDP socket on {}", config.listen))?;
    let bound_port = socket.local_addr()?.port();
    tracing::info!(listen = %config.listen, "smkb-master ready, waiting for a slave to connect");

    let _advertiser = if config.discovery.mdns {
        match discovery::Advertiser::start(bound_port) {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!(error = %e, "mDNS advertising failed to start, slaves will need an explicit `master:` address");
                None
            }
        }
    } else {
        None
    };

    let mut app = App::new(config, psk, socket, capture, cursor)?;

    tokio::select! {
        result = app.run_forever() => {
            if let Err(e) = &result {
                tracing::error!(error = %e, "fatal error, shutting down");
            }
            app.cleanup().await;
            return result;
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
        }
    }
    app.cleanup().await;
    Ok(())
}
