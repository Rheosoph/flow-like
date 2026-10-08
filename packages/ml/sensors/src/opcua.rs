#[cfg(feature = "opcua")]
use crate::Error;
use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpcUaSecurityMode {
    None,
    Sign,
    SignAndEncrypt,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReadConfig {
    pub endpoint: String,
    pub pki_dir: String,
    pub security_policy: String,
    pub security_mode: OpcUaSecurityMode,
    pub node_ids: Vec<String>,
    pub timeout_ms: u64,
    pub max_age_ms: f64,
}

impl OpcUaReadConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.endpoint.starts_with("opc.tcp://") && !self.endpoint.contains('@'),
            "OPC UA requires an opc.tcp endpoint without embedded credentials",
        )?;
        require(
            !self.pki_dir.is_empty(),
            "OPC UA requires an explicit certificate store directory",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "OPC UA timeout must be in 1..300000 ms",
        )?;
        require(
            !self.node_ids.is_empty()
                && self.node_ids.len() <= 4096
                && self
                    .node_ids
                    .iter()
                    .all(|n| !n.is_empty() && n.len() <= 4096),
            "OPC UA read needs 1..4096 bounded node ids",
        )?;
        require(
            self.max_age_ms.is_finite() && self.max_age_ms >= 0.0,
            "OPC UA maximum value age must be finite and nonnegative",
        )?;
        require(
            matches!(
                self.security_policy.as_str(),
                "None" | "Basic256Sha256" | "Aes128Sha256RsaOaep" | "Aes256Sha256RsaPss"
            ),
            "Unsupported OPC UA security policy",
        )?;
        require(
            (self.security_mode == OpcUaSecurityMode::None) == (self.security_policy == "None"),
            "OPC UA security policy and message security mode disagree",
        )
    }
}

