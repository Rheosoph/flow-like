//! EtherNet/IP explicit messaging for Allen-Bradley Logix tags.

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CipConfig {
    pub host: String,
    pub port: u16,
    /// Backplane slot for routed ControlLogix connections; absent for direct connections.
    pub slot: Option<u8>,
    pub timeout_ms: u64,
}
impl Default for CipConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 44818,
            slot: None,
            timeout_ms: 5000,
        }
    }
}
impl CipConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.host.trim().is_empty(),
            "EtherNet/IP host must not be empty",
        )?;
        require(self.port > 0, "EtherNet/IP port must be nonzero")?;
        require(
            self.timeout_ms > 0 && self.timeout_ms <= 300_000,
            "EtherNet/IP timeout must be between 1 and 300000 ms",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CipValue {
    Bool(bool),
    Sint(i8),
    Int(i16),
    Dint(i32),
    Lint(i64),
    Usint(u8),
    Uint(u16),
    Udint(u32),
    Ulint(u64),
    Real(f32),
    Lreal(f64),
    String(String),
    Udt { symbol_id: i32, data: Vec<u8> },
}
fn validate_tag(tag: &str) -> Result<()> {
    require(
        !tag.trim().is_empty() && tag.len() <= 1024 && !tag.contains('\0'),
        "Logix tag must contain 1 to 1024 bytes without NUL",
    )
}

#[cfg(any(feature = "execute", test))]
fn needs_indexed_write_workaround(tag: &str) -> bool {
    let Some((base, index)) = tag.strip_suffix(']').and_then(|tag| tag.rsplit_once('[')) else {
        return false;
    };
    !base.contains('[') && !base.contains(']') && index.parse::<u32>().is_ok()
}

impl CipValue {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Real(value) => require(value.is_finite(), "Logix REAL must be finite"),
            Self::Lreal(value) => require(value.is_finite(), "Logix LREAL must be finite"),
            Self::String(value) => require(
                value.len() <= 1_048_576,
                "Logix string exceeds 1048576 bytes",
            ),
            Self::Udt { symbol_id, data } => {
                require(
                    *symbol_id > 0,
                    "Writing a Logix UDT requires its PLC template symbol ID",
                )?;
                require(
                    !data.is_empty() && data.len() <= 1_048_576,
                    "Logix UDT must contain 1 to 1048576 bytes",
                )
            }
            _ => Ok(()),
        }
    }
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use rust_ethernet_ip::{EipClient, PlcValue, RoutePath, UdtData};
    use std::time::Duration;
    use tokio::sync::Mutex;

    pub struct CipClient {
        client: Mutex<Option<EipClient>>,
        timeout: Duration,
    }
    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    impl From<PlcValue> for CipValue {
        fn from(value: PlcValue) -> Self {
            match value {
                PlcValue::Bool(v) => Self::Bool(v),
                PlcValue::Sint(v) => Self::Sint(v),
                PlcValue::Int(v) => Self::Int(v),
                PlcValue::Dint(v) => Self::Dint(v),
                PlcValue::Lint(v) => Self::Lint(v),
                PlcValue::Usint(v) => Self::Usint(v),
                PlcValue::Uint(v) => Self::Uint(v),
                PlcValue::Udint(v) => Self::Udint(v),
                PlcValue::Ulint(v) => Self::Ulint(v),
                PlcValue::Real(v) => Self::Real(v),
                PlcValue::Lreal(v) => Self::Lreal(v),
                PlcValue::String(v) => Self::String(v),
                PlcValue::Udt(v) => Self::Udt {
                    symbol_id: v.symbol_id,
                    data: v.data,
                },
            }
        }
    }
    impl From<CipValue> for PlcValue {
        fn from(value: CipValue) -> Self {
            match value {
                CipValue::Bool(v) => Self::Bool(v),
                CipValue::Sint(v) => Self::Sint(v),
                CipValue::Int(v) => Self::Int(v),
                CipValue::Dint(v) => Self::Dint(v),
                CipValue::Lint(v) => Self::Lint(v),
                CipValue::Usint(v) => Self::Usint(v),
                CipValue::Uint(v) => Self::Uint(v),
                CipValue::Udint(v) => Self::Udint(v),
                CipValue::Ulint(v) => Self::Ulint(v),
                CipValue::Real(v) => Self::Real(v),
                CipValue::Lreal(v) => Self::Lreal(v),
                CipValue::String(v) => Self::String(v),
                CipValue::Udt { symbol_id, data } => Self::Udt(UdtData { symbol_id, data }),
            }
        }
    }
    impl CipClient {
        pub async fn connect(config: CipConfig) -> Result<Self> {
            config.validate()?;
            let timeout = Duration::from_millis(config.timeout_ms);
            let host = config.host.trim_matches(['[', ']']);
            let address = if host.contains(':') {
                format!("[{host}]:{}", config.port)
            } else {
                format!("{host}:{}", config.port)
            };
            let connect = async {
                match config.slot {
                    Some(slot) => {
                        EipClient::with_route_path(&address, RoutePath::new().add_slot(slot)).await
                    }
                    None => EipClient::connect(&address).await,
                }
            };
            let client = tokio::time::timeout(timeout, connect)
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(failure)?;
            Ok(Self {
                client: Mutex::new(Some(client)),
                timeout,
            })
        }
        pub async fn read(&self, tag: &str) -> Result<CipValue> {
            validate_tag(tag)?;
            let mut slot = self.client.lock().await;
            let mut client = slot
                .take()
                .ok_or_else(|| failure("EtherNet/IP session closed; reconnect before retrying"))?;
            // Taking the client prevents reuse after a cancelled or timed-out request.
            let result = tokio::time::timeout(self.timeout, client.read_tag(tag)).await;
            match result {
                Ok(Ok(value)) => {
                    *slot = Some(client);
                    Ok(value.into())
                }
                Ok(Err(error)) => Err(failure(error)),
                Err(_) => Err(Error::Timeout),
            }
        }
        pub async fn write(&self, tag: &str, value: CipValue) -> Result<()> {
            validate_tag(tag)?;
            value.validate()?;
            let mut slot = self.client.lock().await;
            let mut client = slot
                .take()
                .ok_or_else(|| failure("EtherNet/IP session closed; reconnect before retrying"))?;
            let result = tokio::time::timeout(self.timeout, async {
                if needs_indexed_write_workaround(tag) {
                    // The SDK's single indexed-write path substitutes the target's
                    // type for the supplied value and omits structure handles. Its
                    // batch encoder preserves both, including for STRING arrays.
                    // Nested paths retain the SDK's packed BOOL and custom STRING handling.
                    let mut results = client
                        .write_tags_batch(&[(tag, value.into())])
                        .await
                        .map_err(failure)?;
                    require(results.len() == 1, "EtherNet/IP write returned no result")?;
                    results.pop().unwrap().1.map_err(failure)
                } else {
                    client.write_tag(tag, value.into()).await.map_err(failure)
                }
            })
            .await;
            match result {
                Ok(Ok(())) => {
                    *slot = Some(client);
                    Ok(())
                }
                Ok(Err(error)) => Err(failure(error)),
                Err(_) => Err(Error::Timeout),
            }
        }
        pub async fn close(&self) {
            self.client.lock().await.take();
        }
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_write_workaround_preserves_nested_and_multidimensional_paths() {
        for tag in [
            "Values[0]",
            "Program:Main.Values[1]",
            "Motor.Values[4294967295]",
        ] {
            assert!(needs_indexed_write_workaround(tag), "{tag}");
        }
        for tag in [
            "Value",
            "Devices[0].Flags[1]",
            "Devices[0].CustomStrings[1]",
            "Devices[0].Member",
            "Matrix[1,2]",
            "Matrix[1][2]",
            "Values[-1]",
            "Values[4294967296]",
        ] {
            assert!(!needs_indexed_write_workaround(tag), "{tag}");
        }
    }

    #[test]
    fn validates_logix_write_values() {
        assert!(CipValue::Real(f32::NAN).validate().is_err());
        assert!(CipValue::Lreal(f64::INFINITY).validate().is_err());
        assert!(
            CipValue::Udt {
                symbol_id: 0,
                data: vec![1]
            }
            .validate()
            .is_err()
        );
        assert!(validate_tag("Program:Main.Motor[2].Speed").is_ok());
        assert!(validate_tag("Motor\0Speed").is_err());
    }
    #[cfg(feature = "execute")]
    #[test]
    fn preserves_unsigned_and_udt_values() {
        for value in [
            CipValue::Ulint(u64::MAX),
            CipValue::Udt {
                symbol_id: 42,
                data: vec![0, 255],
            },
        ] {
            let sdk: rust_ethernet_ip::PlcValue = value.clone().into();
            assert_eq!(CipValue::from(sdk), value);
        }
    }
}
