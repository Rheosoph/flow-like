#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use std::{collections::HashMap, sync::Mutex as StdMutex};

    use axum::{
        Json, Router,
        extract::{DefaultBodyLimit, State},
        http::{HeaderMap, StatusCode, header::AUTHORIZATION},
        response::{
            IntoResponse, Response, Sse,
            sse::{Event as SseEvent, KeepAlive},
        },
        routing::{get, post},
    };
    use flow_like_model_provider::llm::{ModelLogic, mlx::MlxModel as MlxProviderModel};
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_types::{
        Result, Value, async_trait,
        json::{self, json},
        tokio::{net::TcpListener, sync::mpsc, task::JoinHandle},
        utils::constant_time_eq,
    };
    use serde::{Deserialize, Serialize};

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use flow_like_types::tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        process::{Child, ChildStdin},
        sync::Mutex,
    };

    use crate::{
        bit::{Bit, BitTypes, MLX_PROVIDER_NAME, can_host_mlx},
        models::{
            ModelMeta,
            llm::{DEFAULT_MAX_CONTEXT_SIZE, ExecutionSettings},
            local_utils::ensure_local_weights,
        },
        state::FlowLikeState,
    };

    use crate::models::llm::mlx_pack::materialize_mlx_model;

    static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    const MAX_PROXY_BODY_SIZE: usize = 64 * 1024 * 1024;
    #[cfg(target_os = "ios")]
    const IOS_DEFAULT_MAX_KV_SIZE: u32 = 4_096;

    #[derive(Clone, Copy, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum MlxModelKind {
        Llm,
        Vlm,
    }

    impl MlxModelKind {
        fn from_bit(bit: &Bit) -> Result<Self> {
            match bit.bit_type {
                BitTypes::Llm => Ok(Self::Llm),
                BitTypes::Vlm => Ok(Self::Vlm),
                _ => Err(flow_like_types::anyhow!(
                    "MLX provider requires an LLM or VLM bit"
                )),
            }
        }
    }

    #[derive(Debug, Serialize)]
    struct MlxBridgeRequest {
        id: String,
        command: &'static str,
        model_directory: String,
        model_kind: MlxModelKind,
        request: Value,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct MlxBridgeEvent {
        id: String,
        event: String,
        #[serde(default)]
        data: Option<Value>,
        #[serde(default)]
        error: Option<String>,
    }

    impl MlxBridgeEvent {
        fn error(id: impl Into<String>, error: impl Into<String>) -> Self {
            Self {
                id: id.into(),
                event: "error".to_string(),
                data: None,
                error: Some(error.into()),
            }
        }

        fn is_terminal(&self) -> bool {
            matches!(self.event.as_str(), "complete" | "error")
        }
    }

    #[async_trait]
    trait MlxTransport: Send + Sync {
        async fn generate(
            &self,
            request: MlxBridgeRequest,
        ) -> Result<mpsc::UnboundedReceiver<MlxBridgeEvent>>;

        fn cancel(&self, _request_id: &str) {}

        fn unload(&self) {}
    }

    struct MlxRequestGuard {
        request_id: Option<String>,
        transport: Arc<dyn MlxTransport>,
    }

    impl MlxRequestGuard {
        fn new(request_id: String, transport: Arc<dyn MlxTransport>) -> Self {
            Self {
                request_id: Some(request_id),
                transport,
            }
        }

        fn disarm(&mut self) {
            self.request_id = None;
        }
    }

    impl Drop for MlxRequestGuard {
        fn drop(&mut self) {
            if let Some(request_id) = self.request_id.take() {
                self.transport.cancel(&request_id);
            }
        }
    }

    #[derive(Clone)]
    struct MlxProxyState {
        bearer_token: String,
        model_directory: String,
        model_kind: MlxModelKind,
        transport: Arc<dyn MlxTransport>,
    }

    fn next_request_id() -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_micros());
        let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!("mlx-{timestamp}-{sequence}")
    }

    fn has_valid_authorization(headers: &HeaderMap, bearer_token: &str) -> bool {
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| constant_time_eq(value.as_bytes(), bearer_token.as_bytes()))
    }

    fn unauthorized_response() -> Response {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "Missing or invalid MLX proxy authorization"
                }
            })),
        )
            .into_response()
    }

    async fn health(State(state): State<MlxProxyState>, headers: HeaderMap) -> Response {
        if !has_valid_authorization(&headers, &state.bearer_token) {
            return unauthorized_response();
        }
        StatusCode::OK.into_response()
    }

    fn tool_calls_as_stream_delta(tool_calls: &Value) -> Value {
        let Some(tool_calls) = tool_calls.as_array() else {
            return tool_calls.clone();
        };

        Value::Array(
            tool_calls
                .iter()
                .enumerate()
                .map(|(index, tool_call)| {
                    let mut tool_call = tool_call.clone();
                    if let Some(tool_call) = tool_call.as_object_mut() {
                        tool_call
                            .entry("index".to_string())
                            .or_insert_with(|| json!(index));
                    }
                    tool_call
                })
                .collect(),
        )
    }

    fn completion_as_stream_chunk(completion: &Value) -> Option<Value> {
        let choice = completion.get("choices")?.as_array()?.first()?;
        let message = choice.get("message")?;
        let mut delta = json::Map::new();
        delta.insert("role".to_string(), json!("assistant"));

        if let Some(content) = message.get("content") {
            delta.insert("content".to_string(), content.clone());
        }
        if let Some(reasoning) = message.get("reasoning_content") {
            delta.insert("reasoning_content".to_string(), reasoning.clone());
        }
        if let Some(tool_calls) = message.get("tool_calls") {
            delta.insert(
                "tool_calls".to_string(),
                tool_calls_as_stream_delta(tool_calls),
            );
        }

        Some(json!({
            "id": completion.get("id").cloned().unwrap_or_else(|| json!(next_request_id())),
            "object": "chat.completion.chunk",
            "created": completion.get("created").cloned().unwrap_or_else(|| json!(0)),
            "model": completion.get("model").cloned().unwrap_or_else(|| json!("mlx")),
            "choices": [{
                "index": choice.get("index").cloned().unwrap_or_else(|| json!(0)),
                "delta": Value::Object(delta),
                "finish_reason": choice.get("finish_reason").cloned().unwrap_or(Value::Null)
            }],
            "usage": completion.get("usage").cloned().unwrap_or(Value::Null)
        }))
    }

    fn usage_as_stream_chunk(completion: &Value) -> Option<Value> {
        let usage = completion.get("usage")?.clone();
        if usage.is_null() {
            return None;
        }

        Some(json!({
            "id": completion.get("id").cloned().unwrap_or_else(|| json!(next_request_id())),
            "object": "chat.completion.chunk",
            "created": completion.get("created").cloned().unwrap_or_else(|| json!(0)),
            "model": completion.get("model").cloned().unwrap_or_else(|| json!("mlx")),
            "choices": [],
            "usage": usage
        }))
    }

    async fn chat_completions(
        State(state): State<MlxProxyState>,
        headers: HeaderMap,
        Json(request): Json<Value>,
    ) -> Response {
        if !has_valid_authorization(&headers, &state.bearer_token) {
            return unauthorized_response();
        }

        let request_id = next_request_id();
        let is_streaming = request
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let bridge_request = MlxBridgeRequest {
            id: request_id.clone(),
            command: "generate",
            model_directory: state.model_directory.clone(),
            model_kind: state.model_kind,
            request,
        };

        let mut receiver = match state.transport.generate(bridge_request).await {
            Ok(receiver) => receiver,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": { "message": error.to_string() } })),
                )
                    .into_response();
            }
        };
        let mut cancellation = MlxRequestGuard::new(request_id.clone(), state.transport.clone());

        if !is_streaming {
            while let Some(event) = receiver.recv().await {
                match event.event.as_str() {
                    "complete" => {
                        cancellation.disarm();
                        return match event.data {
                            Some(data) => Json(data).into_response(),
                            None => (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({
                                    "error": {
                                        "message": "MLX returned an empty completion"
                                    }
                                })),
                            )
                                .into_response(),
                        };
                    }
                    "error" => {
                        cancellation.disarm();
                        return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": {
                                "message": event.error.unwrap_or_else(|| "MLX generation failed".to_string())
                            }
                        })),
                    )
                        .into_response();
                    }
                    _ => {}
                }
            }

            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": "MLX bridge closed before completing" } })),
            )
                .into_response();
        }

        let stream = flow_like_types::async_stream::stream! {
            let mut saw_chunk = false;
            while let Some(event) = receiver.recv().await {
                match event.event.as_str() {
                    "chunk" => {
                        if let Some(data) = event.data {
                            saw_chunk = true;
                            yield Ok::<SseEvent, Infallible>(
                                SseEvent::default().data(data.to_string())
                            );
                        }
                    }
                    "complete" => {
                        cancellation.disarm();
                        if let Some(data) = event.data {
                            if !saw_chunk
                                && let Some(chunk) = completion_as_stream_chunk(&data)
                            {
                                yield Ok::<SseEvent, Infallible>(
                                    SseEvent::default().data(chunk.to_string())
                                );
                            } else if let Some(usage) = usage_as_stream_chunk(&data) {
                                yield Ok::<SseEvent, Infallible>(
                                    SseEvent::default().data(usage.to_string())
                                );
                            }
                        }
                        yield Ok::<SseEvent, Infallible>(SseEvent::default().data("[DONE]"));
                        break;
                    }
                    "error" => {
                        cancellation.disarm();
                        let error = event.error.unwrap_or_else(|| "MLX generation failed".to_string());
                        yield Ok::<SseEvent, Infallible>(
                            SseEvent::default().data(
                                json!({ "error": { "message": error } }).to_string()
                            )
                        );
                        yield Ok::<SseEvent, Infallible>(SseEvent::default().data("[DONE]"));
                        break;
                    }
                    _ => {}
                }
            }
        };

        Sse::new(stream)
            .keep_alive(KeepAlive::new().text("keep-alive"))
            .into_response()
    }

    async fn start_proxy(state: MlxProxyState) -> Result<(u16, JoinHandle<()>)> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let router = Router::new()
            .route("/health", get(health))
            .route("/v1/chat/completions", post(chat_completions))
            .layer(DefaultBodyLimit::max(MAX_PROXY_BODY_SIZE))
            .with_state(state);
        let task = flow_like_types::tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::error!(%error, "MLX compatibility proxy stopped");
            }
        });
        Ok((port, task))
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    struct MacMlxTransport {
        child: Mutex<Child>,
        stdin: Arc<Mutex<ChildStdin>>,
        pending: Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<MlxBridgeEvent>>>>,
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    impl MacMlxTransport {
        async fn new() -> Result<Self> {
            use std::{path::PathBuf, process::Stdio};

            let mut command =
                crate::utils::execute::async_sidecar(&PathBuf::from("flow-like-mlx-service"))
                    .await?;
            command.kill_on_drop(true);
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| {
                    flow_like_types::anyhow!("Failed to start the MLX service: {error}")
                })?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| flow_like_types::anyhow!("MLX service stdin is unavailable"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| flow_like_types::anyhow!("MLX service stdout is unavailable"))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| flow_like_types::anyhow!("MLX service stderr is unavailable"))?;

            let pending: Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<MlxBridgeEvent>>>> =
                Arc::new(StdMutex::new(HashMap::new()));
            let stdout_pending = pending.clone();
            drop(flow_like_types::tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Some(line) = lines.next_line().await.transpose() {
                    let line = match line {
                        Ok(line) => line,
                        Err(error) => {
                            tracing::error!(%error, "Failed reading from MLX service");
                            break;
                        }
                    };
                    let event = match json::from_str::<MlxBridgeEvent>(&line) {
                        Ok(event) => event,
                        Err(error) => {
                            tracing::warn!(%error, output = %line, "Ignoring invalid MLX service event");
                            continue;
                        }
                    };
                    let terminal = event.is_terminal();
                    let sender = stdout_pending
                        .lock()
                        .ok()
                        .and_then(|pending| pending.get(&event.id).cloned());
                    if let Some(sender) = sender {
                        let _ = sender.send(event.clone());
                    }
                    if terminal && let Ok(mut pending) = stdout_pending.lock() {
                        pending.remove(&event.id);
                    }
                }

                if let Ok(mut pending) = stdout_pending.lock() {
                    for (id, sender) in pending.drain() {
                        let _ = sender.send(MlxBridgeEvent::error(
                            id,
                            "MLX service stopped unexpectedly",
                        ));
                    }
                }
            }));
            drop(flow_like_types::tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => tracing::info!(output = %line, "MLX service"),
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(%error, "Failed reading MLX service diagnostics");
                            break;
                        }
                    }
                }
            }));

            Ok(Self {
                child: Mutex::new(child),
                stdin: Arc::new(Mutex::new(stdin)),
                pending,
            })
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[async_trait]
    impl MlxTransport for MacMlxTransport {
        async fn generate(
            &self,
            request: MlxBridgeRequest,
        ) -> Result<mpsc::UnboundedReceiver<MlxBridgeEvent>> {
            let id = request.id.clone();
            let (sender, receiver) = mpsc::unbounded_channel();
            self.pending
                .lock()
                .map_err(|_| flow_like_types::anyhow!("MLX request map is poisoned"))?
                .insert(id.clone(), sender);

            let mut payload = json::to_vec(&request)?;
            payload.push(b'\n');
            let write_result = {
                let mut stdin = self.stdin.lock().await;
                stdin.write_all(&payload).await
            };
            if let Err(error) = write_result {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                return Err(flow_like_types::anyhow!(
                    "Failed to send request to MLX service: {error}"
                ));
            }
            Ok(receiver)
        }

        fn cancel(&self, request_id: &str) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(request_id);
            }

            let mut payload = json!({
                "id": request_id,
                "command": "cancel",
            })
            .to_string()
            .into_bytes();
            payload.push(b'\n');

            let stdin = self.stdin.clone();
            let request_id = request_id.to_string();
            let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() else {
                tracing::warn!(%request_id, "Could not send MLX cancellation outside a Tokio runtime");
                return;
            };
            drop(runtime.spawn(async move {
                let write_result = {
                    let mut stdin = stdin.lock().await;
                    stdin.write_all(&payload).await
                };
                if let Err(error) = write_result {
                    tracing::warn!(%error, %request_id, "Failed to send MLX cancellation");
                }
            }));
        }

        fn unload(&self) {
            if let Ok(mut child) = self.child.try_lock() {
                let _ = child.start_kill();
            }
        }
    }

    #[cfg(all(
        target_os = "ios",
        target_arch = "aarch64",
        not(any(target_abi = "sim", target_abi = "macabi"))
    ))]
    mod ios {
        use std::{
            ffi::{CStr, CString, c_char, c_int, c_void},
            sync::atomic::{AtomicBool, Ordering},
        };

        use super::*;

        type BridgeCallback = extern "C" fn(*const c_char, *mut c_void);

        unsafe extern "C" {
            fn flow_like_mlx_is_available() -> c_int;
            fn flow_like_mlx_generate(
                request_json: *const c_char,
                callback: BridgeCallback,
                context: *mut c_void,
            ) -> c_int;
            fn flow_like_mlx_cancel(request_id: *const c_char);
            fn flow_like_mlx_unload(model_directory: *const c_char);
        }

        struct CallbackContext {
            sender: mpsc::UnboundedSender<MlxBridgeEvent>,
            finished: AtomicBool,
        }

        extern "C" fn bridge_callback(event_json: *const c_char, context: *mut c_void) {
            if event_json.is_null() || context.is_null() {
                return;
            }

            // Swift promises that the event string remains valid for this callback.
            let payload = unsafe { CStr::from_ptr(event_json) }.to_string_lossy();
            let event = match json::from_str::<MlxBridgeEvent>(&payload) {
                Ok(event) => event,
                Err(error) => {
                    tracing::error!(%error, "MLX bridge returned invalid JSON");
                    return;
                }
            };
            let terminal = event.is_terminal();
            let callback_context = unsafe { &*(context.cast::<CallbackContext>()) };
            let _ = callback_context.sender.send(event);
            if terminal
                && callback_context
                    .finished
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                // Terminal callbacks transfer the allocation back to Rust.
                unsafe {
                    drop(Box::from_raw(context.cast::<CallbackContext>()));
                }
            }
        }

        pub(super) struct IosMlxTransport {
            model_directory: CString,
        }

        impl IosMlxTransport {
            pub(super) fn new(model_directory: &str) -> Result<Self> {
                let available = unsafe { flow_like_mlx_is_available() };
                if available == 0 {
                    return Err(flow_like_types::anyhow!(
                        "MLX is unavailable on this iOS device"
                    ));
                }
                Ok(Self {
                    model_directory: CString::new(model_directory)?,
                })
            }
        }

        #[async_trait]
        impl MlxTransport for IosMlxTransport {
            async fn generate(
                &self,
                request: MlxBridgeRequest,
            ) -> Result<mpsc::UnboundedReceiver<MlxBridgeEvent>> {
                let payload = CString::new(json::to_string(&request)?)?;
                let (sender, receiver) = mpsc::unbounded_channel();
                let context = Box::new(CallbackContext {
                    sender,
                    finished: AtomicBool::new(false),
                });
                let context = Box::into_raw(context);
                let status = unsafe {
                    flow_like_mlx_generate(
                        payload.as_ptr(),
                        bridge_callback,
                        context.cast::<c_void>(),
                    )
                };
                if status != 0 {
                    unsafe {
                        drop(Box::from_raw(context));
                    }
                    return Err(flow_like_types::anyhow!(
                        "MLX bridge rejected the generation request ({status})"
                    ));
                }
                Ok(receiver)
            }

            fn cancel(&self, request_id: &str) {
                if let Ok(request_id) = CString::new(request_id) {
                    unsafe {
                        flow_like_mlx_cancel(request_id.as_ptr());
                    }
                }
            }

            fn unload(&self) {
                unsafe {
                    flow_like_mlx_unload(self.model_directory.as_ptr());
                }
            }
        }
    }

    async fn native_transport(model_directory: &str) -> Result<Arc<dyn MlxTransport>> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let _ = model_directory;
            Ok(Arc::new(MacMlxTransport::new().await?))
        }
        #[cfg(all(
            target_os = "ios",
            target_arch = "aarch64",
            not(any(target_abi = "sim", target_abi = "macabi"))
        ))]
        {
            return Ok(Arc::new(ios::IosMlxTransport::new(model_directory)?));
        }
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(
                target_os = "ios",
                target_arch = "aarch64",
                not(any(target_abi = "sim", target_abi = "macabi"))
            )
        )))]
        {
            let _ = model_directory;
            Err(flow_like_types::anyhow!(
                "MLX is only available on Apple-silicon macOS and iOS devices"
            ))
        }
    }

    struct MlxRuntime {
        server: JoinHandle<()>,
        transport: Arc<dyn MlxTransport>,
    }

    impl Drop for MlxRuntime {
        fn drop(&mut self) {
            self.server.abort();
            self.transport.unload();
        }
    }

    pub struct MlxModel {
        bit: Bit,
        _runtime: Arc<MlxRuntime>,
        model: Arc<MlxProviderModel>,
        pub port: u16,
    }

    impl ModelMeta for MlxModel {
        fn get_bit(&self) -> Bit {
            self.bit.clone()
        }
    }

    #[async_trait]
    impl ModelLogic for MlxModel {
        async fn provider(&self) -> Result<flow_like_model_provider::llm::ModelConstructor> {
            self.model.provider().await
        }

        async fn default_model(&self) -> Option<String> {
            self.model.default_model().await
        }

        fn additional_params(
            &self,
            history: &Option<flow_like_model_provider::history::History>,
        ) -> Option<Value> {
            self.model.additional_params(history)
        }

        fn usage_reporting(&self) -> flow_like_model_provider::llm::UsageReportingMode {
            self.model.usage_reporting()
        }
    }

    fn default_max_kv_size(bit: &Bit, execution_settings: &ExecutionSettings) -> u32 {
        let configured_limit = if execution_settings.max_context_size == 0 {
            DEFAULT_MAX_CONTEXT_SIZE as u32
        } else {
            u32::try_from(execution_settings.max_context_size).unwrap_or(u32::MAX)
        };
        let model_limit = bit
            .try_to_context_length()
            .filter(|context_length| *context_length > 0)
            .unwrap_or(configured_limit);
        let resolved = model_limit.min(configured_limit).max(1);

        #[cfg(target_os = "ios")]
        {
            resolved.min(IOS_DEFAULT_MAX_KV_SIZE)
        }
        #[cfg(not(target_os = "ios"))]
        {
            resolved
        }
    }

    impl MlxModel {
        pub async fn new(
            bit: &Bit,
            app_state: Arc<FlowLikeState>,
            execution_settings: &ExecutionSettings,
        ) -> Result<Self> {
            if !bit.is_mlx_model() {
                return Err(flow_like_types::anyhow!(
                    "Expected an LLM or VLM bit using the {MLX_PROVIDER_NAME} provider"
                ));
            }
            if !can_host_mlx() {
                return Err(flow_like_types::anyhow!(
                    "MLX can only run on supported Apple-silicon macOS or iOS devices"
                ));
            }

            let model_kind = MlxModelKind::from_bit(bit)?;
            let bit_store = FlowLikeState::bit_store(&app_state).await?;
            let FlowLikeStore::Local(bit_store) = bit_store else {
                return Err(flow_like_types::anyhow!("MLX requires a local model store"));
            };

            let pack = bit.pack(app_state.clone()).await?;
            ensure_local_weights(&pack, &app_state, bit.id.as_str(), "MLX model").await?;
            let materialization_bit = bit.clone();
            let materialization_store = bit_store.clone();
            let materialized = flow_like_types::tokio::task::spawn_blocking(move || {
                materialize_mlx_model(&materialization_bit, &pack, &materialization_store)
            })
            .await
            .map_err(|error| {
                flow_like_types::anyhow!("MLX model materialization task failed: {error}")
            })??;
            let model_directory = materialized.path.to_string_lossy().into_owned();
            let transport = native_transport(&model_directory).await?;
            let bearer_token = flow_like_types::create_id();
            let proxy_state = MlxProxyState {
                bearer_token: bearer_token.clone(),
                model_directory,
                model_kind,
                transport: transport.clone(),
            };
            let (port, server) = start_proxy(proxy_state).await?;
            let runtime = Arc::new(MlxRuntime { server, transport });

            let mut provider = bit.try_to_provider().ok_or_else(|| {
                flow_like_types::anyhow!("Failed to read the MLX provider configuration")
            })?;
            if provider.model_id.as_deref().is_none_or(str::is_empty) {
                provider.model_id = Some(bit.id.clone());
            }
            provider
                .params
                .get_or_insert_default()
                .entry("max_kv_size".to_string())
                .or_insert_with(|| json!(default_max_kv_size(bit, execution_settings)));
            let model = Arc::new(
                MlxProviderModel::new_with_keepalive(
                    &provider,
                    port,
                    &bearer_token,
                    runtime.clone(),
                )
                .await?,
            );

            Ok(Self {
                bit: bit.clone(),
                _runtime: runtime,
                model,
                port,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn completion_response_can_be_buffered_into_one_stream_chunk() {
            let response = json!({
                "id": "answer-1",
                "model": "mlx-test",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hello"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
            });
            let chunk = completion_as_stream_chunk(&response).expect("stream chunk");
            assert_eq!(chunk["choices"][0]["delta"]["content"], "hello");
            assert_eq!(chunk["choices"][0]["finish_reason"], "stop");
            assert_eq!(chunk["usage"]["total_tokens"], 3);
        }

        #[test]
        fn buffered_tool_calls_receive_streaming_indices() {
            let response = json!({
                "id": "answer-1",
                "model": "mlx-test",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call-1",
                                "type": "function",
                                "function": {"name": "first", "arguments": "{}"}
                            },
                            {
                                "id": "call-2",
                                "type": "function",
                                "function": {"name": "second", "arguments": "{}"}
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
            });
            let chunk = completion_as_stream_chunk(&response).expect("stream chunk");
            assert_eq!(chunk["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
            assert_eq!(chunk["choices"][0]["delta"]["tool_calls"][1]["index"], 1);
        }

        #[test]
        fn default_kv_size_honors_model_and_execution_limits() {
            let parameters = crate::bit::LLMParameters {
                context_length: 16_384,
                provider: flow_like_model_provider::provider::ModelProvider {
                    api_surface: None,
                    provider_name: MLX_PROVIDER_NAME.to_string(),
                    model_id: None,
                    version: None,
                    params: None,
                },
                model_classification: crate::bit::BitModelClassification::default(),
            };
            let bit = Bit {
                bit_type: BitTypes::Llm,
                parameters: json::to_value(parameters).unwrap(),
                ..Bit::default()
            };
            let settings = ExecutionSettings {
                gpu_mode: true,
                max_context_size: 8_192,
            };

            #[cfg(target_os = "macos")]
            assert_eq!(default_max_kv_size(&bit, &settings), 8_192);
            #[cfg(target_os = "ios")]
            assert_eq!(
                default_max_kv_size(&bit, &settings),
                IOS_DEFAULT_MAX_KV_SIZE
            );
        }

        #[test]
        fn proxy_authorization_requires_the_runtime_bearer_token() {
            let mut headers = HeaderMap::new();
            assert!(!has_valid_authorization(&headers, "runtime-secret"));

            headers.insert(AUTHORIZATION, "Bearer wrong-secret".parse().unwrap());
            assert!(!has_valid_authorization(&headers, "runtime-secret"));

            headers.insert(AUTHORIZATION, "Bearer runtime-secret".parse().unwrap());
            assert!(has_valid_authorization(&headers, "runtime-secret"));

            headers.insert(AUTHORIZATION, "bearer runtime-secret".parse().unwrap());
            assert!(!has_valid_authorization(&headers, "runtime-secret"));

            headers.insert(AUTHORIZATION, "Bearer  runtime-secret".parse().unwrap());
            assert!(!has_valid_authorization(&headers, "runtime-secret"));
        }

        #[test]
        fn only_llm_and_vlm_bits_have_an_mlx_kind() {
            let mut bit = Bit {
                bit_type: BitTypes::Llm,
                ..Bit::default()
            };
            assert!(matches!(
                MlxModelKind::from_bit(&bit),
                Ok(MlxModelKind::Llm)
            ));
            bit.bit_type = BitTypes::Vlm;
            assert!(matches!(
                MlxModelKind::from_bit(&bit),
                Ok(MlxModelKind::Vlm)
            ));
            bit.bit_type = BitTypes::Embedding;
            assert!(MlxModelKind::from_bit(&bit).is_err());
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::MlxModel;

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod unsupported {
    use std::sync::Arc;

    use flow_like_model_provider::llm::{ModelConstructor, ModelLogic};
    use flow_like_types::{Result, async_trait};

    use crate::{bit::Bit, models::llm::ExecutionSettings, state::FlowLikeState};

    /// Compile-time placeholder that keeps the model factory portable. The
    /// constructor always fails before an instance can be created.
    pub struct MlxModel;

    #[async_trait]
    impl ModelLogic for MlxModel {
        async fn provider(&self) -> Result<ModelConstructor> {
            Err(flow_like_types::anyhow!(
                "MLX is only available on Apple-silicon macOS and physical iOS devices"
            ))
        }

        async fn default_model(&self) -> Option<String> {
            None
        }
    }

    impl MlxModel {
        pub async fn new(
            _bit: &Bit,
            _app_state: Arc<FlowLikeState>,
            _execution_settings: &ExecutionSettings,
        ) -> Result<Self> {
            Err(flow_like_types::anyhow!(
                "MLX is only available on Apple-silicon macOS and physical iOS devices"
            ))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub use unsupported::MlxModel;
