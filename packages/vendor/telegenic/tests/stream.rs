//! Integration tests for the GVSP stream receiver against the fake camera.
//!
//! The fake sends synthetic Mono8 frames; tests use a fixed SCPS of
//! `block + 36` (standard ids) / `block + 48` (extended) so the receiver's
//! block-size math matches the generator's.

mod fake_camera;

use std::time::Duration;

use fake_camera::{FAKE_TIMESTAMP_TICKS, FakeCamera, FrameOpts, wait_until};
use telegenic::gige::{GigECamera, GigeConfig};
use telegenic::{FrameStatus, PacketSize, PayloadKind, PixelFormat, StreamConfig};

const BLOCK: usize = 500;

fn connect(fake: &FakeCamera) -> GigECamera {
    let mut cfg = GigeConfig::new(std::net::Ipv4Addr::LOCALHOST);
    cfg.addr = fake.addr();
    cfg.gvcp_timeout = Duration::from_millis(500);
    cfg.retries = 2;
    let mut cam = GigECamera::with_config(cfg);
    cam.connect().expect("connect to fake camera");
    cam
}

fn stream_config(payload_size: usize, extended: bool) -> StreamConfig {
    let mut cfg = StreamConfig::new();
    cfg.payload_size = Some(payload_size);
    cfg.packet_size = PacketSize::Fixed((BLOCK + if extended { 48 } else { 36 }) as u16);
    cfg
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

#[test]
fn clean_frame_reassembles() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let stream = cam
        .open_stream(stream_config(2000, false))
        .expect("open stream");
    let frames = stream.subscribe(4);

    let payload = pattern(2000);
    fake.send_gvsp_frame(1, &payload, &FrameOpts::new(BLOCK));

    let frame = frames.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.frame_id, 1);
    assert_eq!(frame.payload, PayloadKind::Image { has_chunks: false });
    assert_eq!(frame.pixel_format, PixelFormat::MONO8);
    assert_eq!(frame.width, 2000);
    assert_eq!(frame.height, 1);
    assert_eq!(frame.received_size, 2000);
    assert_eq!(frame.data(), &payload[..]);
    assert_eq!(frame.timestamp_ticks, FAKE_TIMESTAMP_TICKS);

    let stats = stream.stats();
    assert_eq!(stats.completed_frames, 1);
    assert_eq!(stats.resend_requests, 0);
}

#[test]
fn single_register_camera_preserves_timestamp_frequency() {
    use telegenic::gige::proto::bootstrap;

    let fake = FakeCamera::start();
    fake.knobs().lock().single_register_only = true;
    fake.write_reg(bootstrap::TIMESTAMP_TICK_FREQUENCY_HIGH, 0);
    fake.write_reg(bootstrap::TIMESTAMP_TICK_FREQUENCY_LOW, 125_000_000);
    let cam = connect(&fake);
    let stream = cam.open_stream(stream_config(1000, false)).unwrap();
    let frames = stream.subscribe(4);
    fake.send_gvsp_frame(1, &pattern(1000), &FrameOpts::new(BLOCK));
    let frame = frames.wait_for(Duration::from_secs(1)).unwrap();
    assert_eq!(frame.timestamp_ticks, FAKE_TIMESTAMP_TICKS);
    assert_eq!(frame.timestamp_ns, 1_000_000_000);
}

#[test]
fn consecutive_frames_and_wraparound_ids() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let stream = cam
        .open_stream(stream_config(1000, false))
        .expect("open stream");
    let frames = stream.subscribe(16);

    // Cross the 16-bit wrap; id 0 is invalid and skipped by devices.
    for id in [0xfffeu64, 0xffff, 1, 2] {
        fake.send_gvsp_frame(id, &pattern(1000), &FrameOpts::new(BLOCK));
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut got = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while got.len() < 4 && std::time::Instant::now() < deadline {
        if let Some(f) = frames.wait_for(Duration::from_millis(100)) {
            got.push((f.frame_id, f.status));
        }
    }
    assert_eq!(
        got,
        vec![
            (0xfffe, FrameStatus::Complete),
            (0xffff, FrameStatus::Complete),
            (1, FrameStatus::Complete),
            (2, FrameStatus::Complete)
        ]
    );
}

#[test]
fn extended_ids_reassemble() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let stream = cam
        .open_stream(stream_config(1500, true))
        .expect("open stream");
    let frames = stream.subscribe(4);

    let payload = pattern(1500);
    let mut opts = FrameOpts::new(BLOCK);
    opts.extended_ids = true;
    fake.send_gvsp_frame(0x1_0000_0001, &payload, &opts);

    let frame = frames.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.frame_id, 0x1_0000_0001);
    assert_eq!(frame.data(), &payload[..]);
}

#[test]
fn out_of_order_and_duplicates() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let stream = cam
        .open_stream(stream_config(2000, false))
        .expect("open stream");
    let frames = stream.subscribe(4);

    let payload = pattern(2000);
    let mut opts = FrameOpts::new(BLOCK);
    opts.reverse = true;
    opts.duplicate = vec![2, 3];
    fake.send_gvsp_frame(7, &payload, &opts);

    let frame = frames.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.data(), &payload[..]);
    assert_eq!(stream.stats().duplicated_packets, 2);
}

