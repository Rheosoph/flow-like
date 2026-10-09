//! Server-side GigE Vision device emulator: the camera end of the GVCP control
//! and GVSP stream protocols conductor's real [`GigECamera`](crate::gige::GigECamera)
//! dials.
//!
//! The real driver is client-only; this module supplies the inverse — a
//! bootstrap register/memory map (discovery block, device strings, a zipped
//! GenICam description), the GVCP acknowledge encoders, and a GVSP SingleFrame
//! leader/payload/trailer encoder. It is socket-agnostic: [`GigeDevice`] turns an
//! inbound GVCP datagram into a [`Reaction`] and builds GVSP packets for a mono8
//! buffer; the host owns the sockets (std loopback in unit tests, `snare::net` in
//! theater) and the frame source.

use std::net::{Ipv4Addr, SocketAddr};

use crate::gige::proto::gvcp;
use crate::gige::proto::{bootstrap, gvsp};

mod xml;
pub use xml::GENICAM_XML;

/// Size of the emulated device register/memory space.
pub const MEM_SIZE: usize = 0x10000;
/// Address the zipped GenICam description is installed at.
pub const XML_ADDRESS: u32 = 0x8000;

const WIDTH_REG: u32 = 0x2000;
const HEIGHT_REG: u32 = 0x2004;
const ACQ_REG: u32 = 0x2008;
const ACQ_MODE_REG: u32 = 0x200C;
const PIXEL_FORMAT_REG: u32 = 0x2010;
const EXPOSURE_REG: u32 = 0x2014;
const GAIN_REG: u32 = 0x2018;

/// Static device identity + default geometry the emulator advertises.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceConfig {
    pub width: u32,
    pub height: u32,
    pub exposure_us: f32,
    pub gain: f32,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial: String,
    pub mac: [u8; 6],
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 1024,
            exposure_us: 5000.0,
            gain: 1.0,
            manufacturer: "Valstad".to_string(),
            model: "TheaterCam".to_string(),
            device_version: "1.0.0".to_string(),
            serial: "THEATER-CAM-0001".to_string(),
            mac: [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
        }
    }
}

/// What one inbound GVCP datagram implies for the host.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Reaction {
    /// The GVCP acknowledge to send back to the command's source, if any.
    pub reply: Option<Vec<u8>>,
    /// A fire-test packet of this payload size to send to the stream destination
    /// (packet-size negotiation).
    pub fire_test: Option<u16>,
    /// `AcquisitionStart` was executed — the host should render a frame and emit
    /// it via [`GigeDevice::frame_packets`], then clear the trigger.
    pub acquisition_started: bool,
}

/// Emulated GigE Vision device. Holds the register/memory image; the host feeds
/// it GVCP datagrams and drives GVSP from its reactions.
#[derive(Debug)]
pub struct GigeDevice {
    mem: Vec<u8>,
    /// Source address of the most recent valid GVCP command — the requester
    /// the stream falls back to when the client advertises SCDA=0.
    ctrl_peer: Option<SocketAddr>,
}

impl GigeDevice {
    /// Builds a device at `ip` with `cfg`, seeding the bootstrap map, installing
    /// the GenICam description, and priming the feature registers.
    pub fn new(ip: Ipv4Addr, cfg: &DeviceConfig) -> Self {
        let mut dev = Self {
            mem: vec![0u8; MEM_SIZE],
            ctrl_peer: None,
        };
        dev.seed(ip, cfg);
        dev
    }

