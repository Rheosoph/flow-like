//! The private, bounded pipe between the device supervisor and one bot container.

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) const MAX_FRAME: usize = 32 * 1024 * 1024;
pub(super) const PIPE_BYTES: usize = 512 * 1024;

// Credentials and private inbox keys pass through this pipe. Never derive Debug or log frames.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Frame {
    Init {
        base_url: String,
        device_id: String,
        bot_id: String,
        keys: Option<String>,
    },
    Call {
        id: u64,
        method: String,
        path: String,
        body: Option<Value>,
    },
    Reply {
        id: u64,
        status: u16,
        body: String,
    },
    Keys {
        data: String,
    },
    OpenView {
        id: u64,
        observations: bool,
    },
    ViewData {
        id: u64,
        data: String,
    },
    CloseView {
        id: u64,
    },
    Ping,
    Shutdown,
}

pub(super) fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub(super) fn decode(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len() <= MAX_FRAME,
        "Container payload exceeds its limit"
    );
    STANDARD.decode(value).context("Decode container payload")
}

pub(super) async fn read(reader: &mut (impl AsyncRead + Unpin)) -> Result<Option<Frame>> {
    let mut header = [0u8; 4];
    let first = reader.read(&mut header[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await?;
    let length = u32::from_be_bytes(header) as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        "Invalid container frame length"
    );
    let mut data = vec![0; length];
    reader.read_exact(&mut data).await?;
    serde_json::from_slice(&data)
        .context("Decode container frame")
        .map(Some)
}

pub(super) async fn write(writer: &mut (impl AsyncWrite + Unpin), frame: &Frame) -> Result<()> {
    let data = serde_json::to_vec(frame)?;
    ensure!(data.len() <= MAX_FRAME, "Container frame exceeds its limit");
    writer.write_all(&(data.len() as u32).to_be_bytes()).await?;
    writer.write_all(&data).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_frames_before_allocating_the_body() {
        let mut bytes = std::io::Cursor::new(((MAX_FRAME + 1) as u32).to_be_bytes());
        assert!(
            read(&mut bytes)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("length")
        );
    }

    #[tokio::test]
    async fn frames_round_trip_and_end_cleanly() {
        let mut bytes = Vec::new();
        write(
            &mut bytes,
            &Frame::Call {
                id: 7,
                method: "POST".into(),
                path: "/poll".into(),
                body: None,
            },
        )
        .await
        .unwrap();
        let mut reader = std::io::Cursor::new(bytes);
        assert!(matches!(
            read(&mut reader).await.unwrap(),
            Some(Frame::Call { id: 7, .. })
        ));
        assert!(read(&mut reader).await.unwrap().is_none());
    }
}
