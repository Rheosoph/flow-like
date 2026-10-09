//! Loopback tests for discovery and force IP, against the fake camera.
//!
//! A synthetic adapter with `broadcast = 127.0.0.1` routes the beacon over
//! loopback — the discovery socket sends to whatever `broadcast` it is given.

mod fake_camera;

use std::net::Ipv4Addr;
use std::time::Duration;

use fake_camera::FakeCamera;
use telegenic::gige::discovery::{self, DiscoveryConfig, ForceIpConfig, NetworkAdapter};
use telegenic::gige::proto::bootstrap;

fn loopback_config(fake: &FakeCamera) -> DiscoveryConfig {
    DiscoveryConfig {
        adapters: Some(vec![NetworkAdapter {
            name: "loopback-test".into(),
            ip: Ipv4Addr::LOCALHOST,
            netmask: Ipv4Addr::new(255, 0, 0, 0),
            broadcast: Ipv4Addr::LOCALHOST,
        }]),
        recv_window: Duration::from_secs(1),
        limited_broadcast: false,
        source_port: 0,
        device_port: fake.addr().port(),
    }
}

#[test]
fn discover_finds_fake_camera() {
    let fake = FakeCamera::start();
    let devices = discovery::discover(&loopback_config(&fake)).expect("discover");

    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(d.info.model, "Fake2000");
    assert_eq!(d.info.mac, [0xaa, 0xbb, 0xcc, 0x00, 0x00, 0x01]);
    assert_eq!(d.info.ip, Ipv4Addr::LOCALHOST);
    assert_eq!(d.from.ip(), std::net::IpAddr::from(Ipv4Addr::LOCALHOST));
    assert!(
        discovery::is_reachable(d),
        "127.0.0.1 is inside 127.0.0.0/8"
    );
}

#[test]
fn force_ip_repoints_the_fake_camera() {
    let fake = FakeCamera::start();
    let cfg = ForceIpConfig {
        discovery: loopback_config(&fake),
        ack_window: Duration::from_secs(1),
    };

    let acked = discovery::force_ip(
        [0xaa, 0xbb, 0xcc, 0x00, 0x00, 0x01],
        Ipv4Addr::new(10, 1, 2, 3),
        Ipv4Addr::new(255, 255, 255, 0),
        Ipv4Addr::new(10, 1, 2, 1),
        &cfg,
    )
    .expect("force ip");
    assert!(acked, "fake camera should acknowledge");

    assert_eq!(
        fake.read_reg(bootstrap::CURRENT_IP_ADDRESS),
        u32::from(Ipv4Addr::new(10, 1, 2, 3))
    );
    assert_eq!(
        fake.read_reg(bootstrap::CURRENT_SUBNET_MASK),
        u32::from(Ipv4Addr::new(255, 255, 255, 0))
    );
    assert_eq!(
        fake.read_reg(bootstrap::CURRENT_GATEWAY),
        u32::from(Ipv4Addr::new(10, 1, 2, 1))
    );
}

#[test]
fn force_ip_ignores_other_macs() {
    let fake = FakeCamera::start();
    let cfg = ForceIpConfig {
        discovery: loopback_config(&fake),
        ack_window: Duration::from_millis(200),
    };

    let acked = discovery::force_ip(
        [0xde, 0xad, 0xbe, 0xef, 0x00, 0x00],
        Ipv4Addr::new(10, 9, 9, 9),
        Ipv4Addr::new(255, 255, 255, 0),
        Ipv4Addr::UNSPECIFIED,
        &cfg,
    )
    .expect("force ip");
    assert!(!acked, "wrong MAC must not be acknowledged");
    assert_eq!(
        fake.read_reg(bootstrap::CURRENT_IP_ADDRESS),
        u32::from(Ipv4Addr::LOCALHOST)
    );
}
