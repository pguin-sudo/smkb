use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::{PlatformError, Result};

fn socket_path(file_name: &str) -> Result<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| PlatformError::Hyprland("XDG_RUNTIME_DIR is not set".into()))?;
    let his = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .map_err(|_| PlatformError::Hyprland("HYPRLAND_INSTANCE_SIGNATURE is not set (Hyprland not running?)".into()))?;
    Ok(PathBuf::from(runtime_dir).join("hypr").join(his).join(file_name))
}

pub fn control_socket_path() -> Result<PathBuf> {
    socket_path(".socket.sock")
}

pub async fn request(req: &str) -> Result<String> {
    let path = control_socket_path()?;
    let mut stream = UnixStream::connect(&path)
        .await
        .map_err(|e| PlatformError::Hyprland(format!("connecting to {}: {e}", path.display())))?;
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| PlatformError::Hyprland(format!("writing request: {e}")))?;
    stream
        .shutdown()
        .await
        .map_err(|e| PlatformError::Hyprland(format!("half-closing request: {e}")))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .map_err(|e| PlatformError::Hyprland(format!("reading response: {e}")))?;
    Ok(response)
}

pub async fn ctl(args: &str) -> Result<String> {
    request(args).await
}

pub async fn ctl_json(args: &str) -> Result<serde_json::Value> {
    let raw = request(&format!("j/{args}")).await?;
    serde_json::from_str(&raw)
        .map_err(|e| PlatformError::Hyprland(format!("parsing JSON response to `{args}`: {e} (raw: {raw})")))
}

pub async fn ctl_ok(cmd: &str) -> Result<()> {
    let response = request(cmd).await?;
    if response.trim() != "ok" {
        return Err(PlatformError::Hyprland(format!("`{cmd}` failed: {response}")));
    }
    Ok(())
}
