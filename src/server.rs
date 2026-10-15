use crate::config::Config;
use crate::dhcp::{NoReply, ResponseKind, process_packet};
use dhcproto::v4::MessageType;
use log::{debug, error, info, warn};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag;
use socket2::{Domain, Protocol, Socket, Type};
use std::ffi::CStr;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SERVER_PORT: u16 = 67;
const CLIENT_PORT: u16 = 68;

pub fn validate_runtime_interface(config: &Config) -> Result<(), String> {
    let type_path = Path::new("/sys/class/net")
        .join(&config.interface)
        .join("type");
    let interface_type = fs::read_to_string(&type_path).map_err(|error| {
        format!(
            "interface '{}' does not exist or cannot be inspected: {error}",
            config.interface
        )
    })?;
    if interface_type.trim() != "1" {
        return Err(format!(
            "interface '{}' is not Ethernet-compatible (ARPHRD type {})",
            config.interface,
            interface_type.trim()
        ));
    }

    let addresses = interface_ipv4_addresses()
        .map_err(|error| format!("cannot inspect interface '{}': {error}", config.interface))?;
    let found = addresses.iter().any(|(name, address, mask)| {
        name == &config.interface && *address == config.server_ip && *mask == config.subnet_mask
    });
    if !found {
        return Err(format!(
            "server_ip {}/{} is not assigned to interface '{}'",
            config.server_ip, config.subnet_mask, config.interface
        ));
    }
    Ok(())
}

pub fn run(config_path: &Path, initial: Config) -> Result<(), String> {
    validate_runtime_interface(&initial)?;
    let socket = open_socket(&initial.interface).map_err(|error| {
        format!(
            "cannot open DHCP socket on '{}': {error}",
            initial.interface
        )
    })?;

    let terminate = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, Arc::clone(&terminate))
        .and_then(|_| flag::register(SIGINT, Arc::clone(&terminate)))
        .and_then(|_| flag::register(SIGHUP, Arc::clone(&reload)))
        .map_err(|error| format!("cannot install signal handlers: {error}"))?;

    info!(
        "DHCP server started on {} ({})",
        initial.interface, initial.server_ip
    );
    let mut config = initial;
    let mut buffer = [0_u8; 2048];
    let mut malformed_count = 0_u64;

    while !terminate.load(Ordering::Relaxed) {
        if reload.swap(false, Ordering::Relaxed) {
            match Config::load(config_path).and_then(|replacement| {
                config.reload_compatible_with(&replacement)?;
                validate_runtime_interface(&replacement)?;
                Ok(replacement)
            }) {
                Ok(replacement) => {
                    config = replacement;
                    info!("Configuration reloaded");
                }
                Err(error) => error!("Configuration reload rejected: {error}"),
            }
        }

        let length = match socket.recv(&mut buffer) {
            Ok(length) => length,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(format!("failed to receive DHCP packet: {error}")),
        };

        match process_packet(&buffer[..length], &config) {
            Ok(reply) => {
                debug!("DHCP {:?} xid={:#010x}", reply.kind, reply.xid);
                match reply.request_type {
                    MessageType::Discover => info!("DISCOVER mac={}", reply.mac),
                    MessageType::Request => info!("REQUEST mac={}", reply.mac),
                    MessageType::Inform => info!("INFORM mac={}", reply.mac),
                    _ => {}
                }
                match reply.kind {
                    ResponseKind::Offer => {
                        info!("OFFER mac={} ip={}", reply.mac, reply.address.unwrap());
                    }
                    ResponseKind::Ack => match reply.address {
                        Some(ip) => info!("ACK mac={} ip={ip}", reply.mac),
                        None => info!("ACK INFORM mac={}", reply.mac),
                    },
                    ResponseKind::Nak => warn!("NAK mac={}", reply.mac),
                }
                let destination = SocketAddrV4::new(reply.destination, CLIENT_PORT);
                if let Err(error) = socket.send_to(&reply.bytes, destination) {
                    error!(
                        "failed to send {:?} to {}: {error}",
                        reply.kind, destination
                    );
                }
            }
            Err(NoReply::Release { mac }) => info!("RELEASE mac={mac}"),
            Err(NoReply::Decline { mac, address }) => match address {
                Some(ip) => warn!("DECLINE mac={mac} ip={ip}"),
                None => warn!("DECLINE mac={mac} ip=unknown"),
            },
            Err(NoReply::Malformed) => {
                malformed_count += 1;
                if malformed_count == 1 || malformed_count.is_multiple_of(100) {
                    debug!("ignored malformed DHCP packet (count={malformed_count})");
                }
            }
            Err(NoReply::Unsupported) => debug!("ignored unsupported DHCP packet"),
            Err(NoReply::OtherServer) => debug!("ignored REQUEST selecting another server"),
        }
    }
    info!("DHCP server stopped");
    Ok(())
}

fn open_socket(interface: &str) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.bind_device(Some(interface.as_bytes()))?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SERVER_PORT).into())?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok(socket.into())
}

fn interface_ipv4_addresses() -> io::Result<Vec<(String, Ipv4Addr, Ipv4Addr)>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs initializes `head` on success. The linked list remains valid until the
    // matching freeifaddrs call below, and all pointers are checked before dereferencing.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut result = Vec::new();
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` points into the list returned by getifaddrs and is valid until freed.
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null()
            && !entry.ifa_netmask.is_null()
            // SAFETY: non-null sockaddr pointers are guaranteed by the checks above.
            && unsafe { (*entry.ifa_addr).sa_family as i32 } == libc::AF_INET
        {
            // SAFETY: AF_INET guarantees sockaddr_in layout; ifa_name is a NUL-terminated C string.
            let name = unsafe { CStr::from_ptr(entry.ifa_name) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: family was verified as AF_INET above.
            let address = unsafe { *(entry.ifa_addr.cast::<libc::sockaddr_in>()) };
            // SAFETY: IPv4 address entries have an IPv4 netmask.
            let mask = unsafe { *(entry.ifa_netmask.cast::<libc::sockaddr_in>()) };
            result.push((
                name,
                Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr)),
                Ipv4Addr::from(u32::from_be(mask.sin_addr.s_addr)),
            ));
        }
        current = entry.ifa_next;
    }
    // SAFETY: `head` is the exact pointer returned by getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(head) };
    Ok(result)
}
