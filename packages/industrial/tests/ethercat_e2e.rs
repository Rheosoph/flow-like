#![cfg(all(feature = "execute", target_os = "linux"))]

use flow_like_industrial::ethercat::{
    EthercatConfig, EthercatMaster, EthercatOutput, ExpectedDevice, SdoValue, StartupSdo,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INPUT: [u8; 12] = [
    0x00, 0xff, 0x80, 0x7f, 0x34, 0x12, 0x78, 0x56, 0xab, 0xcd, 0xef, 0x42,
];

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set; use tools/test-industrial-e2e.sh --devices")
    })
}

fn config() -> EthercatConfig {
    EthercatConfig {
        interface: required("FLOW_LIKE_ETHERCAT_INTERFACE"),
        cycle_us: 10_000,
        response_timeout_ms: 1000,
        startup_timeout_ms: 30_000,
        expected_devices: vec![ExpectedDevice {
            vendor_id: 0x6a5,
            product_id: 0xdefede,
            revision: Some(0x5a01),
            serial: None,
        }],
        ..Default::default()
    }
}

struct Peer {
    child: Child,
    observation: PathBuf,
}

impl Peer {
    async fn start() -> Self {
        let observation = std::env::temp_dir().join(format!(
            "flow-ethercat-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let child = Command::new(required("FLOW_LIKE_ETHERCAT_PEER_BIN"))
            .arg(required("FLOW_LIKE_ETHERCAT_PEER_INTERFACE"))
            .arg(required("FLOW_LIKE_ETHERCAT_PEER_CONFIG"))
            .arg(&observation)
            .spawn()
            .expect("start independent KickCAT slave");
        let mut peer = Self { child, observation };
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                assert!(
                    peer.child.try_wait().unwrap().is_none(),
                    "KickCAT peer exited before readiness"
                );
                if peer.read().is_some_and(|v| v["ready"] == true) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("KickCAT peer readiness");
        peer
    }

    fn read(&self) -> Option<Value> {
        std::fs::read(&self.observation)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    async fn wait_for(&self, description: &str, check: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(value) = self.read()
                    && check(&value)
                {
                    return value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "Timed out waiting for {description}; peer: {:?}",
                self.read()
            )
        })
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_file(&self.observation);
        let _ = std::fs::remove_file(format!("{}.new", self.observation.display()));
    }
}

async fn wait_for_inputs(master: &EthercatMaster) {
    let mut updates = master.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = updates.borrow().clone();
            assert!(state.error.is_none(), "{state:?}");
            if state.operational && state.devices[0].inputs == INPUT {
                assert_eq!(state.working_counter, 3);
                assert_eq!(state.expected_working_counter, 3);
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .expect("cyclic process inputs from independent slave");
}

#[tokio::test]
#[ignore = "requires the KickCAT Docker fixture and private raw Ethernet interfaces"]
async fn kickcat_startup_sdo_pdo_identity_and_shutdown_interoperate() {
    let peer = Peer::start().await;
    let mut configuration = config();
    let values = [
        SdoValue::U8(0xa5),
        SdoValue::U16(0xbeef),
        SdoValue::U32(0x89abcdef),
        SdoValue::I8(-42),
        SdoValue::I16(-12345),
        SdoValue::I32(-123456789),
    ];
    configuration.startup_sdos = values
        .into_iter()
        .enumerate()
        .map(|(i, value)| StartupSdo {
            device: 0,
            index: 0x2000 + i as u16,
            sub_index: 0,
            value,
        })
        .collect();
    let master = EthercatMaster::start(configuration).await.unwrap();
    assert_eq!(master.devices.len(), 1);
    let device = &master.devices[0];
    assert_eq!(
        (device.vendor_id, device.product_id, device.revision),
        (0x6a5, 0xdefede, 0x5a01)
    );
    assert_eq!((device.input_bytes, device.output_bytes), (12, 3));
    assert_eq!(device.name, "Board");
    wait_for_inputs(&master).await;
    let observed = peer.wait_for("OP", |v| v["state"] == 8).await;
    assert_eq!(
        observed["delayed_eeprom"], true,
        "slow EEPROM reply must be exercised"
    );
    assert_eq!(
        observed["parameters"],
        serde_json::json!([
            [0xa5],
            [0xef, 0xbe],
            [0xef, 0xcd, 0xab, 0x89],
            [0xd6],
            [0xc7, 0xcf],
            [0xeb, 0x32, 0xa4, 0xf8]
        ])
    );
    for state in [1, 2, 4, 8] {
        assert!(
            observed["states"]
                .as_array()
                .unwrap()
                .contains(&state.into()),
            "{observed}"
        );
    }
    let duplicate = EthercatMaster::start(config())
        .await
        .err()
        .expect("interface ownership must be exclusive");
    assert!(duplicate.to_string().contains("already owns"));
    master
        .write(EthercatOutput {
            device: 0,
            offset: 0,
            data: vec![0x00, 0xff, 0x80],
        })
        .await
        .unwrap();
    peer.wait_for("output PDO received by slave", |v| {
        v["outputs"] == serde_json::json!([0, 255, 128])
    })
    .await;
    master
        .write(EthercatOutput {
            device: 0,
            offset: 1,
            data: vec![0x42],
        })
        .await
        .unwrap();
    peer.wait_for("partial output update", |v| {
        v["outputs"] == serde_json::json!([0, 66, 128])
    })
    .await;
    assert!(
        master
            .write(EthercatOutput {
                device: 0,
                offset: 3,
                data: vec![1]
            })
            .await
            .is_err()
    );
    master.close().await.unwrap();
    assert!(master.snapshot().unwrap().stopped);
    peer.wait_for("INIT after close", |v| v["state"] == 1).await;
    assert!(
        master
            .write(EthercatOutput {
                device: 0,
                offset: 0,
                data: vec![1]
            })
            .await
            .is_err()
    );
    let reconnected = EthercatMaster::start(config()).await.unwrap();
    wait_for_inputs(&reconnected).await;
    reconnected.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the KickCAT Docker fixture and private raw Ethernet interfaces"]
async fn kickcat_identity_and_sdo_abort_prevent_operational_startup() {
    let peer = Peer::start().await;
    let mut wrong = config();
    wrong.expected_devices[0].product_id ^= 1;
    let error = EthercatMaster::start(wrong)
        .await
        .err()
        .expect("reject different slave identity");
    assert!(error.to_string().contains("identity"), "{error}");
    let observed = peer
        .wait_for("INIT after rejected identity", |v| v["state"] == 1)
        .await;
    assert!(!observed["states"].as_array().unwrap().contains(&8.into()));
    let mut invalid = config();
    invalid.startup_sdos.push(StartupSdo {
        device: 0,
        index: 0xdead,
        sub_index: 0,
        value: SdoValue::U32(7),
    });
    let error = EthercatMaster::start(invalid)
        .await
        .err()
        .expect("slave must abort unknown SDO");
    assert!(
        error.to_string().to_lowercase().contains("abort"),
        "{error}"
    );
    let observed = peer
        .wait_for("INIT after SDO abort", |v| v["state"] == 1)
        .await;
    assert!(!observed["states"].as_array().unwrap().contains(&8.into()));
    let recovered = EthercatMaster::start(config()).await.unwrap();
    wait_for_inputs(&recovered).await;
    recovered.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the KickCAT Docker fixture and private raw Ethernet interfaces"]
async fn kickcat_peer_loss_stops_worker_and_releases_interface() {
    let mut peer = Peer::start().await;
    let master = EthercatMaster::start(config()).await.unwrap();
    wait_for_inputs(&master).await;
    let mut updates = master.subscribe();
    peer.stop();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if updates.borrow().stopped {
                break;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .expect("master must stop after missing raw Ethernet responses");
    let stopped = updates.borrow().clone();
    assert!(!stopped.operational);
    assert!(stopped.error.is_some());
    assert!(master.snapshot().is_err());
    assert!(master.close().await.is_err());
    assert!(
        master
            .write(EthercatOutput {
                device: 0,
                offset: 0,
                data: vec![1]
            })
            .await
            .is_err()
    );
    let replacement = Peer::start().await;
    let reconnected = EthercatMaster::start(config()).await.unwrap();
    wait_for_inputs(&reconnected).await;
    reconnected.close().await.unwrap();
    replacement
        .wait_for("replacement returned to INIT", |v| v["state"] == 1)
        .await;
}
