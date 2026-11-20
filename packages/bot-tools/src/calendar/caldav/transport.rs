//! How a CalDAV request leaves this host: the egress rules and address pinning of
//! `connector_request` (a server reaches public https hosts only, a desktop its LAN too), Basic
//! sign-in, no automatic redirects, 1 MiB out, 5 MiB back, 30 s. Behind [`DavTransport`] so tests
//! answer from fixtures.

use std::time::Duration;

use async_trait::async_trait;
use flow_like_types::reqwest::{self, Method, Url, header};

use crate::env::Environment;
use crate::services::Credential;
use crate::services::egress::resolve_reachable;

pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const XML: &str = "application/xml; charset=utf-8";
const CALENDAR: &str = "text/calendar; charset=utf-8";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DavMethod {
    Propfind,
    Report,
    Get,
    Put,
    Delete,
}

impl DavMethod {
    pub fn name(self) -> &'static str {
        match self {
            Self::Propfind => "PROPFIND",
            Self::Report => "REPORT",
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }

    /// Whether the request only reads.
    pub fn reads(self) -> bool {
        matches!(self, Self::Propfind | Self::Report | Self::Get)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Zero,
    One,
}

impl Depth {
    fn header(self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::One => "1",
        }
    }
}

/// What must hold on the server for a write to go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    None,
    /// `If-Match`: the resource still has this ETag.
    IfMatch(String),
    /// `If-None-Match: *`: nothing exists at the URL yet.
    IfAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DavBody {
    Xml(String),
    Calendar(String),
}

impl DavBody {
    fn parts(&self) -> (&'static str, &str) {
        match self {
            Self::Xml(text) => (XML, text),
            Self::Calendar(text) => (CALENDAR, text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavRequest {
    pub method: DavMethod,
    pub url: Url,
    pub depth: Option<Depth>,
    pub precondition: Precondition,
    pub body: Option<DavBody>,
}

impl DavRequest {
    pub fn new(method: DavMethod, url: Url) -> Self {
        Self {
            method,
            url,
            depth: None,
            precondition: Precondition::None,
            body: None,
        }
    }

    pub fn depth(mut self, depth: Depth) -> Self {
        self.depth = Some(depth);
        self
    }

    pub fn precondition(mut self, precondition: Precondition) -> Self {
        self.precondition = precondition;
        self
    }

    pub fn body(mut self, body: DavBody) -> Self {
        self.body = Some(body);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavResponse {
    pub status: u16,
    pub etag: Option<String>,
    pub location: Option<String>,
    pub text: String,
}

/// How a CalDAV request leaves this host; a fake in tests.
#[async_trait]
pub trait DavTransport: Send + Sync {
    async fn send(
        &self,
        request: &DavRequest,
        credential: &Credential,
    ) -> Result<DavResponse, String>;
}

/// HTTP(S) with the egress rules of `environment`.
pub struct HttpDavTransport(pub Environment);

fn method_of(method: DavMethod) -> Result<Method, String> {
    Method::from_bytes(method.name().as_bytes())
        .map_err(|error| format!("Method {} is not valid: {error}", method.name()))
}

fn header_text(response: &reqwest::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[async_trait]
impl DavTransport for HttpDavTransport {
    async fn send(
        &self,
        request: &DavRequest,
        credential: &Credential,
    ) -> Result<DavResponse, String> {
        let host = request.url.host_str().unwrap_or_default().to_string();
        if self.0 == Environment::Server && request.url.scheme() != "https" {
            return Err(format!(
                "'{host}' is not https; a server reaches calendars over https only"
            ));
        }
        let reachable = resolve_reachable(self.0, request.url.as_str()).await?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT);
        if !reachable.addrs.is_empty() {
            builder = builder.resolve_to_addrs(&reachable.host, &reachable.addrs);
        }
        let client = builder
            .build()
            .map_err(|error| format!("The HTTP client could not be created: {error}"))?;
        let mut outgoing = client
            .request(method_of(request.method)?, reachable.url)
            .header(header::USER_AGENT, "Flow-Like-Bot");
        if let Some(value) = credential.header() {
            outgoing = outgoing.header(header::AUTHORIZATION, value);
        }
        if let Some(depth) = request.depth {
            outgoing = outgoing.header("Depth", depth.header());
        }
        outgoing = match &request.precondition {
            Precondition::None => outgoing,
            Precondition::IfMatch(etag) => outgoing.header(header::IF_MATCH, etag.as_str()),
            Precondition::IfAbsent => outgoing.header(header::IF_NONE_MATCH, "*"),
        };
        if let Some(body) = &request.body {
            let (content_type, text) = body.parts();
            if text.len() > MAX_BODY_BYTES {
                return Err(format!(
                    "The request body is larger than {MAX_BODY_BYTES} bytes"
                ));
            }
            outgoing = outgoing
                .header(header::CONTENT_TYPE, content_type)
                .body(text.to_string());
        }
        let mut response = outgoing.send().await.map_err(|error| {
            format!(
                "{} to '{host}' failed: {}",
                request.method.name(),
                error.without_url()
            )
        })?;
        let status = response.status().as_u16();
        let etag = header_text(&response, header::ETAG);
        let location = header_text(&response, header::LOCATION);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            format!(
                "Reading the answer of '{host}' failed: {}",
                error.without_url()
            )
        })? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(format!(
                    "The answer of '{host}' is larger than {MAX_RESPONSE_BYTES} bytes; ask for a shorter span"
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(DavResponse {
            status,
            etag,
            location,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}
