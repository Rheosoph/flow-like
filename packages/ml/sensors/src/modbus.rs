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
}
