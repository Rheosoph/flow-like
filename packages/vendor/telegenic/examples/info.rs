//! Connect to a camera and print its identity:
//! `cargo run --example info <camera-ip>`

use std::net::IpAddr;
use std::time::Duration;

use telegenic::gige::GigECamera;
use telegenic::gige::proto::bootstrap;

fn main() {
    tracing_subscriber::fmt::init();
    let ip: IpAddr = std::env::args()
        .nth(1)
        .expect("usage: info <camera-ip>")
        .parse()
        .expect("camera ip");

    let mut cam = GigECamera::new(ip);
    cam.connect().expect("connect");
    let info = cam.device_info().expect("device info").clone();
    println!(
        "{} {} (serial {}, fw {})",
        info.manufacturer, info.model, info.serial, info.device_version
    );
    println!(
        "  GEV {}.{}  capabilities {:#010x}",
        info.spec_version.0,
        info.spec_version.1,
        cam.capabilities().expect("capabilities")
    );

    let url = cam
        .read_memory(bootstrap::XML_URL_0, bootstrap::XML_URL_SIZE as u32)
        .expect("submit")
        .wait()
        .expect("read xml url");
    let end = url.iter().position(|&b| b == 0).unwrap_or(url.len());
    println!("  genicam url: {}", String::from_utf8_lossy(&url[..end]));

    let n_streams = cam
        .read_register(bootstrap::N_STREAM_CHANNELS)
        .expect("submit")
        .wait()
        .expect("read stream count");
    println!("  stream channels: {n_streams}");

    // Hold the connection so a few heartbeats go out.
    std::thread::sleep(Duration::from_secs(3));
    println!("  link stats after 3s: {:?}", cam.stats().expect("stats"));
    assert!(cam.is_connected(), "control should still be held");
    cam.disconnect(Duration::from_millis(500));
    println!("  disconnected cleanly");
}
