//! AWS-only reply framing. The API's forwarder splits a large reply into these frames and the
//! executor's router reassembles them. Older waiters reject `reply_chunk` as an unknown kind
//! instead of delivering a partial reply, so deploy waiters before the API.

use std::time::{Duration, Instant};

use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
#[cfg(any(feature = "aws", test))]
use flow_like_types::channel::new_request_id;
use flow_like_types::channel::{ChannelPush, ChannelPushKind};
use flow_like_types::{Result, bail};
use serde::{Deserialize, Serialize};

// The forwarder splits replies with these limits and the router reassembles them. The API's push
// limits in packages/api/src/channel/forward.rs must stay within them.
pub(crate) const MAX_PAYLOAD_BYTES: usize = 128 * 1024;
pub(crate) const CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_ACTIVE_TRANSFERS: usize = 8;
pub(crate) const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CHUNKS: usize = MAX_REPLY_BYTES / CHUNK_BYTES;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChunkKind {
    ReplyChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyChunk {
    pub channel_id: String,
    pub request_id: String,
    pub kind: ChunkKind,
    pub transfer_id: String,
    pub index: usize,
    pub total: usize,
    pub total_bytes: usize,
    pub data: String,
}

/// Validate the complete message budget before publishing any frames.
#[cfg(any(feature = "aws", test))]
pub(crate) fn reply_payloads(push: &ChannelPush) -> Result<Vec<Vec<u8>>> {
    let body = serde_json::to_vec(push)?;
    if body.len() <= MAX_PAYLOAD_BYTES {
        return Ok(vec![body]);
    }
    let Some(request_id) = push.request_id.as_ref().filter(|id| !id.is_empty()) else {
        bail!("AWS channel messages larger than 128 KiB must be replies with a request_id");
    };
    if push.kind != ChannelPushKind::Reply {
        bail!("AWS channel messages larger than 128 KiB must be replies with a request_id");
    }
    if body.len() > MAX_REPLY_BYTES {
        bail!("AWS channel reply exceeds the 2 MiB reassembly limit; request a smaller result");
    }
    let transfer_id = new_request_id();
    let total = body.len().div_ceil(CHUNK_BYTES);
    body.chunks(CHUNK_BYTES)
        .enumerate()
        .map(|(index, bytes)| {
            let payload = serde_json::to_vec(&ReplyChunk {
                channel_id: push.channel_id.clone(),
                request_id: request_id.clone(),
                kind: ChunkKind::ReplyChunk,
                transfer_id: transfer_id.clone(),
                index,
                total,
                total_bytes: body.len(),
                data: STANDARD.encode(bytes),
            })?;
            if payload.len() > MAX_PAYLOAD_BYTES {
                bail!(
                    "AWS channel reply identifiers leave insufficient space for the chunk envelope"
                );
            }
            Ok(payload)
        })
        .collect()
}

pub(crate) struct ReplyAssembly {
    pub transfer_id: String,
    last_progress_at: Instant,
    total_bytes: usize,
    total: usize,
    bytes: Vec<u8>,
}

pub(crate) enum AppendResult {
    Pending,
    Duplicate,
    Complete(ChannelPush),
}

impl ReplyAssembly {
    pub fn new(chunk: &ReplyChunk, now: Instant) -> Result<Self> {
        if chunk.index != 0
            || chunk.transfer_id.is_empty()
            || chunk.transfer_id.len() > 64
            || chunk.request_id.is_empty()
            || chunk.total_bytes == 0
            || chunk.total_bytes > MAX_REPLY_BYTES
            || chunk.total == 0
            || chunk.total > MAX_CHUNKS
            || chunk.total != chunk.total_bytes.div_ceil(CHUNK_BYTES)
        {
            bail!("Invalid AWS reply chunk metadata");
        }
        Ok(Self {
            transfer_id: chunk.transfer_id.clone(),
            last_progress_at: now,
            total_bytes: chunk.total_bytes,
            total: chunk.total,
            bytes: Vec::new(),
        })
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.last_progress_at) >= TRANSFER_IDLE_TIMEOUT
    }

    pub fn append(&mut self, chunk: &ReplyChunk, now: Instant) -> Result<AppendResult> {
        if chunk.transfer_id != self.transfer_id
            || chunk.total_bytes != self.total_bytes
            || chunk.total != self.total
            || chunk.index >= self.total
            || chunk.data.len() > CHUNK_BYTES.div_ceil(3) * 4
        {
            bail!("Inconsistent AWS reply chunk metadata");
        }
        let start = chunk.index * CHUNK_BYTES;
        let expected_length = (self.total_bytes - start).min(CHUNK_BYTES);
        let bytes = STANDARD.decode(&chunk.data)?;
        if bytes.len() != expected_length {
            bail!("Invalid AWS reply chunk length");
        }
        if start < self.bytes.len() {
            if self.bytes.get(start..start + bytes.len()) == Some(bytes.as_slice()) {
                return Ok(AppendResult::Duplicate);
            }
            bail!("Conflicting duplicate AWS reply chunk");
        }
        if start != self.bytes.len() {
            bail!("Out-of-order AWS reply chunk");
        }
        self.bytes.extend(bytes);
        if self.bytes.len() != self.total_bytes {
            // Only new, accepted bytes extend the inactivity deadline. Retransmitted frames
            // cannot retain an abandoned assembly indefinitely.
            self.last_progress_at = now;
            return Ok(AppendResult::Pending);
        }
        let push: ChannelPush = serde_json::from_slice(&self.bytes)?;
        if push.kind != ChannelPushKind::Reply
            || push.channel_id != chunk.channel_id
            || push.request_id.as_deref() != Some(chunk.request_id.as_str())
        {
            bail!("Reassembled AWS reply does not match its channel and request");
        }
        Ok(AppendResult::Complete(push))
    }
}

#[cfg(test)]
#[path = "reply_chunks_tests.rs"]
mod tests;
