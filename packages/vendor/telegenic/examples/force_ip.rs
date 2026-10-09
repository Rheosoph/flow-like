//! Repoint a camera's IP: `cargo run --example force_ip <mac> <ip> <mask> <gateway>`

use telegenic::gige::discovery::{self, ForceIpConfig};

fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [mac, ip, mask, gw] = args.as_slice() else {
        eprintln!("usage: force_ip <aa:bb:cc:dd:ee:ff> <ip> <mask> <gateway>");
        std::process::exit(2);
    };

    let mac = parse_mac(mac).expect("malformed mac");
    let acked = discovery::force_ip(
        mac,
        ip.parse().expect("ip"),
        mask.parse().expect("mask"),
        gw.parse().expect("gateway"),
        &ForceIpConfig::default(),
    )
    .expect("force ip failed");
    println!("force ip sent, acknowledged: {acked}");
}

fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = s.split(':');
    for byte in &mut mac {
        *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}
