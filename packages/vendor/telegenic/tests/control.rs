//! Integration tests for the GVCP control driver against the fake camera.

mod fake_camera;

use std::sync::atomic::Ordering;
use std::time::Duration;

use fake_camera::{FakeCamera, wait_until};
use telegenic::CameraError;
use telegenic::gige::proto::bootstrap;
use telegenic::gige::{GigECamera, GigeConfig, GvcpStatus};

fn config_for(fake: &FakeCamera) -> GigeConfig {
    let mut cfg = GigeConfig::new(std::net::Ipv4Addr::LOCALHOST);
    cfg.addr = fake.addr();
    // Generous baseline: timing-sensitive tests override it themselves; a
    // loaded CI runner must not turn an ordinary round-trip into a timeout.
    cfg.gvcp_timeout = Duration::from_millis(500);
    cfg.retries = 2;
    cfg
}

fn connect(fake: &FakeCamera) -> GigECamera {
    let mut cam = GigECamera::with_config(config_for(fake));
    cam.connect().expect("connect to fake camera");
    cam
}

#[test]
fn connect_reads_identity_and_takes_control() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    let info = cam.device_info().expect("device info");
    assert_eq!(info.manufacturer, "FakeWorks");
    assert_eq!(info.model, "Fake2000");
    assert_eq!(info.serial, "FK-0001");
    assert_eq!(info.spec_version, (2, 0));
    assert_eq!(info.mac, [0xaa, 0xbb, 0xcc, 0x00, 0x00, 0x01]);

    let capabilities = cam.capabilities().expect("capabilities");
    assert!(capabilities & bootstrap::CAP_PACKET_RESEND != 0);
    assert!(capabilities & bootstrap::CAP_PENDING_ACK != 0);

    assert_eq!(
        fake.read_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE),
        bootstrap::CCP_CONTROL,
        "connect should take (non-exclusive) control"
    );
    assert_eq!(fake.read_reg(bootstrap::HEARTBEAT_TIMEOUT), 3000);
    assert!(cam.is_connected());
}

#[test]
fn single_register_camera_bootstraps_but_short_batch_replies_remain_errors() {
    let fake = FakeCamera::start();
    fake.knobs().lock().single_register_only = true;
    let mut cam = connect(&fake);
    assert_eq!(cam.device_info().unwrap().serial, "FK-0001");
    let error = cam
        .read_registers(vec![bootstrap::VERSION, bootstrap::DEVICE_MODE])
        .unwrap()
        .wait()
        .unwrap_err();
    assert!(matches!(&*error, CameraError::Protocol(_)), "{error}");
    cam.disconnect(Duration::from_millis(500));
    assert_eq!(fake.read_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE), 0);
}

#[test]
fn construction_is_free_and_disconnected() {
    let cam = GigECamera::new(std::net::Ipv4Addr::LOCALHOST);
    assert!(!cam.is_connected());
    assert!(matches!(cam.device_info(), Err(CameraError::Disconnected)));
    assert!(matches!(
        cam.read_register(0),
        Err(CameraError::Disconnected)
    ));
    assert!(cam.stats().is_none());
}

#[test]
fn reconnect_after_disconnect() {
    let fake = FakeCamera::start();
    let mut cam = connect(&fake);
    let initial_commands = cam.stats().expect("stats").commands;

    cam.disconnect(Duration::from_millis(500));
    assert!(!cam.is_connected());
    assert!(matches!(
        cam.read_register(0),
        Err(CameraError::Disconnected)
    ));

    cam.connect().expect("reconnect");
    assert!(cam.is_connected());
    let value = cam
        .read_register(bootstrap::VERSION)
        .expect("submit")
        .wait()
        .expect("read after reconnect");
    assert_eq!(value, 0x0002_0000);
    // Per-connection stats started fresh.
    assert!(cam.stats().expect("stats").commands <= initial_commands + 2);
}

#[test]
fn connect_is_idempotent() {
    let fake = FakeCamera::start();
    let mut cam = connect(&fake);
    let before = cam.stats().expect("stats").commands;
    cam.connect().expect("second connect is a no-op");
    assert_eq!(cam.stats().expect("stats").commands, before);
}

#[test]
fn register_roundtrip() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    cam.write_register(0x2000, 0xdead_beef)
        .expect("submit")
        .wait()
        .expect("write register");
    let value = cam
        .read_register(0x2000)
        .expect("submit")
        .wait()
        .expect("read register");
    assert_eq!(value, 0xdead_beef);

    let values = cam
        .read_registers(vec![0x2000, bootstrap::VERSION])
        .expect("submit")
        .wait()
        .expect("read registers");
    assert_eq!(values, vec![0xdead_beef, 0x0002_0000]);
}

#[test]
fn memory_io_chunks_across_transactions() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    let pattern: Vec<u8> = (0..1500u32).map(|i| (i * 7) as u8).collect();
    fake.write_mem(0x4000, &pattern);
    let read = cam
        .read_memory(0x4000, 1500)
        .expect("submit")
        .wait()
        .expect("read memory");
    assert_eq!(read, pattern);

    let data: Vec<u8> = (0..1024u32).map(|i| (i * 3) as u8).collect();
    cam.write_memory(0x6000, data.clone())
        .expect("submit")
        .wait()
        .expect("write memory");
    assert_eq!(fake.read_mem(0x6000, 1024), data);

    // 1500 B read = 3 chunks, 1024 B write = 2 chunks.
    assert!(cam.stats().expect("stats").acks >= 5);
}

