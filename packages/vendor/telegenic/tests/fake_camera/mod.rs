//! An in-process fake GigE Vision device for integration tests: a
//! register/memory map served over a loopback UDP socket (with knobs for packet loss, pending-ack delays and control
//! denial), plus a GVSP side that answers fire-test packets, sends synthetic
//! frames, and replays cached packets on resend requests.

#![allow(dead_code)]

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use telegenic::gige::proto::bootstrap;
use telegenic::gige::proto::gvcp;

pub const MEM_SIZE: usize = 0x10000;

/// Poll `cond` every 2 ms until it holds or `timeout` elapses. Use instead
/// of a fixed sleep wherever a test waits on worker-thread progress — a
/// loaded CI runner can deschedule a thread far longer than any sleep.
pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return cond();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub struct Knobs {
    /// Answer a multi-register request with only its first value, as Aravis does.
    pub single_register_only: bool,
    /// Drop the next N inbound datagrams (simulated loss).
    pub drop_next: usize,
    /// Answer every command with PENDING_ACK first, then the real ack after
    /// this delay.
    pub pending_ack_delay: Option<Duration>,
    /// Reject CCP writes with ACCESS_DENIED.
    pub deny_control: bool,
    /// Largest SCPS fire-test size answered with a test packet.
    pub mtu: u16,
    /// Replay cached frame packets when a PACKETRESEND arrives.
    pub resend_replay: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            single_register_only: false,
            drop_next: 0,
            pending_ack_delay: None,
            deny_control: false,
            mtu: u16::MAX,
            resend_replay: true,
        }
    }
}

#[derive(Default)]
pub struct Counters {
    pub datagrams: AtomicU64,
    pub ccp_reads: AtomicU64,
    pub resend_requests: AtomicU64,
    pub event_acks: AtomicU64,
}

/// The fake's stream side: its own socket plus a packet cache for resends.
pub struct Gvsp {
    socket: UdpSocket,
    cache: Mutex<Vec<(u64, u32, Vec<u8>)>>,
}

impl Gvsp {
    fn stream_dest(mem: &Mutex<Vec<u8>>) -> Option<SocketAddr> {
        let mem = mem.lock();
        let reg = |addr: u32| {
            let a = addr as usize;
            u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
        };
        let ip = reg(bootstrap::STREAM_CHANNEL_DEST_ADDRESS);
        let port = reg(bootstrap::STREAM_CHANNEL_PORT) & 0xffff;
        if ip == 0 || port == 0 {
            return None;
        }
        Some(SocketAddr::new(
            std::net::Ipv4Addr::from(ip).into(),
            port as u16,
        ))
    }

    fn send(&self, mem: &Mutex<Vec<u8>>, datagram: &[u8]) {
        if let Some(dest) = Self::stream_dest(mem) {
            let _ = self.socket.send_to(datagram, dest);
        }
    }
}

pub struct FakeCamera {
    addr: SocketAddr,
    mem: Arc<Mutex<Vec<u8>>>,
    knobs: Arc<Mutex<Knobs>>,
    pub counters: Arc<Counters>,
    gvsp: Arc<Gvsp>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeCamera {
    pub fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind fake camera");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("set fake camera read timeout");
        let addr = socket.local_addr().expect("fake camera local addr");

        let mem = Arc::new(Mutex::new(seed_memory()));
        let knobs = Arc::new(Mutex::new(Knobs::default()));
        let counters = Arc::new(Counters::default());
        let gvsp = Arc::new(Gvsp {
            socket: UdpSocket::bind("127.0.0.1:0").expect("bind fake gvsp"),
            cache: Mutex::new(Vec::new()),
        });
        let stop = Arc::new(AtomicBool::new(false));

        let thread = std::thread::Builder::new()
            .name("fake-camera-gvcp".into())
            .spawn({
                let mem = mem.clone();
                let knobs = knobs.clone();
                let counters = counters.clone();
                let gvsp = gvsp.clone();
                let stop = stop.clone();
                move || serve(&socket, &mem, &knobs, &counters, &gvsp, &stop)
            })
            .expect("spawn fake camera");

        Self {
            addr,
            mem,
            knobs,
            counters,
            gvsp,
            stop,
            thread: Some(thread),
        }
    }

