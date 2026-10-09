#![cfg(feature = "execute")]

use flow_like_industrial::{
    Error, ModbusClient, ModbusConnectionConfig, ModbusReadKind, ModbusReadRequest, ModbusValues,
    ModbusWriteRequest, SerialParity, SerialStopBits,
};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

const TIMEOUT_MS: u64 = 2_000;

fn config() -> ModbusConnectionConfig {
    ModbusConnectionConfig::Rtu {
        port: std::env::var("FLOW_LIKE_MODBUS_RTU_E2E_PORT")
            .expect("Set FLOW_LIKE_MODBUS_RTU_E2E_PORT to the Docker RTU fixture PTY"),
        baud_rate: 19200,
        parity: SerialParity::None,
        stop_bits: SerialStopBits::Two,
        timeout_ms: TIMEOUT_MS,
    }
}

fn state_dir() -> PathBuf {
    std::env::var("FLOW_LIKE_MODBUS_RTU_E2E_STATE")
        .expect("Set FLOW_LIKE_MODBUS_RTU_E2E_STATE to the PyModbus observation directory")
        .into()
}

fn observed() -> Value {
    serde_json::from_slice(&std::fs::read(state_dir().join("observed.json")).unwrap()).unwrap()
}

async fn wait_observed(predicate: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = observed();
            if predicate(&value) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("PyModbus did not observe the expected request or datastore values")
}

fn read_request(kind: ModbusReadKind, start_address: u16, count: u16) -> ModbusReadRequest {
    ModbusReadRequest {
        unit_id: 7,
        kind,
        start_address,
        count,
        timeout_ms: TIMEOUT_MS,
    }
}

