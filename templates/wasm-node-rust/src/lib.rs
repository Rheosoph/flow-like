//! Flow-Like WASM Node Template — Component Model
//!
//! Uses the `flow-like-wasm-sdk` crate. Mirrors the native catalog pattern:
//! `#[register_node]` + `impl WasmNode` + `wasm_main!()`.
//!
//! # Building
//!
//! ```bash
//! cargo build --release    # outputs a WASM component directly
//! ```
//!
//! The compiled component is at:
//! `target/wasm32-wasip2/release/flow_like_wasm_node_template.wasm`

use flow_like_wasm_sdk::*;

// ── Node 1: Repeat Text ────────────────────────────────────────────────

#[register_node]
#[derive(Default)]
pub struct RepeatTextNode;

impl WasmNode for RepeatTextNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "repeat_text",
            "Repeat Text",
            "Repeats input text N times",
            "Custom/WASM",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "input_text",
            "Input Text",
            "Text to repeat",
            VariableType::String,
        )
        .set_default_value(json!(""));
        node.add_input_pin(
            "multiplier",
            "Multiplier",
            "Number of repetitions",
            VariableType::Integer,
        )
        .set_default_value(json!(1));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "output_text",
            "Output Text",
            "Repeated text result",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("input_text").unwrap_or_default();
        let mult = ctx.get_i64("multiplier").unwrap_or(1);
        let output = text.repeat(mult.max(0) as usize);
        ctx.set_output("output_text", output);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 2: Character Count ────────────────────────────────────────────

#[register_node]
#[derive(Default)]
pub struct CharCountNode;

impl WasmNode for CharCountNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "char_count",
            "Character Count",
            "Counts the number of characters in input text",
            "Custom/WASM",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "input_text",
            "Input Text",
            "Text to measure",
            VariableType::String,
        )
        .set_default_value(json!(""));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "char_count",
            "Char Count",
            "Number of characters",
            VariableType::Integer,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("input_text").unwrap_or_default();
        ctx.set_output("char_count", text.len() as i64);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 3: Greeting Generator (struct-typed pins) ─────────────────────

#[derive(Default, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct GreetingConfig {
    pub greeting: String,
    pub uppercase: bool,
    pub repeat: u32,
}

#[derive(Default, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct GreetingResult {
    pub message: String,
    pub length: u64,
}

#[register_node]
#[derive(Default)]
pub struct GreetingNode;

impl WasmNode for GreetingNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "greeting",
            "Greeting Generator",
            "Generates a greeting from a typed config struct",
            "Custom/WASM",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "config",
            "Config",
            "Greeting configuration",
            VariableType::Struct,
        )
        .set_schema::<GreetingConfig>()
        .set_enforce_schema(true)
        .set_default_value(json!({
            "greeting": "Hello",
            "uppercase": false,
            "repeat": 1
        }));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin("result", "Result", "Greeting result", VariableType::Struct)
            .set_schema::<GreetingResult>()
            .set_enforce_schema(true);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let config: GreetingConfig = ctx.get_input_as("config").unwrap_or_default();

        let base = if config.uppercase {
            config.greeting.to_uppercase()
        } else {
            config.greeting
        };
        let message = base.repeat(config.repeat.max(1) as usize);
        let length = message.len() as u64;

        let result = GreetingResult { message, length };
        ctx.set_output("result", serde_json::to_value(&result).unwrap_or_default());
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 4: File Writer (FlowPath demo) ───────────────────────────────

/// Demonstrates using FlowPath to write data to storage and list contents.
/// Accepts a FlowPath directory, a filename, and text content — writes the
/// file and outputs the updated listing of the directory.
#[register_node]
#[derive(Default)]
pub struct FileWriterNode;

impl WasmNode for FileWriterNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "file_writer",
            "File Writer",
            "Writes text to a file in a FlowPath directory and lists the directory contents",
            "Custom/WASM/Storage",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "directory",
            "Directory",
            "Storage directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "filename",
            "Filename",
            "Name of the file to write",
            VariableType::String,
        )
        .set_default_value(json!("output.txt"));
        node.add_input_pin(
            "content",
            "Content",
            "Text content to write",
            VariableType::String,
        )
        .set_default_value(json!(""));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "file_path",
            "File Path",
            "Path of the written file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "file_count",
            "File Count",
            "Number of files in directory",
            VariableType::Integer,
        );
        node.add_permission(NodePermission::StorageRead);
        node.add_permission(NodePermission::StorageWrite);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let dir: FlowPath = match ctx.require_input_as("directory") {
            Ok(d) => d,
            Err(e) => return ctx.fail(e),
        };
        let filename = ctx
            .get_string("filename")
            .unwrap_or_else(|| "output.txt".into());
        let content = ctx.get_string("content").unwrap_or_default();

        let file = dir.child(&filename);
        if !file.put_string(&ctx, &content) {
            return ctx.fail(format!("Failed to write {filename}"));
        }

        let count = dir.list(&ctx).map(|v| v.len() as i64).unwrap_or(0);

        ctx.set_output_json("file_path", &file);
        ctx.set_output("file_count", count);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 5: File Reader (FlowPath demo) ───────────────────────────────