    /// Send one synthetic image frame as GVSP packets to the configured
    /// stream destination, caching every packet for resend replay.
    pub fn send_gvsp_frame(&self, frame_id: u64, payload: &[u8], opts: &FrameOpts) {
        let mut packets: Vec<(u32, Vec<u8>)> = Vec::new();
        packets.push((
            0,
            build_leader(
                opts.extended_ids,
                frame_id,
                opts.width(payload),
                opts.height,
            ),
        ));
        let blocks = payload.chunks(opts.block_size);
        let n_blocks = blocks.len() as u32;
        for (i, block) in blocks.enumerate() {
            packets.push((
                i as u32 + 1,
                build_payload(opts.extended_ids, frame_id, i as u32 + 1, block),
            ));
        }
        packets.push((
            n_blocks + 1,
            build_trailer(opts.extended_ids, frame_id, n_blocks + 1),
        ));

        {
            let mut cache = self.gvsp.cache.lock();
            cache.retain(|(fid, _, _)| *fid != frame_id);
            for (pid, bytes) in &packets {
                cache.push((frame_id, *pid, bytes.clone()));
            }
        }

        if opts.reverse {
            packets.reverse();
        }
        for (pid, bytes) in &packets {
            if opts.drop.contains(pid) {
                continue;
            }
            self.gvsp.send(&self.mem, bytes);
            if opts.duplicate.contains(pid) {
                self.gvsp.send(&self.mem, bytes);
            }
        }
    }

    pub fn send_gvsp_raw(&self, datagram: &[u8]) {
        self.gvsp.send(&self.mem, datagram);
    }

