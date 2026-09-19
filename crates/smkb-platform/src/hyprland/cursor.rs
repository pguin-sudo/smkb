use async_trait::async_trait;
use smkb_proto::{DisplayGeometry, Point};

use super::ipc;
use crate::{CursorSource, PlatformError, RealMonitor, Result, ScreenInfo};

fn real_monitor_from_json(v: &serde_json::Value) -> Result<RealMonitor> {
    let bad = |field: &str| PlatformError::Hyprland(format!("monitor JSON missing/wrong-typed field `{field}`: {v}"));
    Ok(RealMonitor {
        name: v.get("name").and_then(|x| x.as_str()).ok_or_else(|| bad("name"))?.to_string(),
        x: v.get("x").and_then(|x| x.as_i64()).ok_or_else(|| bad("x"))? as i32,
        y: v.get("y").and_then(|x| x.as_i64()).ok_or_else(|| bad("y"))? as i32,
        width: v.get("width").and_then(|x| x.as_u64()).ok_or_else(|| bad("width"))? as u32,
        height: v.get("height").and_then(|x| x.as_u64()).ok_or_else(|| bad("height"))? as u32,
        focused: v.get("focused").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

pub struct HyprlandCursor;

impl HyprlandCursor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HyprlandCursor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CursorSource for HyprlandCursor {
    async fn real_monitors(&mut self) -> Result<Vec<RealMonitor>> {
        let json = ipc::ctl_json("monitors").await?;
        let arr = json.as_array().ok_or_else(|| PlatformError::Hyprland("`monitors` response was not a JSON array".into()))?;
        arr.iter().map(real_monitor_from_json).collect()
    }

    async fn position(&mut self) -> Result<Point> {
        let raw = ipc::ctl("cursorpos").await?;
        let (x, y) = raw
            .trim()
            .split_once(',')
            .ok_or_else(|| PlatformError::Hyprland(format!("unexpected `cursorpos` response: {raw}")))?;
        let parse = |s: &str| {
            s.trim()
                .parse::<i32>()
                .map_err(|e| PlatformError::Hyprland(format!("parsing cursorpos `{raw}`: {e}")))
        };
        Ok(Point::new(parse(x)?, parse(y)?))
    }

    async fn warp(&mut self, p: Point) -> Result<()> {
        ipc::ctl_ok(&format!("dispatch movecursor {} {}", p.x, p.y)).await
    }

    async fn set_visible(&mut self, visible: bool) -> Result<()> {
        ipc::ctl_ok(&format!("keyword cursor:invisible {}", !visible)).await
    }
}

#[async_trait]
impl ScreenInfo for HyprlandCursor {
    async fn primary(&mut self) -> Result<DisplayGeometry> {
        let monitors = self.real_monitors().await?;
        let mon = monitors
            .iter()
            .find(|m| m.focused)
            .or_else(|| monitors.first())
            .ok_or_else(|| PlatformError::Hyprland("no monitors reported".into()))?;

        let json = ipc::ctl_json("monitors").await?;
        let arr = json.as_array().ok_or_else(|| PlatformError::Hyprland("`monitors` response was not a JSON array".into()))?;
        let entry = arr
            .iter()
            .find(|v| v.get("name").and_then(|n| n.as_str()) == Some(mon.name.as_str()))
            .ok_or_else(|| PlatformError::Hyprland(format!("monitor {} vanished between queries", mon.name)))?;
        let refresh_hz = entry.get("refreshRate").and_then(|x| x.as_f64()).unwrap_or(60.0);
        let scale = entry.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0);

        Ok(DisplayGeometry {
            width: mon.width,
            height: mon.height,
            refresh_mhz: (refresh_hz * 1000.0).round() as u32,
            scale_pct: (scale * 100.0).round() as u32,
        })
    }
}
