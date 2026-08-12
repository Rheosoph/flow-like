//! Safe, read-only public-web page retrieval for FlowPilot.
//!
//! This deliberately does less than a general HTTP client: one GET, public HTTP(S) hosts on their
//! default ports, textual responses, bounded bodies, and fully revalidated redirects. Fetched text
//! is evidence for the model, never instructions.

use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use flow_like_types::{reqwest, tokio};
use htmd::HtmlToMarkdownBuilder;
use serde_json::{Value, json};
use url::{Host, Url};

use super::tool_spec::{ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL};

const MAX_URL_BYTES: usize = 4_096;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 20_000;
const MAX_OUTPUT_CHARS: usize = 40_000;
const MAX_FIND_QUERY_CHARS: usize = 256;
const MAX_FIND_MATCHES: usize = 8;
const FIND_CONTEXT_CHARS: usize = 180;
const MIN_EVIDENCE_NON_WHITESPACE_CHARS: usize = 12;
const MAX_REDIRECTS: usize = 5;
const MAX_CONCURRENT_OPEN_URL_CALLS: usize = 4;
const MAX_CONCURRENT_ARCHIVE_LOOKUPS: usize = 2;
const MAX_CONCURRENT_HTML_CONVERSIONS: usize = 4;
const MAX_OPEN_URL_CALLS_PER_ROUND: usize = 4;
const MAX_OPEN_URL_CALLS_PER_SESSION: usize = 10;
const MAX_OPEN_URL_CHARS_PER_ROUND: usize = 60_000;
const MAX_OPEN_URL_CHARS_PER_SESSION: usize = 120_000;
/// Session-wide caps. On the tool-driven path these are shared across every agent researching
/// within one turn (see [`WebResearchSession`]); the rig loop in `platform.rs` applies the same
/// numbers to its own per-run counters, so they are declared once here.
pub(crate) const MAX_SEARCH_CALLS_PER_SESSION: usize = 12;
pub(crate) const MAX_ARCHIVE_CALLS_PER_SESSION: usize = 4;
const MAX_ARCHIVE_RESPONSE_BYTES: usize = 64 * 1024;
const WAYBACK_AVAILABILITY_ENDPOINT: &str = "https://archive.org/wayback/available";
const WAYBACK_CDX_ENDPOINT: &str = "https://web.archive.org/cdx/search/cdx";
const WAYBACK_CDX_FIELDS: [&str; 5] = ["timestamp", "original", "statuscode", "mimetype", "digest"];
const ARCHIVE_CAVEAT: &str = "This is an Internet Archive capture, not the live page. Captures may be incomplete, replayed incorrectly, or captured before or after the requested time; do not use one as evidence of current facts.";
const ARCHIVE_PRE_CUTOFF_CAVEAT: &str = "This is an Internet Archive capture, not the live page. It was selected as the latest exact-URL HTTP-200 capture at or before the requested time, but the capture time is not the page's publication or event time. Open and verify the snapshot before relying on it; captures may be incomplete or replayed incorrectly.";
const ARCHIVE_CLOSEST_LEAD_CAVEAT: &str = "No qualifying exact-URL HTTP-200 capture at or before the requested time was found in the Wayback CDX index. This closest Availability result is a research lead only, may be after the cutoff, and must not support a claim about what the page said by the requested time. Open it only to discover further evidence.";
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
static OPEN_URL_CONCURRENCY: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_OPEN_URL_CALLS));
static ARCHIVE_LOOKUP_CONCURRENCY: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_ARCHIVE_LOOKUPS));
static HTML_CONVERSION_CONCURRENCY: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_HTML_CONVERSIONS)));

/// Per-turn provenance ledger and spend budget for outbound page reads. Untrusted page text cannot
/// grant itself network authority: `open_url` accepts only an exact URL supplied by the user or
/// returned by this session's reviewed search/archive tools.
///
/// The call budgets live here rather than beside the tool handlers because a turn may run several
/// research agents concurrently. They share one session, so they must share one budget — otherwise
/// N parallel researchers each get a full allowance and the turn spends N times the intended cap.
/// Sharing the ledger is also what makes their citations interoperable: a URL authorized by one
/// researcher's search is citable by another's synthesis.
#[derive(Debug, Default)]
pub struct WebResearchSession {
    authorized_urls: Mutex<HashSet<String>>,
    opened_urls: Mutex<HashSet<String>>,
    non_citable_urls: Mutex<HashSet<String>>,
    public_web_closed: AtomicBool,
    search_calls: AtomicUsize,
    archive_calls: AtomicUsize,
    open_url_budget: Mutex<OpenUrlSessionBudget>,
}

impl WebResearchSession {
    pub fn new(user_prompt: &str) -> Self {
        let session = Self::default();
        session.authorize_user_text(user_prompt);
        session
    }

    /// Reserve one `internet_search` call against the session-wide cap. Returns false when the
    /// turn's search allowance is spent, whichever agent spent it.
    pub fn reserve_search_call(&self) -> bool {
        reserve_capped_call(&self.search_calls, MAX_SEARCH_CALLS_PER_SESSION)
    }

    /// Reserve one `archive_lookup` call against the session-wide cap.
    pub fn reserve_archive_call(&self) -> bool {
        reserve_capped_call(&self.archive_calls, MAX_ARCHIVE_CALLS_PER_SESSION)
    }

    /// Reserve a bounded slice of the session's `open_url` allowance for one unbatched call,
    /// returning the arguments to run with, or a ready-made refusal payload.
    pub fn prepare_open_url_call(&self, arguments: Value) -> Result<Value, String> {
        self.open_url_budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prepare_unbatched_call(arguments)
    }

    /// Open a batched round of `open_url` calls, for the rig loop which plans a whole round at once.
    pub fn begin_open_url_round(&self, planned_open_url_calls: usize) {
        self.open_url_budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_round(planned_open_url_calls);
    }

    /// Reserve one call of a batched round. Mirrors [`Self::prepare_open_url_call`] for the rig loop.
    pub fn prepare_open_url_round_call(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        self.open_url_budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prepare_call(tool_name, arguments)
    }

    /// How much of the session's web allowance is already spent. Surfaced so a synthesising agent
    /// can say "I stopped searching because the budget ran out" rather than implying the evidence
    /// was exhaustive.
    pub fn spend_summary(&self) -> Value {
        json!({
            "search_calls_used": self.search_calls.load(Ordering::Relaxed),
            "search_calls_limit": MAX_SEARCH_CALLS_PER_SESSION,
            "archive_calls_used": self.archive_calls.load(Ordering::Relaxed),
            "archive_calls_limit": MAX_ARCHIVE_CALLS_PER_SESSION,
        })
    }

    pub fn authorize_user_text(&self, text: &str) {
        for url in public_urls_in_user_text(text) {
            self.authorize_url(&url);
        }
    }

    pub fn register_tool_result(&self, tool_name: &str, result: &Value) {
        if result.get("status").and_then(Value::as_str) != Some("ok") {
            return;
        }
        match tool_name {
            INTERNET_SEARCH_TOOL => {
                if let Some(results) = result.get("results").and_then(Value::as_array) {
                    for result in results {
                        if let Some(url) = result.get("url").and_then(Value::as_str) {
                            self.authorize_url(url);
                        }
                    }
                }
            }
            ARCHIVE_LOOKUP_TOOL => {
                if result.get("available").and_then(Value::as_bool) == Some(true)
                    && let Some(url) = result.get("url").and_then(Value::as_str)
                {
                    self.authorize_url(url);
                    if result.get("research_lead_only").and_then(Value::as_bool) == Some(true)
                        || result
                            .get("citation_eligible_after_open")
                            .and_then(Value::as_bool)
                            == Some(false)
                        || result
                            .get("usable_as_evidence_for_requested_cutoff")
                            .and_then(Value::as_bool)
                            == Some(false)
                    {
                        self.mark_non_citable_url(url);
                    }
                }
            }
            OPEN_URL_TOOL => {
                if let Some(url) = result.get("url").and_then(Value::as_str) {
                    self.authorize_url(url);
                    let citation_eligible =
                        result.get("citation_eligible").and_then(Value::as_bool) == Some(true)
                            && !self.is_non_citable_url(url);
                    if citation_eligible {
                        if let Ok(url) = parse_public_url(url) {
                            self.opened_urls
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .insert(url.as_str().to_string());
                        }
                    } else {
                        self.mark_non_citable_url(url);
                    }
                }
            }
            _ => {}
        }
    }

    /// Register provenance and expose the cumulative, host-verified citation allowlist to SDK
    /// backends that do not have the core runner's final-synthesis interception point.
    pub fn register_and_decorate_tool_result(&self, tool_name: &str, result: &mut Value) {
        self.register_tool_result(tool_name, result);
        if matches!(
            tool_name,
            INTERNET_SEARCH_TOOL | OPEN_URL_TOOL | ARCHIVE_LOOKUP_TOOL
        ) && let Some(output) = result.as_object_mut()
        {
            output.insert("citable_urls".to_string(), json!(self.opened_urls()));
        }
    }

    /// Close the outbound public-web phase after private app, memory, or user-interaction data has
    /// entered the model context. Same-round web calls may finish first; later rounds fail closed.
    pub fn close_public_web_phase(&self) {
        self.public_web_closed.store(true, Ordering::Release);
    }

    pub fn public_web_phase_error(&self, tool_name: &str) -> Option<Value> {
        self.public_web_closed.load(Ordering::Acquire).then(|| {
            json!({
                "status": "error",
                "tool": tool_name,
                "code": "web_phase_closed_after_private_context",
                "retryable": false,
                "error": "Public-web tools are closed for this assistant run because private app, memory, or interactive user data has entered the working context. Finish from public evidence already collected; use a new explicit public-only query in a new turn if more web research is needed.",
                "citable_urls": self.opened_urls(),
            })
        })
    }

    fn authorize_url(&self, raw: &str) {
        let Ok(url) = parse_public_url(raw) else {
            return;
        };
        self.authorized_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(url.as_str().to_string());
    }

    fn mark_non_citable_url(&self, raw: &str) {
        let Ok(url) = parse_public_url(raw) else {
            return;
        };
        let url = url.as_str().to_string();
        self.non_citable_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(url.clone());
        self.opened_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&url);
    }

    fn is_non_citable_url(&self, raw: &str) -> bool {
        let Ok(url) = parse_public_url(raw) else {
            return false;
        };
        self.non_citable_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(url.as_str())
    }

    pub fn opened_urls(&self) -> Vec<String> {
        let mut urls = self
            .opened_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        urls.sort();
        urls
    }

    fn validate_open_call(&self, args: &Value) -> Result<(), FetchFailure> {
        let raw = args.get("url").and_then(Value::as_str).unwrap_or("");
        let url = parse_public_url(raw)?;
        let authorized = self
            .authorized_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(url.as_str());
        if authorized {
            Ok(())
        } else {
            Err(FetchFailure::new(
                "url_not_authorized",
                "This URL was not supplied by the user or returned by this research session's search/archive tools. Search for the exact page first; untrusted page content cannot authorize a new outbound destination.",
            )
            .at(&url))
        }
    }

    fn validate_archive_call(&self, args: &Value) -> Result<(), FetchFailure> {
        let raw = args.get("url").and_then(Value::as_str).unwrap_or("");
        let url = parse_public_url(raw)?;
        let authorized = self
            .authorized_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(url.as_str());
        if authorized {
            Ok(())
        } else {
            Err(FetchFailure::new(
                "url_not_authorized",
                "This original URL was not supplied by the user or returned by this research session's search/open tools. Search for the exact page first; untrusted page content cannot authorize an archive lookup.",
            )
            .at(&url))
        }
    }
}