/// Demonstrates reading a file via FlowPath and outputting its contents.
#[register_node]
#[derive(Default)]
pub struct FileReaderNode;

impl WasmNode for FileReaderNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "file_reader",
            "File Reader",
            "Reads text content from a FlowPath file",
            "Custom/WASM/Storage",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin("file", "File", "FlowPath to the file", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "content",
            "Content",
            "File text content",
            VariableType::String,
        );
        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the file exists",
            VariableType::Boolean,
        );
        node.add_permission(NodePermission::StorageRead);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let file: FlowPath = match ctx.require_input_as("file") {
            Ok(f) => f,
            Err(e) => return ctx.fail(e),
        };

        match file.get_string(&ctx) {
            Some(content) => {
                ctx.set_output("content", content);
                ctx.set_output("exists", true);
            }
            None => {
                ctx.set_output("content", "");
                ctx.set_output("exists", false);
            }
        }

        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 6: Weather Agent (WasiAgent + tool demo) ─────────────────────

/// Demonstrates building an agent with a custom tool via the WASM SDK's
/// `WasiAgent`. Uses `FlowLikeCompletionModel` to call the host LLM and
/// handles tool calls in a synchronous loop compatible with WASI/Wasmtime.
#[register_node]
#[derive(Default)]
pub struct WeatherAgentNode;

impl WasmNode for WeatherAgentNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "weather_agent",
            "Weather Agent",
            "A WASI agent that can look up weather using a tool",
            "Custom/WASM/AI",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "model",
            "Model",
            "Bit model descriptor",
            VariableType::Struct,
        )
        .set_schema_raw(&Bit::schema());
        node.add_input_pin("message", "Message", "User message", VariableType::String)
            .set_default_value(json!("What's the weather like in San Francisco?"));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "response",
            "Response",
            "Agent response",
            VariableType::String,
        );
        node.set_long_running(true);
        node.add_permission(NodePermission::Models);
        node.add_permission(NodePermission::NetworkHttp);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        log::info("WeatherAgent: starting run");
        let model: Bit = match ctx.require_input_as("model") {
            Ok(m) => m,
            Err(e) => return ctx.fail(e),
        };
        let message = ctx.get_string("message").unwrap_or_default();
        log::info(&format!("WeatherAgent: message={message}"));

        let completion_model = FlowLikeCompletionModel::new(model, &ctx);

        let weather_def = rig::completion::ToolDefinition {
            name: "get_weather".to_string(),
            description: "Get the current weather for a given location.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name, e.g. 'San Francisco' or 'Tokyo'"
                    }
                },
                "required": ["location"]
            }),
        };

        let agent = WasiAgent::new(completion_model)
            .preamble(
                "You are a helpful weather assistant. \
                 Use the get_weather tool to look up current weather conditions \
                 when the user asks about the weather.",
            )
            .tool(weather_def, |args: serde_json::Value| {
                let location = args
                    .get("location")
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .unwrap_or("unknown");

                // Step 1: Geocode the location name → lat/lon via Open-Meteo
                let geo_url = format!(
                    "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
                    location.replace(' ', "+")
                );
                let geo_resp = flow_like_wasm_sdk::http_ns::http_request(0, &geo_url, "{}", &[]);
                let geo_json: serde_json::Value = match geo_resp {
                    Some(r) => serde_json::from_str(&r).unwrap_or_default(),
                    None => return Ok(format!("Could not geocode location: {location}")),
                };

                let body_str = flow_like_wasm_sdk::http_ns::decode_response_body(&geo_json);
                let body: serde_json::Value = serde_json::from_str(&body_str).unwrap_or_default();
                let results = body.get("results").and_then(|r| r.as_array());
                let (lat, lon, resolved_name) = match results.and_then(|r| r.first()) {
                    Some(hit) => {
                        let lat = hit.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let lon = hit.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let name = hit.get("name").and_then(|v| v.as_str()).unwrap_or(location);
                        (lat, lon, name.to_string())
                    }
                    None => return Ok(format!("Location not found: {location}")),
                };

                // Step 2: Fetch current weather from Open-Meteo
                let weather_url = format!(
                    "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
                     &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
                     wind_speed_10m,wind_direction_10m,weather_code"
                );
                let wx_resp = flow_like_wasm_sdk::http_ns::http_request(0, &weather_url, "{}", &[]);
                let wx_json: serde_json::Value = match wx_resp {
                    Some(r) => serde_json::from_str(&r).unwrap_or_default(),
                    None => return Ok(format!("Weather API request failed for {resolved_name}")),
                };

                let wx_body_str = flow_like_wasm_sdk::http_ns::decode_response_body(&wx_json);
                let wx: serde_json::Value = serde_json::from_str(&wx_body_str).unwrap_or_default();
                let current = wx.get("current").unwrap_or(&serde_json::Value::Null);

                let temp = current.get("temperature_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let feels = current.get("apparent_temperature").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let humidity = current.get("relative_humidity_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let wind = current.get("wind_speed_10m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let wind_dir = current.get("wind_direction_10m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let code = current.get("weather_code").and_then(|v| v.as_u64()).unwrap_or(0);

                let condition = match code {
                    0 => "Clear sky",
                    1..=3 => "Partly cloudy",
                    45 | 48 => "Foggy",
                    51..=57 => "Drizzle",
                    61..=67 => "Rain",
                    71..=77 => "Snow",
                    80..=82 => "Rain showers",
                    85 | 86 => "Snow showers",
                    95..=99 => "Thunderstorm",
                    _ => "Unknown",
                };

                Ok(format!(
                    "Current weather in {resolved_name}: {temp}°C (feels like {feels}°C), \
                     {condition}, humidity {humidity}%, wind {wind} km/h from {wind_dir}°"
                ))
            });

        log::info("WeatherAgent: agent built, calling prompt");
        match agent.prompt(&message) {
            Ok(response) => {
                log::info(&format!(
                    "WeatherAgent: success, response len={}",
                    response.len()
                ));
                ctx.set_output("response", response);
                ctx.activate_exec("exec_out");
                ctx.success()
            }
            Err(e) => {
                log::error(&format!("WeatherAgent: agent error: {e}"));
                ctx.fail(format!("Agent error: {e}"))
            }
        }
    }
}