// Kept outside the serializable node configuration so callers can obtain credentials from their secret store.
pub struct OpcUaCredentials {
    pub username: String,
    pub password: String,
}
impl std::fmt::Debug for OpcUaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpcUaCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum OpcUaValue {
    Boolean(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReading {
    pub node_id: String,
    pub quality_good: bool,
    pub status: String,
    pub value: Option<OpcUaValue>,
    pub source_timestamp: Option<String>,
    pub server_timestamp: Option<String>,
}

#[cfg(feature = "opcua")]
pub async fn read_opcua(
    config: &OpcUaReadConfig,
    credentials: Option<&OpcUaCredentials>,
) -> Result<Vec<OpcUaReading>> {
    use ::opcua::{
        client::{ClientBuilder, IdentityToken},
        types::{
            MessageSecurityMode, NodeId, ReadValueId, TimestampsToReturn, UserTokenPolicy,
            UserTokenType,
        },
    };
    use std::{str::FromStr, time::Duration};
    use tokio::time::timeout;
    config.validate()?;
    require(
        credentials.is_none() || config.security_mode == OpcUaSecurityMode::SignAndEncrypt,
        "Username authentication requires an encrypted OPC UA channel",
    )?;
    let nodes: Vec<ReadValueId> = config
        .node_ids
        .iter()
        .map(|id| {
            NodeId::from_str(id)
                .map(ReadValueId::new_value)
                .map_err(|_| Error::Invalid(format!("Invalid OPC UA node id: {id}")))
        })
        .collect::<Result<_>>()?;
    let duration = Duration::from_millis(config.timeout_ms);
    timeout(duration, async {
        let mut client = ClientBuilder::new()
            .application_name("Flow-Like Inspection")
            .application_uri("urn:flow-like:inspection")
            .pki_dir(&config.pki_dir)
            .create_sample_keypair(false)
            .trust_server_certs(false)
            .verify_server_certs(true)
            .session_retry_limit(0)
            .request_timeout(duration)
            .max_array_length(65_536)
            .max_string_length(1_048_576)
            .max_byte_string_length(1_048_576)
            .max_incoming_chunk_size(4_194_304)
            .client().map_err(|errors| Error::Invalid(errors.join("; ")))?;
        let (token, identity) = match credentials {
            Some(credentials) => (UserTokenPolicy { token_type: UserTokenType::UserName, ..Default::default() }, IdentityToken::new_user_name(credentials.username.clone(), credentials.password.clone())),
            None => (UserTokenPolicy::anonymous(), IdentityToken::Anonymous),
        };
        let mode = match config.security_mode { OpcUaSecurityMode::None => MessageSecurityMode::None, OpcUaSecurityMode::Sign => MessageSecurityMode::Sign, OpcUaSecurityMode::SignAndEncrypt => MessageSecurityMode::SignAndEncrypt };
        let (session, event_loop) = client.connect_to_matching_endpoint((config.endpoint.as_str(), config.security_policy.as_str(), mode, token), identity).await.map_err(|error| Error::Invalid(format!("OPC UA connection failed: {error}")))?;
        let mut task = event_loop.spawn();
        struct AbortOnDrop(tokio::task::AbortHandle);
        impl Drop for AbortOnDrop { fn drop(&mut self) { self.0.abort(); } }
        let _abort = AbortOnDrop(task.abort_handle());
        let _session_guard = session.close_on_drop();
        tokio::select! {
            _ = &mut task => return Err(Error::Invalid("OPC UA session ended before connecting".into())),
            connected = session.wait_for_connection() => require(connected, "OPC UA session did not connect")?,
        }
        let values = session.read(&nodes, TimestampsToReturn::Both, config.max_age_ms).await.map_err(|error| Error::Invalid(format!("OPC UA read failed: {error}")))?;
        require(values.len() == nodes.len(), "OPC UA returned a different number of node values")?;
        let readings = config.node_ids.iter().zip(values).map(|(node_id, data)| {
            let quality_good = data.status.as_ref().is_none_or(|status| status.is_good());
            let status = data.status.map(|status| status.to_string()).unwrap_or_else(|| "Good".into());
            let value = if quality_good { data.value.as_ref().map(convert_value).transpose()? } else { None };
            Ok(OpcUaReading { node_id: node_id.clone(), quality_good, status, value, source_timestamp: data.source_timestamp.map(|t| t.to_string()), server_timestamp: data.server_timestamp.map(|t| t.to_string()) })
        }).collect::<Result<Vec<_>>>();
        let _ = session.disconnect().await;
        readings
    }).await.map_err(|_| Error::Timeout)?
}

#[cfg(feature = "opcua")]
fn convert_value(value: &::opcua::types::Variant) -> Result<OpcUaValue> {
    use ::opcua::types::Variant;
    let output = match value {
        Variant::Boolean(v) => OpcUaValue::Boolean(*v),
        Variant::SByte(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int16(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int32(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int64(v) => OpcUaValue::Signed(*v),
        Variant::Byte(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt16(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt32(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt64(v) => OpcUaValue::Unsigned(*v),
        Variant::Float(v) => OpcUaValue::Float(*v as f64),
        Variant::Double(v) => OpcUaValue::Float(*v),
        Variant::String(v) => OpcUaValue::Text(v.to_string()),
        _ => {
            return Err(Error::Invalid(
                "OPC UA inspection reads support scalar numeric, boolean and string values".into(),
            ));
        }
    };
    if let OpcUaValue::Float(v) = output {
        require(v.is_finite(), "OPC UA returned a non-finite numeric value")?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_rejects_security_mismatch_and_credential_urls() {
        let mut config = OpcUaReadConfig {
            endpoint: "opc.tcp://localhost:4840".into(),
            pki_dir: "/tmp/fixture-pki".into(),
            security_policy: "Basic256Sha256".into(),
            security_mode: OpcUaSecurityMode::SignAndEncrypt,
            node_ids: vec!["ns=2;s=temperature".into()],
            timeout_ms: 1000,
            max_age_ms: 0.0,
        };
        config.validate().unwrap();
        config.security_mode = OpcUaSecurityMode::None;
        assert!(config.validate().is_err());
        config.security_policy = "None".into();
        config.endpoint = "opc.tcp://name:password@localhost:4840".into();
        assert!(config.validate().is_err());
    }
    #[cfg(feature = "opcua")]
    #[test]
    fn scalar_conversion_keeps_integer_precision_and_rejects_nan() {
        assert_eq!(
            convert_value(&::opcua::types::Variant::UInt64(u64::MAX)).unwrap(),
            OpcUaValue::Unsigned(u64::MAX)
        );
        assert!(convert_value(&::opcua::types::Variant::Double(f64::NAN)).is_err());
    }
}
