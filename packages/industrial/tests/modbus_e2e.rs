#![cfg(feature = "execute")]

use flow_like_industrial::{
    Error, ModbusClient, ModbusConnectionConfig, ModbusReadKind, ModbusReadRequest,
    ModbusTcpClient, ModbusValues, ModbusWriteRequest,
};

const TIMEOUT_MS: u64 = 5_000;

fn endpoint() -> String {
    std::env::var("FLOW_LIKE_MODBUS_E2E_ADDR").expect(
        "Set FLOW_LIKE_MODBUS_E2E_ADDR to the dedicated PyModbus fixture before running ignored tests",
    )
}

fn config(endpoint: &str) -> ModbusConnectionConfig {
    ModbusConnectionConfig::Tcp {
        endpoint: endpoint.to_owned(),
        timeout_ms: TIMEOUT_MS,
    }
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
#[ignore = "requires the Docker PyModbus fixture and FLOW_LIKE_MODBUS_E2E_ADDR"]
async fn pymodbus_reads_all_tables_with_both_clients() {
    let endpoint = endpoint();
    let mut adapter = ModbusClient::connect(&config(&endpoint)).await.unwrap();
    let mut reader = ModbusTcpClient::connect(&endpoint, TIMEOUT_MS)
        .await
        .unwrap();

    // Startup values are never written by these tests, so reads can run in parallel.
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
            adapter.read(&request).await.unwrap(),
            expected,
            "SDK adapter: {request:?}"
        );
        assert_eq!(
            reader.read(&request).await.unwrap(),
            expected,
            "TCP reader: {request:?}"
        );
    }
    adapter.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the Docker PyModbus fixture and FLOW_LIKE_MODBUS_E2E_ADDR"]
async fn pymodbus_writes_are_visible_to_another_connection_after_reconnect() {
    let endpoint = endpoint();
    let mut writer = ModbusClient::connect(&config(&endpoint)).await.unwrap();
    let mut observer = ModbusTcpClient::connect(&endpoint, TIMEOUT_MS)
        .await
        .unwrap();

    // Addresses 32..63 are reserved for mutation, leaving the read fixtures intact.
    // One value selects FC 5/6; multiple values select FC 15/16.
    let cases = [
        (32, ModbusValues::Bits(vec![true])),
        (32, ModbusValues::Bits(vec![false])),
        (
            35,
            ModbusValues::Bits(vec![
                true, false, true, true, false, false, true, false, true, true, false,
            ]),
        ),
        (32, ModbusValues::Registers(vec![0xbeef])),
        (
            35,
            ModbusValues::Registers(vec![0, 0xffff, 0x1234, 0x8001, 0xff00]),
        ),
    ];

    for (address, values) in &cases {
        writer
            .write(&ModbusWriteRequest {
                unit_id: 7,
                start_address: *address,
                values: values.clone(),
                timeout_ms: TIMEOUT_MS,
            })
            .await
            .unwrap();
        let (kind, count) = match values {
            ModbusValues::Bits(values) => (ModbusReadKind::Coils, values.len()),
            ModbusValues::Registers(values) => (ModbusReadKind::HoldingRegisters, values.len()),
        };
        assert_eq!(
            observer
                .read(&read_request(kind, *address, count as u16))
                .await
                .unwrap(),
            *values,
            "write at {address} must update the independent server datastore"
        );
    }

    writer.disconnect().await.unwrap();
    writer.disconnect().await.unwrap();
    assert!(matches!(
        writer
            .read(&read_request(ModbusReadKind::HoldingRegisters, 32, 1))
            .await,
        Err(Error::Invalid(_))
    ));

    let mut reconnected = ModbusClient::connect(&config(&endpoint)).await.unwrap();
    assert_eq!(
        reconnected
            .read(&read_request(ModbusReadKind::HoldingRegisters, 32, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0xbeef])
    );
    assert_eq!(
        reconnected
            .read(&read_request(ModbusReadKind::Coils, 35, 11))
            .await
            .unwrap(),
        cases[2].1
    );
    reconnected.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the Docker PyModbus fixture and FLOW_LIKE_MODBUS_E2E_ADDR"]
async fn pymodbus_address_exceptions_preserve_adapter_session() {
    let endpoint = endpoint();
    let mut adapter = ModbusClient::connect(&config(&endpoint)).await.unwrap();
    for kind in [
        ModbusReadKind::Coils,
        ModbusReadKind::DiscreteInputs,
        ModbusReadKind::HoldingRegisters,
        ModbusReadKind::InputRegisters,
    ] {
        assert!(matches!(
            adapter.read(&read_request(kind, 64, 1)).await,
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
            adapter
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
    assert_eq!(
        adapter
            .read(&read_request(ModbusReadKind::InputRegisters, 0, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0x4321])
    );
    adapter.disconnect().await.unwrap();

    let mut reader = ModbusTcpClient::connect(&endpoint, TIMEOUT_MS)
        .await
        .unwrap();
    assert!(matches!(
        reader
            .read(&read_request(ModbusReadKind::HoldingRegisters, 63, 2))
            .await,
        Err(Error::Exception(2))
    ));
    // The lightweight reader closes on any failed exchange, including exceptions.
    assert!(matches!(
        reader
            .read(&read_request(ModbusReadKind::HoldingRegisters, 0, 1))
            .await,
        Err(Error::Invalid(_))
    ));
    let mut reader = ModbusTcpClient::connect(&endpoint, TIMEOUT_MS)
        .await
        .unwrap();
    assert_eq!(
        reader
            .read(&read_request(ModbusReadKind::HoldingRegisters, 0, 1))
            .await
            .unwrap(),
        ModbusValues::Registers(vec![0])
    );
}
