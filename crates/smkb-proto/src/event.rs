use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayGeometry {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub scale_pct: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    Motion { x: i32, y: i32 },
    Button { code: u16, pressed: bool, at: Point },
    Key { code: u16, pressed: bool },
    Scroll { dx: i32, dy: i32, hi_res: bool },
    Enter { at: Point },
    Leave,
    ReleaseAll,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloInfo {
    pub proto_version: u32,
    pub hostname: String,
    pub screen: DisplayGeometry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WelcomeInfo {
    pub proto_version: u32,
}

pub const PROTO_VERSION: u32 = 1;
