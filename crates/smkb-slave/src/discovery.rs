use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::net::SocketAddr;
use std::time::Duration;

const SERVICE_TYPE: &str = "_smkb._udp.local.";

pub async fn find_master(timeout: Duration) -> anyhow::Result<SocketAddr> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE_TYPE)?;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = daemon.stop_browse(SERVICE_TYPE);
                anyhow::bail!("no smkb-master found via mDNS within {timeout:?} (is one running, and is discovery.mdns enabled on it?)");
            }
            event = receiver.recv_async() => {
                let event = event?;
                if let ServiceEvent::ServiceResolved(resolved) = event
                    && let Some(ip) = resolved.get_addresses_v4().into_iter().next() {
                        let addr = SocketAddr::from((ip, resolved.get_port()));
                        tracing::info!(%addr, fullname = resolved.get_fullname(), "found master via mDNS");
                        let _ = daemon.stop_browse(SERVICE_TYPE);
                        return Ok(addr);
                    }
            }
        }
    }
}
