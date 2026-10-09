//! On-demand single-frame capture: the camera stays idle (zero stream
//! bandwidth) between snaps.
//! `cargo run --example snap <camera-ip> [n-snaps]`

use std::time::{Duration, Instant};

use telegenic::{GenICamera, StreamConfig};

fn main() {
    tracing_subscriber::fmt::init();
    let mut args = std::env::args().skip(1);
    let ip: std::net::IpAddr = args
        .next()
        .expect("usage: snap <camera-ip> [n-snaps]")
        .parse()
        .expect("camera ip");
    let n_snaps: usize = args.next().map_or(3, |s| s.parse().expect("snap count"));

    let mut cam = GenICamera::new(ip);
    cam.connect().expect("connect");

    let mut session = cam
        .snapshot_session(StreamConfig::new())
        .expect("open snapshot session");
    println!(
        "session open on {} (packet size {}), camera idle",
        session.stream().local_addr(),
        session.stream().packet_size()
    );

    for i in 0..n_snaps {
        let start = Instant::now();
        let frame = session.snap(Duration::from_secs(5)).expect("snap");
        println!(
            "snap {i}: frame {} {:?} {}x{} {} ({} bytes) in {:.1} ms",
            frame.frame_id,
            frame.status,
            frame.width,
            frame.height,
            frame.pixel_format,
            frame.received_size,
            start.elapsed().as_secs_f64() * 1e3,
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}
