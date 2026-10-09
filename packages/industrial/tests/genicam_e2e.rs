#![cfg(feature = "genicam")]

use flow_like_industrial::genicam::{GenicamCaptureConfig, GenicamFrame, capture_genicam};
use std::{
    net::{SocketAddr, SocketAddrV4},
    sync::Mutex,
    time::{Duration, Instant},
};
use telegenic::{GenICamera, gige::GigeConfig};

static CAMERA: Mutex<()> = Mutex::new(());
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

fn config() -> GenicamCaptureConfig {
    GenicamCaptureConfig {
        device_address: std::env::var("INDUSTRIAL_GENICAM_ADDRESS")
            .expect("set INDUSTRIAL_GENICAM_ADDRESS to the independent Aravis camera"),
        local_interface: Some(
            std::env::var("INDUSTRIAL_GENICAM_INTERFACE")
                .expect("set INDUSTRIAL_GENICAM_INTERFACE to the camera-facing IPv4 interface"),
        ),
        frame_timeout_ms: 2_000,
        command_timeout_ms: 300,
        retries: 1,
        max_frame_bytes: (WIDTH * HEIGHT * 3) as usize,
        pixel_format: Some("Mono8".into()),
        exposure_time_us: Some(10_000.0),
    }
}

fn camera(config: &GenicamCaptureConfig) -> GenICamera {
    let address: SocketAddrV4 = config.device_address.parse().unwrap();
    let mut transport = GigeConfig::new(*address.ip());
    transport.addr = SocketAddr::V4(address);
    transport.local_addr = Some(
        format!("{}:0", config.local_interface.as_ref().unwrap())
            .parse()
            .unwrap(),
    );
    transport.gvcp_timeout = Duration::from_millis(config.command_timeout_ms);
    transport.retries = config.retries;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut camera = GenICamera::with_config(transport.clone());
        match camera.connect() {
            Ok(()) => return camera,
            Err(error) => {
                camera.disconnect(Duration::from_millis(300));
                assert!(
                    Instant::now() < deadline,
                    "Aravis fixture unavailable: {error}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn prepare(config: &GenicamCaptureConfig) {
    let mut camera = camera(config);
    assert_eq!(camera.get_string("DeviceVendorName").unwrap(), "Aravis");
    camera.set_integer("Width", WIDTH.into()).unwrap();
    camera.set_integer("Height", HEIGHT.into()).unwrap();
    camera.set_integer("OffsetX", 4).unwrap();
    camera.set_integer("OffsetY", 6).unwrap();
    camera.set_integer("GainRaw", 0).unwrap();
    camera.set_float("ExposureTimeAbs", 10_000.0).unwrap();
    camera.set_enum("TriggerMode", "Off").unwrap();
    camera.set_enum("AcquisitionMode", "Continuous").unwrap();
    camera.disconnect(Duration::from_millis(300));
}

fn assert_geometry(frame: &GenicamFrame, channels: usize) {
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));
    assert_eq!((frame.x_offset, frame.y_offset), (4, 6));
    assert_eq!((frame.x_padding, frame.y_padding), (0, 0));
    assert!(!frame.has_chunks);
    assert_eq!(
        frame.bytes.len(),
        WIDTH as usize * HEIGHT as usize * channels
    );
    assert!(frame.frame_id > 0);
    assert!(frame.timestamp_ticks > 0);
    assert!(frame.timestamp_ns.is_some_and(|timestamp| timestamp > 0));
    assert!(frame.system_timestamp_ns > 0);
}

fn assert_mono_pattern(frame: &GenicamFrame, scale: u64) {
    assert_geometry(frame, 1);
    assert_eq!(frame.pixel_format, 0x0108_0001);
    assert_eq!(frame.pixel_format_name, "Mono8");
    // Aravis's default test pattern is a diagonal ramp shifted by frame ID.
    for (offset, actual) in frame.bytes.iter().enumerate() {
        let x = offset as u64 % u64::from(WIDTH);
        let y = offset as u64 / u64::from(WIDTH);
        let expected = (((x + y + frame.frame_id) % 255) * scale).min(255) as u8;
        assert_eq!(
            *actual, expected,
            "pixel ({x}, {y}), frame {}",
            frame.frame_id
        );
    }
    let rgb = frame.to_rgb8().unwrap();
    assert_eq!(rgb.len(), frame.bytes.len() * 3);
    for (pixel, value) in rgb.chunks_exact(3).zip(&frame.bytes) {
        assert_eq!(pixel, &[*value; 3]);
    }
}

#[test]
#[ignore = "requires an independent Aravis GigE Vision camera and INDUSTRIAL_GENICAM_* settings"]
fn genicam_aravis_mono_pixels_geometry_and_release_reconnect() {
    let _camera = CAMERA.lock().unwrap_or_else(|error| error.into_inner());
    let config = config();
    prepare(&config);
    let first = capture_genicam(&config).expect("capture independent Aravis Mono8 frame");
    assert_mono_pattern(&first, 1);
    let second = capture_genicam(&config).expect("previous capture must release camera control");
    assert_mono_pattern(&second, 1);
    assert_ne!(first.frame_id, second.frame_id);
}

#[test]
#[ignore = "requires an independent Aravis GigE Vision camera and INDUSTRIAL_GENICAM_* settings"]
fn genicam_aravis_rgb_payload_limit_and_feature_error_release_control() {
    let _camera = CAMERA.lock().unwrap_or_else(|error| error.into_inner());
    let mut config = config();
    prepare(&config);
    config.pixel_format = Some("RGB8".into());
    config.max_frame_bytes = (WIDTH * HEIGHT * 3 - 1) as usize;
    let error = capture_genicam(&config).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("payload exceeds configured frame limit"),
        "{error}"
    );

    config.max_frame_bytes += 1;
    let rgb = capture_genicam(&config).expect("payload rejection must release camera control");
    assert_geometry(&rgb, 3);
    assert_eq!(rgb.pixel_format, 0x0218_0014);
    assert_eq!(rgb.to_rgb8().unwrap(), rgb.bytes);
    assert!(rgb.bytes.windows(3).any(|pixel| pixel[0] != pixel[1]));

    config.pixel_format = Some("MissingPixelFormat".into());
    assert!(capture_genicam(&config).is_err());
    config.pixel_format = Some("Mono8".into());
    assert_mono_pattern(
        &capture_genicam(&config).expect("feature errors must release camera control"),
        1,
    );
}

