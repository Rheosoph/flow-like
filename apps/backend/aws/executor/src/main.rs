//! AWS Lambda Executor with Streaming Support
//!
//! This Lambda function executes flows and streams results back
//! using Lambda's streaming response capability.
//!
//! ## Endpoints
//!
//! - `POST /execute` - Execute with callback (async)
//! - `POST /execute/stream` - Execute with NDJSON streaming
//! - `POST /execute/sse` - Execute with Server-Sent Events
//! - `GET /health` - Health check

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_catalog::initialize as initialize_catalog;
use flow_like_executor::{ExecutorConfig, ExecutorState, executor_router};
use flow_like_types::dispatch::DIRECT_LAMBDA_INVOKE_API_ID;
use flow_like_types::tokio;
use lambda_http::{
    Error, Request, RequestExt, request::RequestContext, run_with_streaming_response, service_fn,
    tower::ServiceExt, tracing,
};
use tracing_subscriber::{EnvFilter, prelude::*};

fn is_direct_async_execute(request: &Request) -> bool {
    request.uri().path() == "/execute"
        && matches!(
            request.request_context_ref(),
            Some(RequestContext::ApiGatewayV2(context))
                if context.apiid.as_deref() == Some(DIRECT_LAMBDA_INVOKE_API_ID)
        )
}

async fn dispatch_request(
    app: axum::Router,
    request: Request,
) -> Result<axum::response::Response, Error> {
    let propagate_failure = is_direct_async_execute(&request);
    let response = app
        .oneshot(request)
        .await
        .expect("Axum routers are infallible");

    if propagate_failure && !response.status().is_success() {
        let status = response.status();
        tracing::error!(%status, "Direct asynchronous Lambda execution failed");
        return Err(std::io::Error::other(format!(
            "direct asynchronous execution returned HTTP {status}"
        ))
        .into());
    }

    Ok(response)
}

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // Initialize Sentry if configured
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn")
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("hyper_util=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
            .add_directive("h2=warn".parse().unwrap())
            .add_directive("tower=warn".parse().unwrap())
    });

    let _sentry_guard = if sentry_endpoint.is_empty() {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
            .init();
        None
    } else {
        let guard = sentry::init((
            sentry_endpoint,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: 0.3,
                ..Default::default()
            },
        ));
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
            .with(sentry_tracing::layer())
            .init();
        Some(guard)
    };

    tracing::info!("Starting Flow-Like AWS Executor Lambda");

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    // Create executor state from environment
    let state = ExecutorState::new(ExecutorConfig::from_env().with_required_terminal_status_ack());

    // Build router with all execution endpoints
    let app = executor_router(state);
    let handler = service_fn(move |request| dispatch_request(app.clone(), request));

    // Run with streaming response support
    run_with_streaming_response(handler).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use lambda_http::{Body, aws_lambda_events::apigw::ApiGatewayV2httpRequestContext};

    fn request_with_api_id(path: &str, api_id: &str) -> Request {
        let mut context = ApiGatewayV2httpRequestContext::default();
        context.apiid = Some(api_id.to_string());

        let mut request = Request::new(Body::Empty);
        *request.uri_mut() = path.parse().unwrap();
        request.with_request_context(RequestContext::ApiGatewayV2(context))
    }

    fn direct_request(path: &str) -> Request {
        request_with_api_id(path, DIRECT_LAMBDA_INVOKE_API_ID)
    }

    fn response_app(status: StatusCode) -> axum::Router {
        axum::Router::new().fallback(move || async move { status })
    }

    #[tokio::test]
    async fn direct_async_execute_failure_becomes_invocation_error() {
        let result = dispatch_request(
            response_app(StatusCode::INTERNAL_SERVER_ERROR),
            direct_request("/execute"),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn successful_direct_async_execute_remains_a_response() {
        let result =
            dispatch_request(response_app(StatusCode::OK), direct_request("/execute")).await;

        assert_eq!(result.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn streaming_failures_remain_http_responses() {
        for path in ["/execute/sse", "/execute/stream"] {
            let result = dispatch_request(
                response_app(StatusCode::INTERNAL_SERVER_ERROR),
                direct_request(path),
            )
            .await;

            assert_eq!(result.unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    #[tokio::test]
    async fn ordinary_http_execute_failure_remains_an_http_response() {
        let request = request_with_api_id("/execute", "real-api-gateway-id");
        let result =
            dispatch_request(response_app(StatusCode::INTERNAL_SERVER_ERROR), request).await;

        assert_eq!(result.unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
