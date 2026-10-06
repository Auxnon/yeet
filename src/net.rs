//! Guessing the local network so a new destination can be prefilled
//! (e.g. `192.168.1.` on a 192.168.1.0/24 LAN).

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use if_addrs::IfAddr;

pub struct LocalNet {
    pub ip: Ipv4Addr,
    pub prefixlen: u8,
}

impl LocalNet {
    /// The fixed leading octets of the network, e.g. `192.168.1.` for a /24.
    pub fn prefill(&self) -> String {
        let octets = (self.prefixlen / 8).min(3) as usize;
        if octets == 0 {
            return String::new();
        }
        let parts: Vec<String> = self.ip.octets()[..octets]
            .iter()
            .map(u8::to_string)
            .collect();
        format!("{}.", parts.join("."))
    }
}

impl std::fmt::Display for LocalNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.ip, self.prefixlen)
    }
}

/// The IPv4 network this machine reaches the LAN on.
pub fn detect() -> Option<LocalNet> {
    let ifaces: Vec<(String, Ipv4Addr, u8)> = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter_map(|i| match i.addr {
            IfAddr::V4(v4) if !v4.ip.is_loopback() => Some((i.name, v4.ip, v4.prefixlen)),
            _ => None,
        })
        .collect();

    // Prefer the interface holding the default route's source address.
    // Connecting a UDP socket sends nothing; it just asks the kernel to route.
    let routed = default_route_ip();
    let pick = ifaces
        .iter()
        .find(|(_, ip, _)| Some(*ip) == routed)
        .or_else(|| {
            ifaces
                .iter()
                .find(|(name, ip, _)| ip.is_private() && !is_virtual(name))
        })
        .or_else(|| ifaces.iter().find(|(name, _, _)| !is_virtual(name)))?;
    Some(LocalNet {
        ip: pick.1,
        prefixlen: pick.2,
    })
}

fn default_route_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Container/VM bridges and tunnels that aren't the LAN.
fn is_virtual(name: &str) -> bool {
    [
        "docker",
        "br-",
        "veth",
        "virbr",
        "vmnet",
        "vboxnet",
        "podman",
        "cni",
        "flannel",
        "tun",
        "tap",
        "wg",
        "tailscale",
        "zt",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ip: [u8; 4], prefixlen: u8) -> LocalNet {
        LocalNet {
            ip: Ipv4Addr::from(ip),
            prefixlen,
        }
    }

    #[test]
    fn prefill_follows_mask() {
        assert_eq!(net([192, 168, 1, 5], 24).prefill(), "192.168.1.");
        assert_eq!(net([10, 0, 3, 7], 16).prefill(), "10.0.");
        assert_eq!(net([172, 20, 9, 1], 20).prefill(), "172.20.");
        assert_eq!(net([10, 1, 2, 3], 8).prefill(), "10.");
        assert_eq!(net([192, 168, 1, 5], 30).prefill(), "192.168.1.");
        assert_eq!(net([1, 2, 3, 4], 0).prefill(), "");
    }

    #[test]
    fn virtual_ifaces() {
        assert!(is_virtual("docker0"));
        assert!(is_virtual("br-0d78826ce292"));
        assert!(!is_virtual("wlo1"));
        assert!(!is_virtual("eth0"));
    }
}
