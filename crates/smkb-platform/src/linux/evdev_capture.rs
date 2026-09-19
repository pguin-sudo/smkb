use async_trait::async_trait;
use evdev::{Device, EventSummary, EventStream, KeyCode, RelativeAxisCode};
use std::path::Path;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::{CaptureBatch, DeviceInfo, DeviceKind, InputCapture, PlatformError, RawInputEvent, Result};

const OWN_DEVICE_PREFIX: &str = "smkb-";

fn classify(device: &Device) -> Option<DeviceKind> {
    let name = device.name().unwrap_or_default();
    if name.starts_with(OWN_DEVICE_PREFIX) {
        return None;
    }
    let keys = device.supported_keys();
    let rel = device.supported_relative_axes();

    let is_pointer = keys.is_some_and(|k| k.contains(KeyCode::BTN_LEFT))
        && rel.is_some_and(|r| r.contains(RelativeAxisCode::REL_X) && r.contains(RelativeAxisCode::REL_Y));
    if is_pointer {
        return Some(DeviceKind::Mouse);
    }

    let is_typewriter = keys.is_some_and(|k| k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_SPACE));
    if is_typewriter {
        return Some(DeviceKind::Keyboard);
    }

    if keys.is_some_and(|k| k.iter().next().is_some()) {
        return Some(DeviceKind::ConsumerControl);
    }

    None
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(mid), _) if pattern.ends_with('*') && pattern.len() > 1 => text.contains(&mid[..mid.len() - 1]),
        (Some(suffix), _) => text.ends_with(suffix),
        (_, Some(prefix)) => text.starts_with(prefix),
        _ => text == pattern,
    }
}

struct Worker {
    shutdown: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

pub struct EvdevCapture {
    devices: Vec<DeviceInfo>,
    workers: Vec<Worker>,
    batch_tx: mpsc::UnboundedSender<CaptureBatch>,
    batch_rx: mpsc::UnboundedReceiver<CaptureBatch>,
}

impl EvdevCapture {
    pub fn discover() -> Result<Self> {
        let mut devices = Vec::new();
        for (path, device) in evdev::enumerate() {
            if let Some(kind) = classify(&device) {
                devices.push(DeviceInfo { path, name: device.name().unwrap_or("<unnamed>").to_string(), kind });
            }
        }
        let (batch_tx, batch_rx) = mpsc::unbounded_channel();
        Ok(Self { devices, workers: Vec::new(), batch_tx, batch_rx })
    }
}

#[async_trait]
impl InputCapture for EvdevCapture {
    fn devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    fn rescan(&mut self) -> Result<()> {
        let fresh = Self::discover()?;
        self.devices = fresh.devices;
        Ok(())
    }

    fn apply_filters(&mut self, explicit: Option<&[String]>, exclude_patterns: &[String], include_consumer_control: bool) {
        if let Some(paths) = explicit {
            self.devices.retain(|d| paths.iter().any(|p| Path::new(p) == d.path));
            return;
        }
        self.devices.retain(|d| {
            if !include_consumer_control && d.kind == DeviceKind::ConsumerControl {
                return false;
            }
            !exclude_patterns.iter().any(|p| glob_match(p, &d.name))
        });
    }

    fn grab(&mut self) -> Result<()> {
        if !self.workers.is_empty() {
            return Ok(());
        }
        if self.devices.is_empty() {
            return Err(PlatformError::NoDevices);
        }

        let mut opened: Vec<(DeviceKind, Device)> = Vec::with_capacity(self.devices.len());
        for info in &self.devices {
            match open_and_grab(&info.path) {
                Ok(device) => opened.push((info.kind, device)),
                Err(e) => {
                    for (_, mut device) in opened {
                        let _ = device.ungrab();
                    }
                    return Err(e);
                }
            }
        }

        for (kind, device) in opened {
            let stream = match device.into_event_stream() {
                Ok(s) => s,
                Err(e) => {
                    for worker in self.workers.drain(..) {
                        let _ = worker.shutdown.send(());
                        worker.join.abort();
                    }
                    return Err(PlatformError::Io(e));
                }
            };
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let batch_tx = self.batch_tx.clone();
            let join = tokio::spawn(device_worker(stream, kind, shutdown_rx, batch_tx));
            self.workers.push(Worker { shutdown: shutdown_tx, join });
        }
        Ok(())
    }

