use async_trait::async_trait;
use flow_like_types::{Bytes, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageState {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotRequest {
    pub max_chars: usize,
    pub interactive_only: bool,
}

#[derive(Debug, Clone)]
pub struct SnapshotView {
    pub page: PageState,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserStep {
    Click {
        reference: String,
    },
    Type {
        reference: String,
        text: String,
    },
    Select {
        reference: String,
        value: String,
    },
    Press {
        reference: Option<String>,
        key: String,
    },
    Scroll {
        reference: Option<String>,
        direction: ScrollTo,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTo {
    Up,
    Down,
    Top,
    Bottom,
    IntoView,
}

#[derive(Debug, Clone)]
pub struct Download {
    pub name: String,
    pub bytes: Bytes,
}

/// A browser the host drives for one bot and conversation. The first `open` starts it; `close`
/// (or the host's idle timeout) ends it. Element references come from the latest `snapshot`.
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    async fn open(&self, url: &str) -> Result<PageState>;

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView>;

    async fn act(&self, step: &BrowserStep) -> Result<PageState>;

    /// A PNG of the visible page.
    async fn screenshot(&self) -> Result<Vec<u8>>;

    /// The page the session is on; `None` before the first `open`.
    async fn current(&self) -> Result<Option<PageState>>;

    /// Moves the page off a site the owner did not approve.
    async fn leave(&self) -> Result<()>;

    /// Files the browser finished downloading since the last call.
    async fn take_downloads(&self) -> Result<Vec<Download>>;

    async fn close(&self) -> Result<()>;
}
