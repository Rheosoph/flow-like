//! GVCP wire format: the GigE Vision control protocol on UDP/3956.
//!
//! Command packets carry a [`CmdHeader`]; acknowledges carry an [`AckHeader`]
//! whose first two bytes are a [`GvcpStatus`]. All fields are big-endian.

use std::net::Ipv4Addr;

use zerocopy::byteorder::network_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Ref, Unaligned};

pub const GVCP_PORT: u16 = 3956;

/// Maximum READMEM/WRITEMEM data bytes per transaction.
pub const DATA_SIZE_MAX: usize = 512;

pub const PACKET_TYPE_CMD: u8 = 0x42;
pub const PACKET_TYPE_ACK: u8 = 0x00;
pub const PACKET_TYPE_ERROR: u8 = 0x80;

pub const FLAG_ACK_REQUIRED: u8 = 0x01;
/// Resend commands with 64-bit frame ids set this instead of ACK_REQUIRED.
pub const FLAG_EXTENDED_IDS: u8 = 0x10;
/// Discovery-specific: the device may answer to the broadcast address.
pub const FLAG_ALLOW_BROADCAST_ACK: u8 = 0x10;

pub const DISCOVERY_CMD: u16 = 0x0002;
pub const DISCOVERY_ACK: u16 = 0x0003;
pub const FORCEIP_CMD: u16 = 0x0004;
pub const FORCEIP_ACK: u16 = 0x0005;
pub const PACKET_RESEND_CMD: u16 = 0x0040;
pub const READ_REGISTER_CMD: u16 = 0x0080;
pub const READ_REGISTER_ACK: u16 = 0x0081;
pub const WRITE_REGISTER_CMD: u16 = 0x0082;
pub const WRITE_REGISTER_ACK: u16 = 0x0083;
pub const READ_MEMORY_CMD: u16 = 0x0084;
pub const READ_MEMORY_ACK: u16 = 0x0085;
pub const WRITE_MEMORY_CMD: u16 = 0x0086;
pub const WRITE_MEMORY_ACK: u16 = 0x0087;
pub const PENDING_ACK: u16 = 0x0089;
pub const EVENT_CMD: u16 = 0x00c0;
pub const EVENT_ACK: u16 = 0x00c1;
pub const EVENTDATA_CMD: u16 = 0x00c2;
pub const EVENTDATA_ACK: u16 = 0x00c3;

/// Discovery commands carry this id; regular transactions use 1..=0xfffe.
pub const DISCOVERY_ID: u16 = 0xffff;

/// Next request id: 0 is reserved as an error value and 0xffff for
/// discovery, so wrap 0xfffe -> 1.
pub fn next_id(id: u16) -> u16 {
    if id >= 0xfffe { 1 } else { id + 1 }
}

/// Status word of an acknowledge (also used by GVSP packets).
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GvcpStatus(pub u16);

impl GvcpStatus {
    pub const SUCCESS: Self = Self(0x0000);
    /// GVSP only: marks a resent packet; not an error.
    pub const PACKET_RESEND: Self = Self(0x0100);
    pub const NOT_IMPLEMENTED: Self = Self(0x8001);
    pub const INVALID_PARAMETER: Self = Self(0x8002);
    pub const INVALID_ADDRESS: Self = Self(0x8003);
    pub const WRITE_PROTECT: Self = Self(0x8004);
    pub const BAD_ALIGNMENT: Self = Self(0x8005);
    pub const ACCESS_DENIED: Self = Self(0x8006);
    pub const BUSY: Self = Self(0x8007);
    pub const PACKET_UNAVAILABLE: Self = Self(0x800c);
    pub const DATA_OVERRUN: Self = Self(0x800d);
    pub const INVALID_HEADER: Self = Self(0x800e);
    pub const PACKET_NOT_YET_AVAILABLE: Self = Self(0x8010);
    pub const PACKET_AND_PREV_REMOVED_FROM_MEMORY: Self = Self(0x8011);
    pub const PACKET_REMOVED_FROM_MEMORY: Self = Self(0x8012);
    pub const NO_REF_TIME: Self = Self(0x8013);
    pub const PACKET_TEMPORARILY_UNAVAILABLE: Self = Self(0x8014);
    pub const OVERFLOW: Self = Self(0x8015);
    pub const ACTION_LATE: Self = Self(0x8016);
    pub const LEADER_TRAILER_OVERFLOW: Self = Self(0x8017);
    pub const GENERIC_ERROR: Self = Self(0x8fff);