    fn seed(&mut self, ip: Ipv4Addr, cfg: &DeviceConfig) {
        self.write_reg(bootstrap::VERSION, 0x0002_0000);
        self.write_reg(
            bootstrap::GVCP_CAPABILITY,
            bootstrap::CAP_WRITE_MEMORY
                | bootstrap::CAP_PACKET_RESEND
                | bootstrap::CAP_HEARTBEAT_DISABLE,
        );
        self.write_reg(bootstrap::HEARTBEAT_TIMEOUT, 3000);
        self.write_reg(bootstrap::N_STREAM_CHANNELS, 1);
        self.write_reg(bootstrap::TIMESTAMP_TICK_FREQUENCY_LOW, 1_000_000_000);
        self.write_reg(bootstrap::CURRENT_IP_ADDRESS, u32::from(ip));
        self.write_reg(
            bootstrap::CURRENT_SUBNET_MASK,
            u32::from(Ipv4Addr::new(255, 0, 0, 0)),
        );
        self.mem[0x0a..0x10].copy_from_slice(&cfg.mac);
        self.write_string(bootstrap::MANUFACTURER_NAME, &cfg.manufacturer);
        self.write_string(bootstrap::MODEL_NAME, &cfg.model);
        self.write_string(bootstrap::DEVICE_VERSION, &cfg.device_version);
        self.write_string(bootstrap::SERIAL_NUMBER, &cfg.serial);

        self.install_genicam_xml(GENICAM_XML, XML_ADDRESS);

        self.write_reg(WIDTH_REG, cfg.width);
        self.write_reg(HEIGHT_REG, cfg.height);
        self.write_reg(ACQ_MODE_REG, 1);
        self.write_reg(PIXEL_FORMAT_REG, gvsp::PixelFormat::MONO8.0);
        self.write_reg(EXPOSURE_REG, cfg.exposure_us.to_bits());
        self.write_reg(GAIN_REG, cfg.gain.to_bits());
    }

    /// Current advertised frame width.
    pub fn width(&self) -> u32 {
        self.read_reg(WIDTH_REG)
    }

    /// Current advertised frame height.
    pub fn height(&self) -> u32 {
        self.read_reg(HEIGHT_REG)
    }

    pub fn read_reg(&self, addr: u32) -> u32 {
        let a = addr as usize;
        u32::from_be_bytes([
            self.mem[a],
            self.mem[a + 1],
            self.mem[a + 2],
            self.mem[a + 3],
        ])
    }

