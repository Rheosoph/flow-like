use flow_like_executor::{
    ExecutionRequest, ExecutorConfig, execute, execute_streaming, types::DispatchPayload,
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// Only this run's full dispatch envelope enters the process. Credentials never
/// pass through argv, Docker environment variables, or an input volume.
pub async fn run(mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !matches!(mode, "callback" | "callback-queued" | "stream" | "warm") {
        return Err("Expected --once callback, callback-queued, stream, or warm".into());
    }
    if std::env::var("SANDBOX_PROXY_SOCKET").is_ok() {
        start_proxy().await?;
    }
    // Only trusted static state is prepared here. Once an envelope is assigned,
    // this process executes once and exits, including failed assignments.
    flow_like_catalog::initialize();
    flow_like_executor::prepare_runtime();
    let mut stdout = tokio::io::stdout();
    let (mode, payload) = if mode == "warm" {
        // A warm sandbox cannot fetch JWKS through an unassigned gateway.
        if std::env::var("BACKEND_PUB").is_err() {
            return Err("Warm execution requires BACKEND_PUB".into());
        }
        flow_like_executor::jwt::prepare_verification_key().await?;
        stdout.write_all(b"ready\n").await?;
        stdout.flush().await?;
        let mut input = Vec::new();
        tokio::io::BufReader::new(tokio::io::stdin().take((MAX_INPUT_BYTES + 1) as u64))
            .read_until(b'\n', &mut input)
            .await?;
        let envelope = decode_warm(&input)?;
        (envelope.mode, envelope.payload)
    } else {
        let mut input = Vec::new();
        tokio::io::stdin()
            .take((MAX_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut input)
            .await?;
        (mode.to_string(), decode(&input)?)
    };
    let run_id = payload.run_id.clone();
    let request = ExecutionRequest::try_from(payload)?;
    let mut config = ExecutorConfig::from_env();
    if mode == "callback-queued" {
        config = config.with_required_terminal_status_ack();
    }
    if mode == "stream" {
        let mut events = execute_streaming(request, config).await?;
        while let Some(event) = events.next().await {
            stdout
                .write_all(flow_like_executor::streaming::event_to_ndjson(&event).as_bytes())
                .await?;
            stdout.flush().await?;
        }
    } else {
        match execute(request, config).await {
            Ok(result) => {
                let event = serde_json::json!({"event_type": "completed", "payload": result});
                stdout.write_all(format!("{event}\n").as_bytes()).await?;
            }
            Err(error) => {
                // Internal diagnostics remain on stderr. This result does not
                // imply that a required terminal callback was acknowledged.
                let event = serde_json::json!({"event_type": "error", "payload": {
                    "run_id": run_id, "message": "Execution failed before terminal acknowledgement"
                }});
                stdout.write_all(format!("{event}\n").as_bytes()).await?;
                stdout.flush().await?;
                return Err(error.into());
            }
        }
    }
    stdout.flush().await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WarmEnvelope {
    mode: String,
    payload: DispatchPayload,
}

fn decode_warm(input: &[u8]) -> Result<WarmEnvelope, Box<dyn std::error::Error>> {
    if input.len() > MAX_INPUT_BYTES || !input.ends_with(b"\n") {
        return Err("Expected one newline-terminated envelope of at most 8 MiB".into());
    }
    let envelope: WarmEnvelope = serde_json::from_slice(input)?;
    if !matches!(
        envelope.mode.as_str(),
        "callback" | "callback-queued" | "stream"
    ) {
        return Err("Invalid warm execution mode".into());
    }
    Ok(envelope)
}

fn decode(input: &[u8]) -> Result<DispatchPayload, Box<dyn std::error::Error>> {
    if input.len() > MAX_INPUT_BYTES {
        return Err("Dispatch input exceeds 8 MiB".into());
    }
    Ok(serde_json::from_slice(input)?)
}

/// The sandbox has no external network interface. A loopback relay reaches
/// only the run's mounted HTTP proxy socket, whose policy runs outside gVisor.
#[cfg(unix)]
async fn start_proxy() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use tokio::net::{TcpListener, UnixStream};
    use tokio::sync::Semaphore;
    let path = std::env::var("SANDBOX_PROXY_SOCKET")?;
    if path != "/gateway/proxy.sock" {
        return Err("Invalid sandbox proxy socket".into());
    }
    // Prove the configured runtime permits this mounted UDS before code loads.
    drop(UnixStream::connect(&path).await?);
    let listener = TcpListener::bind("127.0.0.1:3128").await?;
    let permits = Arc::new(Semaphore::new(64));
    tokio::spawn(async move {
        while let Ok((mut incoming, _)) = listener.accept().await {
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                continue;
            };
            let path = path.clone();
            tokio::spawn(async move {
                let _permit = permit;
                if let Ok(mut outgoing) = UnixStream::connect(path).await {
                    let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
                }
            });
        }
    });
    Ok(())
}

#[cfg(not(unix))]
async fn start_proxy() -> Result<(), Box<dyn std::error::Error>> {
    Err("Sandbox proxy requires Unix sockets".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_multiple_inputs_and_excessive_input() {
        assert!(decode(b"{}\n{}").is_err());
        assert!(decode(&vec![b' '; MAX_INPUT_BYTES + 1]).is_err());
    }

    #[test]
    fn warm_input_requires_bounded_single_frame() {
        assert!(decode_warm(b"{}").is_err());
        assert!(decode_warm(b"{}\n{}\n").is_err());
        assert!(decode_warm(&vec![b' '; MAX_INPUT_BYTES + 1]).is_err());
    }
}
