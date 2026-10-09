//! Discover GigE Vision cameras: `cargo run --example discover [interface-ip]`

use std::time::Duration;

use telegenic::gige::discovery::{self, DiscoveryConfig};

fn main() {
    tracing_subscriber::fmt::init();
    let arg = std::env::args().nth(1);

    let devices = match arg {
        Some(ip) => {
            let ip = ip.parse().expect("interface ip");
            discovery::discover_on(ip, Duration::from_secs(2)).expect("discovery failed")
        }
        None => discovery::discover(&DiscoveryConfig {
            recv_window: Duration::from_secs(2),
            ..DiscoveryConfig::default()
        })
        .expect("discovery failed"),
    };

    if devices.is_empty() {
        println!("no cameras answered");
        return;
    }
    for d in &devices {
        let mac = d
            .info
            .mac
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        println!(
            "{} {} (serial {}, fw {})",
            d.info.manufacturer, d.info.model, d.info.serial, d.info.device_version
        );
        println!(
            "  ip {}  mask {}  gw {}",
            d.info.ip, d.info.subnet_mask, d.info.gateway
        );
        println!("  mac {mac}  via {} ({})", d.adapter.name, d.adapter.ip);
        println!(
            "  ip config {:#x}  reachable from this subnet: {}",
            d.info.current_ip_config,
            discovery::is_reachable(d)
        );
    }
}
