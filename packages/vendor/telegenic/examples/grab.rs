//! Full pipeline smoke test: discover/connect, read features, stream frames.
//! `cargo run --example grab <camera-ip> [n-frames]`

use std::time::Duration;

use telegenic::{GenICamera, StreamConfig};

fn main() {
    tracing_subscriber::fmt::init();
    let mut args = std::env::args().skip(1);
    let ip: std::net::IpAddr = args
        .next()
        .expect("usage: grab <camera-ip> [n-frames]")
        .parse()
        .expect("camera ip");
    let n_frames: usize = args.next().map_or(10, |s| s.parse().expect("frame count"));

    let mut cam = GenICamera::new(ip);
    cam.connect().expect("connect");
    {
        let info = cam.transport().device_info().expect("device info");
        println!(
            "{} {} (serial {})",
            info.manufacturer, info.model, info.serial
        );
    }

    for feature in ["Width", "Height", "PayloadSize"] {
        match cam.get_integer(feature) {
            Ok(v) => println!("  {feature} = {v}"),
            Err(e) => println!("  {feature}: {e}"),
        }
    }
    for feature in ["PixelFormat", "AcquisitionMode", "TriggerMode"] {
        match cam.get_enum(feature) {
            Ok(v) => println!("  {feature} = {v}"),
            Err(e) => println!("  {feature}: {e}"),
        }
    }
    match cam.get_float("ExposureTime") {
        Ok(v) => println!("  ExposureTime = {v}"),
        Err(e) => println!("  ExposureTime: {e}"),
    }

    let acq = cam
        .start_acquisition(StreamConfig::new())
        .expect("start acquisition");
    println!(
        "streaming to {} with packet size {}",
        acq.local_addr(),
        acq.packet_size()
    );

    let mut received = 0usize;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while received < n_frames && std::time::Instant::now() < deadline {
        let Some(frame) = acq.wait_for(Duration::from_millis(500)) else {
            continue;
        };
        received += 1;
        let data = frame.data();
        let sum: u64 = data.iter().map(|&b| u64::from(b)).sum();
        println!(
            "frame {:>4}  {:?}  {}x{} {}  {} bytes  ts {} ns  mean {:.1}",
            frame.frame_id,
            frame.status,
            frame.width,
            frame.height,
            frame.pixel_format,
            frame.received_size,
            frame.timestamp_ns,
            sum as f64 / data.len().max(1) as f64,
        );
    }

    let stats = acq.stats();
    acq.stop().expect("stop acquisition");
    println!("stream stats: {stats:#?}");
    println!("link stats: {:?}", cam.transport().stats().expect("stats"));
    assert!(received > 0, "no frames received");
}
