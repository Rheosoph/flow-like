// A separate SDK peer implements the application framing without importing the adapter.
use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::Duration;
use tokio::io::AsyncReadExt;

const ALPN: &[u8] = b"flow-like/messages/2";
const LIMIT: usize = 16 * 1024 * 1024;

async fn receive(stream: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
    let wire = stream.read_to_end(LIMIT + 4).await?;
    ensure!(wire.len() >= 4, "truncated frame header");
    let length = u32::from_be_bytes(wire[..4].try_into()?) as usize;
    ensure!(
        length <= LIMIT && wire.len() == length + 4,
        "invalid frame length"
    );
    Ok(wire[4..].to_vec())
}

async fn send(stream: &mut iroh::endpoint::SendStream, payload: &[u8]) -> Result<()> {
    let header = (payload.len() as u32).to_be_bytes();
    // Split the header to ensure the adapter handles stream fragmentation.
    stream.write_all(&header[..1]).await?;
    stream.write_all(&header[1..]).await?;
    for chunk in payload.chunks(8191) {
        stream.write_all(chunk).await?;
    }
    stream.finish()?;
    Ok(())
}

async fn run() -> Result<()> {
    let mode = std::env::args().nth(1).context("missing reference mode")?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>()?)?
        .bind()
        .await?;
    println!(
        "{}",
        json!({
            "endpoint_id": endpoint.id().to_string(),
            "addresses": endpoint.addr().ip_addrs().map(ToString::to_string).collect::<Vec<_>>(),
            "relay_url": null
        })
    );
    std::io::stdout().flush()?;
    let connection = if mode.starts_with("client-") {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        let peer: Value = serde_json::from_str(&line)?;
        let mut address = EndpointAddr::new(peer["endpoint_id"].as_str().context("id")?.parse()?);
        for ip in peer["addresses"].as_array().context("addresses")? {
            address = address.with_ip_addr(ip.as_str().context("address")?.parse()?);
        }
        endpoint.connect(address, ALPN).await?
    } else {
        endpoint.accept().await.context("endpoint closed")?.await?
    };
    match mode.as_str() {
        "echo" => {
            while let Ok((mut ack, mut input)) = connection.accept_bi().await {
                let payload = receive(&mut input).await?;
                ack.write_all(&[1]).await?;
                ack.finish()?;
                let _ = ack.stopped().await;
                let (mut output, mut reply) = connection.open_bi().await?;
                send(&mut output, &payload).await?;
                ensure!(
                    reply.read_to_end(2).await? == [1],
                    "invalid echo acknowledgement"
                );
            }
        }
        "oversize" | "truncated" | "trailing" | "short-header" => {
            let (mut output, mut ack) = connection.open_bi().await?;
            let wire = match mode.as_str() {
                "oversize" => ((LIMIT + 1) as u32).to_be_bytes().to_vec(),
                "truncated" => vec![0, 0, 0, 3, 42],
                "trailing" => vec![0, 0, 0, 1, 42, 99],
                _ => vec![0, 0],
            };
            output.write_all(&wire).await?;
            output.finish()?;
            ensure!(
                ack.read_u8().await.ok() != Some(1),
                "malformed frame acknowledged"
            );
            connection.closed().await;
        }
        "bad-ack" | "trailing-ack" | "silent-ack" => {
            let (mut ack, mut input) = connection.accept_bi().await?;
            receive(&mut input).await?;
            if mode != "silent-ack" {
                ack.write_all(if mode == "bad-ack" { &[0] } else { &[1, 2] })
                    .await?;
                ack.finish()?;
            }
            connection.closed().await;
        }
        "client-allowed" => {
            let (mut output, mut ack) = connection.open_bi().await?;
            send(&mut output, b"trusted peer").await?;
            ensure!(
                ack.read_to_end(2).await? == [1],
                "invalid client acknowledgement"
            );
            connection.closed().await;
        }
        "client-denied" => {
            let reason = connection.closed().await.to_string();
            ensure!(
                reason.contains("peer not allowed"),
                "unexpected rejection: {reason}"
            );
        }
        _ => anyhow::bail!("unknown reference mode"),
    }
    endpoint.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), run()).await?
}
