use anyhow::Context;
use smkb_platform::{CursorSource, InputCapture, RawInputEvent, RealMonitor};
use smkb_proto::{reliable::RETRANSMIT_INTERVAL, DisplayGeometry, Frame, Handshake, InputEvent, Point, Psk, Session};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

use crate::config::Config;
use crate::pointer::{self, Accel, PointerIntegrator};
use crate::safety::{PanicHotkey, Watchdog};

const MAX_PACKET: usize = smkb_proto::MAX_PACKET_LEN;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseTag {
    Idle,
    AwaitingHello,
    Attached,
    Captured,
}

pub struct App {
    config: Config,
    psk: Psk,
    socket: UdpSocket,
    capture: Box<dyn InputCapture>,
    cursor: Box<dyn CursorSource>,
    hotkey: PanicHotkey,
    watchdog: Watchdog,

    phase: PhaseTag,
    session: Option<Session>,
    peer: Option<SocketAddr>,
    edge_monitor: Option<RealMonitor>,
    slave_screen: Option<DisplayGeometry>,
    integrator: Option<PointerIntegrator>,
}

impl App {
    pub fn new(config: Config, psk: Psk, socket: UdpSocket, capture: Box<dyn InputCapture>, cursor: Box<dyn CursorSource>) -> anyhow::Result<Self> {
        let hotkey_codes = config.panic_hotkey_codes()?;
        let watchdog_ms = config.safety.watchdog_ms;
        Ok(Self {
            config,
            psk,
            socket,
            capture,
            cursor,
            hotkey: PanicHotkey::new(hotkey_codes),
            watchdog: Watchdog::new(Duration::from_millis(watchdog_ms)),
            phase: PhaseTag::Idle,
            session: None,
            peer: None,
            edge_monitor: None,
            slave_screen: None,
            integrator: None,
        })
    }

    pub async fn run_forever(&mut self) -> anyhow::Result<()> {
        loop {
            match self.phase {
                PhaseTag::Idle => self.run_idle().await?,
                PhaseTag::AwaitingHello => self.run_awaiting_hello().await?,
                PhaseTag::Attached => self.run_attached().await?,
                PhaseTag::Captured => self.run_captured().await?,
            }
        }
    }

    pub async fn cleanup(&mut self) {
        if self.phase == PhaseTag::Captured {
            let _ = self.capture.release().await;
            let _ = self.cursor.set_visible(true).await;
        }
        if self.session.is_some() {
            self.send_control_now(Frame::Bye).await;
        }
    }

