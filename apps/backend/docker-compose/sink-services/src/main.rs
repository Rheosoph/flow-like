mod api_client;
mod cron;
mod discord;
mod storage;
mod telegram;

use api_client::ApiClient;
use cron::CronScheduler;
use serde::Deserialize;
use std::sync::Arc;
use storage::RedisStorage;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
struct Config {
    supported_sinks: SupportedSinks,
}

#[derive(Debug, Deserialize)]
struct SupportedSinks {
    #[serde(default)]
    cron: bool,
    #[serde(default)]
    discord: bool,
    #[serde(default)]
    telegram: bool,
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path =
        std::env::var("CONFIG_PATH").unwrap_or_else(|_| "/app/flow-like.config.json".to_string());

    let content = std::fs::read_to_string(&config_path)?;
    let config: Config = serde_json::from_str(&content)?;
    Ok(config)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("docker_compose_sink_services=info".parse().unwrap()),
        )
        .init();

    info!("Starting Flow-Like Sink Services");

    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            return Err(e);
        }
    };

    info!(
        "Loaded config: cron={}, discord={}, telegram={}",
        config.supported_sinks.cron,
        config.supported_sinks.discord,
        config.supported_sinks.telegram
    );

    let api_base_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://api:8080".to_string());
    let sink_trigger_jwt = std::env::var("SINK_TRIGGER_JWT").unwrap_or_default();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());

    if sink_trigger_jwt.is_empty() {
        warn!("SINK_TRIGGER_JWT not set - API calls will likely fail");
    }

    let storage = match RedisStorage::new(&redis_url).await {
        Ok(s) => {
            info!("Connected to Redis");
            Some(Arc::new(s))
        }
        Err(e) => {
            warn!(
                "Failed to connect to Redis: {} - running without persistence",
                e
            );
            None
        }
    };

    let api_client = Arc::new(ApiClient::new(&api_base_url, &sink_trigger_jwt));

    let mut handles = Vec::new();

    if config.supported_sinks.cron {
        let api_client_cron = Arc::clone(&api_client);
        let storage_cron = storage.clone();
        handles.push(tokio::spawn(async move {
            match CronScheduler::new(api_client_cron, storage_cron).await {
                Ok(scheduler) => {
                    if let Err(e) = scheduler.start().await {
                        error!("Failed to start cron scheduler: {}", e);
                        return;
                    }
                    scheduler.run_sync_loop().await;
                }
                Err(e) => {
                    error!("Failed to create cron scheduler: {}", e);
                }
            }
        }));
        info!("Cron scheduler task spawned");
    }

    if config.supported_sinks.discord {
        let api_client_discord = Arc::clone(&api_client);
        let storage_discord = storage.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = discord::start_discord_bot(api_client_discord, storage_discord).await {
                error!("Discord bot error: {}", e);
            }
        }));
        info!("Discord bot task spawned");
    }

    if config.supported_sinks.telegram {
        let api_client_telegram = Arc::clone(&api_client);
        let storage_telegram = storage.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) =
                telegram::start_telegram_bot(api_client_telegram, storage_telegram).await
            {
                error!("Telegram bot error: {}", e);
            }
        }));
        info!("Telegram bot task spawned");
    }

    if handles.is_empty() {
        warn!("No sinks enabled - service will idle");
    }

    info!("Sink services running, waiting for shutdown signal");

    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c");

    info!("Received shutdown signal, stopping services...");

    for handle in handles {
        handle.abort();
    }

    info!("Sink services stopped");
    Ok(())
}
