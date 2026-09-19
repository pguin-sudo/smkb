use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};

const SERVICE_TYPE: &str = "_smkb._udp.local.";

pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    pub fn start(port: u16) -> anyhow::Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let ip = local_ipv4()?;
        let hostname = format!("{}.local.", local_hostname());
        let instance = local_hostname();
        let service = ServiceInfo::new(SERVICE_TYPE, &instance, &hostname, std::net::IpAddr::V4(ip), port, HashMap::<String, String>::new())?;
        let fullname = service.get_fullname().to_string();
        daemon.register(service)?;
        tracing::info!(%fullname, %ip, port, "advertising via mDNS");
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

fn local_ipv4() -> anyhow::Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    match socket.local_addr()?.ip() {
        std::net::IpAddr::V4(v4) => Ok(v4),
        std::net::IpAddr::V6(_) => anyhow::bail!("no local IPv4 route found"),
    }
}

fn local_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "smkb-master".to_string())
}
