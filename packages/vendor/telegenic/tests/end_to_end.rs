//! Full-stack tests through `GenICamera`: the fake camera serves a zipped
//! device description over READMEM, features evaluate through the node
//! graph against live registers, acquisition wires PayloadSize into the
//! stream, and message-channel events round-trip.

mod fake_camera;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fake_camera::{FakeCamera, FrameOpts};
use telegenic::gige::proto::bootstrap;
use telegenic::gige::{GigECamera, GigeConfig};
use telegenic::{FrameStatus, GenICamera, PacketSize};

const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<RegisterDescription ModelName="Fake2000" VendorName="FakeWorks"
    xmlns="http://www.genicam.org/GenApi/Version_1_1">
  <Integer Name="Width"><pValue>WidthReg</pValue><Min>8</Min><Max>4096</Max></Integer>
  <IntReg Name="WidthReg">
    <Address>0x2000</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Integer Name="Height"><pValue>HeightReg</pValue></Integer>
  <IntReg Name="HeightReg">
    <Address>0x2004</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <IntSwissKnife Name="PayloadSize">
    <pVariable Name="W">Width</pVariable>
    <pVariable Name="H">Height</pVariable>
    <Formula>W * H</Formula>
  </IntSwissKnife>
  <Command Name="AcquisitionStart">
    <pValue>AcqReg</pValue><CommandValue>1</CommandValue>
  </Command>
  <Command Name="AcquisitionStop">
    <pValue>AcqReg</pValue><CommandValue>0</CommandValue>
  </Command>
  <IntReg Name="AcqReg">
    <Address>0x2008</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Enumeration Name="AcquisitionMode">
    <EnumEntry Name="Continuous"><Value>0</Value></EnumEntry>
    <EnumEntry Name="SingleFrame"><Value>1</Value></EnumEntry>
    <pValue>AcqModeReg</pValue>
  </Enumeration>
  <IntReg Name="AcqModeReg">
    <Address>0x200C</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Port Name="Device"/>
</RegisterDescription>"#;

fn connect(fake: &FakeCamera) -> GigECamera {
    let mut cfg = GigeConfig::new(std::net::Ipv4Addr::LOCALHOST);
    cfg.addr = fake.addr();
    cfg.gvcp_timeout = Duration::from_millis(500);
    cfg.retries = 2;
    let mut cam = GigECamera::with_config(cfg);
    cam.connect().expect("connect to fake camera");
    cam
}

fn setup() -> (FakeCamera, GenICamera) {
    let fake = FakeCamera::start();
    fake.install_genicam_xml(XML, 0x8000);
    fake.write_reg(0x2000, 40); // Width
    fake.write_reg(0x2004, 50); // Height
    let mut cam = GenICamera::from_transport(connect(&fake));
    cam.connect().expect("load feature model");
    (fake, cam)
}

#[test]
fn features_evaluate_against_live_registers() {
    let (fake, mut cam) = setup();

    assert_eq!(cam.get_integer("Width").unwrap(), 40);
    assert_eq!(cam.get_integer("PayloadSize").unwrap(), 2000);
    assert_eq!(cam.integer_bounds("Width").unwrap(), (8, 4096));

    cam.set_integer("Width", 64).unwrap();
    assert_eq!(fake.read_reg(0x2000), 64);
    assert_eq!(cam.get_integer("PayloadSize").unwrap(), 3200);

    assert!(
        cam.has_feature("TLParamsLocked"),
        "injected default missing"
    );
    assert!(!cam.has_feature("NoSuchFeature"));
}

#[test]
fn acquisition_uses_payload_size_and_streams() {
    let (fake, mut cam) = setup();

    let mut stream_cfg = telegenic::StreamConfig::new(); // PayloadSize fills it
    stream_cfg.packet_size = PacketSize::Fixed(536); // 500-byte blocks
    let acq = cam
        .start_acquisition(stream_cfg)
        .expect("start acquisition");
    assert_eq!(
        fake.read_reg(0x2008),
        1,
        "AcquisitionStart must hit the register"
    );

    let payload: Vec<u8> = (0..2000u32).map(|i| i as u8).collect(); // Width*Height = 40*50
    fake.send_gvsp_frame(1, &payload, &FrameOpts::new(500));

    let frame = acq.wait_for(Duration::from_secs(1)).expect("frame");
    assert_eq!(frame.status, FrameStatus::Complete);
    assert_eq!(frame.data(), &payload[..]);

    acq.stop().expect("stop");
    assert_eq!(
        fake.read_reg(0x2008),
        0,
        "AcquisitionStop must hit the register"
    );
}

