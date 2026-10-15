use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::str::FromStr;

pub const DEFAULT_LEASE_SECONDS: u32 = 86_400;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 6] {
        self.0
    }
}

impl FromStr for MacAddr {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split(':').collect();
        if parts.len() != 6 || parts.iter().any(|part| part.len() != 2) {
            return Err(format!(
                "invalid MAC address '{value}': expected six colon-separated bytes"
            ));
        }
        let mut bytes = [0; 6];
        for (index, part) in parts.into_iter().enumerate() {
            bytes[index] = u8::from_str_radix(part, 16).map_err(|_| {
                format!("invalid MAC address '{value}': '{part}' is not hexadecimal")
            })?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    interface: String,
    server_ip: Ipv4Addr,
    subnet_mask: Ipv4Addr,
    default_ip: Ipv4Addr,
    #[serde(default = "default_lease")]
    lease_seconds: u64,
    #[serde(default)]
    hosts: HashMap<String, Ipv4Addr>,
}

const fn default_lease() -> u64 {
    DEFAULT_LEASE_SECONDS as u64
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub interface: String,
    pub server_ip: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub default_ip: Ipv4Addr,
    pub lease_seconds: u32,
    hosts: HashMap<MacAddr, Ipv4Addr>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let input = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        input
            .parse()
            .map_err(|error: String| format!("{}: {error}", path.display()))
    }

    fn validate(raw: RawConfig) -> Result<Self, String> {
        validate_interface_name(&raw.interface)?;
        let mask = u32::from(raw.subnet_mask);
        if !is_contiguous_mask(mask) {
            return Err(format!(
                "subnet_mask {} is not a valid contiguous IPv4 mask",
                raw.subnet_mask
            ));
        }
        validate_server_ip(raw.server_ip, mask)?;
        validate_client_ip("default_ip", raw.default_ip, raw.server_ip, mask)?;

        let lease_seconds = u32::try_from(raw.lease_seconds)
            .map_err(|_| "lease_seconds is larger than the DHCP 32-bit interval".to_string())?;
        if lease_seconds < 8 {
            return Err("lease_seconds must be at least 8 so T1 and T2 are ordered".to_string());
        }

        let mut hosts = HashMap::with_capacity(raw.hosts.len());
        let mut assigned = HashSet::with_capacity(raw.hosts.len());
        for (text, ip) in raw.hosts {
            let mac = text.parse::<MacAddr>()?;
            if hosts.contains_key(&mac) {
                return Err(format!("duplicate MAC address after normalization: {text}"));
            }
            validate_client_ip(&format!("address for {mac}"), ip, raw.server_ip, mask)?;
            if !assigned.insert(ip) {
                return Err(format!("duplicate explicit IP assignment: {ip}"));
            }
            hosts.insert(mac, ip);
        }

        Ok(Self {
            interface: raw.interface,
            server_ip: raw.server_ip,
            subnet_mask: raw.subnet_mask,
            default_ip: raw.default_ip,
            lease_seconds,
            hosts,
        })
    }

    pub fn assigned_ip(&self, mac: MacAddr) -> Ipv4Addr {
        self.hosts.get(&mac).copied().unwrap_or(self.default_ip)
    }

    pub const fn renewal_seconds(&self) -> u32 {
        self.lease_seconds / 2
    }

    pub const fn rebinding_seconds(&self) -> u32 {
        (self.lease_seconds as u64 * 7 / 8) as u32
    }

    pub fn reload_compatible_with(&self, replacement: &Self) -> Result<(), String> {
        if replacement.interface != self.interface {
            return Err("interface changes require a service restart".to_string());
        }
        if replacement.server_ip != self.server_ip {
            return Err("server_ip changes require a service restart".to_string());
        }
        Ok(())
    }
}

impl FromStr for Config {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let raw: RawConfig =
            toml::from_str(input).map_err(|error| format!("invalid TOML: {error}"))?;
        Self::validate(raw)
    }
}

fn validate_interface_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > libc::IFNAMSIZ - 1 {
        return Err(format!("invalid Linux interface name '{name}'"));
    }
    if name
        .bytes()
        .any(|byte| byte == 0 || byte == b'/' || byte.is_ascii_whitespace())
    {
        return Err(format!("invalid Linux interface name '{name}'"));
    }
    Ok(())
}

fn is_contiguous_mask(mask: u32) -> bool {
    let inverse = !mask;
    inverse & inverse.wrapping_add(1) == 0
}

fn validate_client_ip(
    label: &str,
    ip: Ipv4Addr,
    server: Ipv4Addr,
    mask: u32,
) -> Result<(), String> {
    if ip == server {
        return Err(format!("{label} must not equal server_ip ({server})"));
    }
    let address = u32::from(ip);
    let server_address = u32::from(server);
    if address & mask != server_address & mask {
        return Err(format!("{label} ({ip}) is outside the configured subnet"));
    }
    let network = server_address & mask;
    let broadcast = network | !mask;
    if address == network || address == broadcast || ip.is_unspecified() || ip.is_multicast() {
        return Err(format!("{label} ({ip}) is not a usable unicast address"));
    }
    Ok(())
}

