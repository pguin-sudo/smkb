use serde::Deserialize;

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
        if s == "auto" {
            Ok(MasterAddr::Auto)
        } else {
            Ok(MasterAddr::Explicit(s))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReconnectConfig {
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
}
impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_interval_ms(),
        }
    }
}
fn default_interval_ms() -> u64 {
    500
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_loads() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/slave.yaml");
        let config: Config = smkb_app::load_config(&path).unwrap();
        assert_eq!(config.psk_file, "~/.config/smkb/psk");
        assert!(matches!(config.master, MasterAddr::Auto));
        assert_eq!(config.reconnect.interval_ms, 500);
    }
}
