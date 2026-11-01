//! Process fixture for the browser-to-executor NATS tests. Configuration arrives on stdin.
use flow_like_channels::ChannelForwarder;
use flow_like_channels::nats::{NatsChannel, NatsForwarder, Role, mint_user_token, subject_for};
use flow_like_types::channel::{
    ChannelClientDescriptor, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome,
    ChannelPush, now_unix,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Config {
    url: String,
    ws_url: String,
    account_seed: String,
    channel_id: String,
    #[serde(default = "lifetime")]
    ttl_secs: i64,
}
fn lifetime() -> i64 {
    120
}
fn emit(value: Value) {
    println!("{value}");
    std::io::stdout().flush().unwrap();
}

#[tokio::main]
async fn main() -> flow_like_types::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let config: Config = serde_json::from_str(
        &lines
            .next_line()
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("worker config required"))?,
    )?;
    let subject = subject_for(&config.channel_id)?;
    let expires_at = now_unix() + config.ttl_secs;
    let grant = ChannelGrant {
        channel_id: config.channel_id.clone(),
        expires_at,
        executor: ChannelExecutorGrant::Nats {
            url: config.url.clone(),
            subject: subject.clone(),
            token: mint_user_token(
                &config.account_seed,
                &config.channel_id,
                Role::Executor,
                config.ttl_secs,
            )?,
        },
        client: ChannelHandle {
            channel_id: config.channel_id.clone(),
            request_id: None,
            expires_at,
            transport: ChannelClientDescriptor::Nats {
                url: config.ws_url,
                subject,
                token: mint_user_token(
                    &config.account_seed,
                    &config.channel_id,
                    Role::Client,
                    config.ttl_secs,
                )?,
                expires_at,
            },
            fallback: None,
        },
    };
    let channel = NatsChannel::connect(&grant).await?;
    let ticket = channel
        .open(Duration::from_secs(config.ttl_secs as u64))
        .await?;
    emit(json!({"event":"ready","handle":ticket.handle}));
    let waiter = channel.clone();
    tokio::spawn(async move {
        match waiter.wait(&ticket, None).await {
            Ok(ChannelOutcome::Responded(value)) => emit(json!({"event":"reply","value":value})),
            Ok(other) => emit(json!({"event":"wait_ended","outcome":format!("{other:?}")})),
            Err(error) => emit(json!({"event":"error","error":error.to_string()})),
        }
    });
    let forwarder = NatsForwarder::new(config.url, config.account_seed)?;
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let values = channel.drain_inbound().await;
                if !values.is_empty() { emit(json!({"event":"inbound","values":values})); }
                if channel.is_cancelled().await { emit(json!({"event":"cancelled"})); break; }
                if now_unix() >= expires_at { emit(json!({"event":"expired"})); break; }
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                let command: Value = serde_json::from_str(&line)?;
                match command["command"].as_str() {
                    Some("forward") => {
                        let push: ChannelPush = serde_json::from_value(command["push"].clone())?;
                        let id = command["id"].as_str().unwrap_or("forward");
                        let result = forwarder.forward_with_id(&push,id).await;
                        emit(json!({"event":"forwarded","id":id,"accepted":result.is_ok(),"error":result.err().map(|e| e.to_string())}));
                    }
                    Some("close") => break,
                    _ => emit(json!({"event":"error","error":"Unknown worker command"})),
                }
            }
        }
    }
    // Allow the receiver's confirmation to flush before the process ends.
    tokio::time::sleep(Duration::from_millis(100)).await;
    channel.close().await;
    Ok(())
}