    pub fn is_error(self) -> bool {
        self.0 & 0x8000 != 0
    }
}

impl std::fmt::Display for GvcpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match *self {
            Self::SUCCESS => "success",
            Self::PACKET_RESEND => "packet resend",
            Self::NOT_IMPLEMENTED => "not implemented",
            Self::INVALID_PARAMETER => "invalid parameter",
            Self::INVALID_ADDRESS => "invalid address",
            Self::WRITE_PROTECT => "write protect",
            Self::BAD_ALIGNMENT => "bad alignment",
            Self::ACCESS_DENIED => "access denied",
            Self::BUSY => "busy",
            Self::PACKET_UNAVAILABLE => "packet unavailable",
            Self::DATA_OVERRUN => "data overrun",
            Self::INVALID_HEADER => "invalid header",
            Self::PACKET_NOT_YET_AVAILABLE => "packet not yet available",
            Self::PACKET_AND_PREV_REMOVED_FROM_MEMORY => "packet and previous removed from memory",
            Self::PACKET_REMOVED_FROM_MEMORY => "packet removed from memory",
            Self::NO_REF_TIME => "no reference time",
            Self::PACKET_TEMPORARILY_UNAVAILABLE => "packet temporarily unavailable",
            Self::OVERFLOW => "overflow",
            Self::ACTION_LATE => "action late",
            Self::LEADER_TRAILER_OVERFLOW => "leader/trailer overflow",
            Self::GENERIC_ERROR => "generic error",
            _ => return write!(f, "status {:#06x}", self.0),
        };
        write!(f, "{name} ({:#06x})", self.0)
    }
}

/// Header of a command packet (host -> device, or device -> host for events).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub struct CmdHeader {
    pub packet_type: u8,
    pub flags: u8,
    pub command: U16,
    pub length: U16,
    pub req_id: U16,
}

/// Header of an acknowledge packet.
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub struct AckHeader {
    pub status: U16,
    pub answer: U16,
    pub length: U16,
    pub ack_id: U16,
}

pub const HEADER_LEN: usize = 8;

/// `true` if the datagram is a command packet (a device-initiated event);
/// anything else coming back on the control socket is an acknowledge.
pub fn is_cmd(buf: &[u8]) -> bool {
    buf.first() == Some(&PACKET_TYPE_CMD)
}

/// A decoded acknowledge: header fields plus the payload sized by `length`.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Ack<'a> {
    pub status: GvcpStatus,
    pub answer: u16,
    pub ack_id: u16,
    pub payload: &'a [u8],
}

impl<'a> Ack<'a> {
    pub fn parse(buf: &'a [u8]) -> Option<Self> {
        let (header, rest) = Ref::<_, AckHeader>::from_prefix(buf).ok()?;
        let length = usize::from(header.length.get());
        Some(Self {
            status: GvcpStatus(header.status.get()),
            answer: header.answer.get(),
            ack_id: header.ack_id.get(),
            payload: rest.get(..length)?,
        })
    }

    /// PENDING_ACK payload: the replacement timeout, in milliseconds.
    pub fn pending_ack_timeout_ms(&self) -> Option<u32> {
        if self.answer != PENDING_ACK {
            return None;
        }
        Some(U32::read_from_prefix(self.payload).ok()?.0.get())
    }

    /// READ_REGISTER_ACK payload: one u32 value per requested address.
    pub fn register_values(&self) -> impl Iterator<Item = u32> + '_ {
        self.payload
            .chunks_exact(4)
            .map(|c| U32::read_from_bytes(c).map_or(0, |v| v.get()))
    }
}

/// A decoded command packet (device-initiated: events).
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Cmd<'a> {
    pub flags: u8,
    pub command: u16,
    pub req_id: u16,
    pub payload: &'a [u8],
}

impl<'a> Cmd<'a> {
    pub fn parse(buf: &'a [u8]) -> Option<Self> {
        let (header, rest) = Ref::<_, CmdHeader>::from_prefix(buf).ok()?;
        if header.packet_type != PACKET_TYPE_CMD {
            return None;
        }
        let length = usize::from(header.length.get());
        Some(Self {
            flags: header.flags,
            command: header.command.get(),
            req_id: header.req_id.get(),
            payload: rest.get(..length)?,
        })
    }
}