    pub fn write_reg(&mut self, addr: u32, value: u32) {
        let a = addr as usize;
        self.mem[a..a + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn write_string(&mut self, addr: u32, s: &str) {
        let a = addr as usize;
        self.mem[a..a + s.len()].copy_from_slice(s.as_bytes());
    }

    fn install_genicam_xml(&mut self, xml: &str, address: u32) {
        let zipped = build_stored_zip(xml.as_bytes());
        let url = format!("Local:theater.zip;{address:X};{:X}", zipped.len());
        let url_at = bootstrap::XML_URL_0 as usize;
        self.mem[url_at..url_at + bootstrap::XML_URL_SIZE].fill(0);
        self.mem[url_at..url_at + url.len()].copy_from_slice(url.as_bytes());
        let at = address as usize;
        self.mem[at..at + zipped.len()].copy_from_slice(&zipped);
    }

    /// The stream destination the client configured (SCDA:SCP), if any.
    ///
    /// A client that only holds wildcard-bound sockets cannot know a concrete
    /// host IP and writes SCDA=0 alongside a real SCP. A physical camera would
    /// stay silent on that; the emulator instead streams back to the GVCP
    /// requester's IP at SCP, so such clients (snare's virtual network routes
    /// by the literal bound address) still receive the burst.
    pub fn stream_dest(&self) -> Option<SocketAddr> {
        let ip = self.read_reg(bootstrap::STREAM_CHANNEL_DEST_ADDRESS);
        let port = (self.read_reg(bootstrap::STREAM_CHANNEL_PORT) & 0xffff) as u16;
        if port == 0 {
            return None;
        }
        if ip != 0 {
            return Some(SocketAddr::new(Ipv4Addr::from(ip).into(), port));
        }
        self.ctrl_peer.map(|peer| SocketAddr::new(peer.ip(), port))
    }

    /// Clears the acquisition trigger (SingleFrame self-stop).
    pub fn clear_acquisition(&mut self) {
        self.write_reg(ACQ_REG, 0);
    }

    /// Encodes a full SingleFrame GVSP burst (leader → payloads → trailer) for a
    /// `width*height` mono8 buffer, sliced to the negotiated packet size.
    pub fn frame_packets(&self, frame_id: u64, mono8: &[u8]) -> Vec<Vec<u8>> {
        let scps = self.read_reg(bootstrap::STREAM_CHANNEL_PACKET_SIZE) & 0xffff;
        let overhead = gvsp::packet_protocol_overhead(false);
        let block = (scps as usize).saturating_sub(overhead).max(1);
        let (w, h) = (self.width(), self.height());

        let mut packets = Vec::new();
        packets.push(build_leader(frame_id, w, h));
        let chunks = mono8.chunks(block);
        let n = chunks.len() as u32;
        for (i, chunk) in chunks.enumerate() {
            packets.push(build_payload(frame_id, i as u32 + 1, chunk));
        }
        packets.push(build_trailer(frame_id, n + 1));
        packets
    }

    /// Processes one inbound GVCP datagram from `src`, mutating the register
    /// map and returning the acknowledge plus any side effects.
    pub fn handle_datagram(&mut self, data: &[u8], src: SocketAddr) -> Reaction {
        let Some(cmd) = gvcp::Cmd::parse(data) else {
            return Reaction::default();
        };
        self.ctrl_peer = Some(src);
        match cmd.command {
            gvcp::DISCOVERY_CMD => {
                let block = self.mem[..bootstrap::DISCOVERY_DATA_SIZE].to_vec();
                Reaction {
                    reply: Some(ack(
                        gvcp::GvcpStatus::SUCCESS,
                        gvcp::DISCOVERY_ACK,
                        gvcp::DISCOVERY_ID,
                        &block,
                    )),
                    ..Default::default()
                }
            }
            gvcp::READ_REGISTER_CMD => Reaction {
                reply: Some(self.read_registers(&cmd)),
                ..Default::default()
            },
            gvcp::WRITE_REGISTER_CMD => self.write_registers(&cmd),
            gvcp::READ_MEMORY_CMD => Reaction {
                reply: Some(self.read_memory(&cmd)),
                ..Default::default()
            },
            gvcp::WRITE_MEMORY_CMD => Reaction {
                reply: Some(self.write_memory(&cmd)),
                ..Default::default()
            },
            _ => Reaction::default(),
        }
    }

    fn read_registers(&self, cmd: &gvcp::Cmd<'_>) -> Vec<u8> {
        let mut values = Vec::new();
        for chunk in cmd.payload.chunks_exact(4) {
            let addr = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
            if addr + 4 > self.mem.len() {
                return nak(
                    gvcp::GvcpStatus::INVALID_ADDRESS,
                    gvcp::READ_REGISTER_ACK,
                    cmd.req_id,
                );
            }
            values.extend_from_slice(&self.mem[addr..addr + 4]);
        }
        ack(
            gvcp::GvcpStatus::SUCCESS,
            gvcp::READ_REGISTER_ACK,
            cmd.req_id,
            &values,
        )
    }

    fn write_registers(&mut self, cmd: &gvcp::Cmd<'_>) -> Reaction {
        let mut fire_test = None;
        let mut acquisition_started = false;
        for chunk in cmd.payload.chunks_exact(8) {
            let addr = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if addr as usize + 4 > self.mem.len() {
                return Reaction {
                    reply: Some(nak(
                        gvcp::GvcpStatus::INVALID_ADDRESS,
                        gvcp::WRITE_REGISTER_ACK,
                        cmd.req_id,
                    )),
                    ..Default::default()
                };
            }
            let value = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            self.write_reg(addr, value);
            if addr == bootstrap::STREAM_CHANNEL_PACKET_SIZE
                && value & bootstrap::SCPS_FIRE_TEST_PACKET != 0
            {
                fire_test = Some((value & bootstrap::SCPS_PACKET_SIZE_MASK) as u16);
            }
            if addr == ACQ_REG && value == 1 {
                acquisition_started = true;
            }
        }
        let written = (cmd.payload.len() / 8) as u32;
        Reaction {
            reply: Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::WRITE_REGISTER_ACK,
                cmd.req_id,
                &written.to_be_bytes(),
            )),
            fire_test,
            acquisition_started,
        }
    }

    fn read_memory(&self, cmd: &gvcp::Cmd<'_>) -> Vec<u8> {
        let Some(addr) = cmd
            .payload
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
        else {
            return nak(
                gvcp::GvcpStatus::INVALID_HEADER,
                gvcp::READ_MEMORY_ACK,
                cmd.req_id,
            );
        };
        let Some(count) = cmd
            .payload
            .get(4..8)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
        else {
            return nak(
                gvcp::GvcpStatus::INVALID_HEADER,
                gvcp::READ_MEMORY_ACK,
                cmd.req_id,
            );
        };
        let (addr, count) = (addr as usize, count as usize & 0xffff);
        if addr + count > self.mem.len() || count > gvcp::DATA_SIZE_MAX {
            return nak(
                gvcp::GvcpStatus::INVALID_ADDRESS,
                gvcp::READ_MEMORY_ACK,
                cmd.req_id,
            );
        }
        let mut payload = Vec::with_capacity(4 + count);
        payload.extend_from_slice(&(addr as u32).to_be_bytes());
        payload.extend_from_slice(&self.mem[addr..addr + count]);
        ack(
            gvcp::GvcpStatus::SUCCESS,
            gvcp::READ_MEMORY_ACK,
            cmd.req_id,
            &payload,
        )
    }