    async fn run_idle(&mut self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; MAX_PACKET];
        loop {
            let (n, addr) = self.socket.recv_from(&mut buf).await.context("recv_from in Idle")?;
            let mut hs = match Handshake::new_responder(&self.psk) {
                Ok(hs) => hs,
                Err(e) => {
                    tracing::error!(error = %e, "failed to start a Noise handshake session");
                    continue;
                }
            };
            if let Err(e) = hs.read_message(&buf[..n]) {
                tracing::debug!(%addr, error = %e, "ignoring datagram that isn't a valid handshake message");
                continue;
            }
            let mut reply = [0u8; 256];
            let len = match hs.write_message(&mut reply) {
                Ok(len) => len,
                Err(e) => {
                    tracing::warn!(%addr, error = %e, "failed to build handshake reply");
                    continue;
                }
            };
            let keys = match hs.finish() {
                Ok(k) => k,
                Err(e) => {
                    tracing::warn!(%addr, error = %e, "handshake did not finish after two messages");
                    continue;
                }
            };
            if let Err(e) = self.socket.send_to(&reply[..len], addr).await {
                tracing::warn!(%addr, error = %e, "failed to send handshake reply");
                continue;
            }

            tracing::info!(%addr, "slave authenticated, awaiting Hello");
            self.peer = Some(addr);
            self.session = Some(Session::new(keys));
            self.watchdog.poke();
            self.phase = PhaseTag::AwaitingHello;
            return Ok(());
        }
    }

    async fn run_awaiting_hello(&mut self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; MAX_PACKET];
        let mut retransmit = tokio::time::interval(RETRANSMIT_INTERVAL);
        let deadline = tokio::time::sleep(HELLO_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                _ = &mut deadline => {
                    tracing::warn!("slave authenticated but never sent Hello, giving up");
                    return self.teardown_to_idle().await;
                }
                _ = retransmit.tick() => self.tick_retransmits().await,
                recv = self.socket.recv_from(&mut buf) => {
                    let (n, addr) = recv.context("recv_from in AwaitingHello")?;
                    let ready = self.on_incoming(addr, &buf[..n]).await;
                    for frame in ready {
                        if let Frame::Hello(hello) = frame {
                            return self.on_hello(hello).await;
                        }
                    }
                }
            }
        }
    }

    async fn on_hello(&mut self, hello: smkb_proto::HelloInfo) -> anyhow::Result<()> {
        if hello.proto_version != smkb_proto::PROTO_VERSION {
            tracing::warn!(got = hello.proto_version, want = smkb_proto::PROTO_VERSION, "protocol version mismatch, rejecting slave");
            return self.teardown_to_idle().await;
        }

        let monitors = match self.cursor.real_monitors().await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "failed to query real monitor layout");
                return self.teardown_to_idle().await;
            }
        };
        let edge_monitor = match monitors.into_iter().find(|m| m.name == self.config.edge.monitor) {
            Some(m) => m,
            None => {
                tracing::error!(monitor = %self.config.edge.monitor, "configured edge.monitor not found");
                return self.teardown_to_idle().await;
            }
        };

        tracing::info!(monitor = %edge_monitor.name, side = ?self.config.edge.side, hostname = %hello.hostname, "slave attached");
        self.edge_monitor = Some(edge_monitor);
        self.slave_screen = Some(hello.screen);
        self.watchdog.poke();
        self.send_reliable_now(Frame::Welcome(smkb_proto::WelcomeInfo { proto_version: smkb_proto::PROTO_VERSION })).await;
        self.phase = PhaseTag::Attached;
        Ok(())
    }

    async fn run_attached(&mut self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; MAX_PACKET];
        let mut retransmit = tokio::time::interval(RETRANSMIT_INTERVAL);
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut edge_poll = tokio::time::interval(Duration::from_millis(self.config.edge.poll_ms));
        loop {
            tokio::select! {
                biased;
                _ = retransmit.tick() => self.tick_retransmits().await,
                _ = heartbeat.tick() => self.send_ping().await,
                recv = self.socket.recv_from(&mut buf) => {
                    let (n, addr) = recv.context("recv_from in Attached")?;
                    let ready = self.on_incoming(addr, &buf[..n]).await;
                    if ready.iter().any(|f| matches!(f, Frame::Bye)) {
                        tracing::info!("slave disconnected");
                        return self.teardown_to_idle().await;
                    }
                }
                _ = edge_poll.tick() => {
                    let monitor = self.edge_monitor.clone().expect("set on entering Attached");
                    match self.cursor.position().await {
                        Ok(pos) => {
                            if pointer::at_trigger_edge(pos, &monitor, self.config.edge.side) {
                                tracing::info!(?pos, "pointer reached the doorway edge");
                                return self.begin_capture(pos).await;
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "failed to read cursor position"),
                    }
                }
            }
            if self.watchdog.expired(Instant::now()) {
                tracing::warn!("watchdog expired while Attached (slave went silent)");
                return self.teardown_to_idle().await;
            }
        }
    }

    async fn begin_capture(&mut self, global: Point) -> anyhow::Result<()> {
        if let Err(e) = self.capture.grab() {
            tracing::error!(error = %e, "failed to grab input devices, staying Attached");
            return Ok(());
        }

        let monitor = self.edge_monitor.clone().expect("set on entering Attached");
        let slave_screen = self.slave_screen.expect("set on entering Attached");
        let local = pointer::entry_point(global, &monitor, self.config.edge.side, slave_screen);
        let mut integrator = PointerIntegrator::new(slave_screen, Accel { sensitivity: self.config.pointer.sensitivity });
        integrator.reset_at(local);
        self.integrator = Some(integrator);
        self.hotkey.reset();
        self.watchdog.poke();

        if let Err(e) = self.cursor.set_visible(false).await {
            tracing::warn!(error = %e, "failed to hide the real cursor");
        }

        tracing::info!(?local, "capture started");
        self.send_reliable_now(Frame::Event(InputEvent::ReleaseAll)).await;
        self.send_reliable_now(Frame::Event(InputEvent::Enter { at: local })).await;
        self.phase = PhaseTag::Captured;
        Ok(())
    }

    async fn run_captured(&mut self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; MAX_PACKET];
        let mut retransmit = tokio::time::interval(RETRANSMIT_INTERVAL);
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            tokio::select! {
                biased;
                _ = retransmit.tick() => self.tick_retransmits().await,
                _ = heartbeat.tick() => self.send_ping().await,
                recv = self.socket.recv_from(&mut buf) => {
                    let (n, addr) = recv.context("recv_from in Captured")?;
                    let ready = self.on_incoming(addr, &buf[..n]).await;
                    if ready.iter().any(|f| matches!(f, Frame::Bye)) {
                        tracing::warn!("slave disconnected while captured, releasing input");
                        let _ = self.capture.release().await;
                        return self.teardown_to_idle().await;
                    }
                }
                batch = self.capture.next() => {
                    match batch {
                        Ok(batch) => {
                            if let Some(done) = self.handle_capture_batch(batch).await {
                                return done;
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "capture error while Captured, releasing input");
                            let _ = self.capture.release().await;
                            return self.teardown_to_idle().await;
                        }
                    }
                }
            }
            if self.watchdog.expired(Instant::now()) {
                tracing::warn!(watchdog_ms = self.config.safety.watchdog_ms, "watchdog expired while Captured (slave went silent), releasing input");
                return self.end_capture(None).await;
            }
        }
    }

    async fn handle_capture_batch(&mut self, batch: smkb_platform::CaptureBatch) -> Option<anyhow::Result<()>> {
        for raw in batch {
            match raw {
                RawInputEvent::RelMotion { dx, dy } => {
                    let (pos, edge) = self.integrator.as_mut().expect("set on entering Captured").apply_delta(dx, dy);
                    self.send_unreliable_now(Frame::Event(InputEvent::Motion { x: pos.x, y: pos.y })).await;
                    if let Some(edge) = edge {
                        tracing::info!(?edge, ?pos, "vcursor crossed slave-side edge");
                        return Some(self.end_capture(Some(pos)).await);
                    }
                }
                RawInputEvent::Key { code, pressed } => {
                    if self.hotkey.observe(code, pressed) {
                        tracing::warn!("panic hotkey triggered, releasing capture");
                        return Some(self.end_capture(None).await);
                    }
                    self.send_reliable_now(Frame::Event(InputEvent::Key { code, pressed })).await;
                }
                RawInputEvent::Button { code, pressed } => {
                    let at = self.integrator.as_ref().expect("set on entering Captured").position();
                    self.send_reliable_now(Frame::Event(InputEvent::Button { code, pressed, at })).await;
                }
                RawInputEvent::Scroll { dx, dy, hi_res } => {
                    self.send_reliable_now(Frame::Event(InputEvent::Scroll { dx, dy, hi_res })).await;
                }
            }
        }
        None
    }

    async fn end_capture(&mut self, local_exit: Option<Point>) -> anyhow::Result<()> {
        let _ = self.capture.release().await;
        self.send_reliable_now(Frame::Event(InputEvent::Leave)).await;
        self.send_reliable_now(Frame::Event(InputEvent::ReleaseAll)).await;

        if let Some(local_exit) = local_exit {
            let monitor = self.edge_monitor.clone().expect("set while Captured");
            let slave_screen = self.slave_screen.expect("set while Captured");
            let target = pointer::exit_point(local_exit, &monitor, self.config.edge.side, slave_screen);
            if let Err(e) = self.cursor.warp(target).await {
                tracing::warn!(error = %e, "failed to warp cursor back onto the real desktop");
            }
        }

        if let Err(e) = self.cursor.set_visible(true).await {
            tracing::warn!(error = %e, "failed to show the real cursor again");
        }

        self.integrator = None;
        self.phase = PhaseTag::Attached;
        tracing::info!("capture ended");
        Ok(())
    }

    async fn teardown_to_idle(&mut self) -> anyhow::Result<()> {
        let _ = self.capture.release().await;
        self.session = None;
        self.peer = None;
        self.integrator = None;
        self.edge_monitor = None;
        self.slave_screen = None;
        self.phase = PhaseTag::Idle;
        Ok(())
    }

    async fn on_incoming(&mut self, addr: SocketAddr, data: &[u8]) -> Vec<Frame> {
        if Some(addr) != self.peer {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else { return Vec::new() };
        match session.on_datagram(data) {
            Ok(received) => {
                self.watchdog.poke();
                if let Some(reply) = received.reply {
                    self.send_packet(&reply).await;
                }
                received.ready
            }
            Err(e) => {
                tracing::debug!(error = %e, "dropping undecryptable/malformed packet");
                Vec::new()
            }
        }
    }

    async fn tick_retransmits(&mut self) {
        let Some(session) = self.session.as_mut() else { return };
        let due = session.due_retransmits(Instant::now());
        for pkt in due {
            self.send_packet(&pkt).await;
        }
    }

    async fn send_ping(&mut self) {
        self.send_control_now(Frame::Ping { nonce: rand::random() }).await;
    }

    async fn send_reliable_now(&mut self, frame: Frame) {
        let Some(session) = self.session.as_mut() else { return };
        let pkt = session.send_reliable(&frame, Instant::now());
        self.send_packet(&pkt).await;
    }

    async fn send_unreliable_now(&mut self, frame: Frame) {
        let Some(session) = self.session.as_mut() else { return };
        let pkt = session.send_unreliable(&frame);
        self.send_packet(&pkt).await;
    }

    async fn send_control_now(&mut self, frame: Frame) {
        let Some(session) = self.session.as_mut() else { return };
        let pkt = session.send_control(&frame);
        self.send_packet(&pkt).await;
    }

    async fn send_packet(&mut self, pkt: &[u8]) {
        if let Some(peer) = self.peer
            && let Err(e) = self.socket.send_to(pkt, peer).await {
                tracing::warn!(error = %e, "failed to send packet");
            }
    }
}