fn header(command: u16, flags: u8, payload_len: usize, id: u16) -> CmdHeader {
    debug_assert!(payload_len <= u16::MAX as usize);
    CmdHeader {
        packet_type: PACKET_TYPE_CMD,
        flags,
        command: U16::new(command),
        length: U16::new(payload_len as u16),
        req_id: U16::new(id),
    }
}

pub fn encode_discovery(allow_broadcast_ack: bool) -> [u8; HEADER_LEN] {
    let flags = FLAG_ACK_REQUIRED
        | if allow_broadcast_ack {
            FLAG_ALLOW_BROADCAST_ACK
        } else {
            0
        };
    let mut buf = [0u8; HEADER_LEN];
    buf.copy_from_slice(header(DISCOVERY_CMD, flags, 0, DISCOVERY_ID).as_bytes());
    buf
}

pub fn encode_read_reg(addrs: &[u32], id: u16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + 4 * addrs.len());
    buf.extend_from_slice(
        header(READ_REGISTER_CMD, FLAG_ACK_REQUIRED, 4 * addrs.len(), id).as_bytes(),
    );
    for &addr in addrs {
        buf.extend_from_slice(U32::new(addr).as_bytes());
    }
    buf
}

pub fn encode_write_reg(pairs: &[(u32, u32)], id: u16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + 8 * pairs.len());
    buf.extend_from_slice(
        header(WRITE_REGISTER_CMD, FLAG_ACK_REQUIRED, 8 * pairs.len(), id).as_bytes(),
    );
    for &(addr, value) in pairs {
        buf.extend_from_slice(U32::new(addr).as_bytes());
        buf.extend_from_slice(U32::new(value).as_bytes());
    }
    buf
}

pub fn encode_read_mem(addr: u32, count: u16, id: u16) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[..HEADER_LEN].copy_from_slice(header(READ_MEMORY_CMD, FLAG_ACK_REQUIRED, 8, id).as_bytes());
    buf[8..12].copy_from_slice(U32::new(addr).as_bytes());
    buf[12..16].copy_from_slice(U32::new(u32::from(count)).as_bytes());
    buf
}

pub fn encode_write_mem(addr: u32, data: &[u8], id: u16) -> Vec<u8> {
    debug_assert!(data.len() <= DATA_SIZE_MAX);
    debug_assert!(data.len().is_multiple_of(4));
    let mut buf = Vec::with_capacity(HEADER_LEN + 4 + data.len());
    buf.extend_from_slice(
        header(WRITE_MEMORY_CMD, FLAG_ACK_REQUIRED, 4 + data.len(), id).as_bytes(),
    );
    buf.extend_from_slice(U32::new(addr).as_bytes());
    buf.extend_from_slice(data);
    buf
}

/// Maximum encoded size of a packet resend command (extended-ids form).
pub const RESEND_MAX_LEN: usize = HEADER_LEN + 20;

/// Encode a PACKETRESEND command into `buf` (at least [`RESEND_MAX_LEN`]
/// bytes), returning the encoded length. Fire-and-forget: no ACK_REQUIRED,
/// the device never acknowledges resend requests.
///
/// Standard ids: payload `[frame_id u32][first & 0xffffff][last & 0xffffff]`.
/// Extended ids: payload `[0u32][first][last][frame_id u64]`, EXTENDED_IDS flag.
pub fn encode_packet_resend(
    buf: &mut [u8],
    frame_id: u64,
    first: u32,
    last: u32,
    extended_ids: bool,
    id: u16,
) -> usize {
    const PACKET_ID_MASK: u32 = 0x00ff_ffff;
    if extended_ids {
        buf[..HEADER_LEN]
            .copy_from_slice(header(PACKET_RESEND_CMD, FLAG_EXTENDED_IDS, 20, id).as_bytes());
        buf[8..12].copy_from_slice(U32::new(0).as_bytes());
        buf[12..16].copy_from_slice(U32::new(first).as_bytes());
        buf[16..20].copy_from_slice(U32::new(last).as_bytes());
        buf[20..28].copy_from_slice(U64::new(frame_id).as_bytes());
        28
    } else {
        buf[..HEADER_LEN].copy_from_slice(header(PACKET_RESEND_CMD, 0, 12, id).as_bytes());
        buf[8..12].copy_from_slice(U32::new(frame_id as u32).as_bytes());
        buf[12..16].copy_from_slice(U32::new(first & PACKET_ID_MASK).as_bytes());
        buf[16..20].copy_from_slice(U32::new(last & PACKET_ID_MASK).as_bytes());
        20
    }
}