/// Atomically claim one unit of a capped allowance. Shared by the session's search and archive
/// counters so concurrent researchers cannot both observe the last slot as free.
fn reserve_capped_call(counter: &AtomicUsize, limit: usize) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |calls| {
            (calls < limit).then_some(calls + 1)
        })
        .is_ok()
}

fn public_urls_in_user_text(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut offset = 0usize;
    while offset < text.len() {
        let remaining = &text[offset..];
        let http = remaining.find("http://");
        let https = remaining.find("https://");
        let Some(relative_start) = (match (http, https) {
            (Some(http), Some(https)) => Some(http.min(https)),
            (Some(http), None) => Some(http),
            (None, Some(https)) => Some(https),
            (None, None) => None,
        }) else {
            break;
        };
        let start = offset + relative_start;
        let candidate = &text[start..];
        let scanned_end = candidate
            .char_indices()
            .find_map(|(index, character)| {
                (index > 0
                    && (character.is_whitespace() || matches!(character, '<' | '>' | '"' | '\'')))
                .then_some(index)
            })
            .unwrap_or(candidate.len());
        let candidate = trim_user_url_candidate(&candidate[..scanned_end]);
        if !candidate.is_empty() {
            urls.push(candidate.to_string());
        }
        offset = start.saturating_add(scanned_end.max(1));
    }
    urls
}

fn trim_user_url_candidate(mut candidate: &str) -> &str {
    candidate = candidate.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    loop {
        let Some(last) = candidate.chars().last() else {
            return candidate;
        };
        let Some(opening) = (match last {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        }) else {
            return candidate;
        };
        let openings = candidate
            .chars()
            .filter(|character| *character == opening)
            .count();
        let closings = candidate
            .chars()
            .filter(|character| *character == last)
            .count();
        if closings <= openings {
            return candidate;
        }
        candidate = &candidate[..candidate.len() - last.len_utf8()];
        candidate = candidate.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    }
}

/// Conservative model-context and network budget shared by one assistant run. Callers reset the
/// round counters for each model response while the session counters survive follow-up rounds.
#[derive(Debug, Default)]
pub struct OpenUrlSessionBudget {
    session_calls: usize,
    session_chars_reserved: usize,
    round_calls: usize,
    round_chars_reserved: usize,
    round_call_limit: usize,
    round_char_limit: usize,
}

impl OpenUrlSessionBudget {
    pub(crate) fn begin_round(&mut self, planned_open_url_calls: usize) {
        self.round_calls = 0;
        self.round_chars_reserved = 0;
        self.round_call_limit = planned_open_url_calls
            .min(MAX_OPEN_URL_CALLS_PER_ROUND)
            .min(MAX_OPEN_URL_CALLS_PER_SESSION.saturating_sub(self.session_calls));
        self.round_char_limit = MAX_OPEN_URL_CHARS_PER_ROUND
            .min(MAX_OPEN_URL_CHARS_PER_SESSION.saturating_sub(self.session_chars_reserved));
    }

    /// Preserve non-web arguments and reserve a bounded slice of this round/session for open_url.
    /// The reservation uses requested maxima, so even fully parallel calls cannot overfill the
    /// next model turn if every page reaches its output limit.
    pub(crate) fn prepare_call(
        &mut self,
        tool_name: &str,
        mut arguments: Value,
    ) -> Result<Value, String> {
        if tool_name != OPEN_URL_TOOL {
            return Ok(arguments);
        }

        if self.session_calls >= MAX_OPEN_URL_CALLS_PER_SESSION {
            return Err(open_url_budget_error(
                "open_url_session_call_budget_exceeded",
                "This assistant run has reached its safe open_url call budget. Use the evidence already collected, or let the user continue research in a new turn.",
                false,
            ));
        }
        if self.round_calls >= self.round_call_limit {
            return Err(open_url_budget_error(
                "open_url_round_call_budget_exceeded",
                "This tool round has reached its safe open_url call budget. Digest the current pages before deciding whether another research round is needed.",
                true,
            ));
        }

        let round_chars_remaining = self
            .round_char_limit
            .saturating_sub(self.round_chars_reserved);
        let session_chars_remaining =
            MAX_OPEN_URL_CHARS_PER_SESSION.saturating_sub(self.session_chars_reserved);
        let remaining_slots = self.round_call_limit.saturating_sub(self.round_calls);
        let fair_share = round_chars_remaining
            .min(session_chars_remaining)
            .checked_div(remaining_slots)
            .unwrap_or(0)
            .min(MAX_OUTPUT_CHARS);
        if fair_share < 1_000 {
            return Err(open_url_budget_error(
                "open_url_output_budget_exceeded",
                "This assistant run has reached its safe fetched-text budget. Use the evidence already collected, or let the user continue research in a new turn.",
                false,
            ));
        }

        let requested = arguments
            .get("max_chars")
            .or_else(|| arguments.get("maxChars"))
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_CHARS as u64)
            .clamp(1_000, MAX_OUTPUT_CHARS as u64) as usize;
        let reserved = requested.min(fair_share);
        if let Some(object) = arguments.as_object_mut() {
            object.insert("max_chars".to_string(), json!(reserved));
        }
        self.round_calls += 1;
        self.session_calls += 1;
        self.round_chars_reserved += reserved;
        self.session_chars_reserved += reserved;
        Ok(arguments)
    }

    /// Apply the same session-wide cap when a provider SDK invokes handlers without exposing model
    /// round boundaries to the host. Reserving an equal share for each remaining call keeps one
    /// early page from consuming the entire research allowance.
    pub fn prepare_unbatched_call(&mut self, mut arguments: Value) -> Result<Value, String> {
        if self.session_calls >= MAX_OPEN_URL_CALLS_PER_SESSION {
            return Err(open_url_budget_error(
                "open_url_session_call_budget_exceeded",
                "This assistant run has reached its safe open_url call budget. Use the evidence already collected, or let the user continue research in a new turn.",
                false,
            ));
        }
        let remaining_calls = MAX_OPEN_URL_CALLS_PER_SESSION - self.session_calls;
        let remaining_chars =
            MAX_OPEN_URL_CHARS_PER_SESSION.saturating_sub(self.session_chars_reserved);
        let fair_share = remaining_chars
            .checked_div(remaining_calls)
            .unwrap_or(0)
            .min(MAX_OUTPUT_CHARS);
        if fair_share < 1_000 {
            return Err(open_url_budget_error(
                "open_url_output_budget_exceeded",
                "This assistant run has reached its safe fetched-text budget. Use the evidence already collected, or let the user continue research in a new turn.",
                false,
            ));
        }

        let requested = arguments
            .get("max_chars")
            .or_else(|| arguments.get("maxChars"))
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_CHARS as u64)
            .clamp(1_000, MAX_OUTPUT_CHARS as u64) as usize;
        let reserved = requested.min(fair_share);
        if let Some(object) = arguments.as_object_mut() {
            object.insert("max_chars".to_string(), json!(reserved));
        }
        self.session_calls += 1;
        self.session_chars_reserved += reserved;
        Ok(arguments)
    }
}

fn open_url_budget_error(code: &'static str, message: &'static str, retryable: bool) -> String {
    json!({
        "status": "error",
        "tool": OPEN_URL_TOOL,
        "code": code,
        "retryable": retryable,
        "error": message,
    })
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Html,
    Text,
}

#[derive(Debug)]
struct FetchFailure {
    code: &'static str,
    message: String,
    retryable: bool,
    url: Option<String>,
    http_status: Option<u16>,
    recovery_hints: Vec<String>,
}

impl FetchFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
            url: None,
            http_status: None,
            recovery_hints: Vec::new(),
        }
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    fn at(mut self, url: &Url) -> Self {
        self.url = Some(url.as_str().to_string());
        self
    }

    fn with_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    fn with_recovery_hints<I, S>(mut self, hints: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.recovery_hints = hints.into_iter().map(Into::into).collect();
        self
    }

    fn as_json(&self) -> Value {
        self.as_json_for(OPEN_URL_TOOL)
    }

    fn as_json_for(&self, tool: &str) -> Value {
        json!({
            "status": "error",
            "tool": tool,
            "code": self.code,
            "retryable": self.retryable,
            "url": self.url,
            "http_status": self.http_status,
            "error": self.message,
            "recovery_hints": self.recovery_hints,
        })
    }
}

#[derive(Debug)]
struct FetchSuccess {
    requested_url: String,
    final_url: Url,
    content_type: String,
    text: String,
    format: &'static str,
    bytes_read: usize,
    truncated: bool,
    redirects: Vec<Value>,
    title: String,
    find: Option<Value>,
}

/// Open one public textual URL without exposing a general-purpose network client to the model.
pub(crate) async fn run_open_url(args: &Value) -> Value {
    let raw_url = args.get("url").and_then(Value::as_str).unwrap_or("");
    let max_chars = args
        .get("max_chars")
        .or_else(|| args.get("maxChars"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_CHARS as u64)
        .clamp(1_000, MAX_OUTPUT_CHARS as u64) as usize;
    let find = match parse_find_query(args.get("find")) {
        Ok(find) => find,
        Err(error) => return error.as_json(),
    };

    match tokio::time::timeout(OPERATION_TIMEOUT, async {
        let _permit = OPEN_URL_CONCURRENCY.acquire().await.map_err(|_| {
            FetchFailure::new(
                "concurrency_unavailable",
                "The safe URL reader is temporarily unavailable.",
            )
            .retryable()
        })?;
        fetch_public_text(raw_url, max_chars, find.as_deref()).await
    })
    .await
    {
        Ok(Ok(result)) => success_json(result),
        Ok(Err(error)) => error.as_json(),
        Err(_) => FetchFailure::new(
            "timeout",
            "Opening the URL exceeded the 30 second safety deadline.",
        )
        .retryable()
        .as_json(),
    }
}

/// Session-gated counterpart used by FlowPilot tool adapters. The unrestricted function above is
/// retained for focused tests and trusted host callers; model-authored calls must use this path.
pub async fn run_open_url_for_session(args: &Value, session: &WebResearchSession) -> Value {
    if let Some(error) = session.public_web_phase_error(OPEN_URL_TOOL) {
        return error;
    }
    if let Err(error) = session.validate_open_call(args) {
        return error.as_json();
    }
    let requested_url = args.get("url").and_then(Value::as_str).unwrap_or("");
    apply_session_citation_policy(run_open_url(args).await, session, requested_url)
}

fn apply_session_citation_policy(
    mut result: Value,
    session: &WebResearchSession,
    requested_url: &str,
) -> Value {
    if result.get("status").and_then(Value::as_str) != Some("ok") {
        return result;
    }

    let final_url = result
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string);
    let is_non_citable = session.is_non_citable_url(requested_url)
        || final_url
            .as_deref()
            .is_some_and(|url| session.is_non_citable_url(url));
    if !is_non_citable {
        return result;
    }

    if let Some(final_url) = final_url {
        session.mark_non_citable_url(&final_url);
    }
    result["citation_eligible"] = json!(false);
    result["research_lead_only"] = json!(true);
    result["citation_ineligibility_reason"] = json!(
        "This opened snapshot is only an after/closest archive research lead and cannot support a citation for the requested historical cutoff."
    );
    if let Some(source) = result.get_mut("source").and_then(Value::as_object_mut) {
        source.remove("citation_markdown");
        source.insert("citation_eligible".to_string(), json!(false));
        source.insert("research_lead_only".to_string(), json!(true));
    }
    result
}