/// Wait for AcquisitionStart to hit the fake's register, then emit a frame.
fn answer_next_start(fake: &FakeCamera, frame_id: u64, payload: &[u8]) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fake.read_reg(0x2008) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    fake.send_gvsp_frame(frame_id, payload, &FrameOpts::new(500));
}

#[test]
fn snapshot_session_snaps_on_demand() {
    let (fake, mut cam) = setup();

    let mut cfg = telegenic::StreamConfig::new();
    cfg.packet_size = PacketSize::Fixed(536);
    let mut session = cam.snapshot_session(cfg).expect("open session");
    assert_eq!(
        fake.read_reg(0x200C),
        1,
        "session must switch to SingleFrame"
    );
    assert_eq!(fake.read_reg(0x2008), 0, "camera must stay idle until snap");

    let payload: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
    for frame_id in [1u64, 2] {
        fake.write_reg(0x2008, 0); // SingleFrame mode: the device "stops itself"
        std::thread::scope(|s| {
            s.spawn(|| answer_next_start(&fake, frame_id, &payload));
            let frame = session.snap(Duration::from_secs(2)).expect("snap");
            assert_eq!(frame.status, FrameStatus::Complete);
            assert_eq!(frame.frame_id, frame_id);
            assert_eq!(frame.data(), &payload[..]);
        });
    }

    drop(session);
    assert_eq!(fake.read_reg(0x200C), 0, "AcquisitionMode must be restored");
}

#[test]
fn snap_grabs_one_frame_and_tears_down() {
    let (fake, mut cam) = setup();

    let payload: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
    let mut cfg = telegenic::StreamConfig::new();
    cfg.packet_size = PacketSize::Fixed(536);
    std::thread::scope(|s| {
        s.spawn(|| answer_next_start(&fake, 9, &payload));
        let frame = cam.snap(cfg, Duration::from_secs(2)).expect("snap");
        assert_eq!(frame.status, FrameStatus::Complete);
        assert_eq!(frame.data(), &payload[..]);
    });

    assert_eq!(fake.read_reg(0x200C), 0, "AcquisitionMode must be restored");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while fake.read_reg(bootstrap::STREAM_CHANNEL_PORT) != 0 && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        fake.read_reg(bootstrap::STREAM_CHANNEL_PORT),
        0,
        "stream channel must close"
    );
}

#[test]
fn message_channel_events_round_trip() {
    let (fake, cam) = setup();
    let transport = cam.transport();

    let events = transport.events().expect("subscribe events");
    transport.enable_events().expect("enable events");
    assert_ne!(fake.read_reg(bootstrap::MESSAGE_CHANNEL_PORT), 0);

    let acked = fake.send_event(0x77, 0x9001, 123_456_789);
    assert!(acked, "host must acknowledge the event");
    assert_eq!(fake.counters.event_acks.load(Ordering::Relaxed), 1);

    let event = events.wait_for(Duration::from_secs(1)).expect("event");
    assert_eq!(event.event_id, 0x9001);
    assert_eq!(event.timestamp, 123_456_789);

    transport.disable_events().expect("disable events");
    assert_eq!(fake.read_reg(bootstrap::MESSAGE_CHANNEL_PORT), 0);
}

#[test]
fn genicam_xml_is_fetched_and_unzipped() {
    let fake = FakeCamera::start();
    fake.install_genicam_xml(XML, 0x8000);
    let mut cam = connect(&fake);

    let xml = cam.genicam_xml().expect("fetch xml");
    let text = String::from_utf8_lossy(&xml);
    assert!(text.contains("RegisterDescription"));
    assert!(text.contains(r#"IntReg Name="WidthReg""#));
}
