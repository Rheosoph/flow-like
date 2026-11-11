//! Conversation files: what people put into a conversation or attached to the message being
//! answered. They live outside every app, so a host reads them through a [`ConversationFiles`]
//! source where its [`AppAccess`](crate::AppAccess) holds no store for them.

use std::time::Duration;

use async_trait::async_trait;
use flow_like_bot_tools::FileData;
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::bot_message::CONVERSATION_FILE_APP_PREFIX;
use flow_like_types::reqwest::{self, Client, Response, StatusCode, redirect};
use flow_like_types::{Bytes, Error, Result, anyhow};

use crate::wire::JobFile;

const LINK_TIMEOUT: Duration = Duration::from_secs(120);

#[async_trait]
pub trait ConversationFiles: Send + Sync {
    /// Whether this source, not the app store, holds the files of `conversation_id`.
    fn serves(&self, conversation_id: &str) -> bool;

    /// The file's size and at most `max_bytes` of it; `None` when the conversation has no such
    /// file.
    async fn read(
        &self,
        conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>>;
}

/// The object's size and at most its first `max_bytes`; `None` when the store has no such object.
pub async fn read_object(
    store: &FlowLikeStore,
    location: &Path,
    max_bytes: u64,
) -> Result<Option<(Bytes, u64)>> {
    let store = store.as_generic();
    let meta = match store.head(location).await {
        Ok(meta) => meta,
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let wanted = meta.size.min(max_bytes);
    let bytes = if wanted == 0 {
        Bytes::new()
    } else {
        store.get_range(location, 0..wanted).await?
    };
    Ok(Some((bytes, meta.size)))
}

/// The conversation files of one tool job, each read through the link the API signed for it.
pub struct LinkedFiles {
    files: Vec<JobFile>,
}

impl LinkedFiles {
    pub fn new(files: Vec<JobFile>) -> Self {
        Self { files }
    }

    fn file(&self, conversation_id: &str, path: &str) -> Option<&JobFile> {
        self.files.iter().find(|file| {
            file.path == path
                && file.app_id.strip_prefix(CONVERSATION_FILE_APP_PREFIX) == Some(conversation_id)
        })
    }
}

#[async_trait]
impl ConversationFiles for LinkedFiles {
    fn serves(&self, _conversation_id: &str) -> bool {
        true
    }

    async fn read(
        &self,
        conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        let Some(file) = self.file(conversation_id, path) else {
            return Ok(None);
        };
        let read = flow_like_types::tokio::time::timeout(LINK_TIMEOUT, fetch(&file.url, max_bytes))
            .await
            .unwrap_or_else(|_| Err(anyhow!("its link did not answer in time")))
            .map_err(|error| anyhow!("Reading '{path}' failed: {error}"))?;
        Ok(read.map(|(bytes, size)| FileData {
            bytes,
            size,
            mime: file.mime.clone().filter(|mime| !mime.is_empty()),
            classification: None,
        }))
    }
}

/// A signed link is a credential, so a transport error is reported without its URL.
fn transport(error: reqwest::Error) -> Error {
    anyhow!("{}", error.without_url())
}

async fn fetch(url: &str, max_bytes: u64) -> Result<Option<(Bytes, u64)>> {
    let response = Client::builder()
        .redirect(redirect::Policy::none())
        .build()
        .map_err(transport)?
        .get(url)
        .send()
        .await
        .map_err(transport)?;
    match response.status() {
        StatusCode::NOT_FOUND => Ok(None),
        status if status.is_success() => read_capped(response, max_bytes).await.map(Some),
        status => Err(anyhow!("its link answered HTTP {}", status.as_u16())),
    }
}

/// The body when it fits `max_bytes`, with the file's size. A larger file comes back without
/// bytes, so the caller learns its size from a read it has to refuse.
async fn read_capped(mut response: Response, max_bytes: u64) -> Result<(Bytes, u64)> {
    if let Some(size) = response.content_length().filter(|size| *size > max_bytes) {
        return Ok((Bytes::new(), size));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max_bytes {
            return Ok((Bytes::new(), body.len() as u64));
        }
    }
    let size = body.len() as u64;
    Ok((Bytes::from(body), size))
}
