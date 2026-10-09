use crate::{Error, Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModbusReadKind {
    Coils,
    DiscreteInputs,
    HoldingRegisters,
    InputRegisters,
}
impl ModbusReadKind {
    fn function(self) -> u8 {
        match self {
            Self::Coils => 1,
            Self::DiscreteInputs => 2,
            Self::HoldingRegisters => 3,
            Self::InputRegisters => 4,
        }
    }
    fn registers(self) -> bool {
        matches!(self, Self::HoldingRegisters | Self::InputRegisters)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ModbusReadRequest {
    pub unit_id: u8,
    pub kind: ModbusReadKind,
    pub start_address: u16,
    pub count: u16,
    pub timeout_ms: u64,
}
impl ModbusReadRequest {
    pub fn validate(&self) -> Result<()> {
        let limit = if self.kind.registers() { 125 } else { 2000 };
        require(
            self.count > 0 && self.count <= limit,
            "Modbus read count exceeds the function limit",
        )?;
        require(
            self.start_address as u32 + self.count as u32 <= 65536,
            "Modbus register range overflows",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "Modbus timeout must be between 1 and 300000 ms",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", content = "values", rename_all = "snake_case")]
pub enum ModbusValues {
    Registers(Vec<u16>),
    Bits(Vec<bool>),
}

pub struct ModbusTcpClient {
    stream: Option<TcpStream>,
    transaction: u16,
}
impl ModbusTcpClient {
    pub async fn connect(endpoint: &str, timeout_ms: u64) -> Result<Self> {
        require(
            (1..=300_000).contains(&timeout_ms),
            "Modbus timeout must be between 1 and 300000 ms",
        )?;
        let stream = timeout(
            Duration::from_millis(timeout_ms),
            TcpStream::connect(endpoint),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream: Some(stream),
            transaction: 0,
        })
    }

    pub async fn read(&mut self, request: &ModbusReadRequest) -> Result<ModbusValues> {
        request.validate()?;
        let stream = self.stream.as_mut().ok_or_else(|| {
            Error::Invalid(
                "Modbus connection closed after a failed request; reconnect before reading".into(),
            )
        })?;
        self.transaction = self.transaction.wrapping_add(1);
        let result = timeout(
            Duration::from_millis(request.timeout_ms),
            exchange(stream, self.transaction, request),
        )
        .await
        .map_err(|_| Error::Timeout)
        .and_then(|result| result);
        // A timeout can leave a partial frame on the stream. Reusing it could pair stale data with a later sample.
        if result.is_err() {
            self.stream = None;
        }
        result
    }
}

pub async fn read_modbus(endpoint: &str, request: &ModbusReadRequest) -> Result<ModbusValues> {
    request.validate()?;
    let mut client = ModbusTcpClient::connect(endpoint, request.timeout_ms).await?;
    client.read(request).await
}

async fn exchange(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    transaction: u16,
    request: &ModbusReadRequest,
) -> Result<ModbusValues> {
    request.validate()?;
    let mut packet = [0u8; 12];
    packet[0..2].copy_from_slice(&transaction.to_be_bytes());
    packet[4..6].copy_from_slice(&6u16.to_be_bytes());
    packet[6] = request.unit_id;
    packet[7] = request.kind.function();
    packet[8..10].copy_from_slice(&request.start_address.to_be_bytes());
    packet[10..12].copy_from_slice(&request.count.to_be_bytes());
    stream.write_all(&packet).await?;
    let mut header = [0u8; 7];
    stream.read_exact(&mut header).await?;
    require(
        u16::from_be_bytes([header[0], header[1]]) == transaction,
        "Modbus transaction id differs from the request",
    )?;
    require(
        header[2] == 0 && header[3] == 0,
        "Modbus protocol id must be zero",
    )?;
    require(
        header[6] == request.unit_id,
        "Modbus response unit id differs from the request",
    )?;
    let length = u16::from_be_bytes([header[4], header[5]]) as usize;
    require(
        (3..=254).contains(&length),
        "Modbus response length is outside the protocol bounds",
    )?;
    let mut pdu = vec![0u8; length - 1];
    stream.read_exact(&mut pdu).await?;
    if pdu[0] == (request.kind.function() | 0x80) {
        require(pdu.len() == 2, "Invalid Modbus exception response length")?;
        return Err(Error::Exception(pdu[1]));
    }
    require(
        pdu[0] == request.kind.function(),
        "Modbus response function differs from the request",
    )?;
    let expected = if request.kind.registers() {
        request.count as usize * 2
    } else {
        (request.count as usize).div_ceil(8)
    };
    require(
        pdu[1] as usize == expected && pdu.len() == expected + 2,
        "Modbus response byte count differs from the requested values",
    )?;
    if request.kind.registers() {
        Ok(ModbusValues::Registers(
            pdu[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect(),
        ))
    } else {
        Ok(ModbusValues::Bits(
            (0..request.count as usize)
                .map(|i| pdu[2 + i / 8] & (1 << (i % 8)) != 0)
                .collect(),
        ))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegisterEncoding {
    U16,
    I16,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
}
impl RegisterEncoding {
    fn words(self) -> usize {
        match self {
            Self::U16 | Self::I16 => 1,
            Self::U32 | Self::I32 | Self::F32 => 2,
            _ => 4,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WordOrder {
    MostSignificantFirst,
    LeastSignificantFirst,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SensorValue {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

pub fn decode_registers(
    registers: &[u16],
    encoding: RegisterEncoding,
    word_order: WordOrder,
    swap_bytes: bool,
) -> Result<Vec<SensorValue>> {
    let width = encoding.words();
    require(
        !registers.is_empty() && registers.len().is_multiple_of(width),
        "Register count is not divisible by the selected value width",
    )?;
    registers
        .chunks_exact(width)
        .map(|words| {
            let mut bytes = Vec::with_capacity(width * 2);
            for index in 0..width {
                let index = if matches!(word_order, WordOrder::LeastSignificantFirst) {
                    width - index - 1
                } else {
                    index
                };
                let word = if swap_bytes {
                    words[index].swap_bytes()
                } else {
                    words[index]
                };
                bytes.extend_from_slice(&word.to_be_bytes());
            }
            let value = match encoding {
                RegisterEncoding::U16 => SensorValue::Unsigned(u16::from_be_bytes(
                    bytes.as_slice().try_into().unwrap(),
                ) as u64),
                RegisterEncoding::I16 => SensorValue::Signed(i16::from_be_bytes(
                    bytes.as_slice().try_into().unwrap(),
                ) as i64),
                RegisterEncoding::U32 => SensorValue::Unsigned(u32::from_be_bytes(
                    bytes.as_slice().try_into().unwrap(),
                ) as u64),
                RegisterEncoding::I32 => SensorValue::Signed(i32::from_be_bytes(
                    bytes.as_slice().try_into().unwrap(),
                ) as i64),
                RegisterEncoding::F32 => SensorValue::Float(f32::from_be_bytes(
                    bytes.as_slice().try_into().unwrap(),
                ) as f64),
                RegisterEncoding::U64 => {
                    SensorValue::Unsigned(u64::from_be_bytes(bytes.as_slice().try_into().unwrap()))
                }
                RegisterEncoding::I64 => {
                    SensorValue::Signed(i64::from_be_bytes(bytes.as_slice().try_into().unwrap()))
                }
                RegisterEncoding::F64 => {
                    SensorValue::Float(f64::from_be_bytes(bytes.as_slice().try_into().unwrap()))
                }
            };
            if let SensorValue::Float(v) = value {
                require(v.is_finite(), "Decoded sensor value is not finite")?;
            }
            Ok(value)
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum ModbusConnectionConfig {
    Tcp {
        endpoint: String,
        timeout_ms: u64,
    },
    Rtu {
        port: String,
        baud_rate: u32,
        parity: SerialParity,
        stop_bits: SerialStopBits,
        timeout_ms: u64,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SerialParity {
    None,
    Even,
    Odd,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SerialStopBits {
    One,
    Two,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ModbusWriteRequest {
    pub unit_id: u8,
    pub start_address: u16,
    pub values: ModbusValues,
    pub timeout_ms: u64,
}
impl ModbusWriteRequest {
    pub fn validate(&self) -> Result<()> {
        let (count, limit) = match &self.values {
            ModbusValues::Registers(v) => (v.len(), 123),
            ModbusValues::Bits(v) => (v.len(), 1968),
        };
        require(
            count > 0 && count <= limit,
            "Modbus write count exceeds the function limit",
        )?;
        require(
            self.start_address as usize + count <= 65536,
            "Modbus write range overflows",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "Modbus timeout must be between 1 and 300000 ms",
        )
    }
}

/// A serialized request stream. A cancelled or failed exchange closes the connection.
#[cfg(feature = "execute")]
pub struct ModbusClient {
    context: Option<tokio_modbus::client::Context>,
    rtu_gap: Option<Duration>,
    last_finished: Option<tokio::time::Instant>,
}
#[cfg(feature = "execute")]
impl ModbusClient {
    pub async fn connect(config: &ModbusConnectionConfig) -> Result<Self> {
        use tokio_modbus::{
            Slave,
            client::{rtu, tcp},
        };
        let (context, rtu_gap) = match config {
            ModbusConnectionConfig::Tcp {
                endpoint,
                timeout_ms,
            } => {
                require(
                    !endpoint.is_empty() && endpoint.len() <= 4096,
                    "A bounded Modbus endpoint is required",
                )?;
                require(
                    (1..=300_000).contains(timeout_ms),
                    "Modbus timeout must be between 1 and 300000 ms",
                )?;
                let stream = timeout(
                    Duration::from_millis(*timeout_ms),
                    TcpStream::connect(endpoint),
                )
                .await
                .map_err(|_| Error::Timeout)??;
                stream.set_nodelay(true)?;
                (tcp::attach_slave(stream, Slave(1)), None)
            }
            ModbusConnectionConfig::Rtu {
                port,
                baud_rate,
                parity,
                stop_bits,
                timeout_ms,
            } => {
                use tokio_serial::{SerialPort, SerialPortBuilderExt};
                require(
                    !port.is_empty() && port.len() <= 4096,
                    "A bounded serial port path is required",
                )?;
                require(
                    (300..=4_000_000).contains(baud_rate),
                    "Modbus baud rate is outside 300..4000000",
                )?;
                require(
                    (1..=300_000).contains(timeout_ms),
                    "Modbus timeout must be between 1 and 300000 ms",
                )?;
                let parity = match parity {
                    SerialParity::None => tokio_serial::Parity::None,
                    SerialParity::Even => tokio_serial::Parity::Even,
                    SerialParity::Odd => tokio_serial::Parity::Odd,
                };
                let stop_bits = match stop_bits {
                    SerialStopBits::One => tokio_serial::StopBits::One,
                    SerialStopBits::Two => tokio_serial::StopBits::Two,
                };
                let stream = tokio_serial::new(port, *baud_rate)
                    .data_bits(tokio_serial::DataBits::Eight)
                    .parity(parity)
                    .stop_bits(stop_bits)
                    .flow_control(tokio_serial::FlowControl::None)
                    .timeout(Duration::from_millis(*timeout_ms))
                    .open_native_async()
                    .map_err(|e| Error::Invalid(format!("Cannot open Modbus serial port: {e}")))?;
                stream.clear(tokio_serial::ClearBuffer::All).map_err(|e| {
                    Error::Invalid(format!("Cannot clear Modbus serial buffers: {e}"))
                })?;
                let gap = if *baud_rate > 19200 {
                    Duration::from_micros(1750)
                } else {
                    Duration::from_secs_f64(38.5 / *baud_rate as f64)
                };
                (rtu::attach_slave(stream, Slave(1)), Some(gap))
            }
        };
        Ok(Self {
            context: Some(context),
            rtu_gap,
            last_finished: None,
        })
    }
    pub async fn connect_tcp_resolved(
        addrs: &[std::net::SocketAddr],
        timeout_ms: u64,
    ) -> Result<Self> {
        require(
            !addrs.is_empty(),
            "Modbus endpoint did not resolve to an address",
        )?;
        require(
            (1..=300_000).contains(&timeout_ms),
            "Modbus timeout must be between 1 and 300000 ms",
        )?;
        let stream = timeout(Duration::from_millis(timeout_ms), TcpStream::connect(addrs))
            .await
            .map_err(|_| Error::Timeout)??;
        stream.set_nodelay(true)?;
        Ok(Self {
            context: Some(tokio_modbus::client::tcp::attach_slave(
                stream,
                tokio_modbus::Slave(1),
            )),
            rtu_gap: None,
            last_finished: None,
        })
    }
    async fn begin(&mut self, unit_id: u8) -> Result<tokio_modbus::client::Context> {
        use tokio_modbus::prelude::SlaveContext;
        if self.rtu_gap.is_some() {
            require(
                (1..=247).contains(&unit_id),
                "Modbus RTU unit ID must be in 1..247; broadcasts are not supported",
            )?;
        }
        if let (Some(gap), Some(last)) = (self.rtu_gap, self.last_finished) {
            tokio::time::sleep_until(last + gap).await;
        }
        let mut context = self.context.take().ok_or_else(|| {
            Error::Invalid("Modbus session is closed; reconnect before another request".into())
        })?;
        context.set_slave(tokio_modbus::Slave(unit_id));
        Ok(context)
    }
    pub async fn read(&mut self, request: &ModbusReadRequest) -> Result<ModbusValues> {
        use tokio_modbus::{Request, Response, client::Client};
        request.validate()?;
        let mut context = self.begin(request.unit_id).await?;
        let operation = match request.kind {
            ModbusReadKind::Coils => Request::ReadCoils(request.start_address, request.count),
            ModbusReadKind::DiscreteInputs => {
                Request::ReadDiscreteInputs(request.start_address, request.count)
            }
            ModbusReadKind::HoldingRegisters => {
                Request::ReadHoldingRegisters(request.start_address, request.count)
            }
            ModbusReadKind::InputRegisters => {
                Request::ReadInputRegisters(request.start_address, request.count)
            }
        };
        let result = timeout(Duration::from_millis(request.timeout_ms), async {
            // The SDK's Reader helpers only check quantities in debug builds.
            let response = flatten(context.call(operation).await)?;
            match (request.kind, response) {
                (ModbusReadKind::Coils, Response::ReadCoils(mut values))
                | (ModbusReadKind::DiscreteInputs, Response::ReadDiscreteInputs(mut values)) => {
                    require(
                        values.len() == usize::from(request.count).div_ceil(8) * 8,
                        "Modbus response byte count differs from the requested bits",
                    )?;
                    values.truncate(usize::from(request.count));
                    Ok(ModbusValues::Bits(values))
                }
                (ModbusReadKind::HoldingRegisters, Response::ReadHoldingRegisters(values))
                | (ModbusReadKind::InputRegisters, Response::ReadInputRegisters(values)) => {
                    require(
                        values.len() == usize::from(request.count),
                        "Modbus response register count differs from the request",
                    )?;
                    Ok(ModbusValues::Registers(values))
                }
                _ => Err(Error::Invalid(
                    "Modbus response function differs from the request".into(),
                )),
            }
        })
        .await
        .map_err(|_| Error::Timeout)
        .and_then(|r| r);
        self.last_finished = Some(tokio::time::Instant::now());
        if result.is_ok() || matches!(&result, Err(Error::Exception(_))) {
            self.context = Some(context);
        }
        result
    }
    pub async fn write(&mut self, request: &ModbusWriteRequest) -> Result<()> {
        use tokio_modbus::{Request, Response, client::Client};
        request.validate()?;
        let mut context = self.begin(request.unit_id).await?;
        let (operation, expected) = match &request.values {
            ModbusValues::Registers(values) if values.len() == 1 => (
                Request::WriteSingleRegister(request.start_address, values[0]),
                Response::WriteSingleRegister(request.start_address, values[0]),
            ),
            ModbusValues::Registers(values) => (
                Request::WriteMultipleRegisters(request.start_address, values.as_slice().into()),
                Response::WriteMultipleRegisters(request.start_address, values.len() as u16),
            ),
            ModbusValues::Bits(values) if values.len() == 1 => (
                Request::WriteSingleCoil(request.start_address, values[0]),
                Response::WriteSingleCoil(request.start_address, values[0]),
            ),
            ModbusValues::Bits(values) => (
                Request::WriteMultipleCoils(request.start_address, values.as_slice().into()),
                Response::WriteMultipleCoils(request.start_address, values.len() as u16),
            ),
        };
        let result = timeout(Duration::from_millis(request.timeout_ms), async {
            let response = flatten(context.call(operation).await)?;
            require(
                response == expected,
                "Modbus write response differs from the requested address, value, or count",
            )
        })
        .await
        .map_err(|_| Error::Timeout)
        .and_then(|r| r);
        self.last_finished = Some(tokio::time::Instant::now());
        if result.is_ok() || matches!(&result, Err(Error::Exception(_))) {
            self.context = Some(context);
        }
        result
    }
    pub async fn disconnect(&mut self) -> Result<()> {
        use tokio_modbus::client::Client;
        if let Some(mut context) = self.context.take() {
            timeout(Duration::from_secs(5), context.disconnect())
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|e| Error::Invalid(e.to_string()))?;
        }
        Ok(())
    }
}
#[cfg(feature = "execute")]
fn flatten<T>(result: tokio_modbus::Result<T>) -> Result<T> {
    result
        .map_err(|e| Error::Invalid(format!("Modbus exchange failed: {e}")))?
        .map_err(|e| Error::Exception(u8::from(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ModbusReadRequest {
        ModbusReadRequest {
            unit_id: 2,
            kind: ModbusReadKind::HoldingRegisters,
            start_address: 10,
            count: 2,
            timeout_ms: 1000,
        }
    }
    async fn reply(mut server: tokio::io::DuplexStream, response: Vec<u8>) {
        let mut request = [0; 12];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[7..], &[3, 0, 10, 0, 2]);
        for byte in response {
            server.write_all(&[byte]).await.unwrap();
        }
    }
    #[tokio::test]
    async fn tcp_exchange_handles_fragmented_response() {
        let (mut client, server) = tokio::io::duplex(64);
        let task = tokio::spawn(reply(
            server,
            vec![0, 7, 0, 0, 0, 7, 2, 3, 4, 0x12, 0x34, 0xff, 0xfe],
        ));
        assert_eq!(
            exchange(&mut client, 7, &request()).await.unwrap(),
            ModbusValues::Registers(vec![0x1234, 65534])
        );
        task.await.unwrap();
    }
    #[tokio::test]
    async fn malformed_response_and_exception_are_rejected() {
        for response in [
            vec![0, 8, 0, 0, 0, 7, 2, 3, 4, 0, 1, 0, 2],
            vec![0, 7, 0, 1, 0, 7, 2, 3, 4, 0, 1, 0, 2],
            vec![0, 7, 0, 0, 0xff, 0xff, 2],
            vec![0, 7, 0, 0, 0, 3, 2, 0x83, 2],
        ] {
            let (mut client, server) = tokio::io::duplex(64);
            let task = tokio::spawn(reply(server, response));
            assert!(exchange(&mut client, 7, &request()).await.is_err());
            task.await.unwrap();
        }
    }
    #[test]
    fn register_decode_preserves_integer_precision_and_device_orders() {
        assert_eq!(
            decode_registers(
                &[0, 0x3f80],
                RegisterEncoding::F32,
                WordOrder::LeastSignificantFirst,
                false
            )
            .unwrap(),
            vec![SensorValue::Float(1.0)]
        );
        assert_eq!(
            decode_registers(
                &[0x803f, 0],
                RegisterEncoding::F32,
                WordOrder::MostSignificantFirst,
                true
            )
            .unwrap(),
            vec![SensorValue::Float(1.0)]
        );
        assert_eq!(
            decode_registers(
                &[u16::MAX; 4],
                RegisterEncoding::U64,
                WordOrder::MostSignificantFirst,
                false
            )
            .unwrap(),
            vec![SensorValue::Unsigned(u64::MAX)]
        );
        assert!(
            decode_registers(
                &[0],
                RegisterEncoding::F32,
                WordOrder::MostSignificantFirst,
                false
            )
            .is_err()
        );
    }
    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn persistent_tcp_writes_then_reads_on_one_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            for expected_function in [16, 3] {
                let mut header = [0u8; 7];
                stream.read_exact(&mut header).await.unwrap();
                let mut request = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize - 1];
                stream.read_exact(&mut request).await.unwrap();
                assert_eq!(request[0], expected_function);
                assert_eq!(header[6], 2);
                let response = if expected_function == 16 {
                    assert_eq!(request, [16, 0, 10, 0, 2, 4, 0x12, 0x34, 0xab, 0xcd]);
                    vec![16, 0, 10, 0, 2]
                } else {
                    vec![3, 4, 0x12, 0x34, 0xab, 0xcd]
                };
                header[4..6].copy_from_slice(&((response.len() + 1) as u16).to_be_bytes());
                stream.write_all(&header).await.unwrap();
                stream.write_all(&response).await.unwrap();
            }
        });
        let mut client = ModbusClient::connect(&ModbusConnectionConfig::Tcp {
            endpoint,
            timeout_ms: 1000,
        })
        .await
        .unwrap();
        client
            .write(&ModbusWriteRequest {
                unit_id: 2,
                start_address: 10,
                values: ModbusValues::Registers(vec![0x1234, 0xabcd]),
                timeout_ms: 1000,
            })
            .await
            .unwrap();
        assert_eq!(
            client.read(&request()).await.unwrap(),
            ModbusValues::Registers(vec![0x1234, 0xabcd])
        );
        client.disconnect().await.unwrap();
        assert!(client.read(&request()).await.is_err());
        peer.await.unwrap();
    }
    #[cfg(feature = "execute")]
    fn client_with_response(response: Vec<u8>) -> (ModbusClient, tokio::task::JoinHandle<()>) {
        let (transport, mut peer) = tokio::io::duplex(512);
        let client = ModbusClient {
            context: Some(tokio_modbus::client::tcp::attach_slave(
                transport,
                tokio_modbus::Slave(1),
            )),
            rtu_gap: None,
            last_finished: None,
        };
        let task = tokio::spawn(async move {
            let mut header = [0; 7];
            peer.read_exact(&mut header).await.unwrap();
            let mut body = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize - 1];
            peer.read_exact(&mut body).await.unwrap();
            header[4..6].copy_from_slice(&((response.len() + 1) as u16).to_be_bytes());
            peer.write_all(&header).await.unwrap();
            peer.write_all(&response).await.unwrap();
        });
        (client, task)
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn sdk_reads_reject_short_and_excess_quantities_without_debug_assertions() {
        for kind in [
            ModbusReadKind::Coils,
            ModbusReadKind::DiscreteInputs,
            ModbusReadKind::HoldingRegisters,
            ModbusReadKind::InputRegisters,
        ] {
            let count = if kind.registers() { 2 } else { 9 };
            let invalid_sizes = if kind.registers() { [2, 6] } else { [1, 3] };
            for size in invalid_sizes {
                let mut response = vec![kind.function(), size];
                response.resize(usize::from(size) + 2, 0);
                let (mut client, peer) = client_with_response(response);
                assert!(
                    client
                        .read(&ModbusReadRequest {
                            kind,
                            count,
                            ..request()
                        })
                        .await
                        .is_err()
                );
                assert!(client.context.is_none());
                peer.await.unwrap();
            }
        }
        // Bit responses include padding to the next byte, which is discarded.
        let (mut client, peer) = client_with_response(vec![1, 2, 0xff, 0xff]);
        assert_eq!(
            client
                .read(&ModbusReadRequest {
                    kind: ModbusReadKind::Coils,
                    count: 9,
                    ..request()
                })
                .await
                .unwrap(),
            ModbusValues::Bits(vec![true; 9])
        );
        peer.await.unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn sdk_writes_validate_address_value_and_quantity_echoes() {
        for (values, function, echo) in [
            (ModbusValues::Registers(vec![42]), 6, 42u16),
            (ModbusValues::Registers(vec![42, 43]), 16, 2),
            (ModbusValues::Bits(vec![true]), 5, 0xff00),
            (ModbusValues::Bits(vec![true, false]), 15, 2),
        ] {
            for (address, echoed, valid) in [(99u16, echo, false), (10, 0, false), (10, echo, true)]
            {
                let mut response = vec![function];
                response.extend_from_slice(&address.to_be_bytes());
                response.extend_from_slice(&echoed.to_be_bytes());
                let (mut client, peer) = client_with_response(response);
                let result = client
                    .write(&ModbusWriteRequest {
                        unit_id: 2,
                        start_address: 10,
                        values: values.clone(),
                        timeout_ms: 1000,
                    })
                    .await;
                assert_eq!(result.is_ok(), valid, "{values:?}: {result:?}");
                assert_eq!(client.context.is_some(), valid);
                peer.await.unwrap();
            }
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn cancelled_exchange_closes_session_instead_of_reusing_partial_reply() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 12];
            stream.read_exact(&mut request).await.unwrap();
            stream.write_all(&request[..2]).await.unwrap();
            let mut eof = [0; 1];
            assert_eq!(stream.read(&mut eof).await.unwrap(), 0);
        });
        let mut client = ModbusClient::connect(&ModbusConnectionConfig::Tcp {
            endpoint,
            timeout_ms: 1000,
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), client.read(&request()))
                .await
                .is_err()
        );
        assert!(
            client
                .read(&request())
                .await
                .unwrap_err()
                .to_string()
                .contains("closed")
        );
        peer.await.unwrap();
    }
    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn rtu_transport_reads_a_register_and_rejects_broadcast() {
        let (transport, mut peer) = tokio::io::duplex(64);
        let mut client = ModbusClient {
            context: Some(tokio_modbus::client::rtu::attach_slave(
                transport,
                tokio_modbus::Slave(1),
            )),
            rtu_gap: Some(Duration::from_micros(1750)),
            last_finished: None,
        };
        let task = tokio::spawn(async move {
            let mut bytes = [0; 8];
            peer.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes[..6], &[1, 3, 0, 0, 0, 1]);
            // CRC bytes are a known Modbus RTU response for register value 42.
            peer.write_all(&[1, 3, 2, 0, 42, 0x39, 0x9b]).await.unwrap();
        });
        let request = ModbusReadRequest {
            unit_id: 1,
            kind: ModbusReadKind::HoldingRegisters,
            start_address: 0,
            count: 1,
            timeout_ms: 1000,
        };
        assert_eq!(
            client.read(&request).await.unwrap(),
            ModbusValues::Registers(vec![42])
        );
        let mut broadcast = request;
        broadcast.unit_id = 0;
        assert!(client.read(&broadcast).await.is_err());
        task.await.unwrap();
    }
    #[test]
    fn invalid_writes_never_reach_the_transport() {
        for values in [
            ModbusValues::Registers(vec![]),
            ModbusValues::Registers(vec![0; 124]),
            ModbusValues::Bits(vec![false; 1969]),
        ] {
            assert!(
                ModbusWriteRequest {
                    unit_id: 1,
                    start_address: 0,
                    values,
                    timeout_ms: 1000
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ModbusWriteRequest {
                unit_id: 1,
                start_address: u16::MAX,
                values: ModbusValues::Registers(vec![1, 2]),
                timeout_ms: 1000
            }
            .validate()
            .is_err()
        );
    }
}