    fn write_memory(&mut self, cmd: &gvcp::Cmd<'_>) -> Vec<u8> {
        let Some(addr) = cmd
            .payload
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
        else {
            return nak(
                gvcp::GvcpStatus::INVALID_HEADER,
                gvcp::WRITE_MEMORY_ACK,
                cmd.req_id,
            );
        };
        let addr = addr as usize;
        let data = cmd.payload.get(4..).unwrap_or_default();
        if addr + data.len() > self.mem.len() {
            return nak(
                gvcp::GvcpStatus::INVALID_ADDRESS,
                gvcp::WRITE_MEMORY_ACK,
                cmd.req_id,
            );
        }
        self.mem[addr..addr + data.len()].copy_from_slice(data);
        ack(
            gvcp::GvcpStatus::SUCCESS,
            gvcp::WRITE_MEMORY_ACK,
            cmd.req_id,
            &(data.len() as u32).to_be_bytes(),
        )
    }
}

fn ack(status: gvcp::GvcpStatus, answer: u16, ack_id: u16, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&status.0.to_be_bytes());
    buf.extend_from_slice(&answer.to_be_bytes());
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.extend_from_slice(&ack_id.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn nak(status: gvcp::GvcpStatus, answer: u16, ack_id: u16) -> Vec<u8> {
    ack(status, answer, ack_id, &[])
}

fn gvsp_header(frame_id: u64, packet_id: u32, content: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&(frame_id as u16).to_be_bytes());
    let infos = (u32::from(content) << 24) | (packet_id & 0x00ff_ffff);
    buf.extend_from_slice(&infos.to_be_bytes());
    buf
}

fn build_leader(frame_id: u64, width: u32, height: u32) -> Vec<u8> {
    let mut buf = gvsp_header(frame_id, 0, 1);
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&gvsp::PAYLOAD_TYPE_IMAGE.to_be_bytes());
    buf.extend_from_slice(&0u64.to_be_bytes());
    buf.extend_from_slice(&gvsp::PixelFormat::MONO8.0.to_be_bytes());
    buf.extend_from_slice(&width.to_be_bytes());
    buf.extend_from_slice(&height.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf
}

fn build_payload(frame_id: u64, packet_id: u32, block: &[u8]) -> Vec<u8> {
    let mut buf = gvsp_header(frame_id, packet_id, 3);
    buf.extend_from_slice(block);
    buf
}

