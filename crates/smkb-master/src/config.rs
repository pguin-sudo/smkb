use serde::{Deserialize, Deserializer};
use smkb_proto::{Edge, Psk};
use std::path::{Path, PathBuf};

pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        None => PathBuf::from(path),
    }
}

#[derive(Debug, Clone, Default)]
pub enum AutoOr<T> {
    #[default]
    Auto,
    Fixed(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for AutoOr<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr<T> {
            Str(String),
            Val(T),
        }
        match Repr::<T>::deserialize(deserializer)? {
            Repr::Str(s) if s == "auto" => Ok(AutoOr::Auto),
            Repr::Str(s) => Err(serde::de::Error::custom(format!("expected \"auto\" or a value, got string {s:?}"))),
            Repr::Val(v) => Ok(AutoOr::Fixed(v)),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub listen: String,
    pub psk_file: String,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    pub edge: EdgeConfig,
    #[serde(default)]
    pub pointer: PointerConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub safety: SafetyConfig,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_true")]
    pub mdns: bool,
}
impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self { mdns: true }
    }
}

#[derive(Debug, Deserialize)]
pub struct EdgeConfig {
    pub monitor: String,
    pub side: Edge,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
}
fn default_poll_ms() -> u64 {
    8
}

#[derive(Debug, Deserialize)]
pub struct PointerConfig {
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f64,
}
impl Default for PointerConfig {
    fn default() -> Self {
        Self { sensitivity: default_sensitivity() }
    }
}
fn default_sensitivity() -> f64 {
    1.0
}

#[derive(Debug, Default, Deserialize)]
pub struct CaptureConfig {
    #[serde(default)]
    pub devices: AutoOr<Vec<String>>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_true")]
    pub include_consumer_control: bool,
}

#[derive(Debug, Deserialize)]
pub struct SafetyConfig {
    #[serde(default = "default_panic_hotkey")]
    pub panic_hotkey: Vec<String>,
    #[serde(default = "default_watchdog_ms")]
    pub watchdog_ms: u64,
}
impl Default for SafetyConfig {
    fn default() -> Self {
        Self { panic_hotkey: default_panic_hotkey(), watchdog_ms: default_watchdog_ms() }
    }
}
fn default_panic_hotkey() -> Vec<String> {
    vec!["KEY_LEFTCTRL".into(), "KEY_LEFTALT".into(), "KEY_ESC".into()]
}
fn default_watchdog_ms() -> u64 {
    1000
}
fn default_true() -> bool {
    true
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

    pub fn panic_hotkey_codes(&self) -> anyhow::Result<Vec<u16>> {
        self.safety
            .panic_hotkey
            .iter()
            .map(|name| name.parse::<evdev::KeyCode>().map(|k| k.0).map_err(|_| anyhow::anyhow!("unknown key name in safety.panic_hotkey: {name:?}")))
            .collect()
    }
}