#[tokio::test]
#[ignore = "requires the isolated Docker PyModbus RTU fixture and explicit serial/state paths"]
async fn pymodbus_rtu_reads_each_table_and_addresses_units() {
    let mut client = ModbusClient::connect(&config()).await.unwrap();
    let cases = [
        (
            read_request(ModbusReadKind::Coils, 0, 13),
            ModbusValues::Bits(vec![
                true, false, false, true, true, false, true, false, false, true, false, true, true,
            ]),
        ),
        (
            read_request(ModbusReadKind::DiscreteInputs, 0, 13),
            ModbusValues::Bits(vec![
                false, true, true, false, false, true, false, true, true, false, true, false, false,
            ]),
        ),
        (
            read_request(ModbusReadKind::HoldingRegisters, 0, 8),
            ModbusValues::Registers(vec![0, 1, 0x1234, 0x8000, 0xff00, 0xffff, 0x5aa5, 0xa55a]),
        ),
        (
            read_request(ModbusReadKind::InputRegisters, 0, 8),
            ModbusValues::Registers(vec![
                0x4321, 0x0100, 0x7fff, 0x8001, 0x00ff, 0xfffe, 0x55aa, 0xaa55,
            ]),
        ),
        (
            read_request(ModbusReadKind::Coils, 3, 9),
            ModbusValues::Bits(vec![
                true, true, false, true, false, false, true, false, true,
            ]),
        ),
        (
            ModbusReadRequest {
                unit_id: 1,
                ..read_request(ModbusReadKind::HoldingRegisters, 0, 3)
            },
            ModbusValues::Registers(vec![0x1111; 3]),
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(
            client.read(&request).await.unwrap(),
            expected,
            "{request:?}"
        );
    }
    let observations = observed();
    for function in 1..=4 {
        assert!(
            observations["requests"]
                .as_array()
                .unwrap()
                .contains(&json!({
                    "unit": 7, "function": function,
                }))
        );
    }
    client.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the isolated Docker PyModbus RTU fixture and explicit serial/state paths"]
async fn pymodbus_rtu_writes_update_independent_store_and_survive_reconnect() {
    let mut client = ModbusClient::connect(&config()).await.unwrap();
    // Mutable addresses are separate from the untouched startup oracle at 0..31.
    let cases = [
        (32, ModbusValues::Bits(vec![true]), 5),
        (32, ModbusValues::Bits(vec![false]), 5),
        (
            35,
            ModbusValues::Bits(vec![
                true, false, true, true, false, false, true, false, true,
            ]),
            15,
        ),
        (32, ModbusValues::Registers(vec![0xbeef]), 6),
        (
            35,
            ModbusValues::Registers(vec![0, 0xffff, 0x1234, 0x8001, 0xff00]),
            16,
        ),
    ];
    for (address, values, function) in &cases {
        let before = observed()["requests"].as_array().unwrap().len();
        client
            .write(&ModbusWriteRequest {
                unit_id: 7,
                start_address: *address,
                values: values.clone(),
                timeout_ms: TIMEOUT_MS,
            })
            .await
            .unwrap();
        let (table, expected) = match values {
            ModbusValues::Bits(values) => ("coils", json!(values)),
            ModbusValues::Registers(values) => ("holding_registers", json!(values)),
        };
        wait_observed(|value| {
            let requests = value["requests"].as_array().unwrap();
            let data = value["devices"]["7"][table].as_array().unwrap();
            let expected = expected.as_array().unwrap();
            requests.len() > before
                && requests.last() == Some(&json!({"unit": 7, "function": function}))
                && data[usize::from(*address)..usize::from(*address) + expected.len()]
                    == expected[..]
        })
        .await;
    }

    client.disconnect().await.unwrap();
    client.disconnect().await.unwrap();
    assert!(matches!(
        client
            .read(&read_request(ModbusReadKind::HoldingRegisters, 32, 1))
            .await,
        Err(Error::Invalid(_))
    ));
    let mut reconnected = ModbusClient::connect(&config()).await.unwrap();
    assert_eq!(
        reconnected
            .read(&read_request(ModbusReadKind::HoldingRegisters, 32, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0xbeef])
    );
    assert_eq!(
        reconnected
            .read(&read_request(ModbusReadKind::Coils, 35, 9))
            .await
            .unwrap(),
        cases[2].1
    );
    reconnected.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the isolated Docker PyModbus RTU fixture and explicit serial/state paths"]
async fn pymodbus_rtu_exceptions_unit_timeout_and_bad_crc() {
    let mut client = ModbusClient::connect(&config()).await.unwrap();
    for kind in [
        ModbusReadKind::Coils,
        ModbusReadKind::DiscreteInputs,
        ModbusReadKind::HoldingRegisters,
        ModbusReadKind::InputRegisters,
    ] {
        assert!(matches!(
            client.read(&read_request(kind, 64, 1)).await,
            Err(Error::Exception(2))
        ));
    }
    for values in [
        ModbusValues::Bits(vec![true]),
        ModbusValues::Bits(vec![true, false]),
        ModbusValues::Registers(vec![0xabcd]),
        ModbusValues::Registers(vec![0xabcd, 0x1234]),
    ] {
        assert!(matches!(
            client
                .write(&ModbusWriteRequest {
                    unit_id: 7,
                    start_address: 64,
                    values,
                    timeout_ms: TIMEOUT_MS,
                })
                .await,
            Err(Error::Exception(2))
        ));
    }
    let before = observed()["requests"].as_array().unwrap().len();
    for unit_id in [0, 248, 255] {
        assert!(matches!(
            client
                .read(&ModbusReadRequest {
                    unit_id,
                    ..read_request(ModbusReadKind::InputRegisters, 0, 1)
                })
                .await,
            Err(Error::Invalid(_))
        ));
    }
    // Neither remote exceptions nor local validation errors poison the session.
    assert_eq!(
        client
            .read(&read_request(ModbusReadKind::InputRegisters, 0, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0x4321])
    );
    assert_eq!(observed()["requests"].as_array().unwrap().len(), before + 1);

    assert!(matches!(
        client
            .read(&ModbusReadRequest {
                unit_id: 42,
                timeout_ms: 150,
                ..read_request(ModbusReadKind::InputRegisters, 0, 1)
            })
            .await,
        Err(Error::Timeout)
    ));
    assert!(
        observed()["requests"]
            .as_array()
            .unwrap()
            .contains(&json!({"unit": 42, "function": 4}))
    );
    assert!(matches!(
        client
            .read(&read_request(ModbusReadKind::InputRegisters, 0, 1))
            .await,
        Err(Error::Invalid(_))
    ));

    let mut reconnected = ModbusClient::connect(&config()).await.unwrap();
    let corruptions = observed()["corruptions"].as_u64().unwrap();
    std::fs::write(
        state_dir().join("corrupt-next-crc"),
        b"corrupt one outgoing CRC byte",
    )
    .unwrap();
    let error = reconnected
        .read(&ModbusReadRequest {
            timeout_ms: 250,
            ..read_request(ModbusReadKind::InputRegisters, 0, 1)
        })
        .await
        .expect_err("a damaged RTU frame must never produce a sensor value");
    assert!(
        matches!(error, Error::Timeout | Error::Invalid(_)),
        "{error:?}"
    );
    assert_eq!(observed()["corruptions"].as_u64().unwrap(), corruptions + 1);
    assert!(matches!(
        reconnected
            .read(&read_request(ModbusReadKind::InputRegisters, 0, 1))
            .await,
        Err(Error::Invalid(_))
    ));
    let mut clean = ModbusClient::connect(&config()).await.unwrap();
    assert_eq!(
        clean
            .read(&read_request(ModbusReadKind::InputRegisters, 0, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0x4321])
    );
    clean.disconnect().await.unwrap();
}