#[test]
#[ignore = "requires an independent Aravis GigE Vision camera and INDUSTRIAL_GENICAM_* settings"]
fn genicam_aravis_legacy_exposure_changes_pixels_and_preserves_setting() {
    let _camera = CAMERA.lock().unwrap_or_else(|error| error.into_inner());
    let mut config = config();
    prepare(&config);
    let mut inspector = camera(&config);
    assert!(inspector.has_feature("ExposureTimeAbs"));
    assert!(!inspector.has_feature("ExposureTime"));
    inspector.disconnect(Duration::from_millis(300));

    config.exposure_time_us = Some(100_000.0);
    let exposed = capture_genicam(&config).expect("set the legacy SFNC exposure feature");
    assert_mono_pattern(&exposed, 2);
    let mut inspector = camera(&config);
    assert_eq!(inspector.get_float("ExposureTimeAbs").unwrap(), 100_000.0);
    inspector.disconnect(Duration::from_millis(300));
}

#[test]
#[ignore = "requires an independent Aravis GigE Vision camera and INDUSTRIAL_GENICAM_* settings"]
fn genicam_aravis_timeout_releases_control_and_next_capture_succeeds() {
    let _camera = CAMERA.lock().unwrap_or_else(|error| error.into_inner());
    let mut config = config();
    prepare(&config);
    let mut controller = camera(&config);
    controller.set_enum("TriggerMode", "On").unwrap();
    controller.set_enum("TriggerSource", "Software").unwrap();
    controller.disconnect(Duration::from_millis(300));
    config.frame_timeout_ms = 100;
    let error = capture_genicam(&config).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no frame arrived within the snap timeout"),
        "{error}"
    );
    prepare(&config);
    config.frame_timeout_ms = 2_000;
    assert_mono_pattern(
        &capture_genicam(&config).expect("timeout must release control and stop acquisition"),
        1,
    );
}