fn validate_server_ip(ip: Ipv4Addr, mask: u32) -> Result<(), String> {
    let address = u32::from(ip);
    let network = address & mask;
    let broadcast = network | !mask;
    if address == network
        || address == broadcast
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip == Ipv4Addr::BROADCAST
    {
        return Err(format!(
            "server_ip ({ip}) is not a usable unicast address for subnet_mask {}",
            Ipv4Addr::from(mask)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
interface = "eth0"
server_ip = "192.168.50.1"
subnet_mask = "255.255.255.0"
default_ip = "192.168.50.2"
lease_seconds = 86400
[hosts]
"00:11:22:33:44:55" = "192.168.50.10"
"AA:BB:CC:DD:EE:FF" = "192.168.50.11"
"#;

    #[test]
    fn parses_valid_configuration_and_selects_addresses() {
        let config = Config::from_str(VALID).unwrap();
        let known = "00:11:22:33:44:55".parse().unwrap();
        let upper = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let lower = "aa:bb:cc:dd:ee:ff".parse().unwrap();
        let unknown = "12:34:56:78:9a:bc".parse().unwrap();
        assert_eq!(config.assigned_ip(known), Ipv4Addr::new(192, 168, 50, 10));
        assert_eq!(config.assigned_ip(upper), config.assigned_ip(lower));
        assert_eq!(config.assigned_ip(unknown), Ipv4Addr::new(192, 168, 50, 2));
        assert_eq!(config.assigned_ip(unknown), config.assigned_ip(unknown));
    }

    #[test]
    fn required_fields_are_required() {
        assert!(
            Config::from_str("interface = \"eth0\"")
                .unwrap_err()
                .contains("missing field")
        );
    }

    #[test]
    fn rejects_invalid_mac() {
        assert!(
            Config::from_str(&VALID.replace("00:11:22:33:44:55", "not-a-mac"))
                .unwrap_err()
                .contains("invalid MAC")
        );
    }

    #[test]
    fn rejects_duplicate_normalized_mac() {
        let input = format!("{VALID}\n\"aa:bb:cc:dd:ee:ff\" = \"192.168.50.12\"\n");
        assert!(
            Config::from_str(&input)
                .unwrap_err()
                .contains("duplicate MAC")
        );
    }

    #[test]
    fn rejects_duplicate_explicit_ip() {
        let input = VALID.replace("192.168.50.11", "192.168.50.10");
        assert!(
            Config::from_str(&input)
                .unwrap_err()
                .contains("duplicate explicit IP")
        );
    }

    #[test]
    fn rejects_ip_outside_subnet_or_equal_to_server() {
        assert!(
            Config::from_str(&VALID.replace("192.168.50.2", "192.168.51.2"))
                .unwrap_err()
                .contains("outside")
        );
        assert!(
            Config::from_str(&VALID.replace(
                "default_ip = \"192.168.50.2\"",
                "default_ip = \"192.168.50.1\""
            ))
            .unwrap_err()
            .contains("must not equal")
        );
    }

    #[test]
    fn rejects_invalid_mask_and_lease() {
        assert!(
            Config::from_str(&VALID.replace("255.255.255.0", "255.0.255.0"))
                .unwrap_err()
                .contains("contiguous")
        );
        assert!(
            Config::from_str(&VALID.replace("86400", "0"))
                .unwrap_err()
                .contains("at least 8")
        );
        assert!(
            Config::from_str(&VALID.replace("86400", "4294967296"))
                .unwrap_err()
                .contains("32-bit")
        );
    }

    #[test]
    fn handles_maximum_lease_without_timer_overflow() {
        let config = Config::from_str(&VALID.replace("86400", "4294967295")).unwrap();
        assert_eq!(config.renewal_seconds(), 2_147_483_647);
        assert_eq!(config.rebinding_seconds(), 3_758_096_383);
    }

    #[test]
    fn reserved_ip_may_equal_default_ip() {
        let input = VALID.replace("192.168.50.10", "192.168.50.2");
        assert!(Config::from_str(&input).is_ok());
    }

    #[test]
    fn default_lease_is_one_day() {
        let config = Config::from_str(&VALID.replace("lease_seconds = 86400\n", "")).unwrap();
        assert_eq!(config.lease_seconds, DEFAULT_LEASE_SECONDS);
    }

    #[test]
    fn rejects_network_broadcast_and_invalid_server_addresses() {
        assert!(
            Config::from_str(&VALID.replace(
                "default_ip = \"192.168.50.2\"",
                "default_ip = \"192.168.50.0\""
            ))
            .unwrap_err()
            .contains("not a usable")
        );
        assert!(
            Config::from_str(&VALID.replace(
                "default_ip = \"192.168.50.2\"",
                "default_ip = \"192.168.50.255\""
            ))
            .unwrap_err()
            .contains("not a usable")
        );
        assert!(
            Config::from_str(&VALID.replace(
                "server_ip = \"192.168.50.1\"",
                "server_ip = \"192.168.50.0\""
            ))
            .unwrap_err()
            .contains("server_ip")
        );
    }

    #[test]
    fn reload_rejects_interface_and_server_changes() {
        let original = Config::from_str(VALID).unwrap();
        let changed_interface = Config::from_str(&VALID.replace("eth0", "eth1")).unwrap();
        let changed_server = Config::from_str(
            &VALID
                .replace("192.168.50.1", "192.168.50.3")
                .replace("192.168.50.10", "192.168.50.12"),
        )
        .unwrap();
        assert!(original.reload_compatible_with(&changed_interface).is_err());
        assert!(original.reload_compatible_with(&changed_server).is_err());
    }

    #[test]
    fn rejects_unsafe_interface_names() {
        assert!(Config::from_str(&VALID.replace("eth0", "../eth0")).is_err());
        assert!(Config::from_str(&VALID.replace("eth0", "interface-name-is-too-long")).is_err());
    }
}