/// Encode a FORCEIP command (64 bytes, broadcast). Layout per the GigE Vision
/// spec, cross-checked against `GigeVision.Core/Services/Gvcp.cs`.
pub fn encode_force_ip(
    mac: [u8; 6],
    ip: Ipv4Addr,
    mask: Ipv4Addr,
    gateway: Ipv4Addr,
    id: u16,
) -> [u8; 64] {
    let mut buf = [0u8; 64];
    buf[..HEADER_LEN].copy_from_slice(header(FORCEIP_CMD, FLAG_ACK_REQUIRED, 56, id).as_bytes());
    buf[10..16].copy_from_slice(&mac);
    buf[28..32].copy_from_slice(&ip.octets());
    buf[44..48].copy_from_slice(&mask.octets());
    buf[60..64].copy_from_slice(&gateway.octets());
    buf
}

/// Encode the acknowledge a host must send back for an EVENT_CMD/EVENTDATA_CMD.
pub fn encode_event_ack(command: u16, req_id: u16) -> [u8; HEADER_LEN] {
    let ack = AckHeader {
        status: U16::new(GvcpStatus::SUCCESS.0),
        answer: U16::new(command + 1),
        length: U16::new(0),
        ack_id: U16::new(req_id),
    };
    let mut buf = [0u8; HEADER_LEN];
    buf.copy_from_slice(ack.as_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_layout() {
        let pkt = encode_discovery(true);
        assert_eq!(pkt, [0x42, 0x11, 0x00, 0x02, 0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn read_reg_roundtrip() {
        let pkt = encode_read_reg(&[0x0a00, 0x0934], 7);
        assert_eq!(pkt.len(), 16);
        assert_eq!(&pkt[..8], &[0x42, 0x01, 0x00, 0x80, 0x00, 0x08, 0x00, 0x07]);
        assert_eq!(&pkt[8..12], &[0x00, 0x00, 0x0a, 0x00]);
        assert_eq!(&pkt[12..16], &[0x00, 0x00, 0x09, 0x34]);
    }

    #[test]
    fn write_reg_layout() {
        let pkt = encode_write_reg(&[(0x0a00, 0x2)], 3);
        assert_eq!(&pkt[..8], &[0x42, 0x01, 0x00, 0x82, 0x00, 0x08, 0x00, 0x03]);
        assert_eq!(&pkt[8..16], &[0, 0, 0x0a, 0, 0, 0, 0, 2]);
    }

    #[test]
    fn read_mem_layout() {
        let pkt = encode_read_mem(0x200, 512, 9);
        assert_eq!(&pkt[..8], &[0x42, 0x01, 0x00, 0x84, 0x00, 0x08, 0x00, 0x09]);
        assert_eq!(&pkt[8..12], &[0, 0, 0x02, 0]);
        assert_eq!(&pkt[12..16], &[0, 0, 0x02, 0]);
    }

    #[test]
    fn write_mem_layout() {
        let pkt = encode_write_mem(0x1000, &[1, 2, 3, 4], 5);
        assert_eq!(&pkt[..8], &[0x42, 0x01, 0x00, 0x86, 0x00, 0x08, 0x00, 0x05]);
        assert_eq!(&pkt[8..12], &[0, 0, 0x10, 0]);
        assert_eq!(&pkt[12..], &[1, 2, 3, 4]);
    }

    #[test]
    fn resend_standard() {
        let mut buf = [0u8; RESEND_MAX_LEN];
        let n = encode_packet_resend(&mut buf, 0x1234, 5, 0x01ff_ffff, false, 65300);
        assert_eq!(n, 20);
        assert_eq!(&buf[..8], &[0x42, 0x00, 0x00, 0x40, 0x00, 0x0c, 0xff, 0x14]);
        assert_eq!(&buf[8..12], &[0, 0, 0x12, 0x34]);
        assert_eq!(&buf[12..16], &[0, 0, 0, 5]);
        // 24-bit mask applied
        assert_eq!(&buf[16..20], &[0x00, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn resend_extended() {
        let mut buf = [0u8; RESEND_MAX_LEN];
        let n = encode_packet_resend(&mut buf, 0x0102_0304_0506_0708, 1, 2, true, 1);
        assert_eq!(n, 28);
        assert_eq!(&buf[..8], &[0x42, 0x10, 0x00, 0x40, 0x00, 0x14, 0x00, 0x01]);
        assert_eq!(&buf[8..12], &[0, 0, 0, 0]);
        assert_eq!(&buf[12..16], &[0, 0, 0, 1]);
        assert_eq!(&buf[16..20], &[0, 0, 0, 2]);
        assert_eq!(&buf[20..28], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn force_ip_layout() {
        let pkt = encode_force_ip(
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            Ipv4Addr::new(192, 168, 1, 10),
            Ipv4Addr::new(255, 255, 255, 0),
            Ipv4Addr::new(192, 168, 1, 1),
            2,
        );
        assert_eq!(&pkt[..8], &[0x42, 0x01, 0x00, 0x04, 0x00, 0x38, 0x00, 0x02]);
        assert_eq!(&pkt[10..16], &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(&pkt[28..32], &[192, 168, 1, 10]);
        assert_eq!(&pkt[44..48], &[255, 255, 255, 0]);
        assert_eq!(&pkt[60..64], &[192, 168, 1, 1]);
    }

    #[test]
    fn ack_parse() {
        // READ_REGISTER_ACK with two values
        let raw = [
            0x00, 0x00, 0x00, 0x81, 0x00, 0x08, 0x00, 0x07, 0x00, 0x00, 0x00, 0x02, 0xde, 0xad,
            0xbe, 0xef,
        ];
        let ack = Ack::parse(&raw).unwrap();
        assert_eq!(ack.status, GvcpStatus::SUCCESS);
        assert!(!ack.status.is_error());
        assert_eq!(ack.answer, READ_REGISTER_ACK);
        assert_eq!(ack.ack_id, 7);
        let values: Vec<u32> = ack.register_values().collect();
        assert_eq!(values, [2, 0xdead_beef]);
    }

    #[test]
    fn ack_parse_truncated_payload() {
        // length claims 8 bytes but only 4 present
        let raw = [0x00, 0x00, 0x00, 0x81, 0x00, 0x08, 0x00, 0x07, 0, 0, 0, 2];
        assert!(Ack::parse(&raw).is_none());
    }

    #[test]
    fn ack_error_status() {
        let raw = [0x80, 0x06, 0x00, 0x83, 0x00, 0x00, 0x00, 0x09];
        let ack = Ack::parse(&raw).unwrap();
        assert_eq!(ack.status, GvcpStatus::ACCESS_DENIED);
        assert!(ack.status.is_error());
        assert!(ack.payload.is_empty());
    }

    #[test]
    fn pending_ack_timeout() {
        let raw = [
            0x00, 0x00, 0x00, 0x89, 0x00, 0x04, 0x00, 0x03, 0x00, 0x00, 0x03, 0xe8,
        ];
        let ack = Ack::parse(&raw).unwrap();
        assert_eq!(ack.pending_ack_timeout_ms(), Some(1000));
    }

    #[test]
    fn cmd_parse_event() {
        let raw = [0x42, 0x00, 0x00, 0xc0, 0x00, 0x04, 0x00, 0x21, 1, 2, 3, 4];
        let cmd = Cmd::parse(&raw).unwrap();
        assert_eq!(cmd.command, EVENT_CMD);
        assert_eq!(cmd.req_id, 0x21);
        assert_eq!(cmd.payload, [1, 2, 3, 4]);
        assert!(is_cmd(&raw));

        let ack = encode_event_ack(cmd.command, cmd.req_id);
        assert_eq!(ack, [0x00, 0x00, 0x00, 0xc1, 0x00, 0x00, 0x00, 0x21]);
    }

    #[test]
    fn id_sequence_skips_zero() {
        assert_eq!(next_id(0xfffe), 1);
        assert_eq!(next_id(0xffff), 1);
        assert_eq!(next_id(1), 2);
    }
}