/// Session-gated archive lookup used by model tool adapters. Archive.org receives only an original
/// URL that was already authorized by the user or a reviewed discovery result.
pub async fn run_archive_lookup_for_session(args: &Value, session: &WebResearchSession) -> Value {
    if let Some(error) = session.public_web_phase_error(ARCHIVE_LOOKUP_TOOL) {
        return error;
    }
    if let Err(error) = session.validate_archive_call(args) {
        return error.as_json_for(ARCHIVE_LOOKUP_TOOL);
    }
    run_archive_lookup(args).await
}

/// Locate a Wayback capture without granting it evidentiary status. Timestamped requests first use
/// the fixed exact-URL CDX index to select the latest HTTP-200 capture at or before the cutoff;
/// untimestamped requests retain the Availability API's latest/closest behavior. Callers must still
/// use `open_url` to read a returned snapshot before relying on its contents.
pub(crate) async fn run_archive_lookup(args: &Value) -> Value {
    match tokio::time::timeout(OPERATION_TIMEOUT, async {
        let _permit = ARCHIVE_LOOKUP_CONCURRENCY.acquire().await.map_err(|_| {
            FetchFailure::new(
                "concurrency_unavailable",
                "The archive lookup service is temporarily unavailable.",
            )
            .retryable()
        })?;
        lookup_wayback_snapshot(args).await
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => error.as_json_for(ARCHIVE_LOOKUP_TOOL),
        Err(_) => FetchFailure::new(
            "timeout",
            "The Internet Archive lookup exceeded the 30 second safety deadline.",
        )
        .retryable()
        .as_json_for(ARCHIVE_LOOKUP_TOOL),
    }
}

async fn lookup_wayback_snapshot(args: &Value) -> Result<Value, FetchFailure> {
    let raw_original = args.get("url").and_then(Value::as_str).unwrap_or("");
    if raw_original.trim().is_empty() {
        return Err(FetchFailure::new(
            "invalid_url",
            "archive_lookup requires a non-empty absolute public URL.",
        ));
    }
    let original = parse_public_url(raw_original)?;
    let requested_timestamp = normalize_archive_timestamp(
        args.get("timestamp")
            .filter(|value| !value.is_null())
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    FetchFailure::new(
                        "invalid_timestamp",
                        "archive_lookup timestamp must be a string when provided.",
                    )
                })
            })
            .transpose()?,
    )?;

    if let Some(requested_timestamp) = requested_timestamp.as_deref() {
        let cdx_endpoint = wayback_cdx_url(&original, requested_timestamp)?;
        let cdx_payload = fetch_wayback_json(&cdx_endpoint, "CDX index", true).await?;
        if let Some(snapshot) =
            archive_cdx_lookup_json(&cdx_payload, &original, requested_timestamp.to_string())?
        {
            return Ok(snapshot);
        }

        // The Availability API deliberately remains only a clue-producing fallback here. Its
        // `closest` result can be after the cutoff and must never silently replace a missing
        // at-or-before capture.
        let endpoint = wayback_availability_url(&original, Some(requested_timestamp))?;
        let payload = fetch_wayback_json(&endpoint, "Availability API", false).await?;
        return archive_lookup_json(
            &payload,
            &original,
            Some(requested_timestamp.to_string()),
            true,
        );
    }

    let endpoint = wayback_availability_url(&original, None)?;
    let payload = fetch_wayback_json(&endpoint, "Availability API", false).await?;
    archive_lookup_json(&payload, &original, None, false)
}

async fn fetch_wayback_json(
    endpoint: &Url,
    service_name: &str,
    allow_text_plain: bool,
) -> Result<Value, FetchFailure> {
    let endpoint = endpoint.clone();
    let addresses = resolve_public_addresses(&endpoint).await?;
    let client = client_for_hop(&endpoint, &addresses)?;
    let response = client
        .get(endpoint.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                FetchFailure::new(
                    "request_timeout",
                    format!("The Internet Archive {service_name} request timed out."),
                )
            } else {
                FetchFailure::new(
                    "request_failed",
                    format!("Internet Archive {service_name} request failed: {error}"),
                )
            }
            .retryable()
            .at(&endpoint)
        })?;

    let status = response.status();
    if status.is_redirection() {
        return Err(FetchFailure::new(
            "redirect_not_allowed",
            format!("The fixed Internet Archive {service_name} endpoint attempted to redirect; the redirect was not followed."),
        )
        .with_status(status.as_u16())
        .at(&endpoint));
    }
    if !status.is_success() {
        return Err(FetchFailure::new(
            "http_error",
            format!("The Internet Archive {service_name} endpoint returned HTTP {status}."),
        )
        .with_status(status.as_u16())
        .retryable()
        .at(&endpoint));
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("identity"))
    {
        return Err(FetchFailure::new(
            "unsupported_content_encoding",
            "The Internet Archive endpoint ignored the identity encoding request.",
        )
        .at(&endpoint));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media_type != "application/json"
        && !media_type.ends_with("+json")
        && !(allow_text_plain && media_type == "text/plain")
    {
        return Err(FetchFailure::new(
            "unsupported_content_type",
            format!("The Internet Archive {service_name} endpoint did not return JSON."),
        )
        .at(&endpoint));
    }

    let (body, truncated) = read_bounded_body_with_limit(response, MAX_ARCHIVE_RESPONSE_BYTES)
        .await
        .map_err(|error| {
            FetchFailure::new(
                "body_read_failed",
                format!("Failed to read the Internet Archive response: {error}"),
            )
            .retryable()
            .at(&endpoint)
        })?;
    if truncated {
        return Err(FetchFailure::new(
            "response_too_large",
            format!(
                "The Internet Archive response exceeded the {MAX_ARCHIVE_RESPONSE_BYTES}-byte limit."
            ),
        )
        .at(&endpoint));
    }
    let payload = serde_json::from_slice::<Value>(&body).map_err(|_| {
        FetchFailure::new(
            "invalid_response",
            format!("The Internet Archive {service_name} endpoint returned invalid JSON."),
        )
        .at(&endpoint)
    })?;
    Ok(payload)
}

fn wayback_availability_url(original: &Url, timestamp: Option<&str>) -> Result<Url, FetchFailure> {
    let mut endpoint = Url::parse(WAYBACK_AVAILABILITY_ENDPOINT).map_err(|_| {
        FetchFailure::new(
            "request_failed",
            "The fixed Internet Archive availability endpoint is invalid.",
        )
    })?;
    {
        let mut query = endpoint.query_pairs_mut();
        query.append_pair("url", original.as_str());
        if let Some(timestamp) = timestamp {
            query.append_pair("timestamp", timestamp);
        }
    }
    Ok(endpoint)
}

fn wayback_cdx_url(original: &Url, requested_timestamp: &str) -> Result<Url, FetchFailure> {
    let mut endpoint = Url::parse(WAYBACK_CDX_ENDPOINT).map_err(|_| {
        FetchFailure::new(
            "request_failed",
            "The fixed Internet Archive CDX endpoint is invalid.",
        )
    })?;
    {
        let mut query = endpoint.query_pairs_mut();
        query.append_pair("url", original.as_str());
        query.append_pair("matchType", "exact");
        query.append_pair("output", "json");
        query.append_pair("gzip", "false");
        query.append_pair("fl", &WAYBACK_CDX_FIELDS.join(","));
        query.append_pair("filter", "statuscode:200");
        query.append_pair("to", requested_timestamp);
        query.append_pair("limit", "-1");
    }
    Ok(endpoint)
}

fn normalize_archive_timestamp(raw: Option<&str>) -> Result<Option<String>, FetchFailure> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Err(FetchFailure::new(
            "invalid_timestamp",
            "archive_lookup timestamp must not be empty when provided.",
        ));
    }

    let normalized = if value.chars().all(|character| character.is_ascii_digit()) {
        let date_time = match value.len() {
            4 => value
                .parse::<i32>()
                .ok()
                .and_then(|year| NaiveDate::from_ymd_opt(year, 1, 1))
                .and_then(|date| date.and_hms_opt(0, 0, 0)),
            6 => value[..4]
                .parse::<i32>()
                .ok()
                .zip(value[4..6].parse::<u32>().ok())
                .and_then(|(year, month)| NaiveDate::from_ymd_opt(year, month, 1))
                .and_then(|date| date.and_hms_opt(0, 0, 0)),
            8 => NaiveDate::parse_from_str(value, "%Y%m%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0)),
            14 => NaiveDateTime::parse_from_str(value, "%Y%m%d%H%M%S").ok(),
            _ => None,
        }
        .ok_or_else(invalid_archive_timestamp)?;
        date_time.format("%Y%m%d%H%M%S").to_string()
    } else {
        DateTime::parse_from_rfc3339(value)
            .map_err(|_| invalid_archive_timestamp())?
            .with_timezone(&Utc)
            .format("%Y%m%d%H%M%S")
            .to_string()
    };
    Ok(Some(normalized))
}

fn invalid_archive_timestamp() -> FetchFailure {
    FetchFailure::new(
        "invalid_timestamp",
        "archive_lookup timestamp must be YYYY, YYYYMM, YYYYMMDD, YYYYMMDDhhmmss, or RFC3339.",
    )
}

fn archive_lookup_json(
    payload: &Value,
    original: &Url,
    requested_timestamp: Option<String>,
    research_lead_only: bool,
) -> Result<Value, FetchFailure> {
    let selection_method = if research_lead_only {
        "availability_closest_fallback"
    } else {
        "availability_closest"
    };
    let archive_caveat = if research_lead_only {
        ARCHIVE_CLOSEST_LEAD_CAVEAT
    } else {
        ARCHIVE_CAVEAT
    };
    let closest = payload
        .get("archived_snapshots")
        .and_then(|snapshots| snapshots.get("closest"));
    if !closest
        .and_then(|closest| closest.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(json!({
            "status": "ok",
            "tool": ARCHIVE_LOOKUP_TOOL,
            "available": false,
            "original_url": original.as_str(),
            "requested_timestamp": requested_timestamp,
            "selection_method": selection_method,
            "research_lead_only": research_lead_only,
            "citation_eligible": false,
            "citation_eligible_after_open": false,
            "archive_caveat": archive_caveat,
            "untrusted_content": true,
        }));
    }
    let closest = closest.expect("available snapshot has closest metadata");
    let snapshot_status_is_ok = closest
        .get("status")
        .is_some_and(|status| status.as_str() == Some("200") || status.as_u64() == Some(200));
    if !snapshot_status_is_ok {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Internet Archive marked a non-200 snapshot as available.",
        ));
    }
    let capture_timestamp = closest
        .get("timestamp")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive response omitted the capture timestamp.",
            )
        })?;
    let capture_date_time = parse_wayback_capture_timestamp(capture_timestamp)?;
    let snapshot_url = closest.get("url").and_then(Value::as_str).ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Internet Archive response omitted the snapshot URL.",
        )
    })?;
    let snapshot_url = validate_wayback_snapshot_url(snapshot_url, capture_timestamp, original)?;
    archive_snapshot_json(
        original,
        requested_timestamp,
        capture_timestamp,
        capture_date_time,
        snapshot_url,
        selection_method,
        research_lead_only,
        None,
        None,
        archive_caveat,
    )
}

