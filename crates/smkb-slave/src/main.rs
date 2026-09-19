mod config;
mod discovery;

use anyhow::Context;
use config::{Config, MasterAddr};
use smkb_platform::InputEmitter;
use smkb_proto::{reliable::RETRANSMIT_INTERVAL, Frame, Handshake, HelloInfo, Psk, Session};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_ATTEMPTS: u32 = 5;
const HANDSHAKE_REPLY_TIMEOUT: Duration = Duration::from_millis(500);
const MDNS_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PACKET: usize = smkb_proto::MAX_PACKET_LEN;

fn default_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
        PathBuf::from(home).join(".config")
    });
    base.join("smkb").join("slave.yaml")
}

fn parse_args() -> PathBuf {
    let mut config = default_config_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--config requires a path argument");
                std::process::exit(2);
            }),
            "-h" | "--help" => {
                println!("smkb-slave [--config PATH]\n\n--config PATH   config file (default: $XDG_CONFIG_HOME/smkb/slave.yaml)");
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

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
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

async fn resolve_master(config: &Config) -> anyhow::Result<SocketAddr> {
    match &config.master {
        MasterAddr::Auto => discovery::find_master(MDNS_DISCOVERY_TIMEOUT).await,
        MasterAddr::Explicit(s) => {
            let mut addrs = tokio::net::lookup_host(s).await.with_context(|| format!("resolving master address {s:?}"))?;
            addrs.next().ok_or_else(|| anyhow::anyhow!("master address {s:?} resolved to no addresses"))
        }
    }
}

fn local_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "smkb-slave".to_string())
}

async fn handshake(socket: &UdpSocket, psk: &Psk) -> anyhow::Result<smkb_proto::SessionKeys> {
    for attempt in 1..=HANDSHAKE_ATTEMPTS {
        let mut hs = Handshake::new_initiator(psk)?;
        let mut msg1 = [0u8; 256];
        let n1 = hs.write_message(&mut msg1)?;
        socket.send(&msg1[..n1]).await.context("sending handshake message 1")?;

        let mut buf = [0u8; 256];
        match tokio::time::timeout(HANDSHAKE_REPLY_TIMEOUT, socket.recv(&mut buf)).await {
            Ok(Ok(n2)) => {
                if hs.read_message(&buf[..n2]).is_ok() {
                    return Ok(hs.finish()?);
                }
                tracing::debug!(attempt, "handshake reply didn't parse, retrying");
            }
            Ok(Err(e)) => tracing::debug!(attempt, error = %e, "recv error during handshake, retrying"),
            Err(_) => tracing::debug!(attempt, "no handshake reply within timeout, retrying"),
        }
    }
    anyhow::bail!("master did not complete the handshake after {HANDSHAKE_ATTEMPTS} attempts")
}

async fn run_session(master_addr: SocketAddr, psk: &Psk, emitter: &mut dyn InputEmitter, screen: smkb_proto::DisplayGeometry) -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.context("binding local UDP socket")?;
    socket.connect(master_addr).await.with_context(|| format!("connecting UDP socket to {master_addr}"))?;
    tracing::info!(%master_addr, "handshaking");

    let keys = handshake(&socket, psk).await?;
    let mut session = Session::new(keys);
    tracing::info!("handshake complete, sending Hello");

    let hello = Frame::Hello(HelloInfo { proto_version: smkb_proto::PROTO_VERSION, hostname: local_hostname(), screen });
    let pkt = session.send_reliable(&hello, Instant::now());
    socket.send(&pkt).await.context("sending Hello")?;

    let result = service_connection(&socket, &mut session, emitter).await;
    if let Err(e) = emitter.release_all() {
        tracing::warn!(error = %e, "failed to release held keys/buttons after disconnect");
    }
    result
}

async fn service_connection(socket: &UdpSocket, session: &mut Session, emitter: &mut dyn InputEmitter) -> anyhow::Result<()> {
    let mut buf = vec![0u8; MAX_PACKET];
    let mut retransmit = tokio::time::interval(RETRANSMIT_INTERVAL);
    let mut last_activity = Instant::now();

    loop {
        tokio::select! {
            biased;
            _ = retransmit.tick() => {
                for pkt in session.due_retransmits(Instant::now()) {
                    let _ = socket.send(&pkt).await;
                }
            }
            recv = socket.recv(&mut buf) => {
                let n = recv.context("recv from master")?;
                match session.on_datagram(&buf[..n]) {
                    Ok(received) => {
                        last_activity = Instant::now();
                        if let Some(reply) = received.reply {
                            let _ = socket.send(&reply).await;
                        }
                        for frame in received.ready {
                            match frame {
                                Frame::Event(event) => {
                                    if let Err(e) = emitter.emit(&event) {
                                        tracing::warn!(error = %e, "failed to emit an input event");
                                    }
                                }
                                Frame::Ping { nonce } => {
                                    let pkt = session.send_control(&Frame::Pong { nonce });
                                    let _ = socket.send(&pkt).await;
                                }
                                Frame::Welcome(_) => {
                                    tracing::info!("master welcomed us, connection established");
                                }
                                Frame::Bye => {
                                    tracing::info!("master said goodbye");
                                    return Ok(());
                                }
                                Frame::Hello(_) | Frame::Ack { .. } | Frame::Pong { .. } => {}
                            }
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "dropping undecryptable/malformed packet"),
                }
            }
        }
        if Instant::now().duration_since(last_activity) > WATCHDOG_TIMEOUT {
            anyhow::bail!("no traffic from master for {WATCHDOG_TIMEOUT:?}, treating the connection as dead");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = parse_args();
    let config = Config::load(&config_path).with_context(|| format!("loading config from {}", config_path.display()))?;
    let psk = config.load_psk().context("loading psk_file")?;

    let backend = smkb_platform::detect_slave().await.context("detecting a supported compositor")?;
    let mut emitter = backend.emitter;
    let mut screen = backend.screen;
    let geometry = screen.primary().await.context("reading local screen geometry")?;
    tracing::info!(?geometry, "reporting this geometry to the master");

    let run = async {
        loop {
            let master_addr = match resolve_master(&config).await {
                Ok(addr) => addr,
                Err(e) => {
                    tracing::warn!(error = %e, "could not resolve master address, retrying");
                    tokio::time::sleep(Duration::from_millis(config.reconnect.interval_ms)).await;
                    continue;
                }
            };
            if let Err(e) = run_session(master_addr, &psk, emitter.as_mut(), geometry).await {
                tracing::warn!(error = %e, "session ended, reconnecting");
            }
            tokio::time::sleep(Duration::from_millis(config.reconnect.interval_ms)).await;
        }
    };

    tokio::select! {
        _ = run => unreachable!("the reconnect loop never returns"),
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
        }
    }
    let _ = emitter.release_all();
    Ok(())
}