#[test]
fn unaligned_memory_access_fails_fast() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    let err = cam
        .read_memory(0x4001, 8)
        .expect("submit")
        .wait()
        .unwrap_err();
    assert!(matches!(*err, CameraError::Protocol(_)));
    let err = cam
        .write_memory(0x4000, vec![1, 2, 3])
        .expect("submit")
        .wait()
        .unwrap_err();
    assert!(matches!(*err, CameraError::Protocol(_)));
}

#[test]
fn lost_datagrams_are_retried() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    fake.knobs().lock().drop_next = 2;
    let value = cam
        .read_register(bootstrap::VERSION)
        .expect("submit")
        .wait()
        .expect("read after retries");
    assert_eq!(value, 0x0002_0000);
    assert!(
        cam.stats().expect("stats").retries >= 2,
        "stats: {:?}",
        cam.stats()
    );
}

#[test]
fn persistent_loss_times_out() {
    let fake = FakeCamera::start();
    let mut cfg = config_for(&fake);
    cfg.gvcp_timeout = Duration::from_millis(100);
    let mut cam = GigECamera::with_config(cfg);
    cam.connect().expect("connect to fake camera");

    // Exactly the read's three attempts — a budget any larger could eat a
    // heartbeat and tear the connection down.
    fake.knobs().lock().drop_next = 3;
    let err = cam
        .read_register(bootstrap::VERSION)
        .expect("submit")
        .wait()
        .unwrap_err();
    assert!(matches!(*err, CameraError::Timeout), "got {err}");
    assert!(cam.stats().expect("stats").timeouts >= 1);
    fake.knobs().lock().drop_next = 0;
    // The link stays up; the next transaction succeeds.
    assert!(cam.is_connected());
    cam.read_register(bootstrap::VERSION)
        .expect("submit")
        .wait()
        .expect("read after timeout");
}

#[test]
fn pending_ack_extends_the_deadline() {
    let fake = FakeCamera::start();
    let mut cfg = config_for(&fake);
    // Shorter than the device delay so only the PENDING_ACK extension can
    // save the transaction, but with room for a slow runner to deliver it.
    cfg.gvcp_timeout = Duration::from_millis(250);
    cfg.retries = 0;
    let mut cam = GigECamera::with_config(cfg);
    cam.connect().expect("connect");

    // More than the base timeout's worth of delay, bridged by a PENDING_ACK.
    fake.knobs().lock().pending_ack_delay = Some(Duration::from_millis(300));
    let value = cam
        .read_register(bootstrap::VERSION)
        .expect("submit")
        .wait()
        .expect("read with pending ack");
    assert_eq!(value, 0x0002_0000);
    assert!(cam.stats().expect("stats").pending_acks >= 1);
    fake.knobs().lock().pending_ack_delay = None;
}

#[test]
fn device_nak_maps_to_error() {
    let fake = FakeCamera::start();
    let cam = connect(&fake);

    let err = cam
        .read_register(0xfff0_0000)
        .expect("submit")
        .wait()
        .unwrap_err();
    match &*err {
        CameraError::Nak { status, .. } => assert_eq!(*status, GvcpStatus::INVALID_ADDRESS),
        other => panic!("expected Nak, got {other}"),
    }
}

#[test]
fn control_denied_fails_connect() {
    let fake = FakeCamera::start();
    fake.knobs().lock().deny_control = true;
    let mut cam = GigECamera::with_config(config_for(&fake));
    let err = cam.connect().unwrap_err();
    assert!(matches!(err, CameraError::ControlDenied), "got {err}");
    assert!(!cam.is_connected());

    // The camera value stays usable: clear the knob and redial.
    fake.knobs().lock().deny_control = false;
    cam.connect().expect("connect after denial cleared");
    assert!(cam.is_connected());
}

#[test]
fn heartbeat_keeps_control_and_detects_loss() {
    let fake = FakeCamera::start();
    let mut cfg = config_for(&fake);
    cfg.heartbeat_timeout_ms = 90; // worker heartbeats every 30ms
    let mut cam = GigECamera::with_config(cfg);
    cam.connect().expect("connect");

    assert!(
        wait_until(Duration::from_secs(2), || {
            fake.counters.ccp_reads.load(Ordering::Relaxed) >= 3
        }),
        "expected several heartbeats, saw {}",
        fake.counters.ccp_reads.load(Ordering::Relaxed)
    );
    assert!(cam.is_connected());

    fake.clear_ccp();
    assert!(
        wait_until(Duration::from_secs(2), || !cam.is_connected()),
        "control loss should stop the worker"
    );
    let err = cam.read_register(bootstrap::VERSION).unwrap_err();
    assert!(matches!(err, CameraError::ControlLost), "got {err}");

    // connect() doubles as the recovery path from a lost link.
    fake.write_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE, 0);
    cam.connect().expect("reconnect after control loss");
    assert!(cam.is_connected());
}

#[test]
fn disconnect_releases_control() {
    let fake = FakeCamera::start();
    let mut cam = connect(&fake);
    assert_eq!(
        fake.read_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE),
        bootstrap::CCP_CONTROL
    );

    cam.disconnect(Duration::from_millis(500));
    assert!(!cam.is_connected());
    // The release write is fire-and-forget; poll until it lands.
    assert!(
        wait_until(Duration::from_secs(1), || {
            fake.read_reg(bootstrap::CONTROL_CHANNEL_PRIVILEGE) == 0
        }),
        "control privilege never released"
    );
}
