use smkb_proto::Psk;
use std::path::{Path, PathBuf};

fn default_config_path(role: &str) -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
            PathBuf::from(home).join(".config")
        });
    base.join("smkb").join(format!("{role}.yaml"))
}

pub fn parse_args(role: &str) -> PathBuf {
    let mut config = default_config_path(role);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                config = args.next().map(PathBuf::from).unwrap_or_else(|| {
                    eprintln!("--config requires a path argument");
                    std::process::exit(2);
                })
            }
            "-h" | "--help" => {
                println!(
                    "smkb-{role} [--config PATH]\n\n--config PATH   config file (default: $XDG_CONFIG_HOME/smkb/{role}.yaml)"
                );
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

pub async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
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

pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        None => PathBuf::from(path),
    }
}

pub fn load_config<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let config: T = serde_yaml_ng::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
    Ok(config)
}

pub fn load_psk(psk_file: &str) -> anyhow::Result<Psk> {
    let path = expand_tilde(psk_file);
    let bytes = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("reading psk_file {}: {e}", path.display()))?;
    smkb_proto::parse_psk(&bytes).map_err(|e| anyhow::anyhow!("psk_file {}: {e}", path.display()))
}
