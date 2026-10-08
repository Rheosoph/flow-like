use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
#[cfg(any(feature = "execute", test))]
use flow_like_industrial::opcua::OpcUaReadConfig;
use flow_like_industrial::opcua::{OpcUaReading, OpcUaSecurityMode};
use flow_like_industrial::{
    ModbusReadRequest, ModbusValues, RegisterEncoding, SensorValue, WordOrder,
};
use flow_like_types::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadModbusRequest {
    pub endpoint: String,
    pub read: ModbusReadRequest,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CameraFrame {
    pub image: flow_like_catalog_core::NodeImage,
    pub frame_id: u64,
    pub timestamp_ns: Option<u64>,
    pub received_at_ns: u64,
    pub width: u32,
    pub height: u32,
}
#[crate::register_node]
#[derive(Default)]
pub struct CaptureGenicamFrameNode;
#[flow_like_types::async_trait]
impl NodeLogic for CaptureGenicamFrameNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        operation_node::<flow_like_industrial::genicam::GenicamCaptureConfig, CameraFrame>(
            "ml_capture_genicam_frame",
            "Capture GenICam Frame",
            "Capture a complete GigE Vision frame and preserve camera and host timestamps",
            "captureGenicam",
            "AI/ML/Sensors",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "genicam")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            use flow_like::flow::execution::ExecutionEnvironment;
            if !matches!(
                context.execution_environment(),
                ExecutionEnvironment::Local | ExecutionEnvironment::Desktop
            ) {
                return Err(anyhow!(
                    "GenICam capture requires a local executor with access to the camera"
                ));
            }
            let config: flow_like_industrial::genicam::GenicamCaptureConfig =
                context.evaluate_pin("request").await?;
            let frame = tokio::task::spawn_blocking(move || {
                flow_like_industrial::genicam::capture_genicam(&config)
            })
            .await??;
            let rgb = frame.to_rgb8()?;
            let buffer = flow_like_types::image::RgbImage::from_raw(frame.width, frame.height, rgb)
                .ok_or_else(|| anyhow!("Camera RGB frame size differs from dimensions"))?;
            let image = flow_like_catalog_core::NodeImage::new(
                context,
                flow_like_types::image::DynamicImage::ImageRgb8(buffer),
            )
            .await;
            let result = CameraFrame {
                image,
                frame_id: frame.frame_id,
                timestamp_ns: frame.timestamp_ns,
                received_at_ns: frame.system_timestamp_ns,
                width: frame.width,
                height: frame.height,
            };
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "genicam"))]
        {
            let _ = context;
            Err(anyhow!("This executor needs the genicam feature"))
        }
    }
}
#[crate::register_node]
#[derive(Default)]
pub struct ReadModbusSensorNode;
#[flow_like_types::async_trait]
impl NodeLogic for ReadModbusSensorNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        operation_node::<ReadModbusRequest, ModbusValues>(
            "ml_read_modbus_sensor",
            "Read Modbus Sensor",
            "Read Modbus TCP input/holding registers or discrete inputs with bounded timeout",
            "readModbus",
            "AI/ML/Sensors",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let input: ReadModbusRequest = context.evaluate_pin("request").await?;
            input.read.validate()?;
            let (host, port) = input
                .endpoint
                .rsplit_once(':')
                .ok_or_else(|| anyhow!("Modbus TCP endpoint requires host:port"))?;
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let output = tokio::select! {
                _ = crate::runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
                result = async {
                    let addrs = flow_like::flow::execution::egress::resolve_socket_addrs(
                        context.execution_environment(), host, port.parse()?).await?;
                    let mut client = flow_like_industrial::modbus::ModbusClient::connect_tcp_resolved(&addrs, input.read.timeout_ms).await?;
                    let result = client.read(&input.read).await;
                    let _ = client.disconnect().await;
                    Ok::<_, flow_like_types::Error>(result?)
                } => result?,
            };
            context
                .set_pin_value("result", flow_like_types::json::json!(output))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("Modbus reads require execution support"))
        }
    }
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecodeRegistersRequest {
    pub registers: Vec<u16>,
    pub encoding: RegisterEncoding,
    pub word_order: WordOrder,
    pub swap_bytes: bool,
}
#[cfg(feature = "execute")]
fn decode(input: DecodeRegistersRequest) -> Result<Vec<SensorValue>> {
    Ok(flow_like_industrial::decode_registers(
        &input.registers,
        input.encoding,
        input.word_order,
        input.swap_bytes,
    )?)
}
#[crate::register_node]
#[derive(Default)]
pub struct DecodeSensorRegistersNode;
crate::operation!(
    DecodeSensorRegistersNode,
    "ml_decode_sensor_registers",
    "Decode Sensor Registers",
    "Decode signed, unsigned or floating-point registers with explicit byte and word order",
    "inspection",
    "decodeRegisters",
    "AI/ML/Sensors",
    DecodeRegistersRequest,
    Vec<SensorValue>,
    decode_operation
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadOpcUaRequest {
    pub endpoint: String,
    pub security_policy: String,
    pub security_mode: OpcUaSecurityMode,
    pub node_ids: Vec<String>,
    pub timeout_ms: u64,
    pub max_age_ms: f64,
}

impl ReadOpcUaRequest {
    #[cfg(any(feature = "execute", test))]
    fn into_config(self, pki_dir: String) -> OpcUaReadConfig {
        OpcUaReadConfig {
            endpoint: self.endpoint,
            pki_dir,
            security_policy: self.security_policy,
            security_mode: self.security_mode,
            node_ids: self.node_ids,
            timeout_ms: self.timeout_ms,
            max_age_ms: self.max_age_ms,
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ReadOpcUaSensorNode;
#[flow_like_types::async_trait]
impl NodeLogic for ReadOpcUaSensorNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        operation_node::<ReadOpcUaRequest, Vec<OpcUaReading>>(
            "ml_read_opcua_sensor",
            "Read OPC UA Sensor",
            "Read timestamped values and quality over a verified OPC UA session; PKI comes from executor app storage",
            "readOpcUa",
            "AI/ML/Sensors",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: ReadOpcUaRequest = context.evaluate_pin("request").await?;
            let pki_dir = crate::runtime::local_storage_directory(context, "opcua-pki")?
                .to_str()
                .ok_or_else(|| anyhow!("PKI storage path is not UTF-8"))?
                .to_string();
            let input = request.into_config(pki_dir);
            input.validate()?;
            let output = tokio::select! {
                _ = crate::runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
                result = async {
                    use flow_like_industrial::opcua::{OpcUaClient, OpcUaConnectConfig, OpcUaReadRequest};
                    let addresses = crate::opcua::resolve_endpoint(context, &input.endpoint, input.timeout_ms).await?;
                    let config = OpcUaConnectConfig {
                        endpoint: input.endpoint, pki_dir: input.pki_dir, security_policy: input.security_policy,
                        security_mode: input.security_mode, timeout_ms: input.timeout_ms,
                    };
                    let client = OpcUaClient::connect_resolved(&config, None, &addresses).await?;
                    let result = client.read(&OpcUaReadRequest { node_ids: input.node_ids, max_age_ms: input.max_age_ms }).await;
                    let _ = client.disconnect().await;
                    Ok::<_, flow_like_types::Error>(result?)
                } => result?,
            };
            context
                .set_pin_value("result", flow_like_types::json::json!(output))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("This executor needs the opcua feature"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_value, json};

    #[test]
    fn opcua_request_uses_executor_certificate_storage() {
        let node = ReadOpcUaSensorNode.get_node();
        let schema: flow_like_types::Value = flow_like_types::json::from_str(
            node.get_pin_by_name("request")
                .unwrap()
                .schema
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        assert!(schema["properties"].get("pki_dir").is_none());
        let request: ReadOpcUaRequest = from_value(json!({
            "endpoint":"opc.tcp://localhost:4840", "security_policy":"None",
            "security_mode":"none", "node_ids":["ns=2;s=temperature"],
            "timeout_ms":1000, "max_age_ms":0.0
        }))
        .unwrap();
        let config = request.into_config("/executor/app/opcua-pki".into());
        config.validate().unwrap();
        assert_eq!(config.pki_dir, "/executor/app/opcua-pki");
    }
}

fn operation_node<I: JsonSchema + Serialize, O: JsonSchema + Serialize>(
    id: &str,
    name: &str,
    description: &str,
    function: &str,
    category: &str,
) -> flow_like::flow::node::Node {
    crate::node::operation_node::<I, O>(id, name, description, "inspection", function, category)
}

#[cfg(feature = "execute")]
async fn decode_operation(
    _: &mut ExecutionContext,
    request: DecodeRegistersRequest,
) -> Result<Vec<SensorValue>> {
    decode(request)
}
