#![allow(dead_code)]

use ipnet::IpNet;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Default)]
pub struct ClientHardware {
    pub adapters_hash: String,
    pub uninstall_id: String,
    pub disk_signature: String,
}

/// Parses the client_hashes parameter from osu! login info:
/// Standard osu! client format: "running_path_md5:adapters_str:adapters_md5:uninstall_id:disk_signature_md5:"
/// Legacy / fallback format: "adapters_hash:uninstall_id:disk_signature"
pub fn parse_client_hashes(raw_hashes: &str) -> ClientHardware {
    let parts: Vec<&str> = raw_hashes.split(':').collect();
    if parts.len() >= 5 {
        ClientHardware {
            adapters_hash: parts[2].to_string(),
            uninstall_id: parts[3].to_string(),
            disk_signature: parts[4].to_string(),
        }
    } else {
        ClientHardware {
            adapters_hash: parts.first().unwrap_or(&"").to_string(),
            uninstall_id: parts.get(1).unwrap_or(&"").to_string(),
            disk_signature: parts.get(2).unwrap_or(&"").to_string(),
        }
    }
}

/// Checks if an IP address is a private, loopback, or LAN address
pub fn is_private_or_loopback_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
                || ipv4.octets()[0] == 0
        }
        IpAddr::V6(ipv6) => ipv6.is_loopback() || (ipv6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

// Common major cloud/datacenter/VPN CIDRs for anti-VPN filter
const KNOWN_DATACENTER_CIDRS: &[&str] = &[
    // Cloudflare WARP / 1.1.1.1
    "104.28.0.0/16",
    "162.158.0.0/15",
    "172.64.0.0/13",
    // AWS EC2 ranges (sample major blocks)
    "3.0.0.0/9",
    "13.32.0.0/15",
    "18.184.0.0/15",
    "52.0.0.0/11",
    "54.0.0.0/12",
    // DigitalOcean
    "104.131.0.0/16",
    "138.68.0.0/16",
    "159.203.0.0/16",
    "167.99.0.0/16",
    "188.166.0.0/16",
    // Hetzner
    "88.198.0.0/16",
    "95.216.0.0/16",
    "135.181.0.0/16",
    "168.119.0.0/16",
    // Linode / Akamai
    "45.33.0.0/16",
    "45.56.0.0/16",
    "172.104.0.0/15",
    // OVH
    "51.254.0.0/15",
    "149.202.0.0/16",
    "178.32.0.0/15",
    // Vultr
    "45.32.0.0/16",
    "45.76.0.0/16",
    "149.28.0.0/16",
];

/// Checks if an IP address is a known datacenter, proxy, or VPN IP
pub fn is_known_vpn_or_datacenter(ip: &IpAddr) -> bool {
    // If it's private/LAN/localhost, it is NOT considered an unauthorized VPN
    if is_private_or_loopback_ip(ip) {
        return false;
    }

    for cidr_str in KNOWN_DATACENTER_CIDRS {
        if let Ok(cidr) = IpNet::from_str(cidr_str) {
            if cidr.contains(ip) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_private_ip() {
        let localhost: IpAddr = "127.0.0.1".parse().unwrap();
        let lan: IpAddr = "192.168.1.10".parse().unwrap();
        let lan2: IpAddr = "10.0.0.5".parse().unwrap();

        assert!(is_private_or_loopback_ip(&localhost));
        assert!(is_private_or_loopback_ip(&lan));
        assert!(is_private_or_loopback_ip(&lan2));
        assert!(!is_known_vpn_or_datacenter(&localhost));
        assert!(!is_known_vpn_or_datacenter(&lan));
    }

    #[test]
    fn test_datacenter_vpn_detection() {
        let aws_ip: IpAddr = "54.10.20.30".parse().unwrap();
        let cf_warp: IpAddr = "104.28.10.20".parse().unwrap();

        assert!(is_known_vpn_or_datacenter(&aws_ip));
        assert!(is_known_vpn_or_datacenter(&cf_warp));
    }

    #[test]
    fn test_client_hashes_parsing() {
        // Standard 5-part format from osu! client: pathMD5:adaptersStr:adaptersMD5:uninstallID:diskMD5:
        let osu_hashes = "d41d8cd98f00b204e9800998ecf8427e:eth0.wlan0:adapter_hash_123:uninstall_guid_456:disk_sig_789:";
        let hw = parse_client_hashes(osu_hashes);
        assert_eq!(hw.adapters_hash, "adapter_hash_123");
        assert_eq!(hw.uninstall_id, "uninstall_guid_456");
        assert_eq!(hw.disk_signature, "disk_sig_789");

        // Fallback 3-part format
        let fallback_hashes = "adapter_hash_123:uninstall_guid_456:disk_sig_789";
        let hw2 = parse_client_hashes(fallback_hashes);
        assert_eq!(hw2.adapters_hash, "adapter_hash_123");
        assert_eq!(hw2.uninstall_id, "uninstall_guid_456");
        assert_eq!(hw2.disk_signature, "disk_sig_789");
    }
}