fn archive_cdx_lookup_json(
    payload: &Value,
    original: &Url,
    requested_timestamp: String,
) -> Result<Option<Value>, FetchFailure> {
    let rows = payload.as_array().ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Internet Archive CDX index returned a non-array response.",
        )
    })?;
    if rows.is_empty() {
        return Ok(None);
    }

    let header = rows[0].as_array().ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Internet Archive CDX index returned an invalid header row.",
        )
    })?;
    let expected_header = WAYBACK_CDX_FIELDS
        .iter()
        .map(|field| Value::String((*field).to_string()))
        .collect::<Vec<_>>();
    if header != &expected_header {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Internet Archive CDX index returned unexpected fields.",
        ));
    }
    if rows.len() == 1 {
        return Ok(None);
    }

    let requested_date_time = NaiveDateTime::parse_from_str(&requested_timestamp, "%Y%m%d%H%M%S")
        .map_err(|_| {
        FetchFailure::new(
            "invalid_timestamp",
            "The normalized archive timestamp was invalid.",
        )
    })?;
    let mut selected: Option<(String, NaiveDateTime, String, String)> = None;

    for row in &rows[1..] {
        let row = row.as_array().ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index returned an invalid capture row.",
            )
        })?;
        if row.len() != WAYBACK_CDX_FIELDS.len() {
            return Err(FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index returned an incomplete capture row.",
            ));
        }

        let capture_timestamp = row[0].as_str().ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index omitted a capture timestamp.",
            )
        })?;
        let capture_date_time = parse_wayback_capture_timestamp(capture_timestamp)?;
        if capture_date_time > requested_date_time {
            return Err(FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index returned a capture after the requested cutoff.",
            ));
        }

        let returned_original = row[1].as_str().ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index omitted the original URL.",
            )
        })?;
        let returned_original = parse_public_url(returned_original).map_err(|_| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index returned an invalid original URL.",
            )
        })?;
        if returned_original != *original {
            return Err(FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX capture did not match the requested original URL.",
            ));
        }

        let status_is_ok = row[2].as_str() == Some("200") || row[2].as_u64() == Some(200);
        if !status_is_ok {
            return Err(FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index returned a non-200 capture despite the fixed filter.",
            ));
        }
        let mime_type = row[3].as_str().ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index omitted the capture MIME type.",
            )
        })?;
        let digest = row[4].as_str().ok_or_else(|| {
            FetchFailure::new(
                "invalid_response",
                "The Internet Archive CDX index omitted the capture digest.",
            )
        })?;

        if selected
            .as_ref()
            .is_none_or(|(_, selected_at, _, _)| capture_date_time > *selected_at)
        {
            selected = Some((
                capture_timestamp.to_string(),
                capture_date_time,
                mime_type.to_string(),
                digest.to_string(),
            ));
        }
    }

    let Some((capture_timestamp, capture_date_time, mime_type, digest)) = selected else {
        return Ok(None);
    };
    let snapshot_url = build_wayback_snapshot_url(&capture_timestamp, original)?;
    archive_snapshot_json(
        original,
        Some(requested_timestamp),
        &capture_timestamp,
        capture_date_time,
        snapshot_url,
        "cdx_at_or_before",
        false,
        Some(&mime_type),
        Some(&digest),
        ARCHIVE_PRE_CUTOFF_CAVEAT,
    )
    .map(Some)
}

#[allow(clippy::too_many_arguments)]
fn archive_snapshot_json(
    original: &Url,
    requested_timestamp: Option<String>,
    capture_timestamp: &str,
    capture_date_time: NaiveDateTime,
    snapshot_url: Url,
    selection_method: &str,
    research_lead_only: bool,
    mime_type: Option<&str>,
    digest: Option<&str>,
    archive_caveat: &str,
) -> Result<Value, FetchFailure> {
    let snapshot_url_string = snapshot_url.as_str().to_string();
    let captured_at = capture_date_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let (capture_relation, capture_offset_seconds, is_at_or_before_requested) =
        archive_capture_relation(capture_timestamp, requested_timestamp.as_deref());
    let original_host = original.host_str().unwrap_or("web page");
    let title = format!("Internet Archive snapshot of {original_host} ({captured_at})");
    let mut output = json!({
        "status": "ok",
        "tool": ARCHIVE_LOOKUP_TOOL,
        "available": true,
        "original_url": original.as_str(),
        "requested_timestamp": requested_timestamp,
        "capture_timestamp": capture_timestamp,
        "captured_at": captured_at,
        "capture_relation_to_requested": capture_relation,
        "capture_offset_seconds": capture_offset_seconds,
        "is_at_or_before_requested": is_at_or_before_requested,
        "selection_method": selection_method,
        "research_lead_only": research_lead_only,
        "citation_eligible": false,
        "citation_eligible_after_open": !research_lead_only,
        "url": snapshot_url_string,
        "source": {
            "source_id": source_id_for_url(snapshot_url.as_str()),
            "kind": "archived_snapshot",
            "title": title,
            "url": snapshot_url.as_str(),
            "original_url": original.as_str(),
            "capture_timestamp": capture_timestamp,
            "selection_method": selection_method,
            "citation_eligible": false,
            "citation_eligible_after_open": !research_lead_only,
        },
        "archive_caveat": archive_caveat,
        "untrusted_content": true,
    });
    if let Some(mime_type) = mime_type {
        output["capture_mime_type"] = json!(mime_type);
        output["source"]["capture_mime_type"] = json!(mime_type);
    }
    if let Some(digest) = digest {
        output["capture_digest"] = json!(digest);
        output["source"]["capture_digest"] = json!(digest);
    }
    if research_lead_only {
        output["usable_as_evidence_for_requested_cutoff"] = json!(false);
        output["research_lead_reason"] = json!(
            "No qualifying exact-URL HTTP-200 capture at or before the requested time was found in CDX; this is only Wayback's closest Availability result."
        );
    }
    Ok(output)
}

fn parse_wayback_capture_timestamp(capture_timestamp: &str) -> Result<NaiveDateTime, FetchFailure> {
    if capture_timestamp.len() != 14
        || !capture_timestamp
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Internet Archive returned an invalid capture timestamp.",
        ));
    }
    NaiveDateTime::parse_from_str(capture_timestamp, "%Y%m%d%H%M%S").map_err(|_| {
        FetchFailure::new(
            "invalid_response",
            "The Internet Archive returned an invalid capture timestamp.",
        )
    })
}

fn build_wayback_snapshot_url(
    capture_timestamp: &str,
    original: &Url,
) -> Result<Url, FetchFailure> {
    let raw = format!(
        "https://web.archive.org/web/{capture_timestamp}/{}",
        original.as_str()
    );
    validate_wayback_snapshot_url(&raw, capture_timestamp, original)
}

fn archive_capture_relation(
    capture_timestamp: &str,
    requested_timestamp: Option<&str>,
) -> (Option<&'static str>, Option<i64>, Option<bool>) {
    let Some(requested_timestamp) = requested_timestamp else {
        return (None, None, None);
    };
    let Ok(capture) = NaiveDateTime::parse_from_str(capture_timestamp, "%Y%m%d%H%M%S") else {
        return (None, None, None);
    };
    let Ok(requested) = NaiveDateTime::parse_from_str(requested_timestamp, "%Y%m%d%H%M%S") else {
        return (None, None, None);
    };
    let offset = capture.signed_duration_since(requested).num_seconds();
    let relation = match offset.cmp(&0) {
        std::cmp::Ordering::Less => "before",
        std::cmp::Ordering::Equal => "exact",
        std::cmp::Ordering::Greater => "after",
    };
    (Some(relation), Some(offset), Some(offset <= 0))
}

fn validate_wayback_snapshot_url(
    raw: &str,
    capture_timestamp: &str,
    expected_original: &Url,
) -> Result<Url, FetchFailure> {
    let mut snapshot = Url::parse(raw).map_err(|_| {
        FetchFailure::new(
            "invalid_response",
            "The Internet Archive returned an invalid snapshot URL.",
        )
    })?;
    if !matches!(snapshot.scheme(), "http" | "https")
        || snapshot.host_str() != Some("web.archive.org")
        || !snapshot.username().is_empty()
        || snapshot.password().is_some()
        || snapshot.port().is_some()
    {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Internet Archive returned a snapshot URL outside the fixed HTTPS Wayback host.",
        ));
    }
    let replay_path = snapshot.path().strip_prefix("/web/").ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL did not use the expected replay path.",
        )
    })?;
    if replay_path.get(..14) != Some(capture_timestamp) {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL did not match the reported capture timestamp.",
        ));
    }
    let after_timestamp = replay_path.get(14..).ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL had an incomplete replay path.",
        )
    })?;
    let separator = after_timestamp.find('/').ok_or_else(|| {
        FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL omitted its original page URL.",
        )
    })?;
    let replay_modifier = &after_timestamp[..separator];
    if !matches!(replay_modifier, "" | "id_" | "if_" | "im_" | "js_" | "cs_") {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL used an unsupported replay modifier.",
        ));
    }
    let embedded_path = &after_timestamp[separator + 1..];
    let embedded_raw = match snapshot.query() {
        Some(query) => format!("{embedded_path}?{query}"),
        None => embedded_path.to_string(),
    };
    let embedded_original = Url::parse(&embedded_raw).map_err(|_| {
        FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL embedded an invalid original page URL.",
        )
    })?;
    if embedded_original != *expected_original {
        return Err(FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL did not match the requested original page URL.",
        ));
    }
    snapshot.set_scheme("https").map_err(|_| {
        FetchFailure::new(
            "invalid_response",
            "The Wayback snapshot URL could not be normalized to HTTPS.",
        )
    })?;
    snapshot.set_fragment(None);
    Ok(snapshot)
}