fn build_trailer(frame_id: u64, packet_id: u32) -> Vec<u8> {
    let mut buf = gvsp_header(frame_id, packet_id, 2);
    buf.extend_from_slice(&gvsp::PAYLOAD_TYPE_IMAGE.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf
}

/// A minimal stored-method (uncompressed) PKZIP archive with one `theater.xml`.
fn build_stored_zip(content: &[u8]) -> Vec<u8> {
    const NAME: &[u8] = b"theater.xml";
    let mut zip = Vec::new();
    zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&[0u8; 8]);
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(NAME.len() as u16).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(NAME);
    zip.extend_from_slice(content);

    let central = zip.len();
    zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&[0u8; 8]);
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(content.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(NAME.len() as u16).to_le_bytes());
    zip.extend_from_slice(&[0u8; 12]);
    zip.extend_from_slice(&[0u8; 4]);
    zip.extend_from_slice(&0u32.to_le_bytes());
    zip.extend_from_slice(NAME);
    let central_size = zip.len() - central;

    zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    zip.extend_from_slice(&[0u8; 4]);
    zip.extend_from_slice(&1u16.to_le_bytes());
    zip.extend_from_slice(&1u16.to_le_bytes());
    zip.extend_from_slice(&(central_size as u32).to_le_bytes());
    zip.extend_from_slice(&(central as u32).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gige::proto::gvsp::{ContentType, GvspView, ImageLeader};

    fn device() -> GigeDevice {
        GigeDevice::new(Ipv4Addr::new(10, 0, 0, 5), &DeviceConfig::default())
    }

    fn client_src() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::new(10, 0, 0, 100).into(), 40010)
    }

    fn write_reg_cmd(addr: u32, value: u32, req_id: u16) -> Vec<u8> {
        let mut buf = vec![0x42, 0x01];
        buf.extend_from_slice(&gvcp::WRITE_REGISTER_CMD.to_be_bytes());
        buf.extend_from_slice(&8u16.to_be_bytes());
        buf.extend_from_slice(&req_id.to_be_bytes());
        buf.extend_from_slice(&addr.to_be_bytes());
        buf.extend_from_slice(&value.to_be_bytes());
        buf
    }

    fn read_reg_cmd(addr: u32, req_id: u16) -> Vec<u8> {
        let mut buf = vec![0x42, 0x01];
        buf.extend_from_slice(&gvcp::READ_REGISTER_CMD.to_be_bytes());
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&req_id.to_be_bytes());
        buf.extend_from_slice(&addr.to_be_bytes());
        buf
    }

    #[test]
    fn read_register_ack_round_trips() {
        let mut dev = device();
        let r = dev.handle_datagram(&read_reg_cmd(WIDTH_REG, 7), client_src());
        let bytes = r.reply.unwrap();
        let ack = gvcp::Ack::parse(&bytes).unwrap();
        assert_eq!(ack.answer, gvcp::READ_REGISTER_ACK);
        assert_eq!(ack.ack_id, 7);
        assert_eq!(ack.register_values().collect::<Vec<_>>(), vec![1280]);
    }

    #[test]
    fn acquisition_start_is_signalled_and_clears() {
        let mut dev = device();
        let r = dev.handle_datagram(&write_reg_cmd(ACQ_REG, 1, 3), client_src());
        assert!(r.acquisition_started);
        assert_eq!(dev.read_reg(ACQ_REG), 1);
        dev.clear_acquisition();
        assert_eq!(dev.read_reg(ACQ_REG), 0);
    }

    #[test]
    fn frame_packets_reassemble_to_the_image() {
        let mut dev = device();
        dev.write_reg(WIDTH_REG, 8);
        dev.write_reg(HEIGHT_REG, 4);
        dev.write_reg(bootstrap::STREAM_CHANNEL_PACKET_SIZE, 36 + 10);
        let image: Vec<u8> = (0..32u32).map(|i| i as u8).collect();
        let packets = dev.frame_packets(1, &image);

        let leader = GvspView::parse(&packets[0]).unwrap();
        assert_eq!(leader.content_type, ContentType::Leader);
        let img = ImageLeader::parse(leader.data).unwrap();
        assert_eq!((img.width, img.height), (8, 4));
        assert_eq!(img.pixel_format, gvsp::PixelFormat::MONO8);

        let mut reassembled = Vec::new();
        for p in &packets[1..packets.len() - 1] {
            let v = GvspView::parse(p).unwrap();
            assert_eq!(v.content_type, ContentType::Payload);
            reassembled.extend_from_slice(v.data);
        }
        assert_eq!(reassembled, image);
        let trailer = GvspView::parse(packets.last().unwrap()).unwrap();
        assert_eq!(trailer.content_type, ContentType::Trailer);
    }

    #[test]
    fn wildcard_scda_streams_to_the_gvcp_requester() {
        let mut dev = device();
        assert_eq!(dev.stream_dest(), None);

        dev.handle_datagram(
            &write_reg_cmd(bootstrap::STREAM_CHANNEL_PORT, 40011, 5),
            client_src(),
        );
        assert_eq!(
            dev.stream_dest(),
            Some(SocketAddr::new(client_src().ip(), 40011)),
            "SCDA=0 must fall back to the requester's IP at SCP"
        );

        let concrete = Ipv4Addr::new(10, 0, 0, 50);
        dev.handle_datagram(
            &write_reg_cmd(
                bootstrap::STREAM_CHANNEL_DEST_ADDRESS,
                u32::from(concrete),
                6,
            ),
            client_src(),
        );
        assert_eq!(
            dev.stream_dest(),
            Some(SocketAddr::new(concrete.into(), 40011)),
            "an explicit SCDA must win over the fallback"
        );
    }

    #[test]
    fn genicam_xml_installed_and_url_points_at_it() {
        let dev = device();
        let url_at = bootstrap::XML_URL_0 as usize;
        let url = String::from_utf8_lossy(&dev.mem[url_at..url_at + 64]);
        assert!(url.starts_with("Local:theater.zip;8000;"));
    }
}