// ── WASM entrypoint (auto-discovers all #[register_node] structs) ──────

wasm_main!();

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_node_definition() {
        let node = RepeatTextNode.get_node();
        assert_eq!(node.name, "repeat_text");
        assert_eq!(node.pins.len(), 5);
    }

    #[test]
    fn count_node_definition() {
        let node = CharCountNode.get_node();
        assert_eq!(node.name, "char_count");
        assert_eq!(node.pins.len(), 4);
    }

    #[test]
    fn greeting_node_definition() {
        let node = GreetingNode.get_node();
        assert_eq!(node.name, "greeting");
        assert_eq!(node.pins.len(), 4);

        let config_pin = &node.pins[1];
        assert_eq!(config_pin.name, "config");
        assert_eq!(config_pin.data_type, VariableType::Struct);
        assert_eq!(config_pin.enforce_schema, Some(true));

        let schema_str = config_pin
            .schema
            .as_ref()
            .expect("config pin must have schema");
        let schema: serde_json::Value =
            serde_json::from_str(schema_str).expect("schema must be valid JSON");
        let props = schema
            .get("properties")
            .expect("schema must have properties");
        assert!(props.get("greeting").is_some());
        assert!(props.get("uppercase").is_some());
        assert!(props.get("repeat").is_some());
    }

    #[test]
    fn greeting_node_output_schema() {
        let node = GreetingNode.get_node();

        let result_pin = &node.pins[3];
        assert_eq!(result_pin.name, "result");
        assert_eq!(result_pin.data_type, VariableType::Struct);
        assert_eq!(result_pin.enforce_schema, Some(true));

        let schema_str = result_pin
            .schema
            .as_ref()
            .expect("result pin must have schema");
        let schema: serde_json::Value = serde_json::from_str(schema_str).unwrap();
        let props = schema.get("properties").unwrap();
        assert!(props.get("message").is_some());
        assert!(props.get("length").is_some());
    }

    #[test]
    fn greeting_node_roundtrip_to_runtime() {
        let node = GreetingNode.get_node();
        let json = serde_json::to_string(&node).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("SDK NodeDefinition must produce valid JSON");

        assert_eq!(parsed["name"], "greeting");
        let pins = parsed["pins"].as_array().unwrap();
        assert_eq!(pins.len(), 4);

        assert_eq!(pins[0]["data_type"], "Execution");
        assert_eq!(pins[0]["pin_type"], "Input");

        assert_eq!(pins[1]["data_type"], "Struct");
        assert_eq!(pins[1]["pin_type"], "Input");
        assert_eq!(pins[1]["enforce_schema"], true);
        assert!(pins[1]["schema"].is_string());
        assert!(pins[1]["default_value"].is_object());

        let schema: serde_json::Value =
            serde_json::from_str(pins[1]["schema"].as_str().unwrap()).unwrap();
        assert!(schema["properties"]["greeting"].is_object());
        assert!(schema["properties"]["uppercase"].is_object());
        assert!(schema["properties"]["repeat"].is_object());

        assert_eq!(pins[2]["pin_type"], "Output");
        assert_eq!(pins[2]["data_type"], "Execution");

        assert_eq!(pins[3]["data_type"], "Struct");
        assert_eq!(pins[3]["pin_type"], "Output");
        assert!(pins[3]["schema"].is_string());

        let out_schema: serde_json::Value =
            serde_json::from_str(pins[3]["schema"].as_str().unwrap()).unwrap();
        assert!(out_schema["properties"]["message"].is_object());
        assert!(out_schema["properties"]["length"].is_object());
    }

    // ── FlowPath node definitions ──────────────────────────────────────

    #[test]
    fn file_writer_node_definition() {
        let node = FileWriterNode.get_node();
        assert_eq!(node.name, "file_writer");
        assert_eq!(node.category, "Custom/WASM/Storage");
        assert_eq!(node.pins.len(), 7);

        let dir_pin = &node.pins[1];
        assert_eq!(dir_pin.name, "directory");
        assert_eq!(dir_pin.data_type, VariableType::Struct);
        assert!(
            dir_pin.schema.is_some(),
            "directory pin must have FlowPath schema"
        );

        let file_path_pin = node.pins.iter().find(|p| p.name == "file_path").unwrap();
        assert_eq!(file_path_pin.data_type, VariableType::Struct);
        assert!(file_path_pin.schema.is_some());
    }

    #[test]
    fn file_reader_node_definition() {
        let node = FileReaderNode.get_node();
        assert_eq!(node.name, "file_reader");
        assert_eq!(node.category, "Custom/WASM/Storage");
        assert_eq!(node.pins.len(), 5);

        let file_pin = &node.pins[1];
        assert_eq!(file_pin.name, "file");
        assert_eq!(file_pin.data_type, VariableType::Struct);
        assert!(file_pin.schema.is_some());

        let exists_pin = node.pins.iter().find(|p| p.name == "exists").unwrap();
        assert_eq!(exists_pin.data_type, VariableType::Boolean);
    }

    #[test]
    fn file_writer_flow_path_schema_is_valid() {
        let node = FileWriterNode.get_node();
        let dir_pin = &node.pins[1];
        let schema_str = dir_pin.schema.as_ref().unwrap();
        let schema: serde_json::Value = serde_json::from_str(schema_str).unwrap();
        let props = schema
            .get("properties")
            .expect("FlowPath schema must have properties");
        assert!(props.get("path").is_some());
        assert!(props.get("store_ref").is_some());
    }

    // ── Weather agent node definition ─────────────────────────────────

    #[test]
    fn weather_agent_node_definition() {
        let node = WeatherAgentNode.get_node();
        assert_eq!(node.name, "weather_agent");
        assert_eq!(node.category, "Custom/WASM/AI");
        assert_eq!(node.long_running, Some(true));
        assert_eq!(node.pins.len(), 5);

        let model_pin = &node.pins[1];
        assert_eq!(model_pin.name, "model");
        assert_eq!(model_pin.data_type, VariableType::Struct);

        let response_pin = node.pins.iter().find(|p| p.name == "response").unwrap();
        assert_eq!(response_pin.data_type, VariableType::String);
    }

    // ── FlowPath path manipulation roundtrip ───────────────────────────

    #[test]
    fn flow_path_child_parent_roundtrip() {
        let root = FlowPath::new("storage".into(), "s3".into(), Some("cache".into()));
        let child = root.child("sub").child("data.json");
        assert_eq!(child.path, "storage/sub/data.json");
        assert_eq!(child.store_ref, "s3");

        let parent = child.parent().unwrap();
        assert_eq!(parent.path, "storage/sub");

        let grandparent = parent.parent().unwrap();
        assert_eq!(grandparent.path, "storage");
    }

    #[test]
    fn flow_path_extension_methods() {
        let fp = FlowPath::new("data/output.csv".into(), "s3".into(), None);
        assert_eq!(fp.extension(), Some("csv".to_string()));
        assert_eq!(fp.file_name(), Some("output.csv".to_string()));

        let json_fp = fp.with_extension("json");
        assert_eq!(json_fp.path, "data/output.json");
    }

    #[test]
    fn flow_path_serde_as_pin_value() {
        let fp = FlowPath::new("dir/file.txt".into(), "local".into(), None);
        let val = serde_json::to_value(&fp).unwrap();
        let fp2: FlowPath = serde_json::from_value(val).unwrap();
        assert_eq!(fp.path, fp2.path);
        assert_eq!(fp.store_ref, fp2.store_ref);
    }
}
