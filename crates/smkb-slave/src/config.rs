use serde::Deserialize;
use smkb_proto::Psk;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub master: MasterAddr,
    pub psk_file: String,
    #[serde(default)]
    pub reconnect: ReconnectConfig,
}

#[derive(Debug, Clone)]
pub enum MasterAddr {
    Auto,
    Explicit(String),
}

impl<'de> Deserialize<'de> for MasterAddr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "auto" { Ok(MasterAddr::Auto) } else { Ok(MasterAddr::Explicit(s)) }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReconnectConfig {
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
}
impl Default for ReconnectConfig {
    fn default() -> Self {
        Self { interval_ms: default_interval_ms() }
    }
}
fn default_interval_ms() -> u64 {
    500
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let config: Config = serde_yaml_ng::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        Ok(config)
    }

    pub fn load_psk(&self) -> anyhow::Result<Psk> {
        let path = expand_tilde(&self.psk_file);
        let bytes = std::fs::read(&path).map_err(|e| anyhow::anyhow!("reading psk_file {}: {e}", path.display()))?;
        smkb_proto::parse_psk(&bytes).map_err(|e| anyhow::anyhow!("psk_file {}: {e}", path.display()))
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
