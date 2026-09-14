use crate::{error::ExecutorError, jwt::ExecutorClaims};
use flow_like_types::{tokio, tokio_util::sync::CancellationToken};
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ComputeContext {
    pub function_name: String,
    pub request_id: String,
    pub memory_mb: i32,
    pub architecture: String,
    pub region: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    started: Instant,
}

impl ComputeContext {
    pub fn new(function_name: String, request_id: String, memory_mb: i32) -> Self {
        Self {
            function_name,
            request_id,
            memory_mb,
            architecture: match std::env::consts::ARCH {
                "aarch64" => "arm64",
                other => other,
            }
            .into(),
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "unknown".into()),
            started_at: chrono::Utc::now(),
            started: Instant::now(),
        }
    }
}

tokio::task_local! { pub static CURRENT_COMPUTE: ComputeContext; }

pub async fn record_compute(claims: &ExecutorClaims, token: &str, status: &str) {
    if claims.runtime_limit_ms.is_none() {
        return;
    }
    let Ok(context) = CURRENT_COMPUTE.try_with(Clone::clone) else {
        return;
    };
    let duration = (status != "started").then(|| {
        context
            .started
            .elapsed()
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i64::MAX as u128) as i64
    });
    let body = json!({"phase":"attempt","attempt_id":context.request_id,"compute":{
        "functionName":context.function_name,"requestId":context.request_id,"memoryMb":context.memory_mb,
        "architecture":context.architecture,"region":context.region,"startedAt":context.started_at,
        "measuredDurationMs":duration,"status":status,"revision":if duration.is_some(){"executor-finished"}else{"started"},
        "operationId":null,"payerId":null,"role":"executor","costClass":"workflow_compute","billedDurationMs":null,
        "costMicroUsd":null,"evidence":"measured_estimate","rateVersion":"aws-lambda-reference-2026-09"
    }});
    if let Err(error) = callback(&claims.callback_url, token, body).await {
        tracing::warn!(%error,run_id=%claims.run_id,"Compute attempt needs AWS report reconciliation");
    }
}

pub struct RuntimeLease {
    token: String,
    callback_url: String,
    receipt_url: Option<String>,
    attempt_id: String,
    started: Instant,
    cancellation_watch: Option<tokio::task::JoinHandle<()>>,
    pub limit: Duration,
}

async fn callback(url: &str, token: &str, body: serde_json::Value) -> Result<bool, ExecutorError> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("flow-like-executor/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ExecutorError::Callback(e.to_string()))?;
    let url = format!("{}/api/v1/execution/quota", url.trim_end_matches('/'));
    let mut last = String::new();
    for attempt in 0..3 {
        match client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>().await {
                    Ok(value) => return Ok(value["accepted"].as_bool().unwrap_or(false)),
                    Err(error) => last = error.to_string(),
                }
            }
            Ok(response) if response.status().is_client_error() => {
                return Err(ExecutorError::Callback(format!(
                    "Quota callback rejected: {}",
                    response.status()
                )));
            }
            Ok(response) => last = format!("Quota callback returned {}", response.status()),
            Err(error) => last = error.to_string(),
        }
        if attempt < 2 {
            flow_like_types::tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
        }
    }
    Err(ExecutorError::Callback(last))
}

pub async fn begin(
    claims: &ExecutorClaims,
    token: &str,
) -> Result<Option<RuntimeLease>, ExecutorError> {
    let Some(limit_ms) = claims.runtime_limit_ms else {
        return Ok(None);
    };
    if limit_ms == 0 {
        return Err(ExecutorError::InvalidRequest(
            "Cloud runtime allowance is empty".into(),
        ));
    }
    let attempt_id = flow_like_types::create_id();
    if !callback(
        &claims.callback_url,
        token,
        json!({"phase":"start","attempt_id":attempt_id}),
    )
    .await?
    {
        return Err(ExecutorError::InvalidRequest(
            "This cloud run is already claimed or completed".into(),
        ));
    }
    Ok(Some(RuntimeLease {
        token: token.into(),
        callback_url: claims.callback_url.clone(),
        receipt_url: claims.quota_receipt_url.clone(),
        attempt_id,
        started: Instant::now(),
        cancellation_watch: None,
        limit: Duration::from_millis(limit_ms),
    }))
}

