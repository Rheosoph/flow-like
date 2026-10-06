#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use std::{
        convert::Infallible,
        path::PathBuf,
        sync::{
            Arc, Weak,
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
    use flow_like_model_provider::{
        llm::{ModelLogic, mlx::MlxModel as MlxProviderModel},
        provider::ModelProvider,
    };
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
        process::{Child, ChildStderr, ChildStdin, ChildStdout},
        sync::{Mutex, watch},
    };

    use crate::{
        bit::{Bit, MLX_PROVIDER_NAME, can_host_mlx},
        models::{
            ModelMeta,
            llm::{
                DEFAULT_MAX_CONTEXT_SIZE, ExecutionSettings, LOCAL_ENGINE_LOADS,
                mlx_pack::{MaterializedMlxModel, MlxModelKind, materialize_mlx_model},
            },
            local_utils::ensure_local_weights,
        },
        state::FlowLikeState,
        utils::execute::RuntimeLocator,
    };

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use crate::utils::execute::MlxServiceRuntime;

    static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    const MAX_PROXY_BODY_SIZE: usize = 64 * 1024 * 1024;
    #[cfg(target_os = "ios")]
    const IOS_DEFAULT_MAX_KV_SIZE: u32 = 4_096;

    /// The loader family as the MLX bridge names it.
    #[derive(Clone, Copy, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum BridgeKind {
        Llm,
        Vlm,
    }

    impl From<MlxModelKind> for BridgeKind {
        fn from(kind: MlxModelKind) -> Self {
            match kind {
                MlxModelKind::Llm => Self::Llm,
                MlxModelKind::Vlm => Self::Vlm,
            }
        }
    }

    #[derive(Debug, Serialize)]
    struct MlxBridgeRequest {
        id: String,
        command: String,
        model_directory: String,
        model_kind: BridgeKind,
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

        /// Resolves once the runtime serves no more requests; an in-process runtime never ends.
        async fn exited(&self) {
            std::future::pending::<()>().await;
        }
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

    /// What a served model allows every request, whatever the request asks for.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct MlxRequestLimits {
        /// Tokens the KV cache holds at most; a request may ask for fewer.
        pub max_kv_size: Option<u32>,
        /// Bits per KV cache value at most, 4 or 8; a request may ask for fewer.
        pub kv_bits: Option<u8>,
        /// Images only as `data:` URLs: the helper reads local files and fetches URLs itself,
        /// which a model served to other people must not do for them.
        pub inline_images_only: bool,
    }

    impl MlxRequestLimits {
        fn apply(&self, request: &mut Value) {
            let Some(request) = request.as_object_mut() else {
                return;
            };
            cap(request, "max_kv_size", self.max_kv_size.map(u64::from));
            cap(request, "kv_bits", self.kv_bits.map(u64::from));
        }

        /// Whether the request links an image, where only inline images may reach the helper.
        fn refuses_images_of(&self, request: &Value) -> bool {
            self.inline_images_only
                && request
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|message| message.get("content")?.as_array())
                    .flatten()
                    .filter_map(|part| part.get("image_url"))
                    .map(|image| image.get("url").unwrap_or(image))
                    .any(|url| !url.as_str().is_some_and(|url| url.starts_with("data:")))
        }
    }

    /// Keeps a field at most `limit`: a larger, missing or non-numeric value becomes the limit.
    fn cap(request: &mut json::Map<String, Value>, field: &str, limit: Option<u64>) {
        let Some(limit) = limit else {
            return;
        };
        let value = request
            .get(field)
            .and_then(Value::as_u64)
            .filter(|value| *value <= limit)
            .unwrap_or(limit);
        request.insert(field.to_owned(), json!(value));
    }

    /// One MLX model and the limits its requests get.
    #[derive(Clone, Debug)]
    pub struct MlxServedModel {
        /// A Hugging Face style MLX directory: `config.json`, tokenizer files and safetensors.
        pub directory: PathBuf,
        pub kind: MlxModelKind,
        pub limits: MlxRequestLimits,
    }

    struct Served {
        directory: String,
        kind: BridgeKind,
        limits: MlxRequestLimits,
        transport: Arc<dyn MlxTransport>,
    }

    impl Served {
        /// Sends a request within the limits; the guard cancels it unless disarmed.
        async fn generate(
            &self,
            mut request: Value,
        ) -> Result<(mpsc::UnboundedReceiver<MlxBridgeEvent>, MlxRequestGuard)> {
            self.limits.apply(&mut request);
            let id = next_request_id();
            let receiver = self
                .transport
                .generate(MlxBridgeRequest {
                    id: id.clone(),
                    command: "generate".to_owned(),
                    model_directory: self.directory.clone(),
                    model_kind: self.kind,
                    request,
                })
                .await?;
            Ok((receiver, MlxRequestGuard::new(id, self.transport.clone())))
        }
    }

    #[derive(Clone)]
    struct MlxProxyState {
        bearer_token: Arc<str>,
        served: Arc<Served>,
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

    fn error_response(status: StatusCode, message: &str) -> Response {
        (status, Json(json!({ "error": { "message": message } }))).into_response()
    }

    fn unauthorized_response() -> Response {
        error_response(
            StatusCode::UNAUTHORIZED,
            "Missing or invalid MLX proxy authorization",
        )
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

    /// A stream chunk that carries the identity of `completion`.
    fn stream_chunk(completion: &Value, choices: Value, usage: Value) -> Value {
        json!({
            "id": completion.get("id").cloned().unwrap_or_else(|| json!(next_request_id())),
            "object": "chat.completion.chunk",
            "created": completion.get("created").cloned().unwrap_or_else(|| json!(0)),
            "model": completion.get("model").cloned().unwrap_or_else(|| json!("mlx")),
            "choices": choices,
            "usage": usage
        })
    }

    fn message_as_stream_delta(message: &Value) -> Value {
        let mut delta = json::Map::new();
        delta.insert("role".to_string(), json!("assistant"));
        for field in ["content", "reasoning_content"] {
            if let Some(value) = message.get(field) {
                delta.insert(field.to_string(), value.clone());
            }
        }
        if let Some(tool_calls) = message.get("tool_calls") {
            delta.insert(
                "tool_calls".to_string(),
                tool_calls_as_stream_delta(tool_calls),
            );
        }
        Value::Object(delta)
    }

    fn completion_as_stream_chunk(completion: &Value) -> Option<Value> {
        let choice = completion.get("choices")?.as_array()?.first()?;
        let delta = message_as_stream_delta(choice.get("message")?);
        let choices = json!([{
            "index": choice.get("index").cloned().unwrap_or_else(|| json!(0)),
            "delta": delta,
            "finish_reason": choice.get("finish_reason").cloned().unwrap_or(Value::Null)
        }]);
        let usage = completion.get("usage").cloned().unwrap_or(Value::Null);
        Some(stream_chunk(completion, choices, usage))
    }

    fn usage_as_stream_chunk(completion: &Value) -> Option<Value> {
        let usage = completion.get("usage")?.clone();
        if usage.is_null() {
            return None;
        }
        Some(stream_chunk(completion, json!([]), usage))
    }

    /// The completion a non-streaming request ends with, or the message of the error that ended it.
    async fn completion(
        receiver: &mut mpsc::UnboundedReceiver<MlxBridgeEvent>,
        cancellation: &mut MlxRequestGuard,
    ) -> std::result::Result<Value, String> {
        while let Some(event) = receiver.recv().await {
            if !event.is_terminal() {
                continue;
            }
            cancellation.disarm();
            if event.event == "error" {
                return Err(event
                    .error
                    .unwrap_or_else(|| "MLX generation failed".to_string()));
            }
            return event
                .data
                .ok_or_else(|| "MLX returned an empty completion".to_string());
        }
        Err("MLX bridge closed before completing".to_string())
    }

    fn sse_data(data: impl AsRef<str>) -> std::result::Result<SseEvent, Infallible> {
        Ok(SseEvent::default().data(data))
    }

    /// What a completion adds to its stream: all of it when nothing streamed before, else its
    /// usage.
    fn closing_chunk(completion: Option<Value>, saw_chunk: bool) -> Option<Value> {
        let completion = completion?;
        (!saw_chunk)
            .then(|| completion_as_stream_chunk(&completion))
            .flatten()
            .or_else(|| usage_as_stream_chunk(&completion))
    }

    /// Server-sent chunks as the bridge emits them; a completion that streamed none arrives as one.
    fn stream_response(
        mut receiver: mpsc::UnboundedReceiver<MlxBridgeEvent>,
        mut cancellation: MlxRequestGuard,
    ) -> Response {
        let stream = flow_like_types::async_stream::stream! {
            let mut saw_chunk = false;
            while let Some(event) = receiver.recv().await {
                match event.event.as_str() {
                    "chunk" => if let Some(data) = event.data {
                        saw_chunk = true;
                        yield sse_data(data.to_string());
                    },
                    "complete" => {
                        cancellation.disarm();
                        if let Some(chunk) = closing_chunk(event.data, saw_chunk) {
                            yield sse_data(chunk.to_string());
                        }
                        yield sse_data("[DONE]");
                        break;
                    }
                    "error" => {
                        cancellation.disarm();
                        let error = event.error.unwrap_or_else(|| "MLX generation failed".to_string());
                        yield sse_data(json!({ "error": { "message": error } }).to_string());
                        yield sse_data("[DONE]");
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

    async fn chat_completions(
        State(state): State<MlxProxyState>,
        headers: HeaderMap,
        Json(request): Json<Value>,
    ) -> Response {
        if !has_valid_authorization(&headers, &state.bearer_token) {
            return unauthorized_response();
        }
        if state.served.limits.refuses_images_of(&request) {
            return error_response(StatusCode::BAD_REQUEST, "Images must be sent as data: URLs");
        }

        let is_streaming = request
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (mut receiver, mut cancellation) = match state.served.generate(request).await {
            Ok(started) => started,
            Err(error) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
            }
        };
        if is_streaming {
            return stream_response(receiver, cancellation);
        }
        match completion(&mut receiver, &mut cancellation).await {
            Ok(data) => Json(data).into_response(),
            Err(message) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
        }
    }

    fn router(state: MlxProxyState) -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/v1/chat/completions", post(chat_completions))
            .layer(DefaultBodyLimit::max(MAX_PROXY_BODY_SIZE))
            .with_state(state)
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    type PendingRequests = Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<MlxBridgeEvent>>>>;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    struct MacMlxTransport {
        child: Mutex<Child>,
        stdin: Arc<Mutex<ChildStdin>>,
        pending: PendingRequests,
        exited: watch::Receiver<bool>,
    }

    /// Hands one event line of the helper to its request; a terminal event ends the request.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn deliver(pending: &PendingRequests, line: &str) {
        let event = match json::from_str::<MlxBridgeEvent>(line) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, output = %line, "Ignoring invalid MLX service event");
                return;
            }
        };
        let sender = {
            let Ok(mut pending) = pending.lock() else {
                return;
            };
            if event.is_terminal() {
                pending.remove(&event.id)
            } else {
                pending.get(&event.id).cloned()
            }
        };
        if let Some(sender) = sender {
            let _ = sender.send(event);
        }
    }

    /// Routes the events of the helper to their requests. Once it exits, every open request
    /// fails and `exited` turns true.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    async fn route_events(
        stdout: ChildStdout,
        pending: PendingRequests,
        exited: watch::Sender<bool>,
    ) {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => deliver(&pending, &line),
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(%error, "Failed reading from MLX service");
                    break;
                }
            }
        }
        exited.send_replace(true);
        if let Ok(mut pending) = pending.lock() {
            for (id, sender) in pending.drain() {
                let _ = sender.send(MlxBridgeEvent::error(
                    id,
                    "MLX service stopped unexpectedly",
                ));
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    async fn log_diagnostics(stderr: ChildStderr) {
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
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    impl MacMlxTransport {
        async fn new(runtime: &MlxServiceRuntime) -> Result<Self> {
            use std::process::Stdio;

            let mut command = runtime.command();
            command.kill_on_drop(true);
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "Failed to start the MLX service at {}: {error}",
                        runtime.entrypoint.display()
                    )
                })?;
            let (Some(stdin), Some(stdout), Some(stderr)) =
                (child.stdin.take(), child.stdout.take(), child.stderr.take())
            else {
                return Err(flow_like_types::anyhow!(
                    "The pipes of the MLX service at {} are unavailable",
                    runtime.entrypoint.display()
                ));
            };

            let pending = PendingRequests::default();
            let (exited_sender, exited) = watch::channel(false);
            drop(flow_like_types::tokio::spawn(route_events(
                stdout,
                pending.clone(),
                exited_sender,
            )));
            drop(flow_like_types::tokio::spawn(log_diagnostics(stderr)));

            Ok(Self {
                child: Mutex::new(child),
                stdin: Arc::new(Mutex::new(stdin)),
                pending,
                exited,
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

        async fn exited(&self) {
            let mut exited = self.exited.clone();
            let _ = exited.wait_for(|done| *done).await;
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

    async fn native_transport(
        model_directory: &str,
        runtimes: &RuntimeLocator,
    ) -> Result<Arc<dyn MlxTransport>> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let _ = model_directory;
            Ok(Arc::new(
                MacMlxTransport::new(runtimes.installed_mlx_service()?).await?,
            ))
        }
        #[cfg(all(
            target_os = "ios",
            target_arch = "aarch64",
            not(any(target_abi = "sim", target_abi = "macabi"))
        ))]
        {
            let _ = runtimes;
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
            let _ = (model_directory, runtimes);
            Err(flow_like_types::anyhow!(
                "MLX is only available on Apple-silicon macOS and iOS devices"
            ))
        }
    }

    /// An MLX model in the MLX runtime: the helper process on macOS, in process on iOS.
    /// `serve` answers `/health` and `/v1/chat/completions` on a listener, both behind a bearer
    /// token. Dropping it stops serving and unloads the model.
    pub struct MlxEndpoint {
        served: Arc<Served>,
        server: Option<JoinHandle<()>>,
    }

    impl MlxEndpoint {
        /// Starts the MLX runtime the locator names, for one model; nothing is served yet.
        pub async fn start(runtimes: &RuntimeLocator, model: MlxServedModel) -> Result<Self> {
            let transport = native_transport(&model.directory.to_string_lossy(), runtimes).await?;
            Ok(Self::with_transport(transport, model))
        }

        fn with_transport(transport: Arc<dyn MlxTransport>, model: MlxServedModel) -> Self {
            Self {
                served: Arc::new(Served {
                    directory: model.directory.to_string_lossy().into_owned(),
                    kind: model.kind.into(),
                    limits: model.limits,
                    transport,
                }),
                server: None,
            }
        }

        /// Loads the weights by generating one token, so the first request does not wait for them.
        pub async fn load(&self) -> Result<()> {
            let warm_up = json!({"messages": [{"role": "user", "content": "Hi"}], "max_tokens": 1});
            let (mut receiver, mut cancellation) = self.served.generate(warm_up).await?;
            completion(&mut receiver, &mut cancellation)
                .await
                .map(drop)
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "Load the MLX model in {}: {error}",
                        self.served.directory
                    )
                })
        }

        /// Serves the model on `listener` behind `bearer_token` and answers the port.
        pub fn serve(&mut self, listener: TcpListener, bearer_token: &str) -> Result<u16> {
            if self.server.is_some() {
                return Err(flow_like_types::anyhow!(
                    "The MLX model in {} is served already",
                    self.served.directory
                ));
            }
            let port = listener.local_addr()?.port();
            let app = router(MlxProxyState {
                bearer_token: bearer_token.into(),
                served: self.served.clone(),
            });
            self.server = Some(flow_like_types::tokio::spawn(async move {
                if let Err(error) = axum::serve(listener, app).await {
                    tracing::error!(%error, "MLX compatibility proxy stopped");
                }
            }));
            Ok(port)
        }

        /// Resolves once the MLX helper exited; an in-process runtime never does.
        pub async fn exited(&self) {
            self.served.transport.exited().await;
        }
    }

    impl Drop for MlxEndpoint {
        fn drop(&mut self) {
            if let Some(server) = self.server.take() {
                server.abort();
            }
            self.served.transport.unload();
        }
    }

    pub struct MlxModel {
        bit: Bit,
        model: MlxProviderModel,
        _endpoint: MlxEndpoint,
        /// Every client holds the model, so the factory sees it in use and keeps its one runtime.
        this: Weak<MlxModel>,
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
            let this = self.this.upgrade().ok_or_else(|| {
                flow_like_types::anyhow!("The MLX runtime of {} stopped", self.bit.id)
            })?;
            Ok(self.model.provider_with_keepalive(this))
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

    /// The provider of an MLX Bit, with a KV cache that fits the execution settings unless the
    /// Bit sets one.
    fn served_provider(bit: &Bit, execution_settings: &ExecutionSettings) -> Result<ModelProvider> {
        let mut provider = bit.try_to_served_provider().ok_or_else(|| {
            flow_like_types::anyhow!("Failed to read the MLX provider configuration")
        })?;
        provider
            .params
            .get_or_insert_default()
            .entry("max_kv_size".to_string())
            .or_insert_with(|| json!(default_max_kv_size(bit, execution_settings)));
        Ok(provider)
    }

    /// The downloaded files of an MLX Bit, laid out as one model directory.
    async fn materialize(
        bit: &Bit,
        app_state: &Arc<FlowLikeState>,
    ) -> Result<MaterializedMlxModel> {
        let FlowLikeStore::Local(bit_store) = FlowLikeState::bit_store(app_state).await? else {
            return Err(flow_like_types::anyhow!("MLX requires a local model store"));
        };
        let pack = bit.pack(app_state.clone()).await?;
        ensure_local_weights(&pack, app_state, bit.id.as_str(), "MLX model").await?;
        let root = bit.clone();
        flow_like_types::tokio::task::spawn_blocking(move || {
            materialize_mlx_model(&root, &pack, &bit_store)
        })
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("MLX model materialization task failed: {error}")
        })?
    }

    fn ensure_servable(bit: &Bit) -> Result<()> {
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
        Ok(())
    }

    /// Starts the MLX runtime and loads the model while no other local engine loads.
    async fn loaded_endpoint(
        runtimes: &RuntimeLocator,
        model: MlxServedModel,
    ) -> Result<MlxEndpoint> {
        let _one_at_a_time = LOCAL_ENGINE_LOADS.lock().await;
        let endpoint = MlxEndpoint::start(runtimes, model).await?;
        endpoint.load().await?;
        Ok(endpoint)
    }

    impl MlxModel {
        pub async fn new(
            bit: &Bit,
            app_state: Arc<FlowLikeState>,
            execution_settings: &ExecutionSettings,
        ) -> Result<Arc<Self>> {
            ensure_servable(bit)?;
            let provider = served_provider(bit, execution_settings)?;
            let materialized = materialize(bit, &app_state).await?;
            let model = MlxServedModel {
                directory: materialized.path,
                kind: materialized.kind,
                limits: MlxRequestLimits::default(),
            };
            let mut endpoint = loaded_endpoint(&app_state.runtime_locator, model).await?;
            let bearer_token = flow_like_types::create_id();
            let port = endpoint.serve(TcpListener::bind(("127.0.0.1", 0)).await?, &bearer_token)?;
            let model = MlxProviderModel::new(&provider, port, &bearer_token).await?;
            Ok(Self::serving(bit.clone(), model, endpoint, port))
        }

        fn serving(
            bit: Bit,
            model: MlxProviderModel,
            endpoint: MlxEndpoint,
            port: u16,
        ) -> Arc<Self> {
            Arc::new_cyclic(|this| Self {
                bit,
                model,
                _endpoint: endpoint,
                this: this.clone(),
                port,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::bit::BitTypes;
        use flow_like_types::reqwest;
        use std::{sync::Mutex as StdMutex, time::Duration};

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
            let provider = served_provider(&bit, &settings).unwrap();
            assert_eq!(
                provider.params.unwrap()["max_kv_size"],
                json!(default_max_kv_size(&bit, &settings))
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
        fn bridge_requests_name_the_loader_family_in_lowercase() {
            assert_eq!(json!(BridgeKind::from(MlxModelKind::Llm)), json!("llm"));
            assert_eq!(json!(BridgeKind::from(MlxModelKind::Vlm)), json!("vlm"));
        }

        #[test]
        fn request_limits_cap_the_kv_cache_and_leave_smaller_asks() {
            let limits = MlxRequestLimits {
                max_kv_size: Some(4_096),
                kv_bits: Some(8),
                inline_images_only: false,
            };
            let mut unbounded = json!({"messages": [], "max_kv_size": 1_000_000});
            limits.apply(&mut unbounded);
            assert_eq!(unbounded["max_kv_size"], 4_096);
            assert_eq!(unbounded["kv_bits"], 8);

            let mut smaller = json!({"max_kv_size": 512, "kv_bits": 4, "max_kv": "x"});
            limits.apply(&mut smaller);
            assert_eq!(smaller["max_kv_size"], 512);
            assert_eq!(smaller["kv_bits"], 4);

            let mut odd = json!({"max_kv_size": "all"});
            limits.apply(&mut odd);
            assert_eq!(odd["max_kv_size"], 4_096);

            let mut untouched = json!({"max_kv_size": 1_000_000});
            MlxRequestLimits::default().apply(&mut untouched);
            assert_eq!(untouched, json!({"max_kv_size": 1_000_000}));
        }

        /// Answers every request with one chunk and a completion, or with an error when the
        /// prompt asks for one, and records what it was sent.
        struct FakeTransport {
            requests: StdMutex<Vec<Value>>,
            unloaded: StdMutex<bool>,
            exit: tokio::sync::watch::Sender<bool>,
        }

        impl FakeTransport {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    requests: StdMutex::default(),
                    unloaded: StdMutex::default(),
                    exit: tokio::sync::watch::Sender::new(false),
                })
            }

            fn sent(&self) -> Vec<Value> {
                self.requests.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl MlxTransport for FakeTransport {
            async fn generate(
                &self,
                request: MlxBridgeRequest,
            ) -> Result<mpsc::UnboundedReceiver<MlxBridgeEvent>> {
                let sent = json::to_value(&request)?;
                let failing = sent.to_string().contains("fail");
                self.requests.lock().unwrap().push(sent);
                let (sender, receiver) = mpsc::unbounded_channel();
                let event = |name: &str, data: Value| MlxBridgeEvent {
                    id: request.id.clone(),
                    event: name.into(),
                    data: Some(data),
                    error: None,
                };
                if failing {
                    sender.send(MlxBridgeEvent::error(&request.id, "Unsupported model type"))?;
                    return Ok(receiver);
                }
                let delta = json!({"choices": [{"index": 0, "delta": {"content": "Hi"}}]});
                sender.send(event("chunk", delta))?;
                let answer = json!({"id": "c1", "choices": [{"index": 0,
                    "message": {"role": "assistant", "content": "Hi"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}});
                sender.send(event("complete", answer))?;
                Ok(receiver)
            }

            fn unload(&self) {
                *self.unloaded.lock().unwrap() = true;
            }

            async fn exited(&self) {
                let _ = self.exit.subscribe().wait_for(|done| *done).await;
            }
        }

        fn served_model(directory: &str, limits: MlxRequestLimits) -> MlxServedModel {
            MlxServedModel {
                directory: PathBuf::from(directory),
                kind: MlxModelKind::Vlm,
                limits,
            }
        }

        const KV_LIMIT: MlxRequestLimits = MlxRequestLimits {
            max_kv_size: Some(4_096),
            kv_bits: None,
            inline_images_only: false,
        };

        /// An endpoint over `transport` served behind the token `secret`, and its base URL.
        async fn serving(transport: Arc<FakeTransport>) -> (MlxEndpoint, String) {
            let model = served_model("/models/qwen-vl", KV_LIMIT);
            let mut endpoint = MlxEndpoint::with_transport(transport, model);
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = endpoint.serve(listener, "secret").unwrap();
            (endpoint, format!("http://127.0.0.1:{port}"))
        }

        #[tokio::test]
        async fn an_endpoint_loads_its_model_with_one_token_within_its_limits() {
            let transport = FakeTransport::new();
            let model = served_model("/models/qwen-vl", KV_LIMIT);
            let mut endpoint = MlxEndpoint::with_transport(transport.clone(), model);
            endpoint.load().await.unwrap();
            let warm_up = &transport.sent()[0];
            assert_eq!(warm_up["command"], "generate");
            assert_eq!(warm_up["model_kind"], "vlm");
            assert_eq!(warm_up["model_directory"], "/models/qwen-vl");
            assert_eq!(warm_up["request"]["max_tokens"], 1);
            assert_eq!(warm_up["request"]["max_kv_size"], 4_096);
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            endpoint.serve(listener, "secret").unwrap();
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            assert!(endpoint.serve(listener, "secret").is_err());

            transport.exit.send_replace(true);
            tokio::time::timeout(Duration::from_secs(5), endpoint.exited())
                .await
                .unwrap();
            drop(endpoint);
            assert!(*transport.unloaded.lock().unwrap());
        }

        #[tokio::test]
        async fn a_served_endpoint_answers_its_token_whole_and_streamed() {
            let transport = FakeTransport::new();
            let (_endpoint, base) = serving(transport.clone()).await;
            let http = reqwest::Client::new();
            let health = |token: &str| http.get(format!("{base}/health")).bearer_auth(token).send();
            assert_eq!(health("wrong").await.unwrap().status(), 401);
            assert_eq!(health("secret").await.unwrap().status(), 200);
            let chat = |body: Value| {
                http.post(format!("{base}/v1/chat/completions"))
                    .bearer_auth("secret")
                    .json(&body)
                    .send()
            };

            let ask = json!({"messages": [{"role": "user", "content": "hi"}],
                             "max_kv_size": 1_000_000});
            let answer: Value = chat(ask).await.unwrap().json().await.unwrap();
            assert_eq!(answer["choices"][0]["message"]["content"], "Hi");
            assert_eq!(transport.sent()[0]["request"]["max_kv_size"], 4_096);

            let streamed = json!({"messages": [], "stream": true,
                                  "stream_options": {"include_usage": true}});
            let text = chat(streamed).await.unwrap().text().await.unwrap();
            assert!(text.contains("\"content\":\"Hi\""), "{text}");
            assert!(text.contains("\"total_tokens\":4"), "{text}");
            assert!(text.contains("[DONE]"), "{text}");

            let failed = chat(json!({"messages": [{"role": "user", "content": "fail"}]}));
            let failed = failed.await.unwrap();
            assert_eq!(failed.status(), 500);
            let failed: Value = failed.json().await.unwrap();
            assert_eq!(failed["error"]["message"], "Unsupported model type");
        }

        #[tokio::test]
        async fn an_endpoint_served_to_other_people_takes_inline_images_only() {
            let shown = |url: &str| {
                let content = json!([{"type": "text", "text": "What is this?"},
                                     {"type": "image_url", "image_url": {"url": url}}]);
                json!({"messages": [{"role": "user", "content": content}]})
            };
            let ask = |base: &str, body: Value| {
                reqwest::Client::new()
                    .post(format!("{base}/v1/chat/completions"))
                    .bearer_auth("secret")
                    .json(&body)
                    .send()
            };
            let transport = FakeTransport::new();
            let limits = MlxRequestLimits {
                inline_images_only: true,
                ..KV_LIMIT
            };
            let mut endpoint = MlxEndpoint::with_transport(
                transport.clone(),
                served_model("/models/qwen-vl", limits),
            );
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let base = format!(
                "http://127.0.0.1:{}",
                endpoint.serve(listener, "secret").unwrap()
            );
            let linked = [
                "file:///Users/owner/scan.png",
                "/Users/owner/scan.png",
                "https://example.com/cat.png",
            ];
            for url in linked {
                let refused = ask(&base, shown(url)).await.unwrap();
                assert_eq!(refused.status(), 400, "{url}");
            }
            assert!(transport.sent().is_empty());
            let inline = ask(&base, shown("data:image/png;base64,iVBORw0KGgo="));
            assert_eq!(inline.await.unwrap().status(), 200);
            assert_eq!(transport.sent().len(), 1);

            let desktop = FakeTransport::new();
            let (_endpoint, base) = serving(desktop.clone()).await;
            let local = ask(&base, shown("/Users/owner/scan.png")).await.unwrap();
            assert_eq!(local.status(), 200);
        }

        #[tokio::test]
        async fn the_factory_keeps_a_model_whose_client_is_still_in_use() {
            use crate::models::factory_cache::{FactoryCache, MODEL_IDLE_TTL};

            let transport = FakeTransport::new();
            let (endpoint, _base) = serving(transport.clone()).await;
            let provider = ModelProvider {
                api_surface: None,
                provider_name: MLX_PROVIDER_NAME.to_string(),
                model_id: Some("qwen-vl".to_string()),
                version: None,
                params: None,
            };
            let model = MlxProviderModel::new(&provider, 1, "secret").await.unwrap();
            let cache = FactoryCache::<dyn ModelLogic>::default();
            let cached = cache
                .get_or_build("qwen-vl", || async move {
                    Ok(MlxModel::serving(Bit::default(), model, endpoint, 1)
                        as Arc<dyn ModelLogic>)
                })
                .await
                .unwrap();
            let client = cached.provider().await.unwrap().into_client();
            drop(cached);

            let idle = std::time::Instant::now() + MODEL_IDLE_TTL * 3;
            cache.gc(idle, MODEL_IDLE_TTL);
            assert!(cache.contains("qwen-vl"), "a client in use keeps the model");
            assert!(!*transport.unloaded.lock().unwrap());

            drop(client);
            cache.gc(
                idle + MODEL_IDLE_TTL + Duration::from_secs(1),
                MODEL_IDLE_TTL,
            );
            assert!(!cache.contains("qwen-vl"));
            assert!(*transport.unloaded.lock().unwrap());
        }

        #[tokio::test]
        async fn a_model_that_cannot_generate_fails_its_load() {
            let model = served_model("/models/fail", MlxRequestLimits::default());
            let endpoint = MlxEndpoint::with_transport(FakeTransport::new(), model);
            let error = endpoint.load().await.unwrap_err().to_string();
            assert_eq!(
                error,
                "Load the MLX model in /models/fail: Unsupported model type"
            );
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::{MlxEndpoint, MlxModel, MlxRequestLimits, MlxServedModel};

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
        ) -> Result<Arc<Self>> {
            Err(flow_like_types::anyhow!(
                "MLX is only available on Apple-silicon macOS and physical iOS devices"
            ))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub use unsupported::MlxModel;
