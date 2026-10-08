//! IO-Link master access through the IO-Link Community JSON Integration 2.0 REST API.
//! API reference: https://github.com/iolinkcommunity/JSON_for_IO-Link/tree/2.0.0

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IolinkAuthentication {
    #[default]
    None,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct IolinkConfig {
    /// Master origin, for example https://192.168.1.20. The API uses /iolink/v2.
    pub origin: String,
    pub authentication: IolinkAuthentication,
    pub timeout_ms: u64,
}
impl Default for IolinkConfig {
    fn default() -> Self {
        Self {
            origin: "http://127.0.0.1".into(),
            authentication: Default::default(),
            timeout_ms: 5000,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IolinkDevice {
    pub device_alias: String,
    pub master_number: u16,
    pub port_number: u16,
    #[serde(default)]
    pub iodd_file_name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkData {
    pub valid: bool,
    pub value: Vec<u8>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IolinkProcessData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iolink: Option<IolinkData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cq_value: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq_value: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkParameter {
    pub index: u16,
    pub sub_index: Option<u8>,
}
fn validate_alias(alias: &str) -> Result<()> {
    require(
        !alias.is_empty()
            && alias.len() <= 32
            && alias
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "IO-Link device alias must contain 1 to 32 ASCII letters, digits, or underscores",
    )
}
impl IolinkProcessData {
    pub fn validate(&self) -> Result<()> {
        require(
            self.iolink.is_some() || self.cq_value.is_some() || self.iq_value.is_some(),
            "IO-Link process data must contain IO-Link bytes or a digital pin value",
        )?;
        if let Some(data) = &self.iolink {
            require(
                data.value.len() <= 32,
                "IO-Link process data exceeds 32 bytes",
            )?;
        }
        Ok(())
    }
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use reqwest::{Client, Method, Url};
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    impl IolinkConfig {
        pub fn validate(&self) -> Result<Url> {
            require(
                (1..=300_000).contains(&self.timeout_ms),
                "IO-Link timeout must be between 1 and 300000 ms",
            )?;
            let origin =
                Url::parse(&self.origin).map_err(|_| failure("Invalid IO-Link master origin"))?;
            require(
                matches!(origin.scheme(), "http" | "https") && origin.host_str().is_some(),
                "IO-Link origin requires an HTTP or HTTPS host",
            )?;
            require(
                origin.username().is_empty() && origin.password().is_none(),
                "Set IO-Link credentials in the authentication field",
            )?;
            require(
                origin.path() == "/" && origin.query().is_none() && origin.fragment().is_none(),
                "IO-Link origin must not contain a path, query, or fragment",
            )?;
            Ok(origin)
        }
    }
    pub struct IolinkClient {
        client: Client,
        origin: Url,
        config: IolinkConfig,
        closed: AtomicBool,
    }
    impl IolinkClient {
        /// The caller supplies a client with its executor's DNS and redirect policy installed.
        pub fn with_client(config: IolinkConfig, client: Client) -> Result<Self> {
            let origin = config.validate()?;
            Ok(Self {
                client,
                origin,
                config,
                closed: AtomicBool::new(false),
            })
        }
        fn endpoint(&self, path: &str, byte_array: bool) -> Result<Url> {
            require(
                !self.closed.load(Ordering::Acquire),
                "IO-Link session is closed",
            )?;
            let mut url = self.origin.clone();
            url.set_path(&format!("/iolink/v2/{path}"));
            if byte_array {
                url.set_query(Some("format=byteArray"));
            }
            Ok(url)
        }
        async fn request(
            &self,
            method: Method,
            path: &str,
            byte_array: bool,
            body: Option<Value>,
        ) -> Result<Vec<u8>> {
            let mut request = self
                .client
                .request(method, self.endpoint(path, byte_array)?)
                .timeout(Duration::from_millis(self.config.timeout_ms));
            request = match &self.config.authentication {
                IolinkAuthentication::None => request,
                IolinkAuthentication::Basic { username, password } => {
                    request.basic_auth(username, Some(password))
                }
                IolinkAuthentication::Bearer { token } => request.bearer_auth(token),
            };
            if let Some(body) = body {
                request = request.json(&body);
            }
            let mut response = request.send().await.map_err(failure)?;
            let status = response.status();
            let mut bytes = Vec::new();
            require(
                response.content_length().is_none_or(|len| len <= 1_048_576),
                "IO-Link response exceeds 1 MiB",
            )?;
            while let Some(chunk) = response.chunk().await.map_err(failure)? {
                require(
                    bytes.len() + chunk.len() <= 1_048_576,
                    "IO-Link response exceeds 1 MiB",
                )?;
                bytes.extend_from_slice(&chunk);
            }
            if !status.is_success() {
                #[derive(Deserialize)]
                struct ApiError {
                    code: u32,
                    message: String,
                }
                let detail = serde_json::from_slice::<ApiError>(&bytes)
                    .map(|e| format!("code {}: {}", e.code, e.message))
                    .unwrap_or_else(|_| "invalid or empty API error body".into());
                return Err(failure(format!(
                    "IO-Link master returned HTTP {status}, {detail}"
                )));
            }
            Ok(bytes)
        }
        async fn get<T: DeserializeOwned>(&self, path: &str, byte_array: bool) -> Result<T> {
            let bytes = self.request(Method::GET, path, byte_array, None).await?;
            serde_json::from_slice(&bytes).map_err(failure)
        }
        pub async fn devices(&self) -> Result<Vec<IolinkDevice>> {
            self.get("devices", false).await
        }
        pub async fn read_process_data(&self, alias: &str) -> Result<IolinkProcessData> {
            validate_alias(alias)?;
            let data: IolinkProcessData = self
                .get(&format!("devices/{alias}/processdata/getdata/value"), true)
                .await?;
            data.validate()?;
            Ok(data)
        }
        pub async fn write_process_data(&self, alias: &str, data: IolinkProcessData) -> Result<()> {
            validate_alias(alias)?;
            data.validate()?;
            self.request(
                Method::POST,
                &format!("devices/{alias}/processdata/value"),
                false,
                Some(json!(data)),
            )
            .await?;
            Ok(())
        }
        pub async fn read_parameter(
            &self,
            alias: &str,
            parameter: &IolinkParameter,
        ) -> Result<Vec<u8>> {
            let path = parameter_path(alias, parameter)?;
            #[derive(Deserialize)]
            struct ParameterValue {
                value: Vec<u8>,
            }
            let parameter: ParameterValue = self.get(&path, true).await?;
            require(
                parameter.value.len() <= 232,
                "IO-Link ISDU result exceeds 232 bytes",
            )?;
            Ok(parameter.value)
        }
        pub async fn write_parameter(
            &self,
            alias: &str,
            parameter: &IolinkParameter,
            value: Vec<u8>,
        ) -> Result<()> {
            let path = parameter_path(alias, parameter)?;
            require(
                !value.is_empty() && value.len() <= 232,
                "IO-Link ISDU value must contain 1 to 232 bytes",
            )?;
            self.request(Method::POST, &path, false, Some(json!({ "value": value })))
                .await?;
            Ok(())
        }
        pub fn close(&self) {
            self.closed.store(true, Ordering::Release);
        }
    }
    fn parameter_path(alias: &str, parameter: &IolinkParameter) -> Result<String> {
        validate_alias(alias)?;
        Ok(match parameter.sub_index {
            Some(subindex) => format!(
                "devices/{alias}/parameters/{}/subindices/{subindex}/value",
                parameter.index
            ),
            None => format!("devices/{alias}/parameters/{}/value", parameter.index),
        })
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn json_v2_paths_preserve_validity_and_parameter_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let cases = [
                (
                    "GET /iolink/v2/devices ",
                    200,
                    r#"[{"deviceAlias":"master1port1","masterNumber":1,"portNumber":1}]"#,
                    None,
                ),
                (
                    "GET /iolink/v2/devices/master1port1/processdata/getdata/value?format=byteArray ",
                    200,
                    r#"{"iolink":{"valid":false,"value":[0,128,255]},"iqValue":true}"#,
                    None,
                ),
                (
                    "GET /iolink/v2/devices/master1port1/parameters/16/subindices/0/value?format=byteArray ",
                    200,
                    r#"{"value":[65,66,67]}"#,
                    None,
                ),
                (
                    "POST /iolink/v2/devices/master1port1/parameters/16/subindices/0/value ",
                    204,
                    "",
                    Some(serde_json::json!({"value":[128,255]})),
                ),
                (
                    "POST /iolink/v2/devices/master1port1/processdata/value ",
                    400,
                    r#"{"code":203,"message":"Output data length mismatch"}"#,
                    Some(serde_json::json!({"iolink":{"valid":true,"value":[1]}})),
                ),
            ];
            for (request_line, status, body, expected_body) in cases {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.unwrap();
                    bytes.push(byte[0]);
                    assert!(bytes.len() < 8192);
                }
                let header = String::from_utf8(bytes).unwrap();
                assert!(
                    header.starts_with(request_line),
                    "unexpected request: {header}"
                );
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if let Some(expected) = expected_body {
                    let mut input = vec![0; length];
                    socket.read_exact(&mut input).await.unwrap();
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&input).unwrap(),
                        expected
                    );
                }
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = IolinkClient::with_client(
            IolinkConfig {
                origin: format!("http://{address}"),
                ..Default::default()
            },
            reqwest::Client::builder().no_proxy().build().unwrap(),
        )
        .unwrap();
        assert_eq!(
            client.devices().await.unwrap()[0].device_alias,
            "master1port1"
        );
        let data = client.read_process_data("master1port1").await.unwrap();
        let iolink = data.iolink.unwrap();
        assert!(!iolink.valid);
        assert_eq!(iolink.value, vec![0, 128, 255]);
        let parameter = IolinkParameter {
            index: 16,
            sub_index: Some(0),
        };
        assert_eq!(
            client
                .read_parameter("master1port1", &parameter)
                .await
                .unwrap(),
            b"ABC"
        );
        client
            .write_parameter("master1port1", &parameter, vec![128, 255])
            .await
            .unwrap();
        let error = client
            .write_process_data(
                "master1port1",
                IolinkProcessData {
                    iolink: Some(IolinkData {
                        valid: true,
                        value: vec![1],
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("code 203"));
        client.close();
        assert!(client.devices().await.is_err());
        server.await.unwrap();
    }
    #[test]
    fn rejects_unsafe_aliases_and_oversized_process_data() {
        for alias in ["", "../gateway", "master1/port1", "sensor-name"] {
            assert!(validate_alias(alias).is_err());
        }
        assert!(validate_alias("master1port1").is_ok());
        assert!(
            IolinkProcessData {
                iolink: Some(IolinkData {
                    valid: true,
                    value: vec![0; 33]
                }),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
