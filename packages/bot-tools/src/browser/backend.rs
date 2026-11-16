use async_trait::async_trait;
use flow_like_types::interaction_observation::InteractionObservation;
use flow_like_types::{Bytes, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ToolError, code};

#[derive(Debug, Clone)]
pub struct BrowserScreen {
    pub png: Vec<u8>,
    pub observation: Option<InteractionObservation>,
}

/// How the browser takes the bot's input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    /// As fast as the protocol delivers it; nothing happens between calls.
    #[default]
    Standard,
    /// At a person's pace: the pointer travels to what it clicks, keys and the wheel keep a
    /// person's rhythm, the hand moves now and then between calls, and the bot's own browser
    /// runs without the switches that announce automation.
    Human,
}

/// What `browser_open` asks of the browser besides the address.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOptions {
    #[serde(default)]
    pub mode: BrowserMode,
    /// The proxy the owner set for the bot's own browser (secret `BROWSER_PROXY`), sign-in
    /// included; the owner's browser never takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

impl std::fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenOptions")
            .field("mode", &self.mode)
            .field("proxy", &self.proxy.is_some())
            .finish()
    }
}

/// A bot check the current page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Challenge {
    /// `turnstile`, `cloudflare_page`, `recaptcha`, `hcaptcha`, `perimeter_x`, `datadome` or
    /// `akamai`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    pub solved: bool,
    /// An image or slider task is showing that a click does not answer.
    pub puzzle: bool,
    /// A box to tick or a button to hold is showing.
    pub pressable: bool,
    pub url: String,
}

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

/// What a host answers whose browser cannot work on bot checks.
pub fn no_challenges() -> flow_like_types::Error {
    flow_like_types::Error::new(ToolError::new(
        code::NOT_SUPPORTED,
        "This host's browser cannot work on bot checks",
    ))
}

/// What a host answers that has no browser of the owner's at hand.
pub fn no_owner_browser() -> flow_like_types::Error {
    flow_like_types::Error::new(ToolError::new(
        code::NOT_SUPPORTED,
        "Only a bot hosted in the desktop app can use the owner's browser; open the page in your own browser",
    ))
}

/// A browser the host drives for one bot and conversation. The first `open` starts it; `close`
/// (or the host's idle timeout) ends it. Element references come from the latest `snapshot`.
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// Opens `url` in the bot's own browser, which takes input in `options.mode` from now on
    /// and goes through `options.proxy`.
    async fn open(&self, url: &str, options: &OpenOptions) -> Result<PageState>;

    /// Opens `url` in a tab of the browser the owner uses every day, with the owner's logins.
    /// The session then reads and acts there, with input in `options.mode`, until the next
    /// `open` or `close`. A failed call leaves the session where it was. Only a host on the
    /// owner's own computer has one.
    async fn open_in_owner_browser(&self, _url: &str, _options: &OpenOptions) -> Result<PageState> {
        Err(no_owner_browser())
    }

    /// The bot check the current page shows, if any.
    async fn challenge(&self) -> Result<Option<Challenge>> {
        Ok(None)
    }

    /// Ticks the check's box or presses and holds its button, the way a person does. Returns
    /// the check as it was before.
    async fn press_challenge(&self) -> Result<Option<Challenge>> {
        Err(no_challenges())
    }

    /// Puts a solving service's token into the page: `callback` when the widget's callback ran,
    /// `fields` when only its response fields took it, `none` when nothing did.
    async fn apply_challenge_token(&self, _token: &str) -> Result<String> {
        Err(no_challenges())
    }

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView>;

    async fn act(&self, step: &BrowserStep) -> Result<PageState>;

    /// A PNG of the visible page.
    async fn screenshot(&self) -> Result<Vec<u8>>;

    /// Captures the image and its action feedback together when the backend supports it.
    async fn capture(&self) -> Result<BrowserScreen> {
        Ok(BrowserScreen {
            png: self.screenshot().await?,
            observation: None,
        })
    }

    async fn observation(&self) -> Result<Option<InteractionObservation>> {
        Ok(None)
    }

    /// The page the session is on; `None` before the first `open`.
    async fn current(&self) -> Result<Option<PageState>>;

    /// Moves the page off a site the owner did not approve.
    async fn leave(&self) -> Result<()>;

    /// Files the browser finished downloading since the last call.
    async fn take_downloads(&self) -> Result<Vec<Download>>;

    async fn close(&self) -> Result<()>;
}