    async fn release(&mut self) -> Result<()> {
        for worker in self.workers.drain(..) {
            let _ = worker.shutdown.send(());
            let _ = worker.join.await;
        }
        Ok(())
    }

    async fn next(&mut self) -> Result<CaptureBatch> {
        self.batch_rx
            .recv()
            .await
            .ok_or_else(|| PlatformError::Hyprland("capture channel closed unexpectedly".into()))
    }
}

fn open_and_grab(path: &Path) -> Result<Device> {
    let mut device = Device::open(path).map_err(PlatformError::Io)?;
    let mut last_err = None;
    for attempt in 0..5 {
        match device.grab() {
            Ok(()) => return Ok(device),
            Err(e) => {
                last_err = Some(e);
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
    Err(PlatformError::Io(last_err.expect("loop always sets last_err before exiting")))
}

async fn device_worker(
    mut stream: EventStream,
    kind: DeviceKind,
    mut shutdown: oneshot::Receiver<()>,
    batch_tx: mpsc::UnboundedSender<CaptureBatch>,
) {
    let mut pending = PendingBatch::default();
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                tokio::task::spawn_blocking(move || drop(stream));
                return;
            }
            event = stream.next_event() => {
                let event = match event {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::warn!(?err, device = ?stream.device().name(), "capture device error, stopping worker");
                        return;
                    }
                };
                if pending.push(event, kind) {
                    let batch = pending.take();
                    if batch_tx.send(batch).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct PendingBatch {
    dx: i32,
    dy: i32,
    wheel_hi_res: i32,
    hwheel_hi_res: i32,
    wheel_legacy: i32,
    hwheel_legacy: i32,
    others: CaptureBatch,
}

impl PendingBatch {
    fn push(&mut self, event: evdev::InputEvent, kind: DeviceKind) -> bool {
        match event.destructure() {
            EventSummary::Synchronization(_, code, _) => code == evdev::SynchronizationCode::SYN_REPORT,
            EventSummary::Key(_, code, value) => {
                let pressed = value != 0;
                let raw = match kind {
                    DeviceKind::Mouse => RawInputEvent::Button { code: code.0, pressed },
                    DeviceKind::Keyboard | DeviceKind::ConsumerControl => RawInputEvent::Key { code: code.0, pressed },
                };
                self.others.push(raw);
                false
            }
            EventSummary::RelativeAxis(_, code, value) => {
                match code {
                    RelativeAxisCode::REL_X => self.dx += value,
                    RelativeAxisCode::REL_Y => self.dy += value,
                    RelativeAxisCode::REL_WHEEL => self.wheel_legacy += value,
                    RelativeAxisCode::REL_HWHEEL => self.hwheel_legacy += value,
                    RelativeAxisCode::REL_WHEEL_HI_RES => self.wheel_hi_res += value,
                    RelativeAxisCode::REL_HWHEEL_HI_RES => self.hwheel_hi_res += value,
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn take(&mut self) -> CaptureBatch {
        let mut batch = std::mem::take(&mut self.others);
        if self.dx != 0 || self.dy != 0 {
            batch.push(RawInputEvent::RelMotion { dx: self.dx, dy: self.dy });
        }
        let (dx, hi_res) = if self.wheel_hi_res != 0 || self.hwheel_hi_res != 0 {
            (self.hwheel_hi_res, true)
        } else if self.hwheel_legacy != 0 {
            (self.hwheel_legacy * 120, false)
        } else {
            (0, false)
        };
        let dy = if self.wheel_hi_res != 0 {
            self.wheel_hi_res
        } else if self.wheel_legacy != 0 {
            self.wheel_legacy * 120
        } else {
            0
        };
        if dx != 0 || dy != 0 {
            batch.push(RawInputEvent::Scroll { dx, dy, hi_res });
        }
        self.dx = 0;
        self.dy = 0;
        self.wheel_hi_res = 0;
        self.hwheel_hi_res = 0;
        self.wheel_legacy = 0;
        self.hwheel_legacy = 0;
        batch
    }
}
