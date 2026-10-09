use crate::client::{checked, decode};
use crate::{
    AsyncInvokeResult, CancelRunResult, Client, Error, EventStream, InvokeBoardRequest,
    InvokeEventRequest, Method, PollOptions, PollResponse, RequestOptions, Result, RunStatus,
    Value, WaitOptions,
};

impl Client {
    pub async fn trigger_workflow(
        &self,
        app_id: &str,
        board_id: &str,
        request: &InvokeBoardRequest,
        options: &RequestOptions,
    ) -> Result<EventStream> {
        self.invoke_board(app_id, board_id, request, options).await
    }

    pub async fn trigger_workflow_async(
        &self,
        app_id: &str,
        board_id: &str,
        request: &InvokeBoardRequest,
        options: &RequestOptions,
    ) -> Result<AsyncInvokeResult> {
        self.invoke_board_async(app_id, board_id, request, options)
            .await
    }

    pub async fn invoke_board(
        &self,
        app_id: &str,
        board_id: &str,
        request: &InvokeBoardRequest,
        options: &RequestOptions,
    ) -> Result<EventStream> {
        self.stream_sse(
            Method::POST,
            &["apps", app_id, "board", board_id, "invoke"],
            &options.clone().json(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn invoke_board_async(
        &self,
        app_id: &str,
        board_id: &str,
        request: &InvokeBoardRequest,
        options: &RequestOptions,
    ) -> Result<AsyncInvokeResult> {
        self.request(
            Method::POST,
            &["apps", app_id, "board", board_id, "invoke", "async"],
            &options.clone().json(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn trigger_event(
        &self,
        app_id: &str,
        event_id: &str,
        request: &InvokeEventRequest,
        options: &RequestOptions,
    ) -> Result<EventStream> {
        self.stream_sse(
            Method::POST,
            &["apps", app_id, "events", event_id, "invoke"],
            &options.clone().json(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn trigger_event_async(
        &self,
        app_id: &str,
        event_id: &str,
        request: &InvokeEventRequest,
        options: &RequestOptions,
    ) -> Result<AsyncInvokeResult> {
        self.request(
            Method::POST,
            &["apps", app_id, "events", event_id, "invoke", "async"],
            &options.clone().json(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn get_run_status(&self, run_id: &str) -> Result<RunStatus> {
        self.request(
            Method::GET,
            &["execution", "run", run_id],
            &RequestOptions::default(),
        )
        .await
    }
    pub async fn cancel_run(&self, run_id: &str) -> Result<CancelRunResult> {
        self.request(
            Method::DELETE,
            &["execution", "run", run_id],
            &RequestOptions::default(),
        )
        .await
    }

    /// The poll token authorizes this request by itself. It never enters a URL.
    pub async fn poll_execution(
        &self,
        poll_token: &str,
        options: &PollOptions,
    ) -> Result<PollResponse> {
        if options.timeout > 30 || options.after_sequence < -1 || options.after_sequence == i32::MAX
        {
            return Err(Error::Configuration(
                "Poll timeout must be 0..=30 and cursor -1..i32::MAX-1".into(),
            ));
        }
        let response = self
            .http
            .get(self.url(&["execution", "poll"])?)
            .bearer_auth(poll_token)
            .query(&[
                ("after_sequence", options.after_sequence.to_string()),
                ("timeout", options.timeout.to_string()),
            ])
            .send()
            .await?;
        decode(checked(response).await?).await
    }

    /// Polls until completion, preserving events from all pages. Failure and cancellation are errors.
    pub async fn wait_for_execution(
        &self,
        poll_token: &str,
        options: &WaitOptions,
    ) -> Result<PollResponse> {
        tokio::time::timeout(options.timeout, async {
            let mut cursor = -1;
            let mut events = Vec::new();
            let mut run_id = None;
            loop {
                let mut response = self
                    .poll_execution(
                        poll_token,
                        &PollOptions {
                            after_sequence: cursor,
                            timeout: options.poll_timeout,
                        },
                    )
                    .await?;
                if run_id.as_ref().is_some_and(|id| *id != response.run_id) {
                    return Err(Error::Stream("Poll response changed run identity".into()));
                }
                run_id = Some(response.run_id.clone());
                let empty = response.events.is_empty();
                if let Some(sequence) = response.last_sequence() {
                    if sequence <= cursor {
                        return Err(Error::Stream("Poll cursor did not advance".into()));
                    }
                    cursor = sequence;
                }
                events.append(&mut response.events);
                // A terminal run can still have multiple pages of persisted events.
                if response.is_terminal() && empty {
                    if !response.status.eq_ignore_ascii_case("completed") {
                        return Err(Error::Execution {
                            run_id: response.run_id,
                            status: response.status,
                            message: response.error.unwrap_or_default(),
                        });
                    }
                    response.events = events;
                    return Ok(response);
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub async fn chat_completions_stream(&self, body: Value) -> Result<EventStream> {
        self.model_stream(&["chat", "completions"], body).await
    }
    pub async fn responses_stream(&self, body: Value) -> Result<EventStream> {
        self.model_stream(&["responses"], body).await
    }
    async fn model_stream(&self, path: &[&str], mut body: Value) -> Result<EventStream> {
        let object = body
            .as_object_mut()
            .ok_or_else(|| Error::Configuration("Model request must be an object".into()))?;
        object.insert("stream".into(), Value::Bool(true));
        self.stream_sse(Method::POST, path, &RequestOptions::new().json(body))
            .await
    }

    pub async fn trigger_http_sink(
        &self,
        app_id: &str,
        path: &str,
        method: Method,
        options: &RequestOptions,
    ) -> Result<Value> {
        let mut segments = vec!["sink", "trigger", "http", app_id];
        segments.extend(path.trim_start_matches('/').split('/'));
        self.request(method, &segments, options).await
    }
}
