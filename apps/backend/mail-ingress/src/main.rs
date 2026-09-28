use flow_like_mail_ingress::{
    ApiClient, IngestApi, Result,
    cache::RecipientCache,
    invalid,
    lmtp::{LmtpConfig, serve},
};
use std::{env, sync::Arc, time::Duration};

const ACCEPTED_RECIPIENT_TTL: Duration = Duration::from_secs(30);
const REJECTED_RECIPIENT_TTL: Duration = Duration::from_secs(10);
const RECIPIENT_CACHE_CAPACITY: usize = 10_000;

async fn sink_token() -> Result<String> {
    Ok(match env::var("SINK_TRIGGER_JWT_FILE") {
        Ok(path) => tokio::fs::read_to_string(path).await?.trim().to_string(),
        Err(_) => env::var("SINK_TRIGGER_JWT")?,
    })
}

fn lmtp_config() -> Result<LmtpConfig> {
    let hostname = env::var("MAIL_POSTFIX_HOSTNAME")?;
    if hostname.is_empty()
        || !hostname
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err(invalid("MAIL_POSTFIX_HOSTNAME must be a DNS hostname"));
    }
    let max_bytes = env::var("MAIL_MAX_MESSAGE_BYTES")
        .unwrap_or_else(|_| "10485760".into())
        .parse::<usize>()?;
    if !(1024..=40 * 1024 * 1024).contains(&max_bytes) {
        return Err(invalid(
            "MAIL_MAX_MESSAGE_BYTES must be between 1 KiB and 40 MiB",
        ));
    }
    Ok(LmtpConfig {
        hostname,
        max_bytes,
    })
}

fn spawn_dispatch_loop(api: ApiClient) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = api.dispatch().await {
                tracing::warn!(%error, "Pending mail dispatch failed");
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let api = ApiClient::new(&env::var("API_BASE_URL")?, sink_token().await?)?;
    let config = Arc::new(lmtp_config()?);
    spawn_dispatch_loop(api.clone());
    let lmtp_address = env::var("MAIL_LMTP_LISTEN").unwrap_or_else(|_| "127.0.0.1:2525".into());
    let socketmap_address =
        env::var("MAIL_SOCKETMAP_LISTEN").unwrap_or_else(|_| "127.0.0.1:2526".into());
    let shared: Arc<dyn IngestApi> = Arc::new(RecipientCache::new(
        api,
        ACCEPTED_RECIPIENT_TTL,
        REJECTED_RECIPIENT_TTL,
        RECIPIENT_CACHE_CAPACITY,
    ));
    tokio::select! {
        result = serve(&lmtp_address, shared.clone(), config.clone(), false) => result,
        result = serve(&socketmap_address, shared, config, true) => result,
        result = tokio::signal::ctrl_c() => { result?; Ok(()) },
    }
}
