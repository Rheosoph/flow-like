//! What a move needs from the account a bot moves to: its hub's JSON API, signed in.

use crate::model::SecretValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fmt, future::Future};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HubMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Serialize)]
struct SecretBody<'a> {
    value: &'a str,
    description: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub enum HubBody<'a> {
    Empty,
    Json(&'a Value),
    /// A secret on its way into the account's secret store: `{ value, description }`.
    Secret(&'a SecretValue),
}

impl HubBody<'_> {
    /// The body as JSON, `None` when empty. Wiped on drop, since it may carry a secret's value.
    pub fn to_json(&self) -> serde_json::Result<Option<Zeroizing<Vec<u8>>>> {
        let bytes = match self {
            Self::Empty => return Ok(None),
            Self::Json(value) => serde_json::to_vec(value)?,
            Self::Secret(secret) => serde_json::to_vec(&SecretBody {
                value: &secret.value,
                description: secret.description.as_deref(),
            })?,
        };
        Ok(Some(Zeroizing::new(bytes)))
    }
}

/// One call to `/api/v1/{path}?{query}`; the host percent-encodes each segment and value.
#[derive(Debug, Clone, Copy)]
pub struct HubCall<'a> {
    pub method: HubMethod,
    pub path: &'a [&'a str],
    pub query: &'a [(&'a str, &'a str)],
    pub body: HubBody<'a>,
}

impl<'a> HubCall<'a> {
    pub fn new(method: HubMethod, path: &'a [&'a str]) -> Self {
        Self {
            method,
            path,
            query: &[],
            body: HubBody::Empty,
        }
    }

    pub fn with(self, body: HubBody<'a>) -> Self {
        Self { body, ..self }
    }

    pub fn query(self, query: &'a [(&'a str, &'a str)]) -> Self {
        Self { query, ..self }
    }
}

/// `METHOD a/b/c`, never the body.
impl fmt::Display for HubCall<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method.as_str(), self.path.join("/"))
    }
}

/// Why a call failed. The message is what the hub said (or why it couldn't be reached) and
/// never carries a secret's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubError {
    /// The HTTP status; `None` when the hub could not be reached.
    pub status: Option<u16>,
    /// The hub's error code, when it sent one.
    pub code: Option<String>,
    pub message: String,
}

impl HubError {
    pub fn unreachable(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: None,
            message: message.into(),
        }
    }

    /// A step on this device failed, under `code`.
    pub fn failed(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: Some(code.into()),
            message: message.into(),
        }
    }

    pub fn answered(status: u16, code: Option<String>, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            code,
            message: message.into(),
        }
    }
}

/// What a workspace app holds that moves with its bot: the files under `upload/` and
/// `storage/`, as the account's `POST bots/{id}/workspace/upload` measures it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub total_size_bytes: u64,
    pub total_object_count: u64,
}

impl WorkspaceSummary {
    pub fn is_empty(&self) -> bool {
        self.total_size_bytes == 0 && self.total_object_count == 0
    }
}

/// The signed-in account a bot moves to: its hub's JSON API with the person's sign-in, and
/// what only this device can do for a workspace: read its files, write them to the account's
/// storage, compare the two copies and delete the local one.
pub trait AccountHub: Sync {
    /// The signed-in person's id in the account; moved chats name them where this device
    /// says `local`.
    fn caller(&self) -> &str;

    /// The response's JSON, `None` when it has none.
    fn call(
        &self,
        call: HubCall<'_>,
    ) -> impl Future<Output = Result<Option<Value>, HubError>> + Send;

    /// What the local workspace app holds that moves; nothing when it has no files.
    fn summarize_workspace(
        &self,
        app_id: &str,
    ) -> impl Future<Output = Result<WorkspaceSummary, HubError>> + Send;

    /// Copies the local workspace's files into the account's storage with the answer of
    /// `POST bots/{id}/workspace/upload` (`upload_path`, `shared_credentials`).
    fn upload_workspace(
        &self,
        app_id: &str,
        grant: &Value,
    ) -> impl Future<Output = Result<(), HubError>> + Send;

    /// The files, by path, that the account's storage lacks or holds at another size than the
    /// local workspace app; empty when the account has them all.
    fn workspace_differences(
        &self,
        app_id: &str,
        grant: &Value,
    ) -> impl Future<Output = Result<Vec<String>, HubError>> + Send;

    /// Deletes the local workspace app with its files and databases; one that is gone is fine.
    fn delete_workspace(&self, app_id: &str) -> impl Future<Output = Result<(), HubError>> + Send;
}