async fn fetch_public_text(
    raw_url: &str,
    max_chars: usize,
    find: Option<&str>,
) -> Result<FetchSuccess, FetchFailure> {
    let mut current = parse_public_url(raw_url)?;
    let requested_url = current.as_str().to_string();
    let mut seen = HashSet::from([requested_url.clone()]);
    let mut redirects = Vec::new();

    for hop in 0..=MAX_REDIRECTS {
        let addresses = resolve_public_addresses(&current).await?;
        let client = client_for_hop(&current, &addresses)?;
        let response = client
            .get(current.clone())
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,text/plain,text/markdown,application/json,application/xml,text/xml;q=0.9,*/*;q=0.1",
            )
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    FetchFailure::new("request_timeout", "The URL request timed out.")
                } else {
                    FetchFailure::new("request_failed", format!("URL request failed: {error}"))
                }
                .retryable()
                .at(&current)
            })?;

        let status = response.status();
        if is_followable_redirect(status.as_u16()) {
            if hop == MAX_REDIRECTS {
                return Err(FetchFailure::new(
                    "redirect_limit",
                    format!("The URL exceeded the limit of {MAX_REDIRECTS} redirects."),
                )
                .at(&current));
            }

            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    FetchFailure::new(
                        "redirect_invalid",
                        "The server returned a redirect without a valid Location header.",
                    )
                    .at(&current)
                })?;
            let joined = current.join(location).map_err(|_| {
                FetchFailure::new(
                    "redirect_invalid",
                    "The server returned an invalid redirect target.",
                )
                .at(&current)
            })?;
            let next = parse_public_url(joined.as_str()).map_err(|mut error| {
                error.code = "redirect_blocked";
                error
            })?;
            if current.scheme() == "https" && next.scheme() != "https" {
                return Err(FetchFailure::new(
                    "redirect_downgrade",
                    "An HTTPS page attempted to redirect to insecure HTTP.",
                )
                .at(&next));
            }
            if !seen.insert(next.as_str().to_string()) {
                return Err(
                    FetchFailure::new("redirect_loop", "The URL entered a redirect loop.")
                        .at(&next),
                );
            }
            redirects.push(json!({
                "status": status.as_u16(),
                "from": current.as_str(),
                "to": next.as_str(),
            }));
            current = next;
            continue;
        }

        if !status.is_success() {
            return Err(FetchFailure::new(
                "http_error",
                format!("The URL returned HTTP {status}."),
            )
            .with_status(status.as_u16())
            .at(&current));
        }

        if response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("attachment"))
        {
            return Err(FetchFailure::new(
                "attachment_not_allowed",
                "Download attachments are not supported; open a textual web page instead.",
            )
            .at(&current));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                !value.trim().is_empty() && !value.eq_ignore_ascii_case("identity")
            })
        {
            return Err(FetchFailure::new(
                "unsupported_content_encoding",
                "The server ignored the identity encoding request; compressed responses are rejected.",
            )
            .at(&current));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| {
                FetchFailure::new(
                    "unsupported_content_type",
                    "The response did not declare a textual Content-Type.",
                )
                .at(&current)
            })?;
        let kind = classify_content_type(&content_type).ok_or_else(|| {
            FetchFailure::new(
                "unsupported_content_type",
                format!(
                    "Unsupported Content-Type '{content_type}'; only textual pages are allowed."
                ),
            )
            .at(&current)
        })?;

        let (body, body_truncated) = read_bounded_body(response).await.map_err(|error| {
            FetchFailure::new(
                "body_read_failed",
                format!("Failed to read URL body: {error}"),
            )
            .retryable()
            .at(&current)
        })?;
        let decoded = String::from_utf8_lossy(&body).into_owned();
        let title = page_title(&decoded, &current, kind);
        let converted = match kind {
            TextKind::Html => {
                let conversion_permit = tokio::time::timeout(
                    DNS_TIMEOUT,
                    HTML_CONVERSION_CONCURRENCY.clone().acquire_owned(),
                )
                .await
                .map_err(|_| {
                    FetchFailure::new(
                        "conversion_busy",
                        "The bounded HTML conversion pool is busy.",
                    )
                    .retryable()
                    .at(&current)
                })?
                .map_err(|_| {
                    FetchFailure::new(
                        "conversion_unavailable",
                        "The bounded HTML conversion pool is unavailable.",
                    )
                    .retryable()
                    .at(&current)
                })?;
                tokio::task::spawn_blocking(move || {
                    // Keep the permit inside the blocking task. If the outer request times out,
                    // the non-cancellable conversion still occupies its slot until it truly ends.
                    let _conversion_permit = conversion_permit;
                    html_to_markdown(&decoded)
                })
                .await
                .map_err(|_| {
                    FetchFailure::new(
                        "conversion_failed",
                        "The isolated HTML conversion task did not complete.",
                    )
                    .at(&current)
                })?
                .map_err(|error| error.at(&current))?
            }
            TextKind::Text => decoded,
        };
        let converted = remove_control_characters(converted);
        ensure_sufficient_evidence_text(&converted, &current)?;
        let find = find.map(|query| find_matches(&converted, query, body_truncated));
        let (text, output_truncated) = truncate_chars(converted, max_chars);

        return Ok(FetchSuccess {
            requested_url,
            final_url: current,
            content_type,
            text,
            format: if kind == TextKind::Html {
                "markdown"
            } else {
                "text"
            },
            bytes_read: body.len(),
            truncated: body_truncated || output_truncated,
            redirects,
            title,
            find,
        });
    }

    unreachable!("redirect loop either returns a response or a redirect-limit error")
}

fn success_json(result: FetchSuccess) -> Value {
    let final_url = result.final_url.as_str().to_string();
    let source_id = source_id_for_url(&final_url);
    let citation_markdown = markdown_citation(&result.title, &final_url);
    json!({
        "status": "ok",
        "tool": OPEN_URL_TOOL,
        "requested_url": result.requested_url,
        "url": final_url,
        "citation_eligible": true,
        "source": {
            "source_id": source_id,
            "title": result.title,
            "url": result.final_url.as_str(),
            "content_type": result.content_type,
            "citation_markdown": citation_markdown,
            "citation_eligible": true,
        },
        "format": result.format,
        "content": result.text,
        "bytes_read": result.bytes_read,
        "truncated": result.truncated,
        "redirects": result.redirects,
        "find": result.find,
        "untrusted_content": true,
    })
}

fn parse_public_url(raw: &str) -> Result<Url, FetchFailure> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(FetchFailure::new(
            "invalid_url",
            "open_url requires a non-empty absolute URL.",
        ));
    }
    if raw.len() > MAX_URL_BYTES {
        return Err(FetchFailure::new(
            "invalid_url",
            format!("The URL exceeds the {MAX_URL_BYTES}-byte limit."),
        ));
    }

    let mut url = Url::parse(raw)
        .map_err(|_| FetchFailure::new("invalid_url", "The URL is not a valid absolute URL."))?;
    let default_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => {
            return Err(FetchFailure::new(
                "unsupported_scheme",
                "Only public http:// and https:// URLs are supported.",
            ));
        }
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchFailure::new(
            "credentials_not_allowed",
            "Credential-bearing URLs are not allowed.",
        ));
    }
    if url.port().is_some_and(|port| port != default_port) {
        return Err(FetchFailure::new(
            "port_not_allowed",
            "Only the default HTTP (80) and HTTPS (443) ports are allowed.",
        ));
    }
    if url.query_pairs().any(|(name, value)| {
        let name = name.to_ascii_lowercase().replace('-', "_");
        !value.is_empty()
            && matches!(
                name.as_str(),
                "access_token"
                    | "api_key"
                    | "apikey"
                    | "auth"
                    | "authorization"
                    | "credential"
                    | "key"
                    | "password"
                    | "secret"
                    | "sig"
                    | "signature"
                    | "token"
                    | "x_amz_credential"
                    | "x_amz_signature"
                    | "x_goog_credential"
                    | "x_goog_signature"
            )
    }) {
        return Err(FetchFailure::new(
            "sensitive_query_not_allowed",
            "URLs containing credential-, token-, key-, password-, or signature-like query parameters are not allowed.",
        ));
    }
    let normalized_domain = match url
        .host()
        .ok_or_else(|| FetchFailure::new("invalid_url", "The URL must include a host."))?
    {
        Host::Ipv4(address) => {
            ensure_public_ip(IpAddr::V4(address))?;
            None
        }
        Host::Ipv6(address) => {
            ensure_public_ip(IpAddr::V6(address))?;
            None
        }
        Host::Domain(domain) => {
            ensure_public_hostname(domain)?;
            Some(domain.trim_end_matches('.').to_string())
        }
    };
    if let Some(domain) = normalized_domain
        && url.host_str() != Some(domain.as_str())
    {
        url.set_host(Some(&domain)).map_err(|_| {
            FetchFailure::new("invalid_url", "The URL contains an invalid hostname.")
        })?;
    }
    url.set_fragment(None);
    Ok(url)
}

/// Apply the same URL, credential, query-secret, hostname, and size policy used by `open_url` to
/// untrusted discovery results, while excluding literal IP results until the user explicitly
/// supplies one and the fetch path performs its full address checks.
pub(crate) fn normalize_public_discovery_url(raw: &str) -> Option<String> {
    let url = parse_public_url(raw).ok()?;
    match url.host()? {
        Host::Domain(_) => Some(url.to_string()),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

fn ensure_public_hostname(host: &str) -> Result<(), FetchFailure> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let blocked = host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host == "home.arpa"
        || host.ends_with(".home.arpa")
        || host == "metadata.google.internal"
        || !host.contains('.');
    if blocked {
        return Err(FetchFailure::new(
            "blocked_address",
            "The URL host is not a public Internet hostname.",
        ));
    }
    Ok(())
}

async fn resolve_public_addresses(url: &Url) -> Result<Vec<SocketAddr>, FetchFailure> {
    let port = url.port_or_known_default().ok_or_else(|| {
        FetchFailure::new("invalid_url", "The URL does not have a known network port.").at(url)
    })?;
    let addresses = match url.host().expect("validated URL always has a host") {
        Host::Ipv4(address) => vec![SocketAddr::new(IpAddr::V4(address), port)],
        Host::Ipv6(address) => vec![SocketAddr::new(IpAddr::V6(address), port)],
        Host::Domain(host) => {
            let lookup = tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
                .await
                .map_err(|_| {
                    FetchFailure::new("dns_timeout", "DNS resolution timed out.")
                        .retryable()
                        .at(url)
                })?
                .map_err(|error| {
                    FetchFailure::new("dns_failed", format!("DNS resolution failed: {error}"))
                        .retryable()
                        .at(url)
                })?;
            lookup.collect::<Vec<_>>()
        }
    };
    validate_resolved_addresses(addresses).map_err(|error| error.at(url))
}

fn validate_resolved_addresses(
    mut addresses: Vec<SocketAddr>,
) -> Result<Vec<SocketAddr>, FetchFailure> {
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(
            FetchFailure::new("dns_failed", "DNS resolution returned no addresses.").retryable(),
        );
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(FetchFailure::new(
            "blocked_address",
            "The URL resolved to a local, private, or special-use address.",
        ));
    }
    Ok(addresses)
}

fn client_for_hop(url: &Url, addresses: &[SocketAddr]) -> Result<reqwest::Client, FetchFailure> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(DNS_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .user_agent("FlowPilot-SafeWeb/1.0");
    if let Some(Host::Domain(host)) = url.host() {
        builder = builder.resolve_to_addrs(host, addresses);
    }
    builder.build().map_err(|error| {
        FetchFailure::new(
            "request_failed",
            format!("Failed to create the safe URL client: {error}"),
        )
        .at(url)
    })
}

async fn read_bounded_body(response: reqwest::Response) -> Result<(Vec<u8>, bool), reqwest::Error> {
    read_bounded_body_with_limit(response, MAX_BODY_BYTES).await
}

async fn read_bounded_body_with_limit(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), reqwest::Error> {
    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(max_bytes as u64) as usize);
    let mut truncated = response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64);
    while let Some(chunk) = response.chunk().await? {
        let remaining = max_bytes.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
        if body.len() == max_bytes {
            truncated = true;
            break;
        }
    }
    Ok((body, truncated))
}

fn classify_content_type(value: &str) -> Option<TextKind> {
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match media_type.as_str() {
        "text/html" | "application/xhtml+xml" => Some(TextKind::Html),
        "text/event-stream" => None,
        value
            if value.starts_with("text/")
                || value == "application/json"
                || value.ends_with("+json")
                || value == "application/xml"
                || value.ends_with("+xml") =>
        {
            Some(TextKind::Text)
        }
        _ => None,
    }
}

fn html_to_markdown(html: &str) -> Result<String, FetchFailure> {
    let converter = HtmlToMarkdownBuilder::new()
        .skip_tags(vec![
            "head", "script", "style", "noscript", "iframe", "svg", "canvas", "template", "form",
            "button", "nav", "footer",
        ])
        .build();
    converter.convert(html).map_err(|error| {
        FetchFailure::new(
            "conversion_failed",
            format!("Could not convert the HTML page to safe text: {error}"),
        )
    })
}

fn ensure_sufficient_evidence_text(value: &str, url: &Url) -> Result<(), FetchFailure> {
    let evidence_chars = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    if evidence_chars >= MIN_EVIDENCE_NON_WHITESPACE_CHARS {
        return Ok(());
    }
    Err(FetchFailure::new(
        "insufficient_text_content",
        "The page returned too little readable text to use as evidence; it may be an empty JavaScript shell.",
    )
    .with_recovery_hints([
        "Search for the exact document title, DOI, report number, or release identifier.",
        "Prefer an official HTML, raw-text, print, changelog, filing, or repository version.",
        "If the question is historical and the live page changed or disappeared, try archive_lookup.",
    ])
    .at(url))
}

fn page_title(text: &str, url: &Url, kind: TextKind) -> String {
    if kind == TextKind::Html {
        let lower = text.to_ascii_lowercase();
        if let Some(open) = lower.find("<title")
            && let Some(open_end) = lower[open..].find('>').map(|index| open + index + 1)
            && let Some(close) = lower[open_end..]
                .find("</title>")
                .map(|index| open_end + index)
        {
            let raw_title = &text[open_end..close];
            if let Ok(title) = html_to_markdown(raw_title) {
                let title = remove_control_characters(collapse_whitespace(&title));
                if !title.is_empty() {
                    return truncate_chars(title, 200).0;
                }
            }
        }
    }
    url.host_str().unwrap_or("Web source").to_string()
}

fn remove_control_characters(value: String) -> String {
    value
        .chars()
        .filter(|character| {
            (*character == '\n' || *character == '\t' || !character.is_control())
                && !matches!(
                    *character,
                    '\u{200b}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
        })
        .collect()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_find_query(value: Option<&Value>) -> Result<Option<String>, FetchFailure> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let query = value.as_str().ok_or_else(|| {
        FetchFailure::new(
            "invalid_find",
            "open_url find must be a string when provided.",
        )
    })?;
    let query = query.trim();
    if query.is_empty() {
        return Err(FetchFailure::new(
            "invalid_find",
            "open_url find must not be empty when provided.",
        ));
    }
    if query.chars().count() > MAX_FIND_QUERY_CHARS {
        return Err(FetchFailure::new(
            "invalid_find",
            format!("open_url find exceeds the {MAX_FIND_QUERY_CHARS}-character limit."),
        ));
    }
    if query.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{200b}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{feff}'
            )
    }) {
        return Err(FetchFailure::new(
            "invalid_find",
            "open_url find contains unsupported control characters.",
        ));
    }
    Ok(Some(query.to_string()))
}

#[derive(Debug)]
struct FoldOrigin {
    folded_start: usize,
    original_start: usize,
    original_end: usize,
}

fn lowercase_with_origins(value: &str) -> (String, Vec<FoldOrigin>) {
    let mut folded = String::with_capacity(value.len());
    let mut origins = Vec::with_capacity(value.chars().count());
    let mut characters = value.char_indices().peekable();
    while let Some((original_start, character)) = characters.next() {
        let original_end = characters
            .peek()
            .map(|(index, _)| *index)
            .unwrap_or(value.len());
        for lowercase in character.to_lowercase() {
            origins.push(FoldOrigin {
                folded_start: folded.len(),
                original_start,
                original_end,
            });
            folded.push(lowercase);
        }
    }
    (folded, origins)
}

fn char_boundary_before(value: &str, byte_index: usize, count: usize) -> usize {
    let mut boundary = byte_index.min(value.len());
    for _ in 0..count {
        let Some((previous, _)) = value[..boundary].char_indices().next_back() else {
            return 0;
        };
        boundary = previous;
    }
    boundary
}

fn char_boundary_after(value: &str, byte_index: usize, count: usize) -> usize {
    value[byte_index.min(value.len())..]
        .char_indices()
        .nth(count)
        .map(|(offset, _)| byte_index + offset)
        .unwrap_or(value.len())
}

/// Locate literal, case-insensitive matches in the complete converted response body. The caller
/// computes this before applying the normal prefix truncation, so a targeted find can surface text
/// near the end of a long page without increasing the general page-output budget.
fn find_matches(value: &str, query: &str, source_body_truncated: bool) -> Value {
    let (folded_value, origins) = lowercase_with_origins(value);
    let folded_query = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let mut match_count = 0usize;
    let mut matches = Vec::new();

    for (folded_start, matched) in folded_value.match_indices(&folded_query) {
        let Ok(origin_start_index) =
            origins.binary_search_by_key(&folded_start, |origin| origin.folded_start)
        else {
            continue;
        };
        let folded_end = folded_start + matched.len();
        let Some(origin_end_index) = origins
            .partition_point(|origin| origin.folded_start < folded_end)
            .checked_sub(1)
        else {
            continue;
        };
        let original_start = origins[origin_start_index].original_start;
        let original_end = origins[origin_end_index].original_end;
        match_count += 1;

        if matches.len() >= MAX_FIND_MATCHES {
            continue;
        }
        let excerpt_start = char_boundary_before(value, original_start, FIND_CONTEXT_CHARS);
        let excerpt_end = char_boundary_after(value, original_end, FIND_CONTEXT_CHARS);
        matches.push(json!({
            "match_index": match_count,
            "start_char": value[..original_start].chars().count(),
            "end_char": value[..original_end].chars().count(),
            "excerpt_start_char": value[..excerpt_start].chars().count(),
            "excerpt_end_char": value[..excerpt_end].chars().count(),
            "excerpt": &value[excerpt_start..excerpt_end],
            "excerpt_truncated_before": excerpt_start > 0,
            "excerpt_truncated_after": excerpt_end < value.len(),
        }));
    }

    json!({
        "query": query,
        "case_sensitive": false,
        "match_count": match_count,
        "returned_match_count": matches.len(),
        "matches_truncated": match_count > matches.len(),
        "searched_chars": value.chars().count(),
        "searched_full_converted_text": true,
        "source_body_truncated": source_body_truncated,
        "matches": matches,
    })
}

fn truncate_chars(mut value: String, max_chars: usize) -> (String, bool) {
    let Some((byte_index, _)) = value.char_indices().nth(max_chars) else {
        return (value, false);
    };
    value.truncate(byte_index);
    value.push_str("\n\n[Content truncated by open_url]");
    (value, true)
}

pub fn source_id_for_url(url: &str) -> String {
    let digest = blake3::hash(url.as_bytes()).to_hex().to_string();
    format!("src_{}", &digest[..16])
}

fn markdown_citation(title: &str, url: &str) -> String {
    let title = title
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('~', "\\~");
    if url.contains('(') || url.contains(')') {
        format!("[{title}](<{url}>)")
    } else {
        format!("[{title}]({url})")
    }
}

fn is_followable_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn ensure_public_ip(address: IpAddr) -> Result<(), FetchFailure> {
    if is_public_ip(address) {
        Ok(())
    } else {
        Err(FetchFailure::new(
            "blocked_address",
            "The URL targets a local, private, or special-use address.",
        ))
    }
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    ![
        ("0.0.0.0", 8),
        ("10.0.0.0", 8),
        ("100.64.0.0", 10),
        ("127.0.0.0", 8),
        // Azure's host virtual IP is reachable from guests despite living outside RFC1918.
        ("168.63.129.16", 32),
        ("169.254.0.0", 16),
        ("172.16.0.0", 12),
        ("192.0.0.0", 24),
        ("192.0.2.0", 24),
        ("192.88.99.0", 24),
        ("192.168.0.0", 16),
        ("198.18.0.0", 15),
        ("198.51.100.0", 24),
        ("203.0.113.0", 24),
        ("224.0.0.0", 4),
        ("240.0.0.0", 4),
    ]
    .into_iter()
    .any(|(network, prefix)| ipv4_in_cidr(value, network.parse().unwrap(), prefix))
}

fn ipv4_in_cidr(value: u32, network: Ipv4Addr, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    value & mask == u32::from(network) & mask
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let value = u128::from(address);
    // Start with globally routable unicast space, then remove special allocations inside it.
    ipv6_in_cidr(value, "2000::".parse().unwrap(), 3)
        && ![
            ("2001::", 23),
            ("2001:db8::", 32),
            ("2002::", 16),
            ("3fff::", 20),
        ]
        .into_iter()
        .any(|(network, prefix)| ipv6_in_cidr(value, network.parse().unwrap(), prefix))
}

fn ipv6_in_cidr(value: u128, network: Ipv6Addr, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    value & mask == u128::from(network) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_session_allows_only_user_or_tool_provenance_urls() {
        let session = WebResearchSession::new(
            "Compare [the user source](https://example.com/report?q=1) with current guidance.",
        );
        assert!(
            session
                .validate_open_call(&json!({ "url": "https://example.com/report?q=1" }))
                .is_ok()
        );
        assert_eq!(
            session
                .validate_open_call(&json!({ "url": "https://example.com/report?q=secret" }))
                .unwrap_err()
                .code,
            "url_not_authorized"
        );

        session.register_tool_result(
            INTERNET_SEARCH_TOOL,
            &json!({
                "status": "ok",
                "results": [{ "url": "https://official.example/source" }]
            }),
        );
        assert!(
            session
                .validate_open_call(&json!({ "url": "https://official.example/source" }))
                .is_ok()
        );
        session.register_tool_result(
            OPEN_URL_TOOL,
            &json!({
                "status": "ok",
                "url": "https://official.example/canonical",
                "citation_eligible": true
            }),
        );
        assert_eq!(
            session.opened_urls(),
            vec!["https://official.example/canonical".to_string()]
        );
        assert!(
            session
                .validate_open_call(&json!({ "url": "https://official.example/canonical" }))
                .is_ok()
        );
    }

    #[test]
    fn user_url_discovery_preserves_balanced_parentheses_and_exact_queries() {
        let urls = public_urls_in_user_text(
            "Read [the entry](https://en.wikipedia.org/wiki/Foo_(bar)?oldid=123) and (https://example.com/report_(final)?view=print).",
        );
        assert_eq!(
            urls,
            vec![
                "https://en.wikipedia.org/wiki/Foo_(bar)?oldid=123".to_string(),
                "https://example.com/report_(final)?view=print".to_string(),
            ]
        );

        let session = WebResearchSession::new(
            "Open (https://en.wikipedia.org/wiki/Foo_(bar)?oldid=123) exactly.",
        );
        assert!(
            session
                .validate_open_call(&json!({
                    "url": "https://en.wikipedia.org/wiki/Foo_(bar)?oldid=123"
                }))
                .is_ok()
        );
    }

    #[test]
    fn tool_results_expose_only_cumulative_opened_urls_and_private_phase_closes_web() {
        let session = WebResearchSession::new("");
        let mut search = json!({
            "status": "ok",
            "tool": INTERNET_SEARCH_TOOL,
            "results": [{ "url": "https://search.example/lead" }],
            "citation_eligible": false,
        });
        session.register_and_decorate_tool_result(INTERNET_SEARCH_TOOL, &mut search);
        assert_eq!(search["citable_urls"], json!([]));

        for url in ["https://z.example/source", "https://a.example/source"] {
            let mut opened = json!({
                "status": "ok",
                "tool": OPEN_URL_TOOL,
                "url": url,
                "citation_eligible": true,
            });
            session.register_and_decorate_tool_result(OPEN_URL_TOOL, &mut opened);
        }
        assert_eq!(
            session.opened_urls(),
            vec![
                "https://a.example/source".to_string(),
                "https://z.example/source".to_string(),
            ]
        );

        assert!(
            session
                .public_web_phase_error(INTERNET_SEARCH_TOOL)
                .is_none()
        );
        session.close_public_web_phase();
        let error = session
            .public_web_phase_error(INTERNET_SEARCH_TOOL)
            .expect("private context closes later web calls");
        assert_eq!(error["code"], "web_phase_closed_after_private_context");
        assert_eq!(
            error["citable_urls"],
            json!(["https://a.example/source", "https://z.example/source"])
        );
    }

    #[test]
    fn archive_discovery_authorizes_only_the_returned_snapshot() {
        let session = WebResearchSession::new("");
        session.register_tool_result(
            ARCHIVE_LOOKUP_TOOL,
            &json!({
                "status": "ok",
                "available": true,
                "url": "https://web.archive.org/web/20240101000000/https://example.com/old"
            }),
        );
        assert!(
            session
                .validate_open_call(&json!({
                    "url": "https://web.archive.org/web/20240101000000/https://example.com/old"
                }))
                .is_ok()
        );
        assert!(
            session
                .validate_open_call(&json!({
                    "url": "https://web.archive.org/web/20240101000000/https://example.com/other"
                }))
                .is_err()
        );
    }

    #[test]
    fn pre_cutoff_archive_becomes_citable_only_after_open() {
        let original = parse_public_url("https://example.com/old").unwrap();
        let cutoff = "20240201000000".to_string();
        let archive_result = archive_cdx_lookup_json(
            &json!([
                ["timestamp", "original", "statuscode", "mimetype", "digest"],
                [
                    "20240131000000",
                    "https://example.com/old",
                    "200",
                    "text/html",
                    "ABC"
                ]
            ]),
            &original,
            cutoff,
        )
        .unwrap()
        .unwrap();
        let snapshot = archive_result["url"].as_str().unwrap();
        assert_eq!(archive_result["research_lead_only"], false);
        assert_eq!(archive_result["citation_eligible"], false);
        assert_eq!(archive_result["citation_eligible_after_open"], true);

        let session = WebResearchSession::new("");
        session.register_tool_result(ARCHIVE_LOOKUP_TOOL, &archive_result);
        assert!(
            session
                .validate_open_call(&json!({ "url": snapshot }))
                .is_ok()
        );

        let opened = apply_session_citation_policy(
            json!({
                "status": "ok",
                "tool": OPEN_URL_TOOL,
                "url": snapshot,
                "citation_eligible": true,
                "source": {
                    "url": snapshot,
                    "citation_markdown": format!("[Archived page]({snapshot})"),
                    "citation_eligible": true
                },
                "content": "Verified pre-cutoff page text"
            }),
            &session,
            snapshot,
        );
        assert_eq!(opened["citation_eligible"], true);
        assert!(opened["source"]["citation_markdown"].is_string());

        session.register_tool_result(OPEN_URL_TOOL, &opened);
        assert_eq!(session.opened_urls(), vec![snapshot.to_string()]);
    }

    #[test]
    fn after_cutoff_archive_remains_inspectable_but_never_citable() {
        let original = parse_public_url("https://example.com/old").unwrap();
        let archive_result = archive_lookup_json(
            &json!({
                "archived_snapshots": {
                    "closest": {
                        "available": true,
                        "status": "200",
                        "timestamp": "20240203040506",
                        "url": "https://web.archive.org/web/20240203040506/https://example.com/old"
                    }
                }
            }),
            &original,
            Some("20240201000000".to_string()),
            true,
        )
        .unwrap();
        let snapshot = archive_result["url"].as_str().unwrap();
        let final_snapshot =
            "https://web.archive.org/web/20240203040506id_/https://example.com/old";
        assert_eq!(archive_result["research_lead_only"], true);
        assert_eq!(archive_result["citation_eligible_after_open"], false);

        let session = WebResearchSession::new("");
        session.register_tool_result(ARCHIVE_LOOKUP_TOOL, &archive_result);
        assert!(
            session
                .validate_open_call(&json!({ "url": snapshot }))
                .is_ok(),
            "a lead-only snapshot must remain inspectable"
        );

        let opened = apply_session_citation_policy(
            json!({
                "status": "ok",
                "tool": OPEN_URL_TOOL,
                "url": final_snapshot,
                "citation_eligible": true,
                "source": {
                    "url": final_snapshot,
                    "citation_markdown": format!("[Archived page]({final_snapshot})"),
                    "citation_eligible": true
                },
                "content": "Useful only as a lead to better evidence"
            }),
            &session,
            snapshot,
        );
        assert_eq!(
            opened["content"],
            "Useful only as a lead to better evidence"
        );
        assert_eq!(opened["research_lead_only"], true);
        assert_eq!(opened["citation_eligible"], false);
        assert_eq!(opened["source"]["citation_eligible"], false);
        assert!(opened["source"].get("citation_markdown").is_none());
        assert!(opened["citation_ineligibility_reason"].is_string());

        session.register_tool_result(OPEN_URL_TOOL, &opened);
        assert!(session.opened_urls().is_empty());
        assert!(
            session
                .validate_open_call(&json!({ "url": final_snapshot }))
                .is_ok(),
            "a revalidated final URL remains inspectable"
        );

        session.register_tool_result(
            OPEN_URL_TOOL,
            &json!({
                "status": "ok",
                "url": final_snapshot,
                "citation_eligible": true
            }),
        );
        assert!(
            session.opened_urls().is_empty(),
            "lead-only provenance must remain sticky for the session"
        );
    }

    #[test]
    fn session_budget_caps_parallel_calls_and_aggregate_text() {
        let mut budget = OpenUrlSessionBudget::default();
        budget.begin_round(5);

        for _ in 0..4 {
            let prepared = budget
                .prepare_call(
                    OPEN_URL_TOOL,
                    json!({ "url": "https://example.com", "max_chars": 40_000 }),
                )
                .unwrap();
            assert_eq!(prepared["max_chars"], 15_000);
        }
        let round_error = budget
            .prepare_call(OPEN_URL_TOOL, json!({ "url": "https://example.com/fifth" }))
            .unwrap_err();
        assert!(round_error.contains("open_url_round_call_budget_exceeded"));

        budget.begin_round(4);
        for _ in 0..4 {
            let prepared = budget
                .prepare_call(
                    OPEN_URL_TOOL,
                    json!({ "url": "https://example.com", "max_chars": 10_000 }),
                )
                .unwrap();
            assert_eq!(prepared["max_chars"], 10_000);
        }

        budget.begin_round(4);
        for _ in 0..2 {
            let prepared = budget
                .prepare_call(
                    OPEN_URL_TOOL,
                    json!({ "url": "https://example.com", "max_chars": 40_000 }),
                )
                .unwrap();
            assert_eq!(prepared["max_chars"], 10_000);
        }
        let session_error = budget
            .prepare_call(
                OPEN_URL_TOOL,
                json!({ "url": "https://example.com/eleventh" }),
            )
            .unwrap_err();
        assert!(session_error.contains("open_url_session_call_budget_exceeded"));

        let untouched = json!({ "query": "Flow-Like" });
        assert_eq!(
            budget
                .prepare_call("internet_search", untouched.clone())
                .unwrap(),
            untouched
        );

        let mut unbatched = OpenUrlSessionBudget::default();
        for _ in 0..MAX_OPEN_URL_CALLS_PER_SESSION {
            let prepared = unbatched
                .prepare_unbatched_call(json!({
                    "url": "https://example.com",
                    "max_chars": 40_000
                }))
                .unwrap();
            assert_eq!(prepared["max_chars"], 12_000);
        }
        assert!(
            unbatched
                .prepare_unbatched_call(json!({ "url": "https://example.com/extra" }))
                .unwrap_err()
                .contains("open_url_session_call_budget_exceeded")
        );
    }

    #[test]
    fn url_validation_accepts_only_public_default_port_http_urls() {
        for raw in [
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "https://localhost/path",
            "https://service.internal/path",
            "https://example.com:8443/path",
            "https://user:secret@example.com/path",
            "https://example.com/path?access_token=secret",
            "https://example.com/path?X-Amz-Signature=secret",
            "file:///etc/passwd",
        ] {
            assert!(parse_public_url(raw).is_err(), "must reject {raw}");
        }

        let parsed = parse_public_url("https://example.com:443/path?q=public#section").unwrap();
        assert_eq!(parsed.as_str(), "https://example.com/path?q=public");
        let trailing_dot = parse_public_url("https://example.com./path").unwrap();
        assert_eq!(trailing_dot.as_str(), "https://example.com/path");
    }

    #[test]
    fn ip_policy_rejects_private_special_and_mixed_dns_answers() {
        for raw in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "168.63.129.16",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "198.18.0.1",
            "203.0.113.5",
            "224.0.0.1",
        ] {
            assert!(!is_public_ip(IpAddr::V4(raw.parse().unwrap())), "{raw}");
        }
        for raw in ["::1", "fe80::1", "fc00::1", "2001:db8::1", "2002::1"] {
            assert!(!is_public_ip(IpAddr::V6(raw.parse().unwrap())), "{raw}");
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));

        let mixed = vec![
            "93.184.216.34:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
        ];
        assert!(validate_resolved_addresses(mixed).is_err());
    }

    #[test]
    fn textual_content_types_are_explicitly_allowlisted() {
        assert_eq!(
            classify_content_type("text/html; charset=utf-8"),
            Some(TextKind::Html)
        );
        assert_eq!(
            classify_content_type("application/problem+json"),
            Some(TextKind::Text)
        );
        assert_eq!(
            classify_content_type("application/xml"),
            Some(TextKind::Text)
        );
        assert_eq!(classify_content_type("text/event-stream"), None);
        assert_eq!(classify_content_type("application/pdf"), None);
        assert_eq!(classify_content_type("application/octet-stream"), None);
    }

    #[test]
    fn html_conversion_drops_active_regions_and_keeps_evidence() {
        let html = r#"<html><head><title>Official &amp; Current</title><script>steal()</script></head>
            <body><nav>Noise</nav><h1>Result</h1><p>Supported <a href="https://example.com/source">fact</a>.</p>
            <form>Ignore me</form><footer>More noise</footer></body></html>"#;
        let markdown = html_to_markdown(html).unwrap();
        assert!(markdown.contains("# Result"));
        assert!(markdown.contains("[fact](https://example.com/source)"));
        assert!(!markdown.contains("steal"));
        assert!(!markdown.contains("Ignore me"));
        assert!(!markdown.contains("More noise"));
        assert_eq!(
            page_title(
                html,
                &Url::parse("https://example.com").unwrap(),
                TextKind::Html
            ),
            "Official & Current"
        );
    }

    #[test]
    fn empty_shells_are_not_evidence_and_citation_titles_cannot_spoof_direction() {
        let url = Url::parse("https://example.com/article").unwrap();
        for text in ["", "   \n\t", "Loading..."] {
            let error = ensure_sufficient_evidence_text(text, &url).unwrap_err();
            assert_eq!(error.code, "insufficient_text_content");
            assert_eq!(error.recovery_hints.len(), 3);
        }
        assert!(ensure_sufficient_evidence_text("Official release 1.2.", &url).is_ok());

        let html = "<html><head><title>Official\u{202e} fake\u{2066}\u{200b}</title></head><body>Enough official evidence.</body></html>";
        let title = page_title(html, &url, TextKind::Html);
        assert_eq!(title, "Official fake");
        let citation = markdown_citation(&title, url.as_str());
        assert!(!citation.contains('\u{202e}'));
        assert!(!citation.contains('\u{2066}'));
        assert!(!citation.contains('\u{200b}'));
    }

    #[test]
    fn find_searches_full_converted_text_before_prefix_truncation() {
        let full_text = format!(
            "{}The Requested Phrase appears near the end.{}",
            "prefix ".repeat(400),
            " Requested Phrase".repeat(10)
        );
        let find = find_matches(&full_text, "requested phrase", false);
        let (prefix, truncated) = truncate_chars(full_text, 1_000);

        assert!(truncated);
        assert!(!prefix.to_ascii_lowercase().contains("requested phrase"));
        assert_eq!(find["case_sensitive"], false);
        assert_eq!(find["match_count"], 11);
        assert_eq!(find["returned_match_count"], MAX_FIND_MATCHES);
        assert_eq!(find["matches_truncated"], true);
        assert!(
            find["matches"][0]["excerpt"]
                .as_str()
                .is_some_and(|excerpt| excerpt.contains("Requested Phrase"))
        );
        assert!(
            find["matches"][0]["start_char"].as_u64().unwrap() > 1_000,
            "the match metadata must address the untruncated converted text"
        );
    }

    #[test]
    fn find_validation_is_bounded_and_rejects_control_text() {
        assert_eq!(
            parse_find_query(Some(&json!("  Needle  "))).unwrap(),
            Some("Needle".to_string())
        );
        for invalid in [json!(""), json!("line\nbreak"), json!(42)] {
            assert!(parse_find_query(Some(&invalid)).is_err());
        }
        let too_long = "x".repeat(MAX_FIND_QUERY_CHARS + 1);
        assert!(parse_find_query(Some(&json!(too_long))).is_err());

        let unicode = find_matches("İstanbul and ÄRGER", "ärger", false);
        assert_eq!(unicode["match_count"], 1);
    }

    #[test]
    fn archive_timestamps_normalize_to_wayback_utc_format() {
        for (input, expected) in [
            ("2024", "20240101000000"),
            ("202402", "20240201000000"),
            ("20240229", "20240229000000"),
            ("20240229010203", "20240229010203"),
            ("2024-02-29T03:02:03+02:00", "20240229010203"),
        ] {
            assert_eq!(
                normalize_archive_timestamp(Some(input)).unwrap(),
                Some(expected.to_string())
            );
        }
        for invalid in ["", "202413", "20230229", "202401010101", "not-a-date"] {
            assert!(
                normalize_archive_timestamp(Some(invalid)).is_err(),
                "{invalid}"
            );
        }
        assert_eq!(normalize_archive_timestamp(None).unwrap(), None);
    }

    #[test]
    fn archive_endpoint_is_fixed_and_original_url_is_encoded_as_data() {
        let original = parse_public_url("https://example.com/a path?q=one#fragment").unwrap();
        let endpoint = wayback_availability_url(&original, Some("20240101000000")).unwrap();
        assert_eq!(endpoint.scheme(), "https");
        assert_eq!(endpoint.host_str(), Some("archive.org"));
        assert_eq!(endpoint.path(), "/wayback/available");
        let pairs = endpoint.query_pairs().collect::<Vec<_>>();
        assert!(
            pairs.iter().any(|(key, value)| {
                key == "url" && value == "https://example.com/a%20path?q=one"
            })
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| { key == "timestamp" && value == "20240101000000" })
        );

        let cdx = wayback_cdx_url(&original, "20240101000000").unwrap();
        assert_eq!(cdx.scheme(), "https");
        assert_eq!(cdx.host_str(), Some("web.archive.org"));
        assert_eq!(cdx.path(), "/cdx/search/cdx");
        let pairs = cdx.query_pairs().collect::<Vec<_>>();
        for (key, expected) in [
            ("url", "https://example.com/a%20path?q=one"),
            ("matchType", "exact"),
            ("output", "json"),
            ("gzip", "false"),
            ("fl", "timestamp,original,statuscode,mimetype,digest"),
            ("filter", "statuscode:200"),
            ("to", "20240101000000"),
            ("limit", "-1"),
        ] {
            assert!(
                pairs
                    .iter()
                    .any(|(actual_key, value)| actual_key == key && value == expected),
                "missing fixed CDX query pair {key}={expected}"
            );
        }
    }

    #[test]
    fn cdx_lookup_selects_latest_valid_pre_cutoff_capture() {
        let original = parse_public_url("http://example.com/old?a=1").unwrap();
        let payload = json!([
            ["timestamp", "original", "statuscode", "mimetype", "digest"],
            [
                "20240101000000",
                "http://example.com/old?a=1",
                "200",
                "text/html",
                "OLD"
            ],
            [
                "20240131235959",
                "http://example.com/old?a=1",
                "200",
                "text/html",
                "LATEST"
            ]
        ]);
        let output = archive_cdx_lookup_json(&payload, &original, "20240201000000".to_string())
            .unwrap()
            .expect("pre-cutoff capture");
        let snapshot = "https://web.archive.org/web/20240131235959/http://example.com/old?a=1";

        assert_eq!(output["tool"], ARCHIVE_LOOKUP_TOOL);
        assert_eq!(output["available"], true);
        assert_eq!(output["url"], snapshot);
        assert_eq!(output["capture_timestamp"], "20240131235959");
        assert_eq!(output["capture_relation_to_requested"], "before");
        assert_eq!(output["is_at_or_before_requested"], true);
        assert!(output["capture_offset_seconds"].as_i64().unwrap() < 0);
        assert_eq!(output["selection_method"], "cdx_at_or_before");
        assert_eq!(output["research_lead_only"], false);
        assert_eq!(output["capture_mime_type"], "text/html");
        assert_eq!(output["capture_digest"], "LATEST");
        assert_eq!(output["source"]["url"], snapshot);
        assert_eq!(output["source"]["source_id"], source_id_for_url(snapshot));
        assert_eq!(output["source"]["capture_digest"], "LATEST");
        assert!(
            output["source"].get("citation_markdown").is_none(),
            "CDX discovery must not mint a citation before open_url verifies the snapshot"
        );
        assert!(
            output["archive_caveat"]
                .as_str()
                .is_some_and(|caveat| caveat.contains("latest exact-URL HTTP-200 capture"))
        );
    }

    #[test]
    fn cdx_lookup_handles_absence_and_rejects_invalid_capture_rows() {
        let original = parse_public_url("https://example.com/old").unwrap();
        let timestamp = "20240201000000".to_string();
        assert!(
            archive_cdx_lookup_json(&json!([]), &original, timestamp.clone())
                .unwrap()
                .is_none()
        );
        assert!(
            archive_cdx_lookup_json(
                &json!([["timestamp", "original", "statuscode", "mimetype", "digest"]]),
                &original,
                timestamp.clone(),
            )
            .unwrap()
            .is_none()
        );

        for invalid_row in [
            json!([
                "20240202000000",
                "https://example.com/old",
                "200",
                "text/html",
                "A"
            ]),
            json!([
                "20240101000000",
                "https://example.com/other",
                "200",
                "text/html",
                "A"
            ]),
            json!([
                "20240101000000",
                "https://example.com/old",
                "302",
                "text/html",
                "A"
            ]),
            json!([
                "not-a-timestamp",
                "https://example.com/old",
                "200",
                "text/html",
                "A"
            ]),
        ] {
            let payload = json!([
                ["timestamp", "original", "statuscode", "mimetype", "digest"],
                invalid_row
            ]);
            assert!(
                archive_cdx_lookup_json(&payload, &original, timestamp.clone()).is_err(),
                "invalid CDX capture metadata must fail closed"
            );
        }

        let unexpected_header = json!([
            ["timestamp", "original", "statuscode"],
            ["20240101000000", "https://example.com/old", "200"]
        ]);
        assert!(archive_cdx_lookup_json(&unexpected_header, &original, timestamp).is_err());
    }

    #[test]
    fn availability_fallback_is_explicitly_research_lead_only() {
        let original = parse_public_url("http://example.com/old?a=1").unwrap();
        let payload = json!({
            "archived_snapshots": {
                "closest": {
                    "available": true,
                    "status": "200",
                    "timestamp": "20240203040506",
                    "url": "http://web.archive.org/web/20240203040506/http://example.com/old?a=1"
                }
            }
        });
        let output = archive_lookup_json(
            &payload,
            &original,
            Some("20240201000000".to_string()),
            true,
        )
        .unwrap();
        let snapshot = "https://web.archive.org/web/20240203040506/http://example.com/old?a=1";

        assert_eq!(output["tool"], ARCHIVE_LOOKUP_TOOL);
        assert_eq!(output["available"], true);
        assert_eq!(output["url"], snapshot);
        assert_eq!(output["capture_timestamp"], "20240203040506");
        assert_eq!(output["captured_at"], "2024-02-03T04:05:06Z");
        assert_eq!(output["capture_relation_to_requested"], "after");
        assert_eq!(output["is_at_or_before_requested"], false);
        assert!(output["capture_offset_seconds"].as_i64().unwrap() > 0);
        assert_eq!(output["selection_method"], "availability_closest_fallback");
        assert_eq!(output["research_lead_only"], true);
        assert_eq!(output["usable_as_evidence_for_requested_cutoff"], false);
        assert!(
            output["research_lead_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("No qualifying exact-URL HTTP-200 capture"))
        );
        assert_eq!(output["original_url"], "http://example.com/old?a=1");
        assert_eq!(output["source"]["url"], snapshot);
        assert_eq!(output["source"]["source_id"], source_id_for_url(snapshot));
        assert!(
            output["source"].get("citation_markdown").is_none(),
            "archive discovery must not mint a citation before open_url verifies the snapshot"
        );
        assert_eq!(output["untrusted_content"], true);
        assert!(
            output["archive_caveat"]
                .as_str()
                .is_some_and(|caveat| caveat.contains("research lead only"))
        );
    }

    #[test]
    fn availability_without_timestamp_preserves_latest_closest_behavior() {
        let original = parse_public_url("https://example.com/current").unwrap();
        let payload = json!({
            "archived_snapshots": {
                "closest": {
                    "available": true,
                    "status": "200",
                    "timestamp": "20240203040506",
                    "url": "https://web.archive.org/web/20240203040506/https://example.com/current"
                }
            }
        });
        let output = archive_lookup_json(&payload, &original, None, false).unwrap();

        assert_eq!(output["available"], true);
        assert_eq!(output["requested_timestamp"], Value::Null);
        assert_eq!(output["capture_relation_to_requested"], Value::Null);
        assert_eq!(output["is_at_or_before_requested"], Value::Null);
        assert_eq!(output["selection_method"], "availability_closest");
        assert_eq!(output["research_lead_only"], false);
    }

    #[test]
    fn archive_lookup_handles_absence_and_rejects_untrusted_snapshot_metadata() {
        let original = parse_public_url("https://example.com/missing").unwrap();
        let unavailable = archive_lookup_json(
            &json!({ "archived_snapshots": {} }),
            &original,
            Some("20200101000000".to_string()),
            true,
        )
        .unwrap();
        assert_eq!(unavailable["status"], "ok");
        assert_eq!(unavailable["available"], false);
        assert_eq!(unavailable["research_lead_only"], true);
        assert_eq!(unavailable["untrusted_content"], true);

        for bad_url in [
            "https://attacker.example/web/20240203040506/https://example.com",
            "https://web.archive.org/web/19990101000000/https://example.com",
            "https://user:secret@web.archive.org/web/20240203040506/https://example.com",
            "https://web.archive.org/web/20240203040506/https://example.com/not-the-requested-page",
        ] {
            let malicious = json!({
                "archived_snapshots": {
                    "closest": {
                        "available": true,
                        "status": "200",
                        "timestamp": "20240203040506",
                        "url": bad_url
                    }
                }
            });
            assert!(archive_lookup_json(&malicious, &original, None, false).is_err());
        }
    }

    #[test]
    fn citation_metadata_uses_supported_inline_markdown() {
        let url = "https://example.com/research?q=a";
        let citation = markdown_citation("A [useful] source", url);
        assert_eq!(
            citation,
            r#"[A \[useful\] source](https://example.com/research?q=a)"#
        );
        assert!(source_id_for_url(url).starts_with("src_"));
        assert_eq!(source_id_for_url(url), source_id_for_url(url));

        let output = success_json(FetchSuccess {
            requested_url: url.to_string(),
            final_url: Url::parse(url).unwrap(),
            content_type: "text/html".to_string(),
            text: "Evidence".to_string(),
            format: "markdown",
            bytes_read: 8,
            truncated: false,
            redirects: Vec::new(),
            title: "A useful source".to_string(),
            find: None,
        });
        assert_eq!(output["tool"], OPEN_URL_TOOL);
        assert_eq!(output["untrusted_content"], true);
        assert_eq!(output["source"]["url"], url);
        assert_eq!(
            output["source"]["citation_markdown"],
            "[A useful source](https://example.com/research?q=a)"
        );
    }

    #[test]
    fn output_truncation_is_unicode_safe() {
        let (value, truncated) = truncate_chars("aé🙂z".to_string(), 3);
        assert!(truncated);
        assert!(value.starts_with("aé🙂"));
        assert_eq!(
            remove_control_characters("left\u{202e}right\u{200b}".to_string()),
            "leftright"
        );
    }
}
