use crate::config::{Config, MacAddr};
use dhcproto::v4::{DhcpOption, HType, Message, MessageType, Opcode, OptionCode};
use dhcproto::{Decodable, Decoder, Encodable};
use std::net::Ipv4Addr;

const MIN_DHCP_PACKET_SIZE: usize = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestState {
    Selecting,
    InitReboot,
    Renewing,
    Rebinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseKind {
    Offer,
    Ack,
    Nak,
}

#[derive(Debug)]
pub struct Reply {
    pub bytes: Vec<u8>,
    pub kind: ResponseKind,
    pub mac: MacAddr,
    pub address: Option<Ipv4Addr>,
    pub destination: Ipv4Addr,
    pub xid: u32,
    pub request_type: MessageType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoReply {
    Malformed,
    Unsupported,
    OtherServer,
    Release {
        mac: MacAddr,
    },
    Decline {
        mac: MacAddr,
        address: Option<Ipv4Addr>,
    },
}

pub fn process_packet(bytes: &[u8], config: &Config) -> Result<Reply, NoReply> {
    let request = Message::decode(&mut Decoder::new(bytes)).map_err(|_| NoReply::Malformed)?;
    validate_request(&request)?;
    let mac = mac_from_message(&request)?;
    let assigned = config.assigned_ip(mac);
    let requested = option_ip(&request, OptionCode::RequestedIpAddress);

    match request.opts().msg_type().ok_or(NoReply::Unsupported)? {
        MessageType::Discover => response(
            &request,
            config,
            mac,
            MessageType::Offer,
            Some(assigned),
            true,
            delivery_destination(&request),
        ),
        MessageType::Request => handle_request(&request, config, mac, assigned, requested),
        MessageType::Release => Err(NoReply::Release { mac }),
        MessageType::Decline => Err(NoReply::Decline {
            mac,
            address: requested,
        }),
        MessageType::Inform if !request.ciaddr().is_unspecified() => response(
            &request,
            config,
            mac,
            MessageType::Ack,
            None,
            false,
            delivery_destination(&request),
        ),
        MessageType::Inform => Err(NoReply::Malformed),
        _ => Err(NoReply::Unsupported),
    }
}

fn validate_request(request: &Message) -> Result<(), NoReply> {
    if request.opcode() != Opcode::BootRequest
        || request.htype() != HType::Eth
        || request.hlen() != 6
        || !request.giaddr().is_unspecified()
        || request.hops() != 0
    {
        return Err(NoReply::Unsupported);
    }
    Ok(())
}

fn mac_from_message(request: &Message) -> Result<MacAddr, NoReply> {
    let bytes: [u8; 6] = request
        .chaddr()
        .get(..6)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(NoReply::Malformed)?;
    Ok(MacAddr::new(bytes))
}

fn handle_request(
    request: &Message,
    config: &Config,
    mac: MacAddr,
    assigned: Ipv4Addr,
    requested: Option<Ipv4Addr>,
) -> Result<Reply, NoReply> {
    let server_identifier = option_ip(request, OptionCode::ServerIdentifier);
    let (state, candidate) = classify_request(request, requested, server_identifier)?;

    if state == RequestState::Selecting && server_identifier != Some(config.server_ip) {
        return Err(NoReply::OtherServer);
    }

    if candidate == assigned {
        response(
            request,
            config,
            mac,
            MessageType::Ack,
            Some(assigned),
            true,
            delivery_destination(request),
        )
    } else {
        response(
            request,
            config,
            mac,
            MessageType::Nak,
            None,
            false,
            Ipv4Addr::BROADCAST,
        )
    }
}

fn classify_request(
    request: &Message,
    requested: Option<Ipv4Addr>,
    server_identifier: Option<Ipv4Addr>,
) -> Result<(RequestState, Ipv4Addr), NoReply> {
    if server_identifier.is_some() {
        let candidate = requested.ok_or(NoReply::Malformed)?;
        if !request.ciaddr().is_unspecified() {
            return Err(NoReply::Malformed);
        }
        return Ok((RequestState::Selecting, candidate));
    }
    if let Some(candidate) = requested {
        if !request.ciaddr().is_unspecified() {
            return Err(NoReply::Malformed);
        }
        return Ok((RequestState::InitReboot, candidate));
    }
    if request.ciaddr().is_unspecified() {
        return Err(NoReply::Malformed);
    }
    let state = if request.flags().broadcast() {
        RequestState::Rebinding
    } else {
        RequestState::Renewing
    };
    Ok((state, request.ciaddr()))
}

fn option_ip(request: &Message, code: OptionCode) -> Option<Ipv4Addr> {
    match request.opts().get(code) {
        Some(DhcpOption::RequestedIpAddress(ip)) | Some(DhcpOption::ServerIdentifier(ip)) => {
            Some(*ip)
        }
        _ => None,
    }
}

fn response(
    request: &Message,
    config: &Config,
    mac: MacAddr,
    message_type: MessageType,
    address: Option<Ipv4Addr>,
    include_lease: bool,
    destination: Ipv4Addr,
) -> Result<Reply, NoReply> {
    let mut reply = Message::default();
    reply
        .set_opcode(Opcode::BootReply)
        .set_htype(HType::Eth)
        .set_xid(request.xid())
        .set_flags(request.flags())
        .set_chaddr(&mac.bytes())
        .set_siaddr(config.server_ip);
    if let Some(ip) = address {
        reply.set_yiaddr(ip);
    }
    reply
        .opts_mut()
        .insert(DhcpOption::MessageType(message_type));
    reply
        .opts_mut()
        .insert(DhcpOption::ServerIdentifier(config.server_ip));
    if message_type != MessageType::Nak {
        reply
            .opts_mut()
            .insert(DhcpOption::SubnetMask(config.subnet_mask));
    }
    if include_lease {
        reply
            .opts_mut()
            .insert(DhcpOption::AddressLeaseTime(config.lease_seconds));
        reply
            .opts_mut()
            .insert(DhcpOption::Renewal(config.renewal_seconds()));
        reply
            .opts_mut()
            .insert(DhcpOption::Rebinding(config.rebinding_seconds()));
    }

    let mut bytes = reply.to_vec().map_err(|_| NoReply::Malformed)?;
    bytes.resize(bytes.len().max(MIN_DHCP_PACKET_SIZE), 0);
    let kind = match message_type {
        MessageType::Offer => ResponseKind::Offer,
        MessageType::Ack => ResponseKind::Ack,
        MessageType::Nak => ResponseKind::Nak,
        _ => return Err(NoReply::Unsupported),
    };
    Ok(Reply {
        bytes,
        kind,
        mac,
        address,
        destination,
        xid: request.xid(),
        request_type: request.opts().msg_type().ok_or(NoReply::Malformed)?,
    })
}

fn delivery_destination(request: &Message) -> Ipv4Addr {
    if request.ciaddr().is_unspecified() {
        Ipv4Addr::BROADCAST
    } else {
        request.ciaddr()
    }
}
