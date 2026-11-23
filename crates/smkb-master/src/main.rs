mod config;
mod discovery;
mod pointer;
mod safety;
mod state;

use anyhow::Context;
use config::Config;
use smkb_app::{parse_args, shutdown_signal};
use state::App;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_path = parse_args("master");
    let config = smkb_app::load_config::<Config>(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    let psk = smkb_app::load_psk(&config.psk_file).context("loading psk_file")?;

    let backend = smkb_platform::detect_master()
        .await
        .context("detecting a supported compositor")?;
    let mut capture = backend.capture;
    let mut cursor = backend.cursor;

    let explicit_devices = match &config.capture.devices {
        config::AutoOr::Fixed(paths) => Some(paths.clone()),
        config::AutoOr::Auto => None,
    };
    capture.apply_filters(
        explicit_devices.as_deref(),
        &config.capture.exclude,
        config.capture.include_consumer_control,
    );
    if capture.devices().is_empty() {
        anyhow::bail!(
            "no input devices left to capture after filtering — check capture.exclude and capture.devices in {}",
            config_path.display()
        );
    }
    for d in capture.devices() {
        tracing::info!(name = %d.name, kind = %d.kind, path = %d.path.display(), "will capture");
    }

    if let Ok(monitors) = cursor.real_monitors().await {
        tracing::debug!(count = monitors.len(), "real monitors visible at startup");
    }

    let socket = tokio::net::UdpSocket::bind(&config.listen)
        .await
        .with_context(|| format!("binding UDP socket on {}", config.listen))?;
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
