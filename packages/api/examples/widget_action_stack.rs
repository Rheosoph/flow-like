//! Run the production API or executor router against the isolated widget benchmark fixture.

use flow_like_api::{
    construct_router,
    state::State,
    storage_config::{BucketConfig, StorageConfig, StorageProvider},
    telemetry::{SpanExportConfig, telemetry_span_layer},
};
use flow_like_executor::{
    ExecutorState, executor_router,
    quota::{CURRENT_COMPUTE, ComputeContext},
};
use flow_like_secrets::{EnvProviderConfig, ProviderConfig, SecretStoreConfig};
use flow_like_types::{Result, anyhow, create_id};
use std::{net::Ipv4Addr, sync::Arc};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<()> {
    let service = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: widget_action_stack api|runtime"))?;
    if service != "api" && service != "runtime" {
        return Err(anyhow!("service must be api or runtime"));
    }
    if std::env::var_os("FLOW_LIKE_CONFIG_FILE").is_none() {
        return Err(anyhow!(
            "FLOW_LIKE_CONFIG_FILE must point to the local fixture"
        ));
    }
    let workers = std::env::var("BENCH_WORKER_THREADS")
        .unwrap_or_else(|_| "1".to_owned())
        .parse::<usize>()?;
    if workers == 0 {
        return Err(anyhow!("BENCH_WORKER_THREADS must be positive"));
    }
    let model = std::env::var("BENCH_RUNTIME_MODEL").unwrap_or_else(|_| "multi-thread".to_owned());
    let mut builder = match model.as_str() {
        "multi-thread" => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.worker_threads(workers);
            builder
        }
        "current-thread" if workers == 1 => tokio::runtime::Builder::new_current_thread(),
        _ => {
            return Err(anyhow!(
                "BENCH_RUNTIME_MODEL must be multi-thread or current-thread (one worker)"
            ));
        }
    };
    builder.enable_all().build()?.block_on(serve(&service))
}

async fn serve(service: &str) -> Result<()> {
    let (router, port) = if service == "api" {
        let (span_layer, span_exporter) = telemetry_span_layer(SpanExportConfig::from_env());
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(flow_like_api::info_env_filter()))
            .with(span_layer)
            .init();
        let provider = StorageProvider::Aws;
        let storage = StorageConfig::from_env_with_provider(provider.clone())?;
        let buckets = BucketConfig::from_env(&provider)?;
        let cdn = Arc::new(storage.build_store(&buckets.cdn)?);
        let secrets = SecretStoreConfig::default()
            .with_provider(ProviderConfig::Env(EnvProviderConfig { prefix: None }));
        let state = Arc::new(
            State::new(
                Arc::new(flow_like_catalog::get_catalog()),
                cdn,
                Some(secrets),
            )
            .await,
        );
        let _span_exporter = span_exporter.spawn_for_state(&state);
        (
            construct_router(state),
            std::env::var("API_PORT").unwrap_or_else(|_| "8440".to_owned()),
        )
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(flow_like_api::info_env_filter())
            .init();
        (
            executor_router(ExecutorState::from_env())
                .layer(axum::middleware::from_fn(compute_context)),
            std::env::var("RUNTIME_PORT").unwrap_or_else(|_| "8441".to_owned()),
        )
    };
    let router = router.route("/__benchmark/runtime", axum::routing::get(runtime_info));
    let listener =
        tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port.parse::<u16>()?)).await?;
    tracing::info!(service, address = %listener.local_addr()?, "Widget benchmark service ready");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn runtime_info() -> axum::Json<flow_like_types::Value> {
    let model = match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => "current-thread",
        tokio::runtime::RuntimeFlavor::MultiThread => "multi-thread",
        _ => "unknown",
    };
    axum::Json(flow_like_types::json::json!({"runtimeModel": model}))
}

async fn compute_context(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Lambda supplies this task-local context. The local HTTP adapter supplies it so
    // initial and final compute evidence follow the same API accounting path.
    if !request.uri().path().starts_with("/execute") {
        return next.run(request).await;
    }
    let context = ComputeContext::new("widget-bench-local".into(), create_id(), 2048);
    CURRENT_COMPUTE.scope(context, next.run(request)).await
}
