use crate::{
    bit::{Bit, BitTypes},
    models::{ModelMeta, local_utils::ensure_local_weights},
    state::FlowLikeState,
};
use flow_like_model_provider::{
    llm::{ModelLogic, llamacpp::LlamaCppModel},
    provider::ModelProvider,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{
    Result, Value as JsonValue, reqwest,
    tokio::{self, time::sleep},
};
use portpicker::pick_unused_port;
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{DEFAULT_MAX_CONTEXT_SIZE, ExecutionSettings};

pub struct LocalModel {
    bit: Bit,
    handle: Arc<Mutex<Option<Child>>>,
    llm_model: Arc<LlamaCppModel>,
    pub port: u16,
}

#[derive(Clone, Debug, Default)]
struct LlamaServerTemplateOverride {
    chat_template: Option<String>,
    chat_template_file: Option<String>,
}

fn provider_param_as_string(provider: &ModelProvider, key: &str) -> Option<String> {
    provider
        .params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn resolve_template_override(provider: &ModelProvider) -> LlamaServerTemplateOverride {
    LlamaServerTemplateOverride {
        chat_template: provider_param_as_string(provider, "chat_template"),
        chat_template_file: provider_param_as_string(provider, "chat_template_file"),
    }
}

fn find_string_field(value: &JsonValue, key: &str) -> Option<String> {
    match value {
        JsonValue::Object(map) => {
            if let Some(string_value) = map.get(key).and_then(|value| value.as_str()) {
                return Some(string_value.to_owned());
            }

            map.values().find_map(|child| find_string_field(child, key))
        }
        JsonValue::Array(values) => values
            .iter()
            .find_map(|child| find_string_field(child, key)),
        _ => None,
    }
}

fn template_supports_tool_use(template: &str) -> bool {
    let template = template.to_lowercase();
    ["tool", "tool_call", "tool_calls", "function", "functions"]
        .iter()
        .any(|marker| template.contains(marker))
}

fn props_support_tool_use(props: &JsonValue) -> bool {
    find_string_field(props, "chat_template_tool_use")
        .is_some_and(|template| !template.trim().is_empty())
        || find_string_field(props, "chat_template")
            .is_some_and(|template| template_supports_tool_use(&template))
}

impl ModelMeta for LocalModel {
    fn get_bit(&self) -> Bit {
        self.bit.clone()
    }
}

#[flow_like_types::async_trait]
impl ModelLogic for LocalModel {
    async fn provider(&self) -> Result<flow_like_model_provider::llm::ModelConstructor> {
        self.llm_model.provider().await
    }

    async fn default_model(&self) -> Option<String> {
        self.llm_model.default_model().await
    }
}

impl LocalModel {
    pub async fn check_health(port: &str) -> Result<bool> {
        let response = reqwest::get(format!("http://127.0.0.1:{}/health", port)).await?;

        if response.status().is_success() {
            Ok(true)
        } else {
            Err(flow_like_types::anyhow!(
                "Model is not healthy: {}",
                response.status()
            ))
        }
    }

    async fn wait_until_ready(port: u16) -> Result<()> {
        let mut remaining_retries = 60;

        while remaining_retries > 0 {
            match Self::check_health(&port.to_string()).await {
                Ok(true) => return Ok(()),
                Ok(false) | Err(_) => {
                    sleep(Duration::from_secs(1)).await;
                    remaining_retries -= 1;
                }
            }
        }

        Err(flow_like_types::anyhow!(
            "Failed to start local model server"
        ))
    }

    async fn server_supports_tool_use(port: u16) -> Result<bool> {
        let response = reqwest::get(format!("http://127.0.0.1:{port}/props")).await?;
        if !response.status().is_success() {
            return Ok(false);
        }

        let props = response.json::<JsonValue>().await?;
        Ok(props_support_tool_use(&props))
    }

    fn resolve_context_length(model_context_length: Option<u32>, max_context_size: usize) -> u32 {
        let max_context_size = if max_context_size == 0 {
            DEFAULT_MAX_CONTEXT_SIZE as u32
        } else {
            u32::try_from(max_context_size).unwrap_or(u32::MAX)
        };
        let model_context_length = model_context_length
            .filter(|context_length| *context_length > 0)
            .unwrap_or(max_context_size);

        std::cmp::min(model_context_length, max_context_size)
    }

    fn server_args(
        gguf_path: &Path,
        context_length: u32,
        port: u16,
        gpu_mode: bool,
        projection_path: Option<&str>,
        template_override: &LlamaServerTemplateOverride,
    ) -> Vec<String> {
        let mut args = vec![
            "--model".to_string(),
            gguf_path.to_string_lossy().into_owned(),
            "--ctx-size".to_string(),
            context_length.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--parallel".to_string(),
            "1".to_string(),
            "--no-webui".to_string(),
            "--jinja".to_string(),
        ];

        if gpu_mode {
            args.extend([
                "--n-gpu-layers".to_string(),
                "auto".to_string(),
                "--flash-attn".to_string(),
                "auto".to_string(),
                "--fit".to_string(),
                "on".to_string(),
            ]);
        } else {
            args.extend([
                "--device".to_string(),
                "none".to_string(),
                "--n-gpu-layers".to_string(),
                "0".to_string(),
                "--flash-attn".to_string(),
                "off".to_string(),
            ]);
        }

        if let Some(projection_path) = projection_path {
            args.push("--mmproj".to_string());
            args.push(projection_path.to_string());
        }

        if let Some(chat_template_file) = template_override.chat_template_file.as_ref() {
            args.push("--chat-template-file".to_string());
            args.push(chat_template_file.clone());
        } else if let Some(chat_template) = template_override.chat_template.as_ref() {
            args.push("--chat-template".to_string());
            args.push(chat_template.clone());
        }

        args
    }

    async fn spawn_server(
        child_handle: &Arc<Mutex<Option<Child>>>,
        gguf_path: &Path,
        context_length: u32,
        port: u16,
        gpu_mode: bool,
        projection_path: Option<&str>,
        template_override: &LlamaServerTemplateOverride,
    ) -> Result<()> {
        let program = PathBuf::from("llama-server");
        let mut sidecar = crate::utils::execute::sidecar(&program, None).await?;
        let args = Self::server_args(
            gguf_path,
            context_length,
            port,
            gpu_mode,
            projection_path,
            template_override,
        );

        println!("Starting LLM Server with args: {:?}", args);

        let mut child = sidecar
            .args(&args)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| {
                flow_like_types::anyhow!("Failed to spawn local model sidecar: {}", error)
            })?;

        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let stderr = child.stderr.take().expect("Failed to capture stderr");

        *child_handle.lock().unwrap() = Some(child);

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let mut stdout_lines = stdout_reader.lines();
        let mut stderr_lines = stderr_reader.lines();

        tokio::spawn(async move {
            stdout_lines.by_ref().flatten().for_each(|line| {
                println!("[LLM] stdout: {}", line);
            });
        });

        tokio::spawn(async move {
            stderr_lines.by_ref().flatten().for_each(|line| {
                eprintln!("[LLM ERROR] stderr: {}", line);
            });
        });

        Ok(())
    }

    fn stop_server(child_handle: &Arc<Mutex<Option<Child>>>) {
        if let Ok(mut guard) = child_handle.lock()
            && let Some(mut child) = guard.take()
        {
            if let Err(error) = child.kill() {
                eprintln!(
                    "Failed to kill local model process during restart: {}",
                    error
                );
            }
            if let Err(error) = child.wait() {
                eprintln!(
                    "Failed to wait for local model process during restart: {}",
                    error
                );
            }
        }
    }

    pub async fn new(
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        execution_settings: &ExecutionSettings,
    ) -> flow_like_types::Result<LocalModel> {
        let bit_store = FlowLikeState::bit_store(&app_state).await?;

        let bit_store = match bit_store {
            FlowLikeStore::Local(store) => store,
            _ => return Err(flow_like_types::anyhow!("Only local store supported")),
        };

        let gguf_path = bit
            .to_path(&bit_store)
            .ok_or(flow_like_types::anyhow!("No model path"))?;
        let pack = bit.pack(app_state.clone()).await?;
        ensure_local_weights(&pack, &app_state, bit.id.as_str(), "local model").await?;
        let provider = bit
            .try_to_provider()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to get provider from bit"))?;
        let template_override = resolve_template_override(&provider);

        let projection_bit = pack
            .bits
            .iter()
            .find(|b| b.bit_type == BitTypes::Projection);
        let projection_bit = projection_bit.cloned();
        let projection_path = projection_bit
            .as_ref()
            .and_then(|bit| bit.to_path(&bit_store))
            .map(|path| path.to_string_lossy().into_owned());

        let child_handle = Arc::new(Mutex::new(None));
        let port = pick_unused_port().unwrap();

        let context_length = Self::resolve_context_length(
            bit.try_to_context_length(),
            execution_settings.max_context_size,
        );

        println!("Execution settings: {:?}", execution_settings);

        Self::spawn_server(
            &child_handle,
            &gguf_path,
            context_length,
            port,
            execution_settings.gpu_mode,
            projection_path.as_deref(),
            &template_override,
        )
        .await?;
        Self::wait_until_ready(port).await?;

        let should_probe_tool_template = projection_path.is_none()
            && template_override.chat_template.is_none()
            && template_override.chat_template_file.is_none();

        if should_probe_tool_template
            && !Self::server_supports_tool_use(port).await.unwrap_or(false)
        {
            println!(
                "Local model template does not advertise tool support. Restarting llama-server with chatml fallback."
            );
            Self::stop_server(&child_handle);

            let fallback_template = LlamaServerTemplateOverride {
                chat_template: Some("chatml".to_string()),
                chat_template_file: None,
            };

            Self::spawn_server(
                &child_handle,
                &gguf_path,
                context_length,
                port,
                execution_settings.gpu_mode,
                projection_path.as_deref(),
                &fallback_template,
            )
            .await?;
            Self::wait_until_ready(port).await?;
        }

        let llm_model = LlamaCppModel::new(&provider, port).await?;

        Ok(LocalModel {
            bit: bit.clone(),
            handle: child_handle,
            llm_model: Arc::new(llm_model),
            port,
        })
    }
}

impl Drop for LocalModel {
    fn drop(&mut self) {
        println!("DROPPING LOCAL MODEL");
        if let Ok(mut guard) = self.handle.lock() {
            if let Some(child) = guard.as_mut() {
                match child.kill() {
                    Ok(_) => println!("Child process was killed successfully."),
                    Err(e) => eprintln!("Failed to kill child process: {}", e),
                }
            } else {
                println!("No child process to kill.");
            }
        } else {
            println!("Failed to lock local model handle for dropping.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg_value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|window| window[0] == key)
            .map(|window| window[1].as_str())
    }

    #[test]
    fn server_args_use_automatic_gpu_offload() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            8192,
            9650,
            true,
            None,
            &LlamaServerTemplateOverride::default(),
        );

        assert_eq!(arg_value(&args, "--n-gpu-layers"), Some("auto"));
        assert_eq!(arg_value(&args, "--flash-attn"), Some("auto"));
        assert_eq!(arg_value(&args, "--fit"), Some("on"));
        assert_eq!(arg_value(&args, "--parallel"), Some("1"));
        assert_eq!(arg_value(&args, "--host"), Some("127.0.0.1"));
        assert_eq!(arg_value(&args, "--ctx-size"), Some("8192"));
        assert!(!args.iter().any(|arg| arg == "-ngl" || arg == "45"));
    }

    #[test]
    fn context_length_is_bounded_by_desktop_default() {
        assert_eq!(
            LocalModel::resolve_context_length(Some(128_000), DEFAULT_MAX_CONTEXT_SIZE),
            DEFAULT_MAX_CONTEXT_SIZE as u32
        );
        assert_eq!(
            LocalModel::resolve_context_length(Some(128_000), 0),
            DEFAULT_MAX_CONTEXT_SIZE as u32
        );
    }

    #[test]
    fn server_args_can_pin_context_size() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            16_384,
            9650,
            true,
            None,
            &LlamaServerTemplateOverride::default(),
        );

        assert_eq!(arg_value(&args, "--ctx-size"), Some("16384"));
    }

    #[test]
    fn server_args_can_disable_gpu_offload() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            8192,
            9650,
            false,
            None,
            &LlamaServerTemplateOverride::default(),
        );

        assert_eq!(arg_value(&args, "--device"), Some("none"));
        assert_eq!(arg_value(&args, "--n-gpu-layers"), Some("0"));
        assert_eq!(arg_value(&args, "--flash-attn"), Some("off"));
    }
}
