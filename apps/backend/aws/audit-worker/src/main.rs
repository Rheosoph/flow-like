#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "../../shared/api_bootstrap.rs"]
mod bootstrap;

use std::sync::Arc;

use flow_like_api::audit::worker::{self as audit_worker, AuditWorkerContext, TickReport};
use flow_like_aws_data::dsql::{DsqlConfig, DsqlDatabase};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use serde_json::Value;

const APPLICATION_NAME: &str = "flow-like-aws-audit-worker";

#[tokio::main]
async fn main() -> Result<(), Error> {
    lambda_runtime::tracing::init_default_subscriber();
    let api = bootstrap::initialize(APPLICATION_NAME, DsqlConfig::from_env()?).await?;
    // Built once per container: the signing budget and the key-service client live here.
    let context = Arc::new(AuditWorkerContext::from_state(&api.state)?);
    tracing::info!(
        signing = context.signing_configured(),
        archiving = context.bucket.is_some(),
        "audit worker ready"
    );
    let database = api.dsql;
    run(service_fn(move |_event: LambdaEvent<Value>| {
        let context = context.clone();
        let database = database.clone();
        async move { tick(&context, database.as_deref()).await }
    }))
    .await
}

/// One worker pass per scheduled invocation. An overlapping invocation finds the lease
/// taken and reports `skipped`.
async fn tick(
    context: &AuditWorkerContext,
    database: Option<&DsqlDatabase>,
) -> Result<TickReport, Error> {
    // A frozen Lambda's timers do not tick, so the DSQL token is checked per invocation.
    if let Some(database) = database
        && let Err(error) = database.refresh_token_if_stale().await
    {
        tracing::warn!(%error, "Aurora DSQL token refresh failed before the audit tick");
    }
    Ok(audit_worker::tick(context, chrono::Utc::now()).await?)
}
