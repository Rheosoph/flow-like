use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
#[cfg(any(feature = "opcua", test))]
use flow_like_ml_sensors::opcua::OpcUaReadConfig;
use flow_like_ml_sensors::opcua::{OpcUaReading, OpcUaSecurityMode};
use flow_like_ml_sensors::{
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
        super::operation_node::<flow_like_ml_sensors::genicam::GenicamCaptureConfig, CameraFrame>(
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
            let config: flow_like_ml_sensors::genicam::GenicamCaptureConfig =
                context.evaluate_pin("request").await?;
            let frame = tokio::task::spawn_blocking(move || {
                flow_like_ml_sensors::genicam::capture_genicam(&config)
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
        super::operation_node::<ReadModbusRequest, ModbusValues>(
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
            let output = flow_like_ml_sensors::read_modbus(&input.endpoint, &input.read).await?;
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
fn decode(input: DecodeRegistersRequest) -> Result<Vec<SensorValue>> {
    Ok(flow_like_ml_sensors::decode_registers(
        &input.registers,
        input.encoding,
        input.word_order,
        input.swap_bytes,
    )?)
}
#[crate::register_node]
#[derive(Default)]
pub struct DecodeSensorRegistersNode;
super::operation!(
    DecodeSensorRegistersNode,
    "ml_decode_sensor_registers",
    "Decode Sensor Registers",
    "Decode signed, unsigned or floating-point registers with explicit byte and word order",
    "decodeRegisters",
    "AI/ML/Sensors",
    DecodeRegistersRequest,
    Vec<SensorValue>,
    decode
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
    #[cfg(any(feature = "opcua", test))]
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
        super::operation_node::<ReadOpcUaRequest, Vec<OpcUaReading>>(
            "ml_read_opcua_sensor",
            "Read OPC UA Sensor",
            "Read timestamped values and quality over a verified OPC UA session; PKI comes from executor app storage",
            "readOpcUa",
            "AI/ML/Sensors",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "opcua")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: ReadOpcUaRequest = context.evaluate_pin("request").await?;
            let pki_dir = super::lifecycle::local_storage_directory(context, "opcua-pki")?
                .to_str()
                .ok_or_else(|| anyhow!("PKI storage path is not UTF-8"))?
                .to_string();
            let input = request.into_config(pki_dir);
            let output = flow_like_ml_sensors::opcua::read_opcua(&input, None).await?;
            context
                .set_pin_value("result", flow_like_types::json::json!(output))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "opcua"))]
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