impl RuntimeLease {
    /// Poll a fixed operation row. Connection loss cannot clear the reservation.
    pub fn watch_cancellation(&mut self, cancellation: CancellationToken) {
        let url = self.callback_url.clone();
        let token = self.token.clone();
        let attempt = self.attempt_id.clone();
        self.cancellation_watch = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                match callback(&url, &token, json!({"phase":"poll","attempt_id":attempt})).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        cancellation.cancel();
                        break;
                    }
                }
            }
        }));
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.started
            .elapsed()
            .as_nanos()
            .div_ceil(1_000_000)
            .min(u64::MAX as u128) as u64
    }

    pub async fn finish(&self, duration_ms: u64, status: &str) -> Result<(), ExecutorError> {
        let receipt = json!({"phase":"finish","attempt_id":self.attempt_id,"runtime_ms":duration_ms,"status":status});
        let mut stored = false;
        if let Some(url) = &self.receipt_url {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| ExecutorError::Callback(e.to_string()))?;
            for _ in 0..3 {
                if client
                    .put(url)
                    .header(reqwest::header::IF_NONE_MATCH, "*")
                    .json(&receipt)
                    .send()
                    .await
                    .is_ok_and(|r| {
                        r.status().is_success()
                            || r.status() == reqwest::StatusCode::PRECONDITION_FAILED
                    })
                {
                    stored = true;
                    break;
                }
            }
        }
        match callback(&self.callback_url, &self.token, receipt).await {
            Ok(_) => Ok(()),
            Err(error) if stored => {
                tracing::warn!(%error,"Runtime receipt saved; scheduled reconciliation will finish accounting");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        if let Some(watch) = self.cancellation_watch.take() {
            watch.abort();
        }
    }
}

pub async fn reject(claims: &ExecutorClaims, token: &str) {
    if claims.runtime_limit_ms.is_some() {
        if let Err(error) = callback(
            &claims.callback_url,
            token,
            json!({"phase":"reject","attempt_id":flow_like_types::create_id()}),
        )
        .await
        {
            tracing::warn!(run_id=%claims.run_id,%error,"Failed to release unstarted runtime reservation");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::{post, put},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn durable_receipt_survives_unavailable_settlement_api() {
        let receipts = Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let failures = Arc::new(AtomicUsize::new(0));
        let received = receipts.clone();
        let observed = receipts.clone();
        let counted = failures.clone();
        let router = Router::new()
            .route(
                "/receipt",
                put(
                    move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                        let received = received.clone();
                        async move {
                            assert_eq!(headers.get(reqwest::header::IF_NONE_MATCH).unwrap(), "*");
                            received.lock().await.push(body);
                            StatusCode::OK
                        }
                    },
                ),
            )
            .route(
                "/api/v1/execution/quota",
                post(move |Json(body): Json<serde_json::Value>| {
                    let observed = observed.clone();
                    let counted = counted.clone();
                    async move {
                        if body["phase"] == "finish" {
                            assert_eq!(
                                observed.lock().await.len(),
                                1,
                                "receipt precedes settlement"
                            );
                            counted.fetch_add(1, Ordering::SeqCst);
                            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
                        } else {
                            (StatusCode::OK, Json(json!({"accepted":true})))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let claims:ExecutorClaims=serde_json::from_value(json!({"sub":"payer","run_id":"run","app_id":"app","board_id":"board","callback_url":base,"runtime_limit_ms":30000,"quota_receipt_url":format!("{base}/receipt"),"typ":"executor","iss":"test","aud":"test","iat":0,"nbf":0,"exp":9999999999_i64,"jti":"id"})).unwrap();
        let lease = begin(&claims, "test-token").await.unwrap().unwrap();
        lease
            .finish(37, "completed")
            .await
            .expect("durable receipt permits later reconciliation");
        assert_eq!(failures.load(Ordering::SeqCst), 3);
        let stored = receipts.lock().await;
        assert_eq!(stored[0]["runtime_ms"], 37);
        assert_eq!(stored[0]["attempt_id"], lease.attempt_id);
        server.abort();
    }
}