    /// Emit an EVENT_CMD over the message channel (MCDA:MCP registers) and
    /// wait briefly for the host's EVENT_ACK. Returns whether it was acked.
    pub fn send_event(&self, req_id: u16, event_id: u16, timestamp: u64) -> bool {
        let dest = {
            let mem = self.mem.lock();
            let reg = |addr: u32| {
                let a = addr as usize;
                u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
            };
            let ip = reg(bootstrap::MESSAGE_CHANNEL_DEST_ADDRESS);
            let port = reg(bootstrap::MESSAGE_CHANNEL_PORT) & 0xffff;
            if ip == 0 || port == 0 {
                return false;
            }
            SocketAddr::new(std::net::Ipv4Addr::from(ip).into(), port as u16)
        };
        let mut pkt = Vec::with_capacity(24);
        pkt.extend_from_slice(&[0x42, 0x01]); // CMD, ACK_REQUIRED
        pkt.extend_from_slice(&gvcp::EVENT_CMD.to_be_bytes());
        pkt.extend_from_slice(&16u16.to_be_bytes());
        pkt.extend_from_slice(&req_id.to_be_bytes());
        pkt.extend_from_slice(&0u16.to_be_bytes()); // reserved
        pkt.extend_from_slice(&event_id.to_be_bytes());
        pkt.extend_from_slice(&0u16.to_be_bytes()); // stream channel
        pkt.extend_from_slice(&0u16.to_be_bytes()); // block id
        pkt.extend_from_slice(&timestamp.to_be_bytes());
        if self.gvsp.socket.send_to(&pkt, dest).is_err() {
            return false;
        }
        self.gvsp
            .socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok();
        let mut buf = [0u8; 64];
        while let Ok((n, _)) = self.gvsp.socket.recv_from(&mut buf) {
            if let Some(ack) = gvcp::Ack::parse(&buf[..n])
                && ack.answer == gvcp::EVENT_ACK
                && ack.ack_id == req_id
            {
                self.counters.event_acks.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Place a zipped GenICam XML in device memory at `address` and write a
    /// `Local:` URL for it into URL register 0.
    pub fn install_genicam_xml(&self, xml: &str, address: u32) {
        let zipped = build_test_zip(xml.as_bytes());
        let url = format!("Local:fake.zip;{address:X};{:X}", zipped.len());
        let mut mem = self.mem.lock();
        let url_at = bootstrap::XML_URL_0 as usize;
        mem[url_at..url_at + bootstrap::XML_URL_SIZE].fill(0);
        mem[url_at..url_at + url.len()].copy_from_slice(url.as_bytes());
        mem[address as usize..address as usize + zipped.len()].copy_from_slice(&zipped);
    }

    /// The fake device's GVCP endpoint, to use as `GigeConfig::addr`.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn knobs(&self) -> &Mutex<Knobs> {
        &self.knobs
    }

    pub fn read_reg(&self, addr: u32) -> u32 {
        let mem = self.mem.lock();
        let a = addr as usize;
        u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
    }

    pub fn write_reg(&self, addr: u32, value: u32) {
        let mut mem = self.mem.lock();
        mem[addr as usize..addr as usize + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn read_mem(&self, addr: u32, len: usize) -> Vec<u8> {
        self.mem.lock()[addr as usize..addr as usize + len].to_vec()
    }

    pub fn write_mem(&self, addr: u32, data: &[u8]) {
        self.mem.lock()[addr as usize..addr as usize + data.len()].copy_from_slice(data);
    }

    /// Simulate another application stealing control.
    pub fn clear_ccp(&self) {
        self.write_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE, 0);
    }
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// How [`FakeCamera::send_gvsp_frame`] mangles the packet sequence.
pub struct FrameOpts {
    pub extended_ids: bool,
    /// Image bytes per payload packet. The receiver must be configured with
    /// the matching SCPS: `block_size + 36` (standard ids) or `+ 48`
    /// (extended ids).
    pub block_size: usize,
    /// Packet ids to withhold (until a resend replays them).
    pub drop: Vec<u32>,
    /// Packet ids to send twice.
    pub duplicate: Vec<u32>,
    /// Send the packets in reverse order.
    pub reverse: bool,
    pub height: u32,
}

impl FrameOpts {
    pub fn new(block_size: usize) -> Self {
        Self {
            extended_ids: false,
            block_size,
            drop: Vec::new(),
            duplicate: Vec::new(),
            reverse: false,
            height: 1,
        }
    }

    fn width(&self, payload: &[u8]) -> u32 {
        payload.len() as u32 / self.height.max(1)
    }
}

pub const FAKE_TIMESTAMP_TICKS: u64 = 125_000_000;

/// A minimal stored-method (uncompressed) PKZIP archive with one file.
fn build_test_zip(content: &[u8]) -> Vec<u8> {
    const NAME: &[u8] = b"fake.xml";
    let mut zip = Vec::new();
    zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    zip.extend_from_slice(&20u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes()); // stored
    zip.extend_from_slice(&[0u8; 8]); // time/date/crc (reader ignores crc)
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
    zip.extend_from_slice(&0u16.to_le_bytes()); // stored
    zip.extend_from_slice(&[0u8; 8]); // time/date/crc
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

fn gvsp_header(extended_ids: bool, frame_id: u64, packet_id: u32, content: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&0u16.to_be_bytes()); // status: success
    if extended_ids {
        buf.extend_from_slice(&0u16.to_be_bytes()); // flags
        let infos = 0x8000_0000 | (u32::from(content) << 24);
        buf.extend_from_slice(&infos.to_be_bytes());
        buf.extend_from_slice(&frame_id.to_be_bytes());
        buf.extend_from_slice(&packet_id.to_be_bytes());
    } else {
        buf.extend_from_slice(&(frame_id as u16).to_be_bytes());
        let infos = (u32::from(content) << 24) | (packet_id & 0x00ff_ffff);
        buf.extend_from_slice(&infos.to_be_bytes());
    }
    buf
}

pub fn build_leader(extended_ids: bool, frame_id: u64, width: u32, height: u32) -> Vec<u8> {
    let mut buf = gvsp_header(extended_ids, frame_id, 0, 1);
    buf.extend_from_slice(&0u16.to_be_bytes()); // flags
    buf.extend_from_slice(&1u16.to_be_bytes()); // payload type: image
    buf.extend_from_slice(&FAKE_TIMESTAMP_TICKS.to_be_bytes());
    buf.extend_from_slice(&0x0108_0001u32.to_be_bytes()); // Mono8
    buf.extend_from_slice(&width.to_be_bytes());
    buf.extend_from_slice(&height.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes()); // x offset
    buf.extend_from_slice(&0u32.to_be_bytes()); // y offset
    buf.extend_from_slice(&0u16.to_be_bytes()); // x padding
    buf.extend_from_slice(&0u16.to_be_bytes()); // y padding
    buf
}

pub fn build_payload(extended_ids: bool, frame_id: u64, packet_id: u32, block: &[u8]) -> Vec<u8> {
    let mut buf = gvsp_header(extended_ids, frame_id, packet_id, 3);
    buf.extend_from_slice(block);
    buf
}

pub fn build_trailer(extended_ids: bool, frame_id: u64, packet_id: u32) -> Vec<u8> {
    let mut buf = gvsp_header(extended_ids, frame_id, packet_id, 2);
    buf.extend_from_slice(&1u32.to_be_bytes()); // payload type
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf
}

fn seed_memory() -> Vec<u8> {
    let mut mem = vec![0u8; MEM_SIZE];
    let mut reg = |addr: u32, value: u32| {
        mem[addr as usize..addr as usize + 4].copy_from_slice(&value.to_be_bytes());
    };
    reg(bootstrap::VERSION, 0x0002_0000); // GEV 2.0
    reg(
        bootstrap::GVCP_CAPABILITY,
        bootstrap::CAP_WRITE_MEMORY
            | bootstrap::CAP_PACKET_RESEND
            | bootstrap::CAP_EVENT
            | bootstrap::CAP_PENDING_ACK,
    );
    reg(bootstrap::HEARTBEAT_TIMEOUT, 3000);
    reg(
        bootstrap::CURRENT_IP_ADDRESS,
        u32::from(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    );
    reg(
        bootstrap::CURRENT_SUBNET_MASK,
        u32::from(std::net::Ipv4Addr::new(255, 0, 0, 0)),
    );
    mem[0x0a..0x10].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0x00, 0x00, 0x01]);
    let mut string = |addr: u32, s: &str| {
        mem[addr as usize..addr as usize + s.len()].copy_from_slice(s.as_bytes());
    };
    string(bootstrap::MANUFACTURER_NAME, "FakeWorks");
    string(bootstrap::MODEL_NAME, "Fake2000");
    string(bootstrap::DEVICE_VERSION, "1.0.0");
    string(bootstrap::SERIAL_NUMBER, "FK-0001");
    mem
}

fn serve(
    socket: &UdpSocket,
    mem: &Mutex<Vec<u8>>,
    knobs: &Mutex<Knobs>,
    counters: &Counters,
    gvsp: &Gvsp,
    stop: &AtomicBool,
) {
    let mut buf = [0u8; 0xffff];
    while !stop.load(Ordering::Relaxed) {
        let (n, src) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return,
        };
        counters.datagrams.fetch_add(1, Ordering::Relaxed);
        {
            let mut k = knobs.lock();
            if k.drop_next > 0 {
                k.drop_next -= 1;
                continue;
            }
        }
        let Some(cmd) = gvcp::Cmd::parse(&buf[..n]) else {
            continue;
        };

        let pending = knobs.lock().pending_ack_delay;
        if let Some(delay) = pending {
            let extension = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX) * 2;
            let _ = socket.send_to(
                &ack(
                    gvcp::GvcpStatus::SUCCESS,
                    gvcp::PENDING_ACK,
                    cmd.req_id,
                    &extension.to_be_bytes(),
                ),
                src,
            );
            std::thread::sleep(delay);
        }

        let reply = handle_cmd(&cmd, mem, knobs, counters, gvsp);
        if let Some(reply) = reply {
            let _ = socket.send_to(&reply, src);
        }
    }
}

fn handle_cmd(
    cmd: &gvcp::Cmd<'_>,
    mem: &Mutex<Vec<u8>>,
    knobs: &Mutex<Knobs>,
    counters: &Counters,
    gvsp: &Gvsp,
) -> Option<Vec<u8>> {
    match cmd.command {
        gvcp::PACKET_RESEND_CMD => {
            counters.resend_requests.fetch_add(1, Ordering::Relaxed);
            let extended = cmd.flags & gvcp::FLAG_EXTENDED_IDS != 0;
            let u32_at = |i: usize| {
                cmd.payload
                    .get(i..i + 4)
                    .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            };
            let (frame_id, first, last) = if extended {
                let frame = cmd.payload.get(12..20).map(|b| {
                    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                })?;
                (frame, u32_at(4)?, u32_at(8)?)
            } else {
                (u64::from(u32_at(0)? & 0xffff), u32_at(4)?, u32_at(8)?)
            };
            if knobs.lock().resend_replay {
                let replay: Vec<Vec<u8>> = gvsp
                    .cache
                    .lock()
                    .iter()
                    .filter(|(fid, pid, _)| *fid == frame_id && (first..=last).contains(pid))
                    .map(|(_, _, bytes)| bytes.clone())
                    .collect();
                for mut bytes in replay {
                    // Mark as a resent packet (status 0x0100).
                    bytes[..2].copy_from_slice(&0x0100u16.to_be_bytes());
                    gvsp.send(mem, &bytes);
                }
            }
            None // resends are never acknowledged
        }
        gvcp::DISCOVERY_CMD => {
            let block = mem.lock()[..bootstrap::DISCOVERY_DATA_SIZE].to_vec();
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::DISCOVERY_ACK,
                gvcp::DISCOVERY_ID,
                &block,
            ))
        }
        gvcp::FORCEIP_CMD => {
            let mac = cmd.payload.get(2..8)?;
            let mut mem = mem.lock();
            if mac != &mem[0x0a..0x10] {
                return None;
            }
            let (ip, mask, gw) = (
                cmd.payload.get(20..24)?.to_vec(),
                cmd.payload.get(36..40)?.to_vec(),
                cmd.payload.get(52..56)?.to_vec(),
            );
            mem[bootstrap::CURRENT_IP_ADDRESS as usize..][..4].copy_from_slice(&ip);
            mem[bootstrap::CURRENT_SUBNET_MASK as usize..][..4].copy_from_slice(&mask);
            mem[bootstrap::CURRENT_GATEWAY as usize..][..4].copy_from_slice(&gw);
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::FORCEIP_ACK,
                cmd.req_id,
                &[],
            ))
        }
        gvcp::READ_REGISTER_CMD => {
            let single_register_only = knobs.lock().single_register_only;
            let mem = mem.lock();
            let mut values = Vec::new();
            for chunk in cmd.payload.chunks_exact(4) {
                let addr = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
                if addr + 4 > mem.len() {
                    return Some(nak(
                        gvcp::GvcpStatus::INVALID_ADDRESS,
                        gvcp::READ_REGISTER_ACK,
                        cmd.req_id,
                    ));
                }
                if addr == bootstrap::CONTROL_CHANNEL_PRIVILEGE as usize {
                    counters.ccp_reads.fetch_add(1, Ordering::Relaxed);
                }
                values.extend_from_slice(&mem[addr..addr + 4]);
                if single_register_only {
                    break;
                }
            }
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::READ_REGISTER_ACK,
                cmd.req_id,
                &values,
            ))
        }
        gvcp::WRITE_REGISTER_CMD => {
            let mut fire_test_size = None;
            {
                let mut mem = mem.lock();
                for chunk in cmd.payload.chunks_exact(8) {
                    let addr =
                        u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
                    if addr + 4 > mem.len() {
                        return Some(nak(
                            gvcp::GvcpStatus::INVALID_ADDRESS,
                            gvcp::WRITE_REGISTER_ACK,
                            cmd.req_id,
                        ));
                    }
                    if addr == bootstrap::CONTROL_CHANNEL_PRIVILEGE as usize
                        && knobs.lock().deny_control
                    {
                        return Some(nak(
                            gvcp::GvcpStatus::ACCESS_DENIED,
                            gvcp::WRITE_REGISTER_ACK,
                            cmd.req_id,
                        ));
                    }
                    mem[addr..addr + 4].copy_from_slice(&chunk[4..8]);

                    let value = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                    if addr == bootstrap::STREAM_CHANNEL_PACKET_SIZE as usize
                        && value & bootstrap::SCPS_FIRE_TEST_PACKET != 0
                    {
                        fire_test_size = Some((value & bootstrap::SCPS_PACKET_SIZE_MASK) as u16);
                    }
                }
            }
            if let Some(size) = fire_test_size
                && size <= knobs.lock().mtu
                && usize::from(size) > 28
            {
                gvsp.send(mem, &vec![0u8; usize::from(size) - 28]);
            }
            let written = (cmd.payload.len() / 8) as u32;
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::WRITE_REGISTER_ACK,
                cmd.req_id,
                &written.to_be_bytes(),
            ))
        }
        gvcp::READ_MEMORY_CMD => {
            let mem = mem.lock();
            let addr = u32::from_be_bytes(cmd.payload.get(..4)?.try_into().ok()?) as usize;
            let count =
                u32::from_be_bytes(cmd.payload.get(4..8)?.try_into().ok()?) as usize & 0xffff;
            if addr + count > mem.len() || count > gvcp::DATA_SIZE_MAX {
                return Some(nak(
                    gvcp::GvcpStatus::INVALID_ADDRESS,
                    gvcp::READ_MEMORY_ACK,
                    cmd.req_id,
                ));
            }
            let mut payload = Vec::with_capacity(4 + count);
            payload.extend_from_slice(&(addr as u32).to_be_bytes());
            payload.extend_from_slice(&mem[addr..addr + count]);
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::READ_MEMORY_ACK,
                cmd.req_id,
                &payload,
            ))
        }
        gvcp::WRITE_MEMORY_CMD => {
            let mut mem = mem.lock();
            let addr = u32::from_be_bytes(cmd.payload.get(..4)?.try_into().ok()?) as usize;
            let data = cmd.payload.get(4..)?;
            if addr + data.len() > mem.len() {
                return Some(nak(
                    gvcp::GvcpStatus::INVALID_ADDRESS,
                    gvcp::WRITE_MEMORY_ACK,
                    cmd.req_id,
                ));
            }
            mem[addr..addr + data.len()].copy_from_slice(data);
            let index = (data.len() as u32).to_be_bytes();
            Some(ack(
                gvcp::GvcpStatus::SUCCESS,
                gvcp::WRITE_MEMORY_ACK,
                cmd.req_id,
                &index,
            ))
        }
        _ => None,
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
