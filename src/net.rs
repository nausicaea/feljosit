//! Client addresses: trusted reverse proxies, `X-Forwarded-For` and the client key.
//!
//! Behind a reverse proxy, all connections come from the address of the proxy. The
//! proxy writes the address of the client in the header `X-Forwarded-For`. A client
//! can also send this header. Thus Feljosit reads the header only from a trusted
//! proxy (`FELJOSIT_TRUSTED_PROXIES`, ADR-0024).

use std::net::IpAddr;

/// An IP network, for example `172.30.87.0/24` or `fd00::/8`. One address is a
/// network with the full prefix length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    address: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// Parse `ADDRESS` or `ADDRESS/PREFIX`.
    pub fn parse(text: &str) -> Option<IpNet> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address.trim().parse().ok()?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok()?,
            Some(_) => return None,
            None => max,
        };
        (prefix <= max).then_some(IpNet { address: canonical(address), prefix })
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, canonical(address)) {
            (IpAddr::V4(net), IpAddr::V4(addr)) => {
                masked(u128::from(net.to_bits()), self.prefix, 32)
                    == masked(u128::from(addr.to_bits()), self.prefix, 32)
            }
            (IpAddr::V6(net), IpAddr::V6(addr)) => {
                masked(net.to_bits(), self.prefix, 128) == masked(addr.to_bits(), self.prefix, 128)
            }
            _ => false,
        }
    }
}

/// Keep the first `prefix` bits of a number with `width` bits.
fn masked(bits: u128, prefix: u8, width: u8) -> u128 {
    if prefix == 0 { 0 } else { bits >> (width - prefix) }
}

/// An IPv4-mapped IPv6 address (`::ffff:192.0.2.1`) is the IPv4 address.
fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        v4 => v4,
    }
}

/// Parse a list of networks, separated by commas.
pub fn parse_networks(text: &str) -> Result<Vec<IpNet>, String> {
    text.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| IpNet::parse(entry).ok_or_else(|| format!("{entry:?} is not an IP address or network.")))
        .collect()
}

fn trusted(networks: &[IpNet], address: IpAddr) -> bool {
    networks.iter().any(|net| net.contains(address))
}

/// The address of the client.
///
/// If the connection comes from a trusted proxy, the function reads the
/// `X-Forwarded-For` values from the right. It ignores the addresses of trusted
/// proxies and returns the first other address. The values to the left of that
/// address come from the client and are not trusted.
pub fn client_address(peer: Option<IpAddr>, forwarded_for: &[&str], proxies: &[IpNet]) -> Option<IpAddr> {
    let peer = canonical(peer?);
    if !trusted(proxies, peer) {
        return Some(peer);
    }
    let mut client = peer;
    for value in forwarded_for.iter().rev().flat_map(|header| header.rsplit(',')) {
        let Some(address) = parse_forwarded_address(value.trim()) else {
            // Not an address: the values to the left are not reliable.
            break;
        };
        client = canonical(address);
        if !trusted(proxies, client) {
            break;
        }
    }
    Some(client)
}

/// An address of `X-Forwarded-For`. Some proxies add a port (`192.0.2.1:4711`,
/// `[2001:db8::1]:4711`).
fn parse_forwarded_address(value: &str) -> Option<IpAddr> {
    if let Ok(address) = value.parse() {
        return Some(address);
    }
    if let Some(rest) = value.strip_prefix('[') {
        return rest.split(']').next()?.parse().ok();
    }
    value.rsplit_once(':').and_then(|(address, _)| address.parse::<std::net::Ipv4Addr>().ok()).map(IpAddr::V4)
}

/// The key of a client for the login limits and the connection limits: the IPv4
/// address, or the /64 network of an IPv6 address. A device can easily use many
/// addresses of its IPv6 network.
pub fn client_key(address: Option<IpAddr>) -> String {
    match address.map(canonical) {
        Some(IpAddr::V4(v4)) => v4.to_string(),
        Some(IpAddr::V6(v6)) => {
            let network = std::net::Ipv6Addr::from_bits(v6.to_bits() & !((1u128 << 64) - 1));
            format!("{network}/64")
        }
        None => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn networks() {
        let net = IpNet::parse("172.30.87.0/24").unwrap();
        assert!(net.contains(ip("172.30.87.1")));
        assert!(net.contains(ip("::ffff:172.30.87.200")));
        assert!(!net.contains(ip("172.30.88.1")));
        assert!(IpNet::parse("10.0.0.1").unwrap().contains(ip("10.0.0.1")));
        assert!(!IpNet::parse("10.0.0.1").unwrap().contains(ip("10.0.0.2")));
        assert!(IpNet::parse("0.0.0.0/0").unwrap().contains(ip("192.0.2.1")));
        assert!(IpNet::parse("fd00::/8").unwrap().contains(ip("fd12::1")));
        assert!(!IpNet::parse("fd00::/8").unwrap().contains(ip("10.0.0.1")));
        for wrong in ["10.0.0.1/33", "::/129", "10.0.0.1/", "10.0.0.1/x", "host", "10.0.0.1/+8"] {
            assert_eq!(IpNet::parse(wrong), None, "{wrong}");
        }
        assert_eq!(parse_networks(" 10.0.0.1, ,fd00::/8 ").unwrap().len(), 2);
        assert!(parse_networks("10.0.0.1,nonsense").is_err());
    }

    #[test]
    fn forwarded_for_only_from_trusted_proxies() {
        let proxies = parse_networks("172.30.87.1").unwrap();
        let proxy = Some(ip("172.30.87.1"));
        // A direct client cannot choose its address with the header.
        assert_eq!(
            client_address(Some(ip("192.168.1.20")), &["10.9.9.9"], &proxies),
            Some(ip("192.168.1.20"))
        );
        assert_eq!(client_address(proxy, &["192.168.1.20"], &proxies), Some(ip("192.168.1.20")));
        // The client sends a false value. The proxy appends the real address.
        assert_eq!(client_address(proxy, &["10.9.9.9, 192.168.1.20"], &proxies), Some(ip("192.168.1.20")));
        assert_eq!(client_address(proxy, &["10.9.9.9", "192.168.1.20"], &proxies), Some(ip("192.168.1.20")));
        // Without the header, the proxy is the client.
        assert_eq!(client_address(proxy, &[], &proxies), proxy);
        assert_eq!(client_address(proxy, &["garbage"], &proxies), proxy);
        assert_eq!(client_address(proxy, &["192.168.1.20:5000"], &proxies), Some(ip("192.168.1.20")));
        assert_eq!(client_address(proxy, &["[2001:db8::7]:5000"], &proxies), Some(ip("2001:db8::7")));
        assert_eq!(client_address(None, &[], &proxies), None);
    }

    #[test]
    fn chain_of_trusted_proxies() {
        let proxies = parse_networks("172.30.87.1,10.0.0.0/8").unwrap();
        let header = ["198.51.100.1, 192.168.1.20, 10.1.1.1"];
        assert_eq!(client_address(Some(ip("172.30.87.1")), &header, &proxies), Some(ip("192.168.1.20")));
    }

    #[test]
    fn client_keys() {
        assert_eq!(client_key(Some(ip("192.168.1.20"))), "192.168.1.20");
        assert_eq!(client_key(Some(ip("::ffff:192.168.1.20"))), "192.168.1.20");
        assert_eq!(client_key(Some(ip("2001:db8:1:2:aaaa::1"))), "2001:db8:1:2::/64");
        assert_eq!(client_key(Some(ip("2001:db8:1:2:bbbb::9"))), "2001:db8:1:2::/64");
        assert_eq!(client_key(None), "unknown");
    }
}
