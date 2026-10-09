//! GVSP wire format: the GigE Vision stream protocol.
//!
//! Every packet starts with a 16-bit status, then either the standard header
//! (16-bit frame id, 24-bit packet id) or the extended-id header (64-bit
//! frame id, 32-bit packet id), selected per packet by bit 31 of the 32-bit
//! word at byte offset 4 — valid for both layouts. All fields big-endian.

use crate::gige::proto::gvcp::GvcpStatus;

pub const PACKET_ID_MASK: u32 = 0x00ff_ffff;
const EXTENDED_ID_MODE: u32 = 0x8000_0000;
const CONTENT_TYPE_MASK: u32 = 0x7f00_0000;
const CONTENT_TYPE_POS: u32 = 24;

/// Smallest/largest GVSP packet on the wire (ethernet frame minus protocol
/// overhead).
pub const MINIMUM_PACKET_SIZE: usize = 64 - 14 - 4;
pub const MAXIMUM_PACKET_SIZE: usize = 65536 - 14 - 4;
/// IP + UDP headers, included in the SCPS packet size.
pub const PACKET_UDP_OVERHEAD: usize = 20 + 8;

/// Bytes of a payload packet that are not image data: IP + UDP + GVSP
/// headers. The data block per payload packet is `scps - this`.
pub fn packet_protocol_overhead(extended_ids: bool) -> usize {
    PACKET_UDP_OVERHEAD + 2 + if extended_ids { 18 } else { 6 }
}

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContentType {
    Leader,
    Trailer,
    Payload,
    AllIn,
    H264,
    Multizone,
    Multipart,
    GenDc,
    Unknown(u8),
}

impl ContentType {
    fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Leader,
            2 => Self::Trailer,
            3 => Self::Payload,
            4 => Self::AllIn,
            5 => Self::H264,
            6 => Self::Multizone,
            7 => Self::Multipart,
            8 => Self::GenDc,
            other => Self::Unknown(other),
        }
    }
}

/// A zero-copy view over one GVSP datagram.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct GvspView<'a> {
    pub status: GvcpStatus,
    pub extended_ids: bool,
    pub frame_id: u64,
    pub packet_id: u32,
    pub content_type: ContentType,
    pub data: &'a [u8],
}

impl<'a> GvspView<'a> {
    #[inline]
    pub fn parse(buf: &'a [u8]) -> Option<Self> {
        let status = GvcpStatus(u16::from_be_bytes([*buf.first()?, *buf.get(1)?]));
        let infos = u32::from_be_bytes(buf.get(4..8)?.try_into().ok()?);
        let extended_ids = infos & EXTENDED_ID_MODE != 0;
        let content_type =
            ContentType::from_bits(((infos & CONTENT_TYPE_MASK) >> CONTENT_TYPE_POS) as u8);
        if extended_ids {
            Some(Self {
                status,
                extended_ids,
                frame_id: u64::from_be_bytes(buf.get(8..16)?.try_into().ok()?),
                packet_id: u32::from_be_bytes(buf.get(16..20)?.try_into().ok()?),
                content_type,
                data: buf.get(20..)?,
            })
        } else {
            Some(Self {
                status,
                extended_ids,
                frame_id: u64::from(u16::from_be_bytes([buf[2], buf[3]])),
                packet_id: infos & PACKET_ID_MASK,
                content_type,
                data: buf.get(8..)?,
            })
        }
    }
}

/// Payload type ids carried in leaders and trailers.
pub const PAYLOAD_TYPE_IMAGE: u16 = 0x0001;
pub const PAYLOAD_TYPE_RAW: u16 = 0x0002;
pub const PAYLOAD_TYPE_FILE: u16 = 0x0003;
pub const PAYLOAD_TYPE_CHUNK_DATA: u16 = 0x0004;
pub const PAYLOAD_TYPE_EXTENDED_CHUNK_DATA: u16 = 0x0005;
pub const PAYLOAD_TYPE_JPEG: u16 = 0x0006;
pub const PAYLOAD_TYPE_H264: u16 = 0x0008;
pub const PAYLOAD_TYPE_MULTIPART: u16 = 0x000a;
/// GEV 2.0 "payload carries trailing chunks" extension bit.
pub const PAYLOAD_TYPE_CHUNK_EXTENSION: u16 = 0x4000;

