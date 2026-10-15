use dhcproto::v4::{DhcpOption, Flags, HType, Message, MessageType, Opcode, OptionCode};
use dhcproto::{Decodable, Decoder, Encodable};
use monitor_dhcp::config::{Config, MacAddr};
use monitor_dhcp::dhcp::{NoReply, ResponseKind, process_packet};
use std::net::Ipv4Addr;

const MAC: [u8; 6] = [0, 0x11, 0x22, 0x33, 0x44, 0x55];

fn config() -> Config {
    r#"
interface = "eth0"
server_ip = "192.168.50.1"
subnet_mask = "255.255.255.0"
default_ip = "192.168.50.2"
lease_seconds = 86400
[hosts]
"00:11:22:33:44:55" = "192.168.50.10"
"#
    .parse()
    .unwrap()
}

fn request(message_type: MessageType) -> Message {
    let mut message = Message::default();
    message
        .set_opcode(Opcode::BootRequest)
        .set_htype(HType::Eth)
        .set_xid(0x1234_5678)
        .set_chaddr(&MAC);
    message
        .opts_mut()
        .insert(DhcpOption::MessageType(message_type));
    message
}

fn process(message: &Message) -> Result<monitor_dhcp::dhcp::Reply, NoReply> {
    process_packet(&message.to_vec().unwrap(), &config())
}

fn decoded(reply: &monitor_dhcp::dhcp::Reply) -> Message {
    Message::decode(&mut Decoder::new(&reply.bytes)).unwrap()
}

fn option_u32(message: &Message, code: OptionCode) -> Option<u32> {
    match message.opts().get(code) {
        Some(DhcpOption::AddressLeaseTime(value))
        | Some(DhcpOption::Renewal(value))
        | Some(DhcpOption::Rebinding(value)) => Some(*value),
        _ => None,
    }
}

#[test]
fn discover_produces_complete_offer() {
    let reply = process(&request(MessageType::Discover)).unwrap();
    assert_eq!(reply.kind, ResponseKind::Offer);
    assert_eq!(reply.destination, Ipv4Addr::BROADCAST);
    assert!(reply.bytes.len() >= 300);
    let message = decoded(&reply);
    assert_eq!(message.opcode(), Opcode::BootReply);
    assert_eq!(message.opts().msg_type(), Some(MessageType::Offer));
    assert_eq!(message.yiaddr(), Ipv4Addr::new(192, 168, 50, 10));
    assert!(
        matches!(message.opts().get(OptionCode::ServerIdentifier), Some(DhcpOption::ServerIdentifier(ip)) if *ip == Ipv4Addr::new(192, 168, 50, 1))
    );
    assert!(
        matches!(message.opts().get(OptionCode::SubnetMask), Some(DhcpOption::SubnetMask(ip)) if *ip == Ipv4Addr::new(255, 255, 255, 0))
    );
    assert_eq!(
        option_u32(&message, OptionCode::AddressLeaseTime),
        Some(86_400)
    );
    assert_eq!(option_u32(&message, OptionCode::Renewal), Some(43_200));
    assert_eq!(option_u32(&message, OptionCode::Rebinding), Some(75_600));
}

fn selecting(address: Ipv4Addr, server: Ipv4Addr) -> Message {
    let mut message = request(MessageType::Request);
    message
        .opts_mut()
        .insert(DhcpOption::RequestedIpAddress(address));
    message
        .opts_mut()
        .insert(DhcpOption::ServerIdentifier(server));
    message
}

#[test]
fn selecting_request_acknowledges_only_configured_address() {
    let correct = selecting(
        Ipv4Addr::new(192, 168, 50, 10),
        Ipv4Addr::new(192, 168, 50, 1),
    );
    assert_eq!(process(&correct).unwrap().kind, ResponseKind::Ack);

    let incorrect = selecting(
        Ipv4Addr::new(192, 168, 50, 20),
        Ipv4Addr::new(192, 168, 50, 1),
    );
    assert_eq!(process(&incorrect).unwrap().kind, ResponseKind::Nak);
}