#[test]
fn missing_packets_are_resent() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let mut cfg = stream_config(5000, false);
    cfg.packet_request_ratio = 0.5;
    // Retention isn't under test; on a loaded runner the resend round-trip
    // can exceed the default 100 ms.
    cfg.frame_retention = Duration::from_secs(2);
    let stream = cam.open_stream(cfg).expect("open stream");
    let frames = stream.subscribe(4);

    let payload = pattern(5000); // 10 payload packets + leader + trailer
    let mut opts = FrameOpts::new(BLOCK);
    opts.drop = vec![3, 4, 8];
    fake.send_gvsp_frame(3, &payload, &opts);

    let frame = frames.wait_for(Duration::from_secs(2)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.data(), &payload[..]);

    let stats = stream.stats();
    assert!(stats.resend_requests >= 3, "stats: {stats:?}");
    assert!(stats.resent_packets >= 3, "stats: {stats:?}");
    assert!(
        fake.counters
            .resend_requests
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1,
        "fake saw no resend command"
    );
}

#[test]
fn unanswered_holes_time_out() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let mut cfg = stream_config(2000, false);
    cfg.frame_retention = Duration::from_millis(80);
    let stream = cam.open_stream(cfg).expect("open stream");
    let frames = stream.subscribe(4);

    fake.knobs().lock().resend_replay = false;
    let mut opts = FrameOpts::new(BLOCK);
    opts.drop = vec![2];
    fake.send_gvsp_frame(5, &pattern(2000), &opts);

    let frame = frames.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Timeout);
    // The data extends to the last received packet — the hole does not cut
    // off the tail — while received_size reflects the missing block.
    assert_eq!(frame.data().len(), 2000);
    assert_eq!(frame.received_size, 1500);
    let stats = stream.stats();
    assert!(stats.timed_out_frames >= 1);
    assert!(stats.missing_packets >= 1);
}

#[test]
fn early_trailer_shrinks_the_frame() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    // Buffer sized for 4000 bytes, actual payload only 1000.
    let stream = cam
        .open_stream(stream_config(4000, false))
        .expect("open stream");
    let frames = stream.subscribe(4);

    let payload = pattern(1000);
    fake.send_gvsp_frame(9, &payload, &FrameOpts::new(BLOCK));

    let frame = frames.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.received_size, 1000);
    assert_eq!(frame.data(), &payload[..]);
}

#[test]
fn pool_exhaustion_counts_underruns() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let mut cfg = stream_config(1000, false);
    cfg.n_buffers = 1;
    let stream = cam.open_stream(cfg).expect("open stream");
    let frames = stream.subscribe(8);

    // First frame completes and sits undelivered in the channel, holding the
    // only buffer; the second frame finds the pool empty.
    fake.send_gvsp_frame(1, &pattern(1000), &FrameOpts::new(BLOCK));
    assert!(
        wait_until(Duration::from_secs(2), || {
            stream.stats().completed_frames >= 1
        }),
        "first frame never completed: {:?}",
        stream.stats()
    );
    fake.send_gvsp_frame(2, &pattern(1000), &FrameOpts::new(BLOCK));
    assert!(
        wait_until(Duration::from_secs(2), || stream.stats().underruns >= 1),
        "stats: {:?}",
        stream.stats()
    );

    // Consuming (and dropping) the frame returns the buffer; streaming
    // recovers.
    drop(frames.recv_all());
    fake.send_gvsp_frame(3, &pattern(1000), &FrameOpts::new(BLOCK));
    let frame = frames
        .wait_for(Duration::from_secs(1))
        .expect("frame after recovery");
    assert_eq!(frame.frame_id, 3);
}

#[test]
fn auto_packet_size_negotiates_below_mtu() {
    let fake = FakeCamera::start();
    fake.knobs().lock().mtu = 1400;
    let cam = connect(&fake);

    let mut cfg = StreamConfig::new();
    cfg.payload_size = Some(1000);
    cfg.packet_size = PacketSize::Auto;
    let stream = cam.open_stream(cfg).expect("open stream");

    let size = stream.packet_size();
    // Probes are 16-aligned, so the best reachable value under a 1400 MTU
    // is 1392.
    assert!(
        (1392..=1400).contains(&size),
        "expected negotiation to land just under the 1400 MTU, got {size}"
    );
    // The final write must leave a plain size in SCPS (no fire-test bit).
    assert_eq!(
        fake.read_reg(telegenic::gige::proto::bootstrap::STREAM_CHANNEL_PACKET_SIZE),
        u32::from(size)
    );
}

#[test]
fn closing_the_stream_zeroes_scp() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);
    let stream = cam
        .open_stream(stream_config(1000, false))
        .expect("open stream");
    let port = fake.read_reg(telegenic::gige::proto::bootstrap::STREAM_CHANNEL_PORT);
    assert_ne!(port, 0);

    // Drop waits for the SCP := 0 ack, and the fake writes the register
    // before acking, so the new value is visible immediately.
    drop(stream);
    assert_eq!(
        fake.read_reg(telegenic::gige::proto::bootstrap::STREAM_CHANNEL_PORT),
        0
    );
}