/// The image leader's data area (after the generic flags/payload_type/
/// timestamp prefix shared by all leaders).
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageLeader {
    pub payload_type: u16,
    pub has_chunks: bool,
    pub timestamp_ticks: u64,
    pub pixel_format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub x_padding: u16,
    pub y_padding: u16,
}

impl ImageLeader {
    /// Parse a leader packet's data area. For image and extended-chunk
    /// payload types every field is filled; for other payload types the
    /// image fields read as zero (the generic leader is only 12 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        let raw_payload_type = u16::from_be_bytes(data.get(2..4)?.try_into().ok()?);
        let payload_type = raw_payload_type & !PAYLOAD_TYPE_CHUNK_EXTENSION;
        let has_chunks = raw_payload_type & PAYLOAD_TYPE_CHUNK_EXTENSION != 0
            || payload_type == PAYLOAD_TYPE_CHUNK_DATA
            || payload_type == PAYLOAD_TYPE_EXTENDED_CHUNK_DATA;
        let timestamp_ticks = u64::from_be_bytes(data.get(4..12)?.try_into().ok()?);
        let u32_at = |i: usize| {
            data.get(i..i + 4)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let u16_at = |i: usize| {
            data.get(i..i + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        Some(Self {
            payload_type,
            has_chunks,
            timestamp_ticks,
            pixel_format: PixelFormat(u32_at(12)),
            width: u32_at(16),
            height: u32_at(20),
            x_offset: u32_at(24),
            y_offset: u32_at(28),
            x_padding: u16_at(32),
            y_padding: u16_at(34),
        })
    }
}

/// A PFNC (Pixel Format Naming Convention) code. Bits 16..24 carry the
/// number of bits per pixel.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub struct PixelFormat(pub u32);

impl PixelFormat {
    pub const MONO8: Self = Self(0x0108_0001);
    pub const MONO10: Self = Self(0x0110_0003);
    pub const MONO12: Self = Self(0x0110_0005);
    pub const MONO16: Self = Self(0x0110_0007);
    pub const BAYER_RG8: Self = Self(0x0108_0009);
    pub const BAYER_GB8: Self = Self(0x0108_000a);
    pub const RGB8: Self = Self(0x0218_0014);
    pub const BGR8: Self = Self(0x0218_0015);
    pub const YUV422_8: Self = Self(0x0210_0032);

    pub fn bits_per_pixel(self) -> u32 {
        (self.0 >> 16) & 0xff
    }

    /// Bytes needed for `width * height` pixels of this format (excluding
    /// line padding).
    pub fn image_size(self, width: u32, height: u32) -> usize {
        (u64::from(width) * u64::from(height) * u64::from(self.bits_per_pixel())).div_ceil(8)
            as usize
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match *self {
            Self::MONO8 => "Mono8",
            Self::MONO10 => "Mono10",
            Self::MONO12 => "Mono12",
            Self::MONO16 => "Mono16",
            Self::BAYER_RG8 => "BayerRG8",
            Self::BAYER_GB8 => "BayerGB8",
            Self::RGB8 => "RGB8",
            Self::BGR8 => "BGR8",
            Self::YUV422_8 => "YUV422_8",
            _ => return write!(f, "{:#010x}", self.0),
        };
        f.write_str(name)
    }
}

/// Convert a device timestamp to nanoseconds using the device tick frequency.
pub fn timestamp_to_ns(ticks: u64, tick_frequency: u64) -> u64 {
    if tick_frequency < 1 {
        return 0;
    }
    let s = ticks / tick_frequency;
    let ns = ((ticks % tick_frequency) * 1_000_000_000) / tick_frequency;
    s * 1_000_000_000 + ns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_packet(frame_id: u16, packet_id: u32, content: u8, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&frame_id.to_be_bytes());
        let infos = (u32::from(content) << CONTENT_TYPE_POS) | (packet_id & PACKET_ID_MASK);
        buf.extend_from_slice(&infos.to_be_bytes());
        buf.extend_from_slice(data);
        buf
    }

    fn ext_packet(frame_id: u64, packet_id: u32, content: u8, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        let infos = EXTENDED_ID_MODE | (u32::from(content) << CONTENT_TYPE_POS);
        buf.extend_from_slice(&infos.to_be_bytes());
        buf.extend_from_slice(&frame_id.to_be_bytes());
        buf.extend_from_slice(&packet_id.to_be_bytes());
        buf.extend_from_slice(data);
        buf
    }

    #[test]
    fn parse_standard_payload() {
        let pkt = std_packet(0x0102, 42, 3, &[9, 8, 7]);
        let v = GvspView::parse(&pkt).unwrap();
        assert!(!v.extended_ids);
        assert_eq!(v.status, GvcpStatus::SUCCESS);
        assert_eq!(v.frame_id, 0x0102);
        assert_eq!(v.packet_id, 42);
        assert_eq!(v.content_type, ContentType::Payload);
        assert_eq!(v.data, [9, 8, 7]);
    }

    #[test]
    fn parse_extended_trailer() {
        let pkt = ext_packet(0x0102_0304_0506_0708, 99, 2, &[0; 8]);
        let v = GvspView::parse(&pkt).unwrap();
        assert!(v.extended_ids);
        assert_eq!(v.frame_id, 0x0102_0304_0506_0708);
        assert_eq!(v.packet_id, 99);
        assert_eq!(v.content_type, ContentType::Trailer);
        assert_eq!(v.data.len(), 8);
    }

    #[test]
    fn parse_resend_status() {
        let mut pkt = std_packet(1, 5, 3, &[1]);
        pkt[..2].copy_from_slice(&0x0100u16.to_be_bytes());
        let v = GvspView::parse(&pkt).unwrap();
        assert_eq!(v.status, GvcpStatus::PACKET_RESEND);
        assert!(!v.status.is_error());
    }

    #[test]
    fn parse_image_leader() {
        let mut data = Vec::new();
        data.extend_from_slice(&0u16.to_be_bytes()); // flags
        data.extend_from_slice(&(PAYLOAD_TYPE_IMAGE | PAYLOAD_TYPE_CHUNK_EXTENSION).to_be_bytes());
        data.extend_from_slice(&0x0000_0001_0000_0002u64.to_be_bytes()); // timestamp hi/lo
        data.extend_from_slice(&PixelFormat::MONO8.0.to_be_bytes());
        data.extend_from_slice(&640u32.to_be_bytes());
        data.extend_from_slice(&480u32.to_be_bytes());
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(&2u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());

        let leader = ImageLeader::parse(&data).unwrap();
        assert_eq!(leader.payload_type, PAYLOAD_TYPE_IMAGE);
        assert!(leader.has_chunks);
        assert_eq!(leader.timestamp_ticks, 0x0000_0001_0000_0002);
        assert_eq!(leader.pixel_format, PixelFormat::MONO8);
        assert_eq!((leader.width, leader.height), (640, 480));
        assert_eq!((leader.x_offset, leader.y_offset), (4, 8));
        assert_eq!((leader.x_padding, leader.y_padding), (2, 0));
    }

    #[test]
    fn pixel_format_helpers() {
        assert_eq!(PixelFormat::MONO8.bits_per_pixel(), 8);
        assert_eq!(PixelFormat::RGB8.bits_per_pixel(), 24);
        // PFNC Mono12 occupies 16 bits per pixel (unpacked).
        assert_eq!(PixelFormat::MONO12.bits_per_pixel(), 16);
        assert_eq!(PixelFormat::MONO12.image_size(4, 2), 16);
        assert_eq!(PixelFormat::MONO8.to_string(), "Mono8");
    }

    #[test]
    fn overheads() {
        assert_eq!(packet_protocol_overhead(false), 36);
        assert_eq!(packet_protocol_overhead(true), 48);
    }

    #[test]
    fn timestamp_conversion() {
        assert_eq!(timestamp_to_ns(125_000_000, 125_000_000), 1_000_000_000);
        assert_eq!(timestamp_to_ns(1, 0), 0);
        assert_eq!(timestamp_to_ns(3, 2), 1_500_000_000);
    }
}