#[test]
fn request_selecting_another_server_is_ignored() {
    let message = selecting(
        Ipv4Addr::new(192, 168, 50, 10),
        Ipv4Addr::new(192, 168, 50, 99),
    );
    assert_eq!(process(&message).unwrap_err(), NoReply::OtherServer);
}

#[test]
fn init_reboot_acknowledges_current_and_naks_obsolete_address() {
    let mut message = request(MessageType::Request);
    message
        .opts_mut()
        .insert(DhcpOption::RequestedIpAddress(Ipv4Addr::new(
            192, 168, 50, 10,
        )));
    assert_eq!(process(&message).unwrap().kind, ResponseKind::Ack);

    message
        .opts_mut()
        .insert(DhcpOption::RequestedIpAddress(Ipv4Addr::new(
            192, 168, 50, 9,
        )));
    assert_eq!(process(&message).unwrap().kind, ResponseKind::Nak);
}

#[test]
fn renewal_and_rebinding_are_acknowledged() {
    let address = Ipv4Addr::new(192, 168, 50, 10);
    let mut renewal = request(MessageType::Request);
    renewal.set_ciaddr(address);
    let renewal_reply = process(&renewal).unwrap();
    assert_eq!(renewal_reply.kind, ResponseKind::Ack);
    assert_eq!(renewal_reply.destination, address);

    let mut rebinding = request(MessageType::Request);
    rebinding
        .set_ciaddr(address)
        .set_flags(Flags::default().set_broadcast());
    let rebinding_reply = process(&rebinding).unwrap();
    assert_eq!(rebinding_reply.kind, ResponseKind::Ack);
}

#[test]
fn invalid_renewal_is_naked() {
    let mut renewal = request(MessageType::Request);
    renewal.set_ciaddr(Ipv4Addr::new(192, 168, 50, 9));
    assert_eq!(process(&renewal).unwrap().kind, ResponseKind::Nak);
}

#[test]
fn release_and_decline_produce_no_response() {
    assert!(matches!(
        process(&request(MessageType::Release)),
        Err(NoReply::Release { .. })
    ));
    assert!(matches!(
        process(&request(MessageType::Decline)),
        Err(NoReply::Decline { .. })
    ));
}

#[test]
fn inform_ack_has_network_parameters_but_no_address_or_lease() {
    let mut inform = request(MessageType::Inform);
    inform.set_ciaddr(Ipv4Addr::new(192, 168, 50, 20));
    let reply = process(&inform).unwrap();
    assert_eq!(reply.kind, ResponseKind::Ack);
    let message = decoded(&reply);
    assert_eq!(message.yiaddr(), Ipv4Addr::UNSPECIFIED);
    assert!(message.opts().get(OptionCode::SubnetMask).is_some());
    assert!(message.opts().get(OptionCode::AddressLeaseTime).is_none());
    assert!(message.opts().get(OptionCode::Renewal).is_none());
    assert!(message.opts().get(OptionCode::Rebinding).is_none());
}

#[test]
fn inform_without_client_address_is_malformed() {
    assert_eq!(
        process(&request(MessageType::Inform)).unwrap_err(),
        NoReply::Malformed
    );
}

#[test]
fn malformed_and_relay_packets_are_ignored() {
    assert_eq!(
        process_packet(&[1, 2, 3], &config()).unwrap_err(),
        NoReply::Malformed
    );
    let mut relay = request(MessageType::Discover);
    relay.set_giaddr(Ipv4Addr::new(192, 168, 50, 254));
    assert_eq!(process(&relay).unwrap_err(), NoReply::Unsupported);
}

#[test]
fn xid_and_mac_are_preserved() {
    let reply = process(&request(MessageType::Discover)).unwrap();
    let message = decoded(&reply);
    assert_eq!(message.xid(), 0x1234_5678);
    assert_eq!(message.chaddr(), MAC);
    assert_eq!(reply.mac, MacAddr::new(MAC));
}

#[test]
fn client_without_broadcast_flag_gets_compatible_broadcast_offer() {
    let message = request(MessageType::Discover);
    assert!(!message.flags().broadcast());
    assert_eq!(process(&message).unwrap().destination, Ipv4Addr::BROADCAST);
}
