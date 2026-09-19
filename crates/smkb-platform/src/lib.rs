pub mod hyprland;
pub mod linux;

use async_trait::async_trait;
use smkb_proto::{DisplayGeometry, InputEvent, Point};
use std::fmt;

pub type Result<T> = std::result::Result<T, PlatformError>;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("no supported compositor/window manager detected (checked: Hyprland)")]
    NoBackend,
    #[error("input device error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Hyprland IPC error: {0}")]
    Hyprland(String),
    #[error("no usable input devices found (mouse + keyboard required)")]
    NoDevices,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: std::path::PathBuf,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    ConsumerControl,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceKind::Mouse => write!(f, "mouse"),
            DeviceKind::Keyboard => write!(f, "keyboard"),
            DeviceKind::ConsumerControl => write!(f, "consumer-control"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawInputEvent {
    RelMotion { dx: i32, dy: i32 },
    Key { code: u16, pressed: bool },
    Button { code: u16, pressed: bool },
    Scroll { dx: i32, dy: i32, hi_res: bool },
}

pub type CaptureBatch = Vec<RawInputEvent>;

#[async_trait]
pub trait InputCapture: Send {
    fn devices(&self) -> &[DeviceInfo];

    fn rescan(&mut self) -> Result<()>;

    fn apply_filters(&mut self, explicit: Option<&[String]>, exclude_patterns: &[String], include_consumer_control: bool);

    fn grab(&mut self) -> Result<()>;

    async fn release(&mut self) -> Result<()>;

    async fn next(&mut self) -> Result<CaptureBatch>;
}

pub trait InputEmitter: Send {
    fn emit(&mut self, event: &InputEvent) -> Result<()>;

    fn release_all(&mut self) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct RealMonitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
}

#[async_trait]
pub trait CursorSource: Send {
    async fn real_monitors(&mut self) -> Result<Vec<RealMonitor>>;

    async fn position(&mut self) -> Result<Point>;

    async fn warp(&mut self, p: Point) -> Result<()>;

    async fn set_visible(&mut self, visible: bool) -> Result<()>;
}

#[async_trait]
pub trait ScreenInfo: Send {
    async fn primary(&mut self) -> Result<DisplayGeometry>;
}

pub struct MasterBackend {
    pub capture: Box<dyn InputCapture>,
    pub cursor: Box<dyn CursorSource>,
}

pub struct SlaveBackend {
    pub emitter: Box<dyn InputEmitter>,
    pub screen: Box<dyn ScreenInfo>,
}

pub async fn detect_master() -> Result<MasterBackend> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let cursor = hyprland::HyprlandCursor::new();
        let capture = linux::evdev_capture::EvdevCapture::discover()?;
        return Ok(MasterBackend { capture: Box::new(capture), cursor: Box::new(cursor) });
    }
    Err(PlatformError::NoBackend)
}

pub async fn detect_slave() -> Result<SlaveBackend> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let mut screen = hyprland::HyprlandCursor::new();
        let geometry = ScreenInfo::primary(&mut screen).await?;
        let emitter = linux::uinput_emitter::UinputEmitter::new(geometry)?;
        return Ok(SlaveBackend { emitter: Box::new(emitter), screen: Box::new(screen) });
    }
    Err(PlatformError::NoBackend)
}
