use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Error(&'static str);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Error {}

pub struct Decoder<'a> {
    bytes: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

pub trait Decodable: Sized {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, Error>;
}

pub trait Encodable {
    fn to_vec(&self) -> Result<Vec<u8>, Error>;
}

pub mod v4 {
    use super::{Decodable, Decoder, Encodable, Error};
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;

    const MAGIC: [u8; 4] = [99, 130, 83, 99];

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Opcode {
        BootRequest,
        BootReply,
        Unknown(u8),
    }

    impl From<u8> for Opcode {
        fn from(value: u8) -> Self {
            match value {
                1 => Self::BootRequest,
                2 => Self::BootReply,
                other => Self::Unknown(other),
            }
        }
    }

    impl From<Opcode> for u8 {
        fn from(value: Opcode) -> Self {
            match value {
                Opcode::BootRequest => 1,
                Opcode::BootReply => 2,
                Opcode::Unknown(value) => value,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum HType {
        Eth,
        Unknown(u8),
    }

    impl From<u8> for HType {
        fn from(value: u8) -> Self {
            if value == 1 {
                Self::Eth
            } else {
                Self::Unknown(value)
            }
        }
    }

    impl From<HType> for u8 {
        fn from(value: HType) -> Self {
            match value {
                HType::Eth => 1,
                HType::Unknown(value) => value,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct Flags(u16);

    impl Flags {
        pub const fn new(value: u16) -> Self {
            Self(value)
        }
        pub const fn broadcast(self) -> bool {
            self.0 & 0x8000 != 0
        }
        pub const fn set_broadcast(self) -> Self {
            Self(self.0 | 0x8000)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum MessageType {
        Discover,
        Offer,
        Request,
        Decline,
        Ack,
        Nak,
        Release,
        Inform,
        Unknown(u8),
    }

    impl From<u8> for MessageType {
        fn from(value: u8) -> Self {
            match value {
                1 => Self::Discover,
                2 => Self::Offer,
                3 => Self::Request,
                4 => Self::Decline,
                5 => Self::Ack,
                6 => Self::Nak,
                7 => Self::Release,
                8 => Self::Inform,
                other => Self::Unknown(other),
            }
        }
    }

    impl From<MessageType> for u8 {
        fn from(value: MessageType) -> Self {
            match value {
                MessageType::Discover => 1,
                MessageType::Offer => 2,
                MessageType::Request => 3,
                MessageType::Decline => 4,
                MessageType::Ack => 5,
                MessageType::Nak => 6,
                MessageType::Release => 7,
                MessageType::Inform => 8,
                MessageType::Unknown(value) => value,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub enum OptionCode {
        SubnetMask,
        RequestedIpAddress,
        AddressLeaseTime,
        MessageType,
        ServerIdentifier,
        ParameterRequestList,
        Renewal,
        Rebinding,
        ClientIdentifier,
        Unknown(u8),
    }

    impl OptionCode {
        const fn number(self) -> u8 {
            match self {
                Self::SubnetMask => 1,
                Self::RequestedIpAddress => 50,
                Self::AddressLeaseTime => 51,
                Self::MessageType => 53,
                Self::ServerIdentifier => 54,
                Self::ParameterRequestList => 55,
                Self::Renewal => 58,
                Self::Rebinding => 59,
                Self::ClientIdentifier => 61,
                Self::Unknown(value) => value,
            }
        }
    }

    impl From<u8> for OptionCode {
        fn from(value: u8) -> Self {
            match value {
                1 => Self::SubnetMask,
                50 => Self::RequestedIpAddress,
                51 => Self::AddressLeaseTime,
                53 => Self::MessageType,
                54 => Self::ServerIdentifier,
                55 => Self::ParameterRequestList,
                58 => Self::Renewal,
                59 => Self::Rebinding,
                61 => Self::ClientIdentifier,
                other => Self::Unknown(other),
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum DhcpOption {
        SubnetMask(Ipv4Addr),
        RequestedIpAddress(Ipv4Addr),
        AddressLeaseTime(u32),
        MessageType(MessageType),
        ServerIdentifier(Ipv4Addr),
        ParameterRequestList(Vec<OptionCode>),
        Renewal(u32),
        Rebinding(u32),
        ClientIdentifier(Vec<u8>),
        Unknown(u8, Vec<u8>),
    }

    impl DhcpOption {
        fn code(&self) -> OptionCode {
            match self {
                Self::SubnetMask(_) => OptionCode::SubnetMask,
                Self::RequestedIpAddress(_) => OptionCode::RequestedIpAddress,
                Self::AddressLeaseTime(_) => OptionCode::AddressLeaseTime,
                Self::MessageType(_) => OptionCode::MessageType,
                Self::ServerIdentifier(_) => OptionCode::ServerIdentifier,
                Self::ParameterRequestList(_) => OptionCode::ParameterRequestList,
                Self::Renewal(_) => OptionCode::Renewal,
                Self::Rebinding(_) => OptionCode::Rebinding,
                Self::ClientIdentifier(_) => OptionCode::ClientIdentifier,
                Self::Unknown(code, _) => OptionCode::Unknown(*code),
            }
        }
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub struct DhcpOptions(BTreeMap<OptionCode, DhcpOption>);

    impl DhcpOptions {
        pub fn insert(&mut self, option: DhcpOption) -> Option<DhcpOption> {
            self.0.insert(option.code(), option)
        }
        pub fn get(&self, code: OptionCode) -> Option<&DhcpOption> {
            self.0.get(&code)
        }
        pub fn msg_type(&self) -> Option<MessageType> {
            match self.get(OptionCode::MessageType) {
                Some(DhcpOption::MessageType(value)) => Some(*value),
                _ => None,
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Message {
        opcode: Opcode,
        htype: HType,
        hlen: u8,
        hops: u8,
        xid: u32,
        secs: u16,
        flags: Flags,
        ciaddr: Ipv4Addr,
        yiaddr: Ipv4Addr,
        siaddr: Ipv4Addr,
        giaddr: Ipv4Addr,
        chaddr: Vec<u8>,
        opts: DhcpOptions,
    }

    impl Default for Message {
        fn default() -> Self {
            Self {
                opcode: Opcode::BootRequest,
                htype: HType::Eth,
                hlen: 0,
                hops: 0,
                xid: 0,
                secs: 0,
                flags: Flags::default(),
                ciaddr: Ipv4Addr::UNSPECIFIED,
                yiaddr: Ipv4Addr::UNSPECIFIED,
                siaddr: Ipv4Addr::UNSPECIFIED,
                giaddr: Ipv4Addr::UNSPECIFIED,
                chaddr: Vec::new(),
                opts: DhcpOptions::default(),
            }
        }
    }

    impl Message {
        pub const fn opcode(&self) -> Opcode {
            self.opcode
        }
        pub fn set_opcode(&mut self, value: Opcode) -> &mut Self {
            self.opcode = value;
            self
        }
        pub const fn htype(&self) -> HType {
            self.htype
        }
        pub fn set_htype(&mut self, value: HType) -> &mut Self {
            self.htype = value;
            self
        }
        pub const fn hlen(&self) -> u8 {
            self.hlen
        }
        pub const fn hops(&self) -> u8 {
            self.hops
        }
        pub fn set_hops(&mut self, value: u8) -> &mut Self {
            self.hops = value;
            self
        }
        pub const fn xid(&self) -> u32 {
            self.xid
        }
        pub fn set_xid(&mut self, value: u32) -> &mut Self {
            self.xid = value;
            self
        }
        pub const fn flags(&self) -> Flags {
            self.flags
        }
        pub fn set_flags(&mut self, value: Flags) -> &mut Self {
            self.flags = value;
            self
        }
        pub const fn ciaddr(&self) -> Ipv4Addr {
            self.ciaddr
        }
        pub fn set_ciaddr<I: Into<Ipv4Addr>>(&mut self, value: I) -> &mut Self {
            self.ciaddr = value.into();
            self
        }
        pub const fn yiaddr(&self) -> Ipv4Addr {
            self.yiaddr
        }
        pub fn set_yiaddr<I: Into<Ipv4Addr>>(&mut self, value: I) -> &mut Self {
            self.yiaddr = value.into();
            self
        }
        pub const fn siaddr(&self) -> Ipv4Addr {
            self.siaddr
        }
        pub fn set_siaddr<I: Into<Ipv4Addr>>(&mut self, value: I) -> &mut Self {
            self.siaddr = value.into();
            self
        }
        pub const fn giaddr(&self) -> Ipv4Addr {
            self.giaddr
        }
        pub fn set_giaddr<I: Into<Ipv4Addr>>(&mut self, value: I) -> &mut Self {
            self.giaddr = value.into();
            self
        }
        pub fn chaddr(&self) -> &[u8] {
            &self.chaddr
        }
        pub fn set_chaddr(&mut self, value: &[u8]) -> &mut Self {
            assert!(value.len() <= 16, "chaddr cannot exceed 16 bytes");
            self.chaddr.clear();
            self.chaddr.extend_from_slice(value);
            self.hlen = value.len() as u8;
            self
        }
        pub const fn opts(&self) -> &DhcpOptions {
            &self.opts
        }
        pub fn opts_mut(&mut self) -> &mut DhcpOptions {
            &mut self.opts
        }
    }

    impl Decodable for Message {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, Error> {
            let bytes = decoder.bytes;
            if bytes.len() < 240 {
                return Err(Error("DHCP packet is shorter than 240 bytes"));
            }
            if bytes[236..240] != MAGIC {
                return Err(Error("invalid DHCP magic cookie"));
            }
            let hlen = bytes[2];
            if hlen > 16 {
                return Err(Error("hardware address is longer than 16 bytes"));
            }
            let mut message = Self {
                opcode: bytes[0].into(),
                htype: bytes[1].into(),
                hlen,
                hops: bytes[3],
                xid: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
                secs: u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
                flags: Flags::new(u16::from_be_bytes(bytes[10..12].try_into().unwrap())),
                ciaddr: ipv4(&bytes[12..16]),
                yiaddr: ipv4(&bytes[16..20]),
                siaddr: ipv4(&bytes[20..24]),
                giaddr: ipv4(&bytes[24..28]),
                chaddr: bytes[28..28 + hlen as usize].to_vec(),
                opts: DhcpOptions::default(),
            };
            let mut offset = 240;
            let mut saw_end = false;
            while offset < bytes.len() {
                let code = bytes[offset];
                offset += 1;
                if code == 0 {
                    continue;
                }
                if code == 255 {
                    saw_end = true;
                    break;
                }
                let length = *bytes.get(offset).ok_or(Error("truncated DHCP option"))? as usize;
                offset += 1;
                let data = bytes
                    .get(offset..offset + length)
                    .ok_or(Error("truncated DHCP option value"))?;
                offset += length;
                let option = decode_option(code, data)?;
                message.opts.insert(option);
            }
            if !saw_end {
                return Err(Error("DHCP options are missing the end marker"));
            }
            Ok(message)
        }
    }

    impl Encodable for Message {
        fn to_vec(&self) -> Result<Vec<u8>, Error> {
            if self.chaddr.len() > 16 {
                return Err(Error("hardware address is longer than 16 bytes"));
            }
            let mut bytes = vec![0; 240];
            bytes[0] = self.opcode.into();
            bytes[1] = self.htype.into();
            bytes[2] = self.hlen;
            bytes[3] = self.hops;
            bytes[4..8].copy_from_slice(&self.xid.to_be_bytes());
            bytes[8..10].copy_from_slice(&self.secs.to_be_bytes());
            bytes[10..12].copy_from_slice(&self.flags.0.to_be_bytes());
            put_ip(&mut bytes[12..16], self.ciaddr);
            put_ip(&mut bytes[16..20], self.yiaddr);
            put_ip(&mut bytes[20..24], self.siaddr);
            put_ip(&mut bytes[24..28], self.giaddr);
            bytes[28..28 + self.chaddr.len()].copy_from_slice(&self.chaddr);
            bytes[236..240].copy_from_slice(&MAGIC);
            for option in self.opts.0.values() {
                encode_option(option, &mut bytes)?;
            }
            bytes.push(255);
            Ok(bytes)
        }
    }

    fn ipv4(bytes: &[u8]) -> Ipv4Addr {
        Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
    }
    fn put_ip(destination: &mut [u8], ip: Ipv4Addr) {
        destination.copy_from_slice(&ip.octets());
    }

    fn decode_option(code: u8, data: &[u8]) -> Result<DhcpOption, Error> {
        let ip = || {
            if data.len() == 4 {
                Ok(ipv4(data))
            } else {
                Err(Error("invalid IPv4 DHCP option length"))
            }
        };
        let number = || {
            if data.len() == 4 {
                Ok(u32::from_be_bytes(data.try_into().unwrap()))
            } else {
                Err(Error("invalid integer DHCP option length"))
            }
        };
        Ok(match code {
            1 => DhcpOption::SubnetMask(ip()?),
            50 => DhcpOption::RequestedIpAddress(ip()?),
            51 => DhcpOption::AddressLeaseTime(number()?),
            53 if data.len() == 1 => DhcpOption::MessageType(data[0].into()),
            53 => return Err(Error("invalid message-type option length")),
            54 => DhcpOption::ServerIdentifier(ip()?),
            55 => DhcpOption::ParameterRequestList(data.iter().copied().map(Into::into).collect()),
            58 => DhcpOption::Renewal(number()?),
            59 => DhcpOption::Rebinding(number()?),
            61 => DhcpOption::ClientIdentifier(data.to_vec()),
            other => DhcpOption::Unknown(other, data.to_vec()),
        })
    }

    fn encode_option(option: &DhcpOption, bytes: &mut Vec<u8>) -> Result<(), Error> {
        let mut data = Vec::new();
        match option {
            DhcpOption::SubnetMask(ip)
            | DhcpOption::RequestedIpAddress(ip)
            | DhcpOption::ServerIdentifier(ip) => data.extend(ip.octets()),
            DhcpOption::AddressLeaseTime(value)
            | DhcpOption::Renewal(value)
            | DhcpOption::Rebinding(value) => data.extend(value.to_be_bytes()),
            DhcpOption::MessageType(value) => data.push((*value).into()),
            DhcpOption::ParameterRequestList(values) => {
                data.extend(values.iter().map(|value| value.number()))
            }
            DhcpOption::ClientIdentifier(value) | DhcpOption::Unknown(_, value) => {
                data.extend(value)
            }
        }
        let length =
            u8::try_from(data.len()).map_err(|_| Error("DHCP option exceeds 255 bytes"))?;
        bytes.push(option.code().number());
        bytes.push(length);
        bytes.extend(data);
        Ok(())
    }
}
