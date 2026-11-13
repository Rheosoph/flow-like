//! Safe, read-only public-web page retrieval for FlowPilot.
//!
//! This deliberately does less than a general HTTP client: one GET, public HTTP(S) hosts on their
//! default ports, textual responses, bounded bodies, and fully revalidated redirects. Fetched text
//! is evidence for the model, never instructions.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::Range,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use dom_smoothie::{Config as ReadabilityConfig, Readability};
use flow_like_types::{egress, reqwest, tokio};
use htmd::{Element, HtmlToMarkdown, HtmlToMarkdownBuilder, element_handler::Handlers};
use serde_json::{Map, Value, json};
use url::{Host, Url};

use super::tool_spec::{ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL};

const MAX_URL_BYTES: usize = 4_096;
/// Content images one page conversion keeps; a page past this is a gallery or a catalogue grid.
const MAX_PAGE_IMAGES: usize = 40;
const MAX_IMAGE_URL_CHARS: usize = 600;
const MAX_IMAGE_LABEL_CHARS: usize = 160;
/// An image that declares a side below this many pixels is an icon, a spacer or a tracking pixel.
const MIN_IMAGE_SIDE: u32 = 50;
/// Image links a session remembers; a turn that saw more keeps the first.
const MAX_SESSION_IMAGE_URLS: usize = 4_096;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 20_000;
const MAX_OUTPUT_CHARS: usize = 100_000;
const MAX_FIND_QUERY_CHARS: usize = 256;
const MAX_FIND_MATCHES: usize = 8;
const FIND_CONTEXT_CHARS: usize = 180;
const MIN_EVIDENCE_NON_WHITESPACE_CHARS: usize = 12;
/// Below this an extracted article is a teaser or a cookie notice, and the whole page is returned.
const MIN_ARTICLE_NON_WHITESPACE_CHARS: usize = 300;
const MAX_READABILITY_ELEMENTS: usize = 60_000;
const MAX_REDIRECTS: usize = 5;
const MAX_CONCURRENT_OPEN_URL_CALLS: usize = 4;
const MAX_CONCURRENT_ARCHIVE_LOOKUPS: usize = 2;
const MAX_CONCURRENT_HTML_CONVERSIONS: usize = 4;
/// Pages one model round may open; the rest are asked for again in the next round, so a single
/// reply cannot fill the context with page text.
pub(crate) const MAX_OPEN_URL_CALLS_PER_ROUND: usize = 8;
/// Runaway guards shared by every agent researching within one turn (see [`WebResearchSession`]).
/// They sit far above what research needs: a run ends at its deadline, not at these.
const MAX_OPEN_URL_CALLS_PER_SESSION: usize = 512;
pub(crate) const MAX_SEARCH_CALLS_PER_SESSION: usize = 256;
pub(crate) const MAX_ARCHIVE_CALLS_PER_SESSION: usize = 64;
/// URLs one turn's ledger holds; it is saved with every checkpoint of a durable run. Half of it
/// covers every result of the searches the runaway guard allows.
const MAX_SESSION_URLS: usize = 16_384;
const MAX_CACHED_PAGES: usize = 64;
const MAX_CACHED_PAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ARCHIVE_RESPONSE_BYTES: usize = 64 * 1024;
const WAYBACK_HOST: &str = "web.archive.org";
const WAYBACK_AVAILABILITY_ENDPOINT: &str = "https://archive.org/wayback/available";
const WAYBACK_CDX_ENDPOINT: &str = "https://web.archive.org/cdx/search/cdx";
const WAYBACK_CDX_FIELDS: [&str; 5] = ["timestamp", "original", "statuscode", "mimetype", "digest"];
const ARCHIVE_CAVEAT: &str = "This is an Internet Archive capture, not the live page. Captures may be incomplete, replayed incorrectly, or captured before or after the requested time; do not use one as evidence of current facts.";
const ARCHIVE_PRE_CUTOFF_CAVEAT: &str = "This is an Internet Archive capture, not the live page. It was selected as the latest exact-URL HTTP-200 capture at or before the requested time, but the capture time is not the page's publication or event time. Open and verify the snapshot before relying on it; captures may be incomplete or replayed incorrectly.";
const ARCHIVE_CLOSEST_LEAD_CAVEAT: &str = "No qualifying exact-URL HTTP-200 capture at or before the requested time was found in the Wayback CDX index. This closest Availability result is a research lead only, may be after the cutoff, and must not support a claim about what the page said by the requested time. Open it only to discover further evidence.";
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MARKDOWN_ALTERNATE_TIMEOUT: Duration = Duration::from_secs(8);
/// Markdown first: sites that negotiate content (e.g. Cloudflare "Markdown for Agents") only serve
/// it when it outranks HTML, and it carries the page without navigation chrome.
const PAGE_ACCEPT: &str = "text/markdown, text/html;q=0.9, application/xhtml+xml;q=0.9, text/plain;q=0.8, application/json;q=0.7, application/xml;q=0.6, text/xml;q=0.6, */*;q=0.1";
const MARKDOWN_ACCEPT: &str = "text/markdown, text/x-markdown;q=0.9";
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
    /// Converted pages of this turn, so paging and `find` on an opened page fetch nothing.
    pages: Mutex<PageCache>,
    /// The images the opened pages showed, as [`comparable_url`] gives them. A synthesis may
    /// carry these links although they are no opened page. Like the pages, they are not saved.
    page_images: Mutex<HashSet<String>>,
}

/// The turn's opened pages by requested and final URL, oldest dropped first.
#[derive(Debug, Default)]
struct PageCache {
    pages: HashMap<String, Arc<LoadedPage>>,
    order: VecDeque<Arc<LoadedPage>>,
    bytes: usize,
}

impl PageCache {
    fn get(&self, url: &str) -> Option<Arc<LoadedPage>> {
        self.pages.get(url).cloned()
    }

    fn insert(&mut self, page: Arc<LoadedPage>) {
        let size = page.cached_bytes();
        if size > MAX_CACHED_PAGE_BYTES {
            return;
        }
        while self.order.len() >= MAX_CACHED_PAGES || self.bytes + size > MAX_CACHED_PAGE_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.bytes -= oldest.cached_bytes();
            self.pages.retain(|_, cached| !Arc::ptr_eq(cached, &oldest));
        }
        self.bytes += size;
        self.pages.insert(page.requested_url.clone(), page.clone());
        self.pages
            .insert(page.final_url.as_str().to_string(), page.clone());
        self.order.push_back(page);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebResearchSnapshot {
    authorized_urls: HashSet<String>,
    opened_urls: HashSet<String>,
    non_citable_urls: HashSet<String>,
    public_web_closed: bool,
    search_calls: usize,
    archive_calls: usize,
    open_url_budget: OpenUrlSessionBudget,
}

impl WebResearchSession {
    pub(super) fn snapshot(&self) -> Value {
        let snapshot = WebResearchSnapshot {
            authorized_urls: self
                .authorized_urls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            opened_urls: self
                .opened_urls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            non_citable_urls: self
                .non_citable_urls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            public_web_closed: self.public_web_closed.load(Ordering::Acquire),
            search_calls: self.search_calls.load(Ordering::Relaxed),
            archive_calls: self.archive_calls.load(Ordering::Relaxed),
            open_url_budget: self
                .open_url_budget
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        };
        serde_json::to_value(snapshot).expect("web research state serializes")
    }

    pub(super) fn restore(value: Value) -> flow_like_types::Result<Self> {
        let saved: WebResearchSnapshot = serde_json::from_value(value)?;
        if saved.search_calls > MAX_SEARCH_CALLS_PER_SESSION
            || saved.archive_calls > MAX_ARCHIVE_CALLS_PER_SESSION
            || saved.open_url_budget.session_calls > MAX_OPEN_URL_CALLS_PER_SESSION
            || saved.open_url_budget.round_calls > MAX_OPEN_URL_CALLS_PER_ROUND
            || saved.open_url_budget.round_call_limit > MAX_OPEN_URL_CALLS_PER_ROUND
            || saved.authorized_urls.len() + saved.opened_urls.len() + saved.non_citable_urls.len()
                > MAX_SESSION_URLS
            || saved
                .authorized_urls
                .iter()
                .chain(&saved.opened_urls)
                .chain(&saved.non_citable_urls)
                .any(|url| url.len() > 8_192 || parse_public_url(url).is_err())
            || saved
                .opened_urls
                .iter()
                .any(|url| saved.non_citable_urls.contains(url))
        {
            return Err(flow_like_types::anyhow!(
                "Saved web research state has invalid limits or provenance"
            ));
        }
        Ok(Self {
            authorized_urls: Mutex::new(saved.authorized_urls),
            opened_urls: Mutex::new(saved.opened_urls),
            non_citable_urls: Mutex::new(saved.non_citable_urls),
            public_web_closed: AtomicBool::new(saved.public_web_closed),
            search_calls: AtomicUsize::new(saved.search_calls),
            archive_calls: AtomicUsize::new(saved.archive_calls),
            open_url_budget: Mutex::new(saved.open_url_budget),
            pages: Mutex::default(),
            page_images: Mutex::default(),
        })
    }

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
            OPEN_URL_TOOL => self.register_opened_page(result),
            _ => {}
        }
    }

    fn register_opened_page(&self, result: &Value) {
        let Some(url) = result.get("url").and_then(Value::as_str) else {
            return;
        };
        self.authorize_url(url);
        let citation_eligible = result.get("citation_eligible").and_then(Value::as_bool)
            == Some(true)
            && !self.is_non_citable_url(url);
        if !citation_eligible {
            self.mark_non_citable_url(url);
            return;
        }
        self.mark_opened_url(url);
        // A redirect lands on the page that was read, so the search-result URL the model asked
        // for cites it just as well as the final one.
        if let Some(requested) = result.get("requested_url").and_then(Value::as_str)
            && !self.is_non_citable_url(requested)
        {
            self.mark_opened_url(requested);
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
        let mut authorized = self
            .authorized_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if authorized.len() < MAX_SESSION_URLS / 2 {
            authorized.insert(url.as_str().to_string());
        }
    }

    fn cached_page(&self, raw: &str) -> Option<Arc<LoadedPage>> {
        let url = parse_public_url(raw).ok()?;
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(url.as_str())
    }

    fn cache_page(&self, page: LoadedPage) -> Arc<LoadedPage> {
        self.remember_page_images(&page);
        let page = Arc::new(page);
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(page.clone());
        page
    }

    fn remember_page_images(&self, page: &LoadedPage) {
        let mut images = self
            .page_images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let texts = [Some(&page.text), page.full_text.as_ref()];
        for url in texts
            .into_iter()
            .flatten()
            .flat_map(|text| markdown_image_urls(text))
            .filter_map(comparable_url)
        {
            if images.len() >= MAX_SESSION_IMAGE_URLS {
                return;
            }
            images.insert(url);
        }
    }

    /// The image links the pages opened in this session showed, in [`comparable_url`] form.
    pub fn page_image_urls(&self) -> Vec<String> {
        self.page_images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn mark_opened_url(&self, raw: &str) {
        if let Ok(url) = parse_public_url(raw) {
            self.opened_urls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(url.as_str().to_string());
        }
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

    /// Adopt the pages another session successfully opened, so this session may cite them and
    /// read them itself. A researcher runs in its own session, but its findings land in this
    /// context, and an answer that cannot cite the pages behind it is worse than useless. Only
    /// the opened set crosses: the rest of the other session's authorization set stays its own.
    pub fn adopt_opened_urls(&self, urls: &[String]) {
        for url in urls {
            self.authorize_url(url);
        }
        let mut opened = self
            .opened_urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for url in urls {
            opened.insert(url.clone());
        }
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
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |calls| {
            (calls < limit).then_some(calls + 1)
        })
        .is_ok()
}

pub(crate) fn public_urls_in_user_text(text: &str) -> Vec<String> {
    public_url_spans(text)
        .into_iter()
        .map(|span| text[span].to_string())
        .collect()
}

/// Byte ranges of the http(s) URLs in free text, with trailing prose and markup punctuation trimmed.
pub(crate) fn public_url_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
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
                (index > 0 && (character.is_whitespace() || is_url_terminator(character)))
                    .then_some(index)
            })
            .unwrap_or(candidate.len());
        let candidate = trim_user_url_candidate(&candidate[..scanned_end]);
        if !candidate.is_empty() {
            spans.push(start..start + candidate.len());
        }
        offset = start.saturating_add(scanned_end.max(1));
    }
    spans
}

fn is_url_terminator(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | '"' | '\'' | '`' | '„' | '“' | '”' | '‘' | '’' | '«' | '»'
    )
}

const URL_TRAILING_PUNCTUATION: [char; 7] = ['.', ',', ';', ':', '!', '?', '*'];

fn trim_user_url_candidate(mut candidate: &str) -> &str {
    candidate = candidate.trim_end_matches(URL_TRAILING_PUNCTUATION);
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
        candidate = candidate.trim_end_matches(URL_TRAILING_PUNCTUATION);
    }
}

/// Page opens of one assistant run: a runaway guard for the session and a per-round count for
/// hosts that see whole model rounds. The character fields are no longer enforced; they stay so a
/// checkpoint written before they were dropped still loads.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUrlSessionBudget {
    session_calls: usize,
    #[serde(default)]
    session_chars_reserved: usize,
    round_calls: usize,
    #[serde(default)]
    round_chars_reserved: usize,
    round_call_limit: usize,
    #[serde(default)]
    round_char_limit: usize,
}

impl OpenUrlSessionBudget {
    pub(crate) fn begin_round(&mut self, planned_open_url_calls: usize) {
        self.round_calls = 0;
        self.round_call_limit = planned_open_url_calls
            .min(MAX_OPEN_URL_CALLS_PER_ROUND)
            .min(MAX_OPEN_URL_CALLS_PER_SESSION.saturating_sub(self.session_calls));
    }

    /// Counts one `open_url` of a batched round and bounds the text it may return; other tools
    /// pass through untouched.
    pub(crate) fn prepare_call(
        &mut self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        if tool_name != OPEN_URL_TOOL {
            return Ok(arguments);
        }
        if self.session_calls < MAX_OPEN_URL_CALLS_PER_SESSION
            && self.round_calls >= self.round_call_limit
        {
            return Err(open_url_round_error());
        }
        let arguments = self.prepare_unbatched_call(arguments)?;
        self.round_calls += 1;
        Ok(arguments)
    }

    /// Counts one `open_url` where the host sees single calls, not model rounds.
    pub fn prepare_unbatched_call(&mut self, mut arguments: Value) -> Result<Value, String> {
        if self.session_calls >= MAX_OPEN_URL_CALLS_PER_SESSION {
            return Err(open_url_budget_error(
                "open_url_session_call_budget_exceeded",
                "This assistant run opened an unusually large number of pages and was stopped. Answer from the evidence already collected.",
                false,
            ));
        }
        let max_chars = requested_max_chars(&arguments);
        if let Some(object) = arguments.as_object_mut() {
            object.insert("max_chars".to_string(), json!(max_chars));
        }
        self.session_calls += 1;
        Ok(arguments)
    }
}

fn requested_max_chars(arguments: &Value) -> usize {
    arguments
        .get("max_chars")
        .or_else(|| arguments.get("maxChars"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_CHARS as u64)
        .clamp(1_000, MAX_OUTPUT_CHARS as u64) as usize
}

/// The refusal for a page beyond [`MAX_OPEN_URL_CALLS_PER_ROUND`] in one model round.
pub(crate) fn open_url_round_error() -> String {
    open_url_budget_error(
        "open_url_round_call_budget_exceeded",
        "Too many pages for one round. Read the pages that opened, then ask for the rest in your next round.",
        true,
    )
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
    Markdown,
    Text,
}

#[derive(Debug)]
pub(super) struct FetchFailure {
    pub(super) code: &'static str,
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

    /// Whether a reader service or a browser can get past this failure: the site refused this
    /// client, rendered nothing without JavaScript, or sent a format only a reader parses. Never
    /// for a URL this host refused to request.
    fn reader_may_help(&self) -> bool {
        match self.code {
            "http_error" => self
                .http_status
                .is_some_and(|status| matches!(status, 401 | 403 | 406 | 429 | 451 | 500..=599)),
            "insufficient_text_content"
            | "unsupported_content_type"
            | "unsupported_content_encoding"
            | "attachment_not_allowed"
            | "request_timeout"
            | "request_failed"
            | "body_read_failed"
            | "timeout" => true,
            _ => false,
        }
    }

    fn with_reader_error(mut self, error: &str) -> Self {
        self.recovery_hints.push(format!(
            "A page reader was tried as well and failed: {}",
            error.chars().take(300).collect::<String>()
        ));
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

/// A fetched page after conversion, before an output window is cut from it.
#[derive(Debug)]
pub(crate) struct LoadedPage {
    requested_url: String,
    final_url: Url,
    content_type: String,
    format: &'static str,
    bytes_read: usize,
    body_truncated: bool,
    redirects: Vec<Value>,
    title: String,
    markdown_source: Option<Url>,
    /// What a read returns: the article when one was found, else the whole page.
    text: String,
    /// The whole converted page when `text` is only its article.
    full_text: Option<String>,
    published: Option<String>,
    byline: Option<String>,
    site_name: Option<String>,
    /// The reader that fetched the page when the plain request could not.
    reader: Option<&'static str>,
}

impl LoadedPage {
    fn cached_bytes(&self) -> usize {
        self.text.len() + self.full_text.as_ref().map_or(0, String::len)
    }
}

/// One validated response body after following every redirect hop.
struct FetchedBody {
    final_url: Url,
    content_type: String,
    kind: TextKind,
    body: Vec<u8>,
    truncated: bool,
    redirects: Vec<Value>,
}

/// The text taken from a response body: its article when one was found, and what was read about it.
#[derive(Default)]
struct Converted {
    text: String,
    full_text: Option<String>,
    title: Option<String>,
    published: Option<String>,
    byline: Option<String>,
    site_name: Option<String>,
}

/// The slice of a page one `open_url` call asks for.
struct PageRequest {
    max_chars: usize,
    offset: usize,
    find: Option<String>,
    full_page: bool,
}

impl PageRequest {
    fn parse(args: &Value) -> Result<Self, FetchFailure> {
        Ok(Self {
            max_chars: requested_max_chars(args),
            offset: args
                .get("offset")
                .and_then(Value::as_u64)
                .and_then(|offset| usize::try_from(offset).ok())
                .unwrap_or(0),
            find: parse_find_query(args.get("find"))?,
            full_page: args
                .get("full_page")
                .or_else(|| args.get("fullPage"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// Fetch and convert one public textual URL within the reader's concurrency and time bounds.
async fn load_page(raw_url: &str) -> Result<LoadedPage, FetchFailure> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        let _permit = OPEN_URL_CONCURRENCY.acquire().await.map_err(|_| {
            FetchFailure::new(
                "concurrency_unavailable",
                "The safe URL reader is temporarily unavailable.",
            )
            .retryable()
        })?;
        fetch_public_page(raw_url).await
    })
    .await
    .unwrap_or_else(|_| {
        Err(FetchFailure::new(
            "timeout",
            "Opening the URL exceeded the 30 second safety deadline.",
        )
        .retryable())
    })
}

/// Open one public textual URL the session's ledger authorizes, without exposing a
/// general-purpose network client to the model.
pub async fn run_open_url_for_session(args: &Value, session: &WebResearchSession) -> Value {
    run_open_url_with_reader(args, session, None).await
}

/// [`run_open_url_for_session`] with a reader to try when the plain request is refused or the
/// page comes back without readable text.
pub async fn run_open_url_with_reader(
    args: &Value,
    session: &WebResearchSession,
    reader: Option<&dyn PageReader>,
) -> Value {
    if let Some(error) = session.public_web_phase_error(OPEN_URL_TOOL) {
        return error;
    }
    if let Err(error) = session.validate_open_call(args) {
        return error.as_json();
    }
    let request = match PageRequest::parse(args) {
        Ok(request) => request,
        Err(error) => return error.as_json(),
    };
    let requested_url = args.get("url").and_then(Value::as_str).unwrap_or("");
    let page = match session.cached_page(requested_url) {
        Some(page) => page,
        None => match load_page_or_read(requested_url, reader).await {
            Ok(page) => session.cache_page(page),
            Err(error) => return error.as_json(),
        },
    };
    apply_session_citation_policy(page_json(&page, &request), session, requested_url)
}

/// What a [`PageReader`] returns for one URL.
#[derive(Debug, Clone, Default)]
pub struct ReaderPage {
    /// The reader's short name for the result and the logs, e.g. `firecrawl`.
    pub reader: &'static str,
    /// The page as markdown or plain text.
    pub text: String,
    pub title: Option<String>,
    /// Where the reader ended up after redirects, when it says so.
    pub final_url: Option<String>,
    pub published: Option<String>,
}

/// Another way to read a public page: a hosted reader API or a browser. Tried only after the
/// plain request failed in a way a different client can get past.
#[async_trait::async_trait]
pub trait PageReader: Send + Sync {
    /// The page, or why the readers that tried could not read it (shown to the model). `None`
    /// when no reader is set up, so the original failure stands as it is.
    async fn read(&self, url: &Url) -> Option<Result<ReaderPage, String>>;
}

async fn load_page_or_read(
    raw_url: &str,
    reader: Option<&dyn PageReader>,
) -> Result<LoadedPage, FetchFailure> {
    let failure = match load_page(raw_url).await {
        Ok(page) => return Ok(page),
        Err(failure) => failure,
    };
    let (Some(reader), true) = (reader, failure.reader_may_help()) else {
        return Err(failure);
    };
    let requested = parse_public_url(raw_url)?;
    match reader.read(&requested).await {
        Some(Ok(read)) => page_from_reader(requested, read).map_err(|_| failure),
        Some(Err(error)) => Err(failure.with_reader_error(&error)),
        None => Err(failure),
    }
}

fn page_from_reader(requested: Url, read: ReaderPage) -> Result<LoadedPage, FetchFailure> {
    let reader = read.reader;
    let text = remove_control_characters(read.text);
    ensure_sufficient_evidence_text(&text, &requested)?;
    // A reader may only report the page it was sent to or a public page it was redirected to.
    let final_url = read
        .final_url
        .as_deref()
        .and_then(|url| parse_public_url(url).ok())
        .unwrap_or_else(|| requested.clone());
    let title = read
        .title
        .or_else(|| markdown_title(&text))
        .map(|title| remove_control_characters(collapse_whitespace(&title)))
        .filter(|title| !title.is_empty())
        .map(truncate_title)
        .unwrap_or_else(|| final_url.host_str().unwrap_or("Web source").to_string());
    Ok(LoadedPage {
        requested_url: requested.as_str().to_string(),
        final_url,
        content_type: "text/markdown".to_string(),
        format: "markdown",
        bytes_read: text.len(),
        body_truncated: false,
        redirects: Vec::new(),
        title,
        markdown_source: None,
        text,
        full_text: None,
        published: read.published,
        byline: None,
        site_name: None,
        reader: Some(reader),
    })
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
        || snapshot.host_str() != Some(WAYBACK_HOST)
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

async fn fetch_public_body(start: Url, accept: &str) -> Result<FetchedBody, FetchFailure> {
    let mut current = start;
    let mut seen = HashSet::from([current.as_str().to_string()]);
    let mut redirects = Vec::new();

    for hop in 0..=MAX_REDIRECTS {
        let addresses = resolve_public_addresses(&current).await?;
        let client = client_for_hop(&current, &addresses)?;
        let response = client
            .get(current.clone())
            .header(reqwest::header::ACCEPT, accept)
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

        let (body, truncated) = read_bounded_body(response).await.map_err(|error| {
            FetchFailure::new(
                "body_read_failed",
                format!("Failed to read URL body: {error}"),
            )
            .retryable()
            .at(&current)
        })?;

        return Ok(FetchedBody {
            final_url: current,
            content_type,
            kind,
            body,
            truncated,
            redirects,
        });
    }

    unreachable!("redirect loop either returns a response or a redirect-limit error")
}

async fn fetch_public_page(raw_url: &str) -> Result<LoadedPage, FetchFailure> {
    let requested = parse_public_url(raw_url)?;
    let requested_url = requested.as_str().to_string();
    let page = fetch_public_body(requested, PAGE_ACCEPT).await?;
    let decoded = String::from_utf8_lossy(&page.body).into_owned();
    let document_title = page_title(&decoded, &page.final_url, page.kind);
    let alternate = match page.kind {
        TextKind::Html => fetch_markdown_alternate(&decoded, &page.final_url).await,
        TextKind::Markdown | TextKind::Text => None,
    };
    let (body, converted) = match alternate {
        Some(alternate) => alternate,
        None => {
            let converted = convert_body(decoded, page.kind, &page.final_url).await?;
            (
                FetchedBody {
                    final_url: page.final_url.clone(),
                    content_type: page.content_type,
                    kind: page.kind,
                    body: page.body,
                    truncated: page.truncated,
                    redirects: Vec::new(),
                },
                converted,
            )
        }
    };
    let markdown_source = (body.final_url != page.final_url).then_some(body.final_url);

    Ok(LoadedPage {
        requested_url,
        final_url: page.final_url,
        content_type: body.content_type,
        format: match body.kind {
            TextKind::Html | TextKind::Markdown => "markdown",
            TextKind::Text => "text",
        },
        bytes_read: body.body.len(),
        body_truncated: body.truncated,
        redirects: page.redirects,
        title: converted.title.unwrap_or(document_title),
        markdown_source,
        text: converted.text,
        full_text: converted.full_text,
        published: converted.published,
        byline: converted.byline,
        site_name: converted.site_name,
        reader: None,
    })
}

/// Read the markdown rendering a page declares through `<link rel="alternate"
/// type="text/markdown">` on its own origin. It is the same content without navigation chrome or a
/// JavaScript shell; any failure falls back to converting the HTML page.
async fn fetch_markdown_alternate(html: &str, page_url: &Url) -> Option<(FetchedBody, Converted)> {
    let alternate = markdown_alternate_url(html, page_url)?;
    let fetched = tokio::time::timeout(
        MARKDOWN_ALTERNATE_TIMEOUT,
        fetch_public_body(alternate, MARKDOWN_ACCEPT),
    )
    .await
    .ok()?
    .ok()?;
    if fetched.kind != TextKind::Markdown || fetched.final_url.origin() != page_url.origin() {
        return None;
    }
    let decoded = String::from_utf8_lossy(&fetched.body).into_owned();
    let converted = convert_body(decoded, fetched.kind, &fetched.final_url)
        .await
        .ok()?;
    Some((fetched, converted))
}

fn markdown_alternate_url(html: &str, page_url: &Url) -> Option<Url> {
    // A snapshot's alternate resolves to whichever capture is nearest, not this one.
    if page_url.host_str() == Some(WAYBACK_HOST) {
        return None;
    }
    let lower = html.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(found) = lower[offset..].find("<link") {
        let start = offset + found;
        let end = start + lower[start..].find('>')?;
        offset = end;
        let tag = &html[start..end];
        let is_markdown_alternate = html_attribute(tag, "rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|value| value.eq_ignore_ascii_case("alternate"))
        }) && html_attribute(tag, "type")
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/markdown"));
        if !is_markdown_alternate {
            continue;
        }
        let href = html_attribute(tag, "href")?.trim().replace("&amp;", "&");
        let url = parse_public_url(page_url.join(&href).ok()?.as_str()).ok()?;
        return (url.origin() == page_url.origin() && url != *page_url).then_some(url);
    }
    None
}

/// The value of one attribute in a single HTML start tag (without its closing `>`).
fn html_attribute<'a>(tag: &'a str, name: &'a str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name) {
        let at = search + found;
        search = at + name.len();
        if !lower[..at].ends_with(|character: char| character.is_ascii_whitespace()) {
            continue;
        }
        let Some(after_equals) = lower[search..].trim_start().strip_prefix('=') else {
            continue;
        };
        let value = &tag[lower.len() - after_equals.trim_start().len()..];
        return match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split(quote).next(),
            _ => value
                .split(|character: char| character.is_ascii_whitespace())
                .next(),
        };
    }
    None
}

async fn convert_body(
    decoded: String,
    kind: TextKind,
    url: &Url,
) -> Result<Converted, FetchFailure> {
    let mut converted = match kind {
        TextKind::Html => convert_html_off_thread(decoded, url).await?,
        TextKind::Markdown | TextKind::Text => Converted {
            text: decoded,
            ..Converted::default()
        },
    };
    converted.text = remove_control_characters(converted.text);
    converted.full_text = converted.full_text.map(remove_control_characters);
    ensure_sufficient_evidence_text(&converted.text, url)?;
    Ok(converted)
}

/// Converts an HTML page on the blocking pool, a bounded number at a time.
async fn convert_html_off_thread(html: String, url: &Url) -> Result<Converted, FetchFailure> {
    let pool_failure = |code: &'static str, message: &'static str| {
        FetchFailure::new(code, message).retryable().at(url)
    };
    let conversion_permit = tokio::time::timeout(
        DNS_TIMEOUT,
        HTML_CONVERSION_CONCURRENCY.clone().acquire_owned(),
    )
    .await
    .map_err(|_| {
        pool_failure(
            "conversion_busy",
            "The bounded HTML conversion pool is busy.",
        )
    })?
    .map_err(|_| {
        pool_failure(
            "conversion_unavailable",
            "The bounded HTML conversion pool is unavailable.",
        )
    })?;
    let page_url = url.clone();
    tokio::task::spawn_blocking(move || {
        // Keep the permit inside the blocking task. If the outer request times out, the
        // non-cancellable conversion still occupies its slot until it truly ends.
        let _conversion_permit = conversion_permit;
        convert_html(&html, &page_url)
    })
    .await
    .map_err(|_| {
        FetchFailure::new(
            "conversion_failed",
            "The isolated HTML conversion task did not complete.",
        )
        .at(url)
    })?
    .map_err(|error| error.at(url))
}

/// The page as a research result: one window of its text, where the window sits, and what the
/// model needs to cite it. Fields without a value are left out.
fn page_json(page: &LoadedPage, request: &PageRequest) -> Value {
    let shown = match (&page.full_text, request.full_page) {
        (Some(full), true) => full.as_str(),
        _ => page.text.as_str(),
    };
    let window = char_window(shown, request.offset, request.max_chars);
    let hidden_page_chars = page
        .full_text
        .as_ref()
        .filter(|_| !request.full_page)
        .map(|full| full.chars().count());

    let mut result = json!({
        "status": "ok",
        "tool": OPEN_URL_TOOL,
        "requested_url": page.requested_url,
        "url": page.final_url.as_str(),
        "citation_eligible": true,
        "source": page_source(page),
        "format": page.format,
        "extraction": if hidden_page_chars.is_some() { "main_content" } else { "full_page" },
        "content": window.text,
        "offset": window.start,
        "total_chars": window.total,
        "bytes_read": page.bytes_read,
        "truncated": page.body_truncated || window.next.is_some(),
        "untrusted_content": true,
    });
    let optional = [
        ("full_page_chars", hidden_page_chars.map(Value::from)),
        ("next_offset", window.next.map(Value::from)),
        ("reader", page.reader.map(Value::from)),
        ("find", page_find(page, request)),
    ];
    for (name, value) in optional.into_iter().chain(page_fetch_facts(page)) {
        if let Some(value) = value {
            result[name] = value;
        }
    }
    result
}

/// Matches of the request's `find` text in the whole page, whichever window is shown.
fn page_find(page: &LoadedPage, request: &PageRequest) -> Option<Value> {
    let query = request.find.as_deref()?;
    let searched = page.full_text.as_deref().unwrap_or(&page.text);
    Some(find_matches(searched, query, page.body_truncated))
}

/// How the page was reached, where that differs from one plain request for it.
fn page_fetch_facts(page: &LoadedPage) -> [(&str, Option<Value>); 2] {
    [
        (
            "markdown_source",
            page.markdown_source.as_ref().map(|url| json!(url.as_str())),
        ),
        (
            "redirects",
            (!page.redirects.is_empty()).then(|| json!(page.redirects)),
        ),
    ]
}

/// What identifies a page as a source: how to cite it and what it states about itself.
fn page_source(page: &LoadedPage) -> Value {
    let url = page.final_url.as_str();
    let mut source = Map::new();
    source.insert("source_id".into(), json!(source_id_for_url(url)));
    source.insert("title".into(), json!(page.title));
    source.insert("url".into(), json!(url));
    source.insert("content_type".into(), json!(page.content_type));
    source.insert(
        "citation_markdown".into(),
        json!(markdown_citation(&page.title, url)),
    );
    source.insert("citation_eligible".into(), json!(true));
    for (name, value) in [
        ("published", &page.published),
        ("byline", &page.byline),
        ("site_name", &page.site_name),
    ] {
        if let Some(value) = value {
            source.insert(name.into(), json!(value));
        }
    }
    Value::Object(source)
}

/// `max_chars` characters of `text` from character `offset`, with where to read on.
struct CharWindow {
    text: String,
    start: usize,
    total: usize,
    next: Option<usize>,
}

fn char_window(text: &str, offset: usize, max_chars: usize) -> CharWindow {
    let total = text.chars().count();
    let start = offset.min(total);
    let start_byte = char_boundary_after(text, 0, start);
    let end_byte = char_boundary_after(text, start_byte, max_chars);
    let end = start + text[start_byte..end_byte].chars().count();
    let mut shown = text[start_byte..end_byte].to_string();
    let next = (end < total).then_some(end);
    if let Some(next) = next {
        shown.push_str(&format!(
            "\n\n[Showing characters {start}-{end} of {total}. Call open_url again with offset {next} to read on, or use find.]"
        ));
    }
    CharWindow {
        text: shown,
        start,
        total,
        next,
    }
}

pub(super) fn parse_public_url(raw: &str) -> Result<Url, FetchFailure> {
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

pub(super) async fn resolve_public_addresses(url: &Url) -> Result<Vec<SocketAddr>, FetchFailure> {
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
    public_client_builder(url, addresses)
        .build()
        .map_err(|error| {
            FetchFailure::new(
                "request_failed",
                format!("Failed to create the safe URL client: {error}"),
            )
            .at(url)
        })
}

pub(super) fn public_client_builder(url: &Url, addresses: &[SocketAddr]) -> reqwest::ClientBuilder {
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
    builder
}

async fn read_bounded_body(response: reqwest::Response) -> Result<(Vec<u8>, bool), reqwest::Error> {
    read_bounded_body_with_limit(response, MAX_BODY_BYTES).await
}

pub(super) async fn read_bounded_body_with_limit(
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
        "text/markdown" | "text/x-markdown" => Some(TextKind::Markdown),
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

/// A content image of a page: what it shows, and its absolute link.
struct PageImage {
    label: String,
    url: String,
}

fn usable_image_source(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && !value.starts_with("data:")).then(|| value.to_string())
}

/// Where an `img` element loads its picture from: its own source, else the one a lazy loader
/// swaps in, else the widest candidate of its source set.
fn image_source(attribute: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    ["src", "data-src", "data-lazy-src", "data-original"]
        .iter()
        .find_map(|name| attribute(name).and_then(usable_image_source))
        .or_else(|| {
            ["srcset", "data-srcset"].iter().find_map(|name| {
                let candidates = attribute(name)?;
                let widest = candidates.split(", ").last()?.split_whitespace().next()?;
                usable_image_source(widest.to_string())
            })
        })
}

fn image_label(text: &str) -> String {
    let text = remove_control_characters(collapse_whitespace(text)).replace(['[', ']'], "");
    text.chars().take(MAX_IMAGE_LABEL_CHARS).collect()
}

/// Whether an `img` element declares a side too short for a content image.
fn declares_tiny_side(attribute: &dyn Fn(&str) -> Option<String>) -> bool {
    ["width", "height"]
        .iter()
        .filter_map(|name| {
            attribute(name)?
                .trim()
                .trim_end_matches("px")
                .parse::<u32>()
                .ok()
        })
        .any(|side| side < MIN_IMAGE_SIDE)
}

/// The absolute link of a raster image source, written so that it ends no markdown target early.
fn image_url(source: &str, base: Option<&Url>) -> Option<String> {
    let url = match base {
        Some(base) => base.join(source).ok()?,
        None => Url::parse(source).ok()?,
    };
    let path = url.path().to_ascii_lowercase();
    let raster = matches!(url.scheme(), "http" | "https")
        && !path.ends_with(".svg")
        && !path.ends_with(".ico");
    let url = url.as_str().replace('(', "%28").replace(')', "%29");
    (raster && url.len() <= MAX_IMAGE_URL_CHARS).then_some(url)
}

/// The content image an `img` element shows, or `None` for an icon, a spacer, a tracking pixel,
/// inline data or a vector graphic.
fn page_image(attribute: &dyn Fn(&str) -> Option<String>, base: Option<&Url>) -> Option<PageImage> {
    if declares_tiny_side(attribute) {
        return None;
    }
    let url = image_url(&image_source(attribute)?, base)?;
    let label = ["alt", "title"]
        .iter()
        .filter_map(|name| attribute(name))
        .map(|text| image_label(&text))
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| "image".to_string());
    Some(PageImage { label, url })
}

/// Markdown without what a reader cannot use: active regions, page chrome, video and audio, and
/// link targets. A link keeps its text only, since a URL inside page content is never authorized
/// to be opened and on news pages those targets outweigh the article. A content image stays as
/// `![what it shows](link)`, its link resolved against `base`, so a reader can hand it on and a
/// model can look at it; each image once, and no more than [`MAX_PAGE_IMAGES`] of a page.
fn markdown_converter(base: Option<Url>) -> HtmlToMarkdown {
    let kept: Mutex<HashSet<String>> = Mutex::default();
    HtmlToMarkdownBuilder::new()
        .skip_tags(vec![
            "head", "script", "style", "noscript", "iframe", "svg", "canvas", "template", "form",
            "button", "nav", "footer", "source", "video", "audio", "object", "embed",
        ])
        .add_handler(vec!["a"], |handlers: &dyn Handlers, element: Element| {
            Some(handlers.walk_children(element.node))
        })
        .add_handler(vec!["img"], move |_: &dyn Handlers, element: Element| {
            let attribute = |name: &str| {
                element
                    .attrs
                    .iter()
                    .find(|attribute| &*attribute.name.local == name)
                    .map(|attribute| attribute.value.to_string())
            };
            let image = page_image(&attribute, base.as_ref())?;
            let mut kept = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            (kept.len() < MAX_PAGE_IMAGES && kept.insert(image.url.clone()))
                .then(|| format!(" ![{}]({}) ", image.label, image.url).into())
        })
        .build()
}

fn html_to_markdown(html: &str, base: Option<&Url>) -> Result<String, FetchFailure> {
    markdown_converter(base.cloned())
        .convert(html)
        .map_err(|error| {
            FetchFailure::new(
                "conversion_failed",
                format!("Could not convert the HTML page to safe text: {error}"),
            )
        })
}

/// The targets of the markdown images in `text`.
fn markdown_image_urls(text: &str) -> Vec<&str> {
    let mut urls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        rest = &rest[start + 2..];
        let Some(close) = rest.find(']') else {
            break;
        };
        let Some(target) = rest[close + 1..].strip_prefix('(') else {
            continue;
        };
        let end = target
            .find(|character: char| character == ')' || character.is_whitespace())
            .unwrap_or(target.len());
        if end > 0 {
            urls.push(&target[..end]);
        }
    }
    urls
}

/// The form a link is compared in when a synthesis is checked against what a session saw.
pub(super) fn comparable_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    Some(parse_public_url(raw).map_or_else(|_| url.to_string(), |public| public.to_string()))
}

/// The article of an HTML page with the whole page behind it, or the whole page alone when it
/// has no article worth the name (a listing, a table of figures, an app shell).
fn convert_html(html: &str, url: &Url) -> Result<Converted, FetchFailure> {
    let full = html_to_markdown(html, Some(url))?;
    // The extractor walks an arbitrary document; a page that trips it is read whole instead.
    let article = std::panic::catch_unwind(|| extract_article(html, url))
        .ok()
        .flatten();
    Ok(match article {
        Some(article) => Converted {
            full_text: (non_whitespace_chars(&full) > non_whitespace_chars(&article.text))
                .then_some(full),
            ..article
        },
        None => Converted {
            text: full,
            ..Converted::default()
        },
    })
}

/// Mozilla Readability's pick of the main content, headed by its title and what the page states
/// about author and date.
fn extract_article(html: &str, url: &Url) -> Option<Converted> {
    let config = ReadabilityConfig {
        max_elements_to_parse: MAX_READABILITY_ELEMENTS,
        ..ReadabilityConfig::default()
    };
    let mut readability = Readability::new(html, Some(url.as_str()), Some(config)).ok()?;
    if !readability.is_probably_readable() {
        return None;
    }
    let article = readability.parse().ok()?;
    let body = html_to_markdown(&article.content, Some(url)).ok()?;
    if non_whitespace_chars(&body) < MIN_ARTICLE_NON_WHITESPACE_CHARS {
        return None;
    }
    let clean = |value: Option<String>| {
        value
            .map(|value| remove_control_characters(collapse_whitespace(&value)))
            .filter(|value| !value.is_empty())
            .map(truncate_title)
    };
    // A title without a space is a slug or an id some pages put in their metadata.
    let title = clean(Some(article.title)).filter(|title| title.contains(' '));
    let mut article = Converted {
        text: String::new(),
        full_text: None,
        title,
        published: clean(article.published_time),
        byline: clean(article.byline),
        site_name: clean(article.site_name),
    };
    article.text = format!("{}{}", article_heading(&article), body.trim());
    Some(article)
}

/// The lines above an article's body: its title, then who wrote it, when and where.
fn article_heading(article: &Converted) -> String {
    let facts: Vec<String> = [
        article.byline.as_ref().map(|byline| format!("By {byline}")),
        article
            .published
            .as_ref()
            .map(|published| format!("Published {published}")),
        article.site_name.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut heading = article
        .title
        .as_ref()
        .map(|title| format!("# {title}\n\n"))
        .unwrap_or_default();
    if !facts.is_empty() {
        heading.push_str(&format!("{}\n\n", facts.join(" | ")));
    }
    heading
}

fn non_whitespace_chars(value: &str) -> usize {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .count()
}

fn truncate_title(title: String) -> String {
    match title.char_indices().nth(200) {
        Some((end, _)) => title[..end].to_string(),
        None => title,
    }
}

fn ensure_sufficient_evidence_text(value: &str, url: &Url) -> Result<(), FetchFailure> {
    if non_whitespace_chars(value) >= MIN_EVIDENCE_NON_WHITESPACE_CHARS {
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
    let title = match kind {
        TextKind::Html => html_title(text),
        TextKind::Markdown => markdown_title(text),
        TextKind::Text => None,
    };
    title
        .map(|title| remove_control_characters(collapse_whitespace(&title)))
        .filter(|title| !title.is_empty())
        .map(truncate_title)
        .unwrap_or_else(|| url.host_str().unwrap_or("Web source").to_string())
}

fn html_title(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let open_end = open + lower[open..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</title>")?;
    html_to_markdown(&text[open_end..close], None).ok()
}

/// The `title:` of a YAML front matter block, else the first level-one heading.
fn markdown_title(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim);
    if text.trim_start().starts_with("---") {
        for line in lines.by_ref().skip(1) {
            if line == "---" {
                break;
            }
            if let Some(title) = line.strip_prefix("title:") {
                return Some(title.trim().trim_matches(['"', '\'']).to_string());
            }
        }
    }
    lines.find_map(|line| {
        line.strip_prefix("# ")
            .map(|title| title.trim().to_string())
    })
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

/// The shared public-address rule, narrowed to global unicast: page fetches also refuse
/// addresses that reach their target through translation or a relay (NAT64, 6to4, Teredo and
/// the 6to4 relay anycast range) and the `3fff::/20` documentation block.
fn is_public_ip(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        IpAddr::V4(_) => address,
    };
    egress::is_public_ip(address)
        && match address {
            IpAddr::V4(v4) => !ipv4_in_cidr(u32::from(v4), Ipv4Addr::new(192, 88, 99, 0), 24),
            IpAddr::V6(v6) => {
                let value = u128::from(v6);
                ipv6_in_cidr(value, "2000::".parse().unwrap(), 3)
                    && ![("2001::", 23), ("2002::", 16), ("3fff::", 20)]
                        .into_iter()
                        .any(|(network, prefix)| {
                            ipv6_in_cidr(value, network.parse().unwrap(), prefix)
                        })
            }
        }
}

fn ipv4_in_cidr(value: u32, network: Ipv4Addr, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    value & mask == u32::from(network) & mask
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

        assert_eq!(
            public_urls_in_user_text(
                "**https://a.example/x** `https://b.example/y` „https://c.example/z“ «https://d.example/w»"
            ),
            vec![
                "https://a.example/x".to_string(),
                "https://b.example/y".to_string(),
                "https://c.example/z".to_string(),
                "https://d.example/w".to_string(),
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
    fn page_opens_are_paced_per_round_and_stopped_only_by_the_runaway_guard() {
        let mut budget = OpenUrlSessionBudget::default();
        budget.begin_round(MAX_OPEN_URL_CALLS_PER_ROUND + 1);
        for _ in 0..MAX_OPEN_URL_CALLS_PER_ROUND {
            let prepared = budget
                .prepare_call(
                    OPEN_URL_TOOL,
                    json!({ "url": "https://example.com", "max_chars": 900_000 }),
                )
                .unwrap();
            assert_eq!(prepared["max_chars"], MAX_OUTPUT_CHARS);
        }
        let round_error = budget
            .prepare_call(OPEN_URL_TOOL, json!({ "url": "https://example.com/next" }))
            .unwrap_err();
        assert!(round_error.contains("open_url_round_call_budget_exceeded"));
        assert!(round_error.contains("\"retryable\":true"));

        budget.begin_round(1);
        let prepared = budget
            .prepare_call(OPEN_URL_TOOL, json!({ "url": "https://example.com/next" }))
            .unwrap();
        assert_eq!(prepared["max_chars"], DEFAULT_MAX_CHARS);

        let untouched = json!({ "query": "Flow-Like" });
        assert_eq!(
            budget
                .prepare_call("internet_search", untouched.clone())
                .unwrap(),
            untouched
        );

        let mut unbatched = OpenUrlSessionBudget::default();
        for _ in 0..MAX_OPEN_URL_CALLS_PER_SESSION {
            unbatched
                .prepare_unbatched_call(json!({ "url": "https://example.com" }))
                .unwrap();
        }
        assert!(
            unbatched
                .prepare_unbatched_call(json!({ "url": "https://example.com/extra" }))
                .unwrap_err()
                .contains("open_url_session_call_budget_exceeded")
        );
    }

    #[test]
    fn a_checkpoint_from_before_the_character_budget_was_dropped_still_loads() {
        let saved = json!({
            "authorized_urls": ["https://example.com/a"],
            "opened_urls": [],
            "non_citable_urls": [],
            "public_web_closed": false,
            "search_calls": 12,
            "archive_calls": 4,
            "open_url_budget": {
                "session_calls": 10,
                "session_chars_reserved": 120_000,
                "round_calls": 4,
                "round_chars_reserved": 60_000,
                "round_call_limit": 4,
                "round_char_limit": 60_000
            }
        });
        let session = WebResearchSession::restore(saved).unwrap();
        assert!(session.reserve_search_call());
        assert!(
            session
                .prepare_open_url_call(json!({ "url": "https://example.com/a" }))
                .is_ok()
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
            "192.88.99.1",
        ] {
            assert!(!is_public_ip(IpAddr::V4(raw.parse().unwrap())), "{raw}");
        }
        for raw in [
            "::1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "2002::1",
            "2001::1",
            "3fff::1",
            "64:ff9b::101:101",
            "::ffff:192.88.99.1",
            "::808:808",
        ] {
            assert!(!is_public_ip(IpAddr::V6(raw.parse().unwrap())), "{raw}");
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("::ffff:8.8.8.8".parse().unwrap()));
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
        assert_eq!(
            classify_content_type("text/markdown; charset=utf-8"),
            Some(TextKind::Markdown)
        );
        assert_eq!(classify_content_type("text/event-stream"), None);
        assert_eq!(classify_content_type("application/pdf"), None);
        assert_eq!(classify_content_type("application/octet-stream"), None);
    }

    #[test]
    fn markdown_alternate_is_followed_only_on_the_same_origin() {
        let page = Url::parse("https://nextjs.org/docs").unwrap();
        let alternate = |html: &str| markdown_alternate_url(html, &page).map(String::from);
        assert_eq!(
            alternate(
                r#"<head><link rel="alternate" type="text/markdown" href="https://nextjs.org/docs.md"/></head>"#
            ),
            Some("https://nextjs.org/docs.md".to_string())
        );
        assert_eq!(
            alternate(r#"<link href="/docs.md?v=1&amp;x=2" rel="alternate" type="text/markdown">"#),
            Some("https://nextjs.org/docs.md?v=1&x=2".to_string())
        );
        assert_eq!(
            alternate(
                r#"<link rel="alternate" hreflang="de" href="/de/docs"><link rel="index" type="text/markdown" href="/llms.txt"><link data-type="text/markdown" rel="alternate" href="/x.md"><link rel="alternate" type="text/markdown" href="/docs.md">"#
            ),
            Some("https://nextjs.org/docs.md".to_string())
        );
        for html in [
            r#"<link rel="alternate" type="text/markdown" href="https://evil.example/docs.md">"#,
            r#"<link rel="alternate" type="text/markdown" href="http://nextjs.org/docs.md">"#,
            r#"<link rel="alternate" type="text/markdown" href="/docs">"#,
            r#"<link rel="alternate" type="application/rss+xml" href="/feed.xml">"#,
        ] {
            assert_eq!(alternate(html), None, "{html}");
        }
        let snapshot =
            Url::parse("https://web.archive.org/web/2024/https://nextjs.org/docs").unwrap();
        assert_eq!(
            markdown_alternate_url(
                r#"<link rel="alternate" type="text/markdown" href="/web/2024/https://nextjs.org/docs.md">"#,
                &snapshot
            ),
            None
        );
    }

    #[test]
    fn markdown_titles_come_from_front_matter_or_the_first_heading() {
        let url = Url::parse("https://vercel.com/docs").unwrap();
        let title = |text: &str| page_title(text, &url, TextKind::Markdown);
        assert_eq!(
            title("---\ndescription: x\ntitle: \"Vercel Documentation\"\n---\n\n# Other"),
            "Vercel Documentation"
        );
        assert_eq!(
            title("---\nproduct: vercel\n---\n# Get started\n"),
            "Get started"
        );
        assert_eq!(title("Intro\n\n## Section\n# Zahlungen"), "Zahlungen");
        assert_eq!(title("No heading here"), "vercel.com");
    }

    #[test]
    fn a_redirected_open_makes_both_the_requested_and_final_url_citable() {
        let session = WebResearchSession::new("");
        session.register_tool_result(
            OPEN_URL_TOOL,
            &json!({
                "status": "ok",
                "tool": OPEN_URL_TOOL,
                "requested_url": "https://example.com/product",
                "url": "https://www.example.com/product/",
                "citation_eligible": true,
            }),
        );
        assert_eq!(
            session.opened_urls(),
            vec![
                "https://example.com/product".to_string(),
                "https://www.example.com/product/".to_string(),
            ]
        );
    }

    #[test]
    fn html_conversion_drops_active_regions_and_keeps_evidence() {
        let html = r#"<html><head><title>Official &amp; Current</title><script>steal()</script></head>
            <body><nav>Noise</nav><h1>Result</h1><p>Supported <a href="https://example.com/source">fact</a>.</p>
            <form>Ignore me</form><footer>More noise</footer></body></html>"#;
        let markdown = html_to_markdown(html, None).unwrap();
        assert!(markdown.contains("# Result"));
        assert!(markdown.contains("Supported fact."));
        assert!(!markdown.contains("example.com/source"));
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
        let prefix = char_window(&full_text, 0, 1_000);

        assert!(prefix.next.is_some());
        assert!(
            !prefix
                .text
                .to_ascii_lowercase()
                .contains("requested phrase")
        );
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

        let page = LoadedPage {
            markdown_source: Some(Url::parse("https://example.com/research.md").unwrap()),
            ..test_page(url, "Evidence", None)
        };
        let output = page_json(&page, &whole_page());
        assert_eq!(output["tool"], OPEN_URL_TOOL);
        assert_eq!(output["untrusted_content"], true);
        assert_eq!(output["source"]["url"], url);
        assert_eq!(output["url"], url);
        assert_eq!(output["markdown_source"], "https://example.com/research.md");
        assert_eq!(
            output["source"]["citation_markdown"],
            "[A useful source](https://example.com/research?q=a)"
        );
        assert_eq!(output["extraction"], "full_page");
        assert!(output.get("find").is_none() && output.get("redirects").is_none());
    }

    fn test_page(url: &str, text: &str, full_text: Option<&str>) -> LoadedPage {
        LoadedPage {
            requested_url: url.to_string(),
            final_url: Url::parse(url).unwrap(),
            content_type: "text/html".to_string(),
            format: "markdown",
            bytes_read: text.len(),
            body_truncated: false,
            redirects: Vec::new(),
            title: "A useful source".to_string(),
            markdown_source: None,
            text: text.to_string(),
            full_text: full_text.map(str::to_string),
            published: None,
            byline: None,
            site_name: None,
            reader: None,
        }
    }

    fn whole_page() -> PageRequest {
        PageRequest::parse(&json!({})).unwrap()
    }

    #[test]
    fn content_images_survive_conversion_and_page_chrome_images_do_not() {
        let base = Url::parse("https://news.example/cars/story").unwrap();
        let html = r#"<html><body><article>
            <p>The new model <img src="/media/ix3 (front).jpg?w=1200" alt="  The iX3 [front]
                view "> was shown.</p>
            <figure><picture><source srcset="/media/plant.webp">
                <img data-src="https://cdn.example/plant.jpg" src="data:image/gif;base64,R0lGOD"
                    title="Plant in Debrecen"></picture></figure>
            <img src="/pixel.gif" width="1" height="1">
            <img src="/icons/logo.svg" alt="Logo">
            <img src="/media/ix3 (front).jpg?w=1200" alt="the same picture again">
            <img srcset="/media/small.jpg 320w, /media/large.jpg 1280w">
            <img src="mailto:press@news.example" alt="Mail">
            <img alt="No source">
            </article></body></html>"#;
        let markdown = html_to_markdown(html, Some(&base)).unwrap();
        let front = "https://news.example/media/ix3%20%28front%29.jpg?w=1200";
        assert!(
            markdown.contains(&format!("![The iX3 front view]({front})")),
            "{markdown}"
        );
        assert!(markdown.contains("![Plant in Debrecen](https://cdn.example/plant.jpg)"));
        assert!(markdown.contains("![image](https://news.example/media/large.jpg)"));
        assert_eq!(
            markdown_image_urls(&markdown),
            [
                front,
                "https://cdn.example/plant.jpg",
                "https://news.example/media/large.jpg"
            ],
            "{markdown}"
        );
        assert!(markdown.contains("The new model") && markdown.contains("was shown."));

        let relative = html_to_markdown(r#"<p><img src="/a.jpg" alt="A"></p>"#, None).unwrap();
        assert!(markdown_image_urls(&relative).is_empty(), "{relative}");
    }

    #[test]
    fn a_page_keeps_a_bounded_number_of_images() {
        let base = Url::parse("https://shop.example/catalogue").unwrap();
        let grid: String = (0..MAX_PAGE_IMAGES + 25)
            .map(|index| format!(r#"<li><img src="/p/{index}.jpg" alt="Product {index}"></li>"#))
            .collect();
        let markdown = html_to_markdown(&format!("<ul>{grid}</ul>"), Some(&base)).unwrap();
        assert_eq!(markdown_image_urls(&markdown).len(), MAX_PAGE_IMAGES);
        let long = format!(
            r#"<img src="/p.jpg?token={}" alt="Too long a link">"#,
            "x".repeat(MAX_IMAGE_URL_CHARS)
        );
        let markdown = html_to_markdown(&long, Some(&base)).unwrap();
        assert!(markdown_image_urls(&markdown).is_empty());
    }

    #[test]
    fn markdown_image_targets_are_found_without_their_titles_or_outer_links() {
        let text = "Intro ![one](https://a.example/1.png) and [![two](https://a.example/2.png \"Two\")](https://a.example/page) then [a link](https://a.example/other) and ![broken] (x).";
        assert_eq!(
            markdown_image_urls(text),
            ["https://a.example/1.png", "https://a.example/2.png"]
        );
    }

    #[test]
    fn a_session_remembers_the_images_its_opened_pages_showed() {
        let session = WebResearchSession::default();
        let signed = "https://cdn.example/hero.jpg?sig=abc";
        session.cache_page(test_page(
            "https://news.example/story",
            &format!("# Story\n\n![Hero]({signed})\n\nText."),
            Some("Whole page ![Chart](https://cdn.example/chart.png) and more."),
        ));
        let images = session.page_image_urls();
        assert_eq!(images.len(), 2);
        for url in [signed, "https://cdn.example/chart.png"] {
            assert!(images.contains(&comparable_url(url).unwrap()), "{url}");
        }
        assert!(session.opened_urls().is_empty());
        let restored = WebResearchSession::restore(session.snapshot()).unwrap();
        assert!(restored.page_image_urls().is_empty());
    }

    #[test]
    fn output_windows_are_unicode_safe_and_say_where_to_read_on() {
        let window = char_window("aé🙂z", 0, 3);
        assert!(
            window
                .text
                .starts_with("aé🙂\n\n[Showing characters 0-3 of 4.")
        );
        assert_eq!((window.start, window.total, window.next), (0, 4, Some(3)));
        let rest = char_window("aé🙂z", 3, 3);
        assert_eq!((rest.text.as_str(), rest.next), ("z", None));
        let past_the_end = char_window("aé🙂z", 9, 3);
        assert_eq!((past_the_end.text.as_str(), past_the_end.start), ("", 4));
        assert_eq!(
            remove_control_characters("left\u{202e}right\u{200b}".to_string()),
            "leftright"
        );
    }

    #[test]
    fn a_page_is_read_in_windows_and_its_article_hides_the_rest_until_asked() {
        let url = "https://example.com/story";
        let full = format!(
            "Menu Menu Menu\n\n{}\n\nRelated: figure 4711",
            "x".repeat(3_000)
        );
        let page = test_page(url, &"x".repeat(3_000), Some(&full));

        let first = page_json(
            &page,
            &PageRequest::parse(&json!({"max_chars": 1_000})).unwrap(),
        );
        assert_eq!(first["extraction"], "main_content");
        assert_eq!(first["full_page_chars"], full.chars().count());
        assert_eq!(
            (&first["offset"], &first["next_offset"]),
            (&json!(0), &json!(1_000))
        );
        assert_eq!(first["total_chars"], 3_000);
        assert_eq!(first["truncated"], true);

        let last = page_json(
            &page,
            &PageRequest::parse(&json!({"max_chars": 1_000, "offset": 2_000})).unwrap(),
        );
        assert_eq!(last["content"], "x".repeat(1_000));
        assert!(last.get("next_offset").is_none());
        assert_eq!(last["truncated"], false);

        let found = page_json(
            &page,
            &PageRequest::parse(&json!({"find": "figure 4711"})).unwrap(),
        );
        assert_eq!(found["find"]["match_count"], 1, "find reads the whole page");
        assert!(!found["content"].as_str().unwrap().contains("4711"));

        let whole = page_json(
            &page,
            &PageRequest::parse(&json!({"full_page": true, "max_chars": 100_000})).unwrap(),
        );
        assert_eq!(whole["extraction"], "full_page");
        assert!(whole["content"].as_str().unwrap().ends_with("figure 4711"));
    }

    #[test]
    fn opened_pages_are_cached_per_turn_and_the_oldest_leave_first() {
        let session = WebResearchSession::new("");
        let first = session.cache_page(test_page("https://example.com/0", "zero", None));
        assert!(Arc::ptr_eq(
            &session.cached_page("https://example.com/0").unwrap(),
            &first
        ));
        for index in 1..=MAX_CACHED_PAGES {
            session.cache_page(test_page(
                &format!("https://example.com/{index}"),
                "page",
                None,
            ));
        }
        assert!(session.cached_page("https://example.com/0").is_none());
        assert!(session.cached_page("https://example.com/1").is_some());
    }

    #[test]
    fn an_article_is_extracted_with_its_date_and_the_page_stays_behind_it() {
        let paragraph = "The carmaker cut its margin forecast for the second time this year, citing restructuring costs and weaker demand in China. ";
        let html = format!(
            r#"<html><head><title>Carmaker cuts outlook | Example News</title>
            <meta property="article:published_time" content="2026-11-04T11:58:19Z">
            <meta name="author" content="Jo Writer"></head>
            <body><div class="menu"><a href="/a">Home</a> <a href="/b">Business</a> <a href="/c">Sport</a></div>
            <article><h1>Carmaker cuts outlook</h1>{}</article>
            <div class="related"><a href="/x">Ten cars you will not believe exist, with a long teaser to follow</a></div>
            </body></html>"#,
            format!("<p>{paragraph}</p>").repeat(8)
        );
        let converted =
            convert_html(&html, &Url::parse("https://example.com/story").unwrap()).unwrap();
        assert!(converted.text.contains("margin forecast"));
        assert!(!converted.text.contains("Ten cars"));
        assert_eq!(converted.published.as_deref(), Some("2026-11-04T11:58:19Z"));
        assert!(converted.text.contains("Published 2026-11-04T11:58:19Z"));
        let full = converted.full_text.expect("the whole page stays readable");
        assert!(full.contains("Ten cars") && full.contains("Business"));

        let listing = "<html><body><ul><li><a href='/1'>One</a></li><li><a href='/2'>Two</a></li></ul><p>Short.</p></body></html>";
        let whole = convert_html(listing, &Url::parse("https://example.com/").unwrap()).unwrap();
        assert!(whole.full_text.is_none() && whole.text.contains("One"));
    }

    #[test]
    fn a_reader_is_tried_only_for_failures_another_client_can_get_past() {
        let failure = |code: &'static str, status: Option<u16>| {
            let failure = FetchFailure::new(code, "x");
            match status {
                Some(status) => failure.with_status(status),
                None => failure,
            }
        };
        for (code, status) in [
            ("http_error", Some(403)),
            ("http_error", Some(401)),
            ("http_error", Some(503)),
            ("insufficient_text_content", None),
            ("unsupported_content_type", None),
            ("timeout", None),
        ] {
            assert!(failure(code, status).reader_may_help(), "{code} {status:?}");
        }
        for (code, status) in [
            ("http_error", Some(404)),
            ("http_error", Some(410)),
            ("url_not_authorized", None),
            ("invalid_url", None),
            ("blocked_address", None),
            ("redirect_blocked", None),
        ] {
            assert!(
                !failure(code, status).reader_may_help(),
                "{code} {status:?}"
            );
        }

        let requested = Url::parse("https://example.com/blocked").unwrap();
        let page = page_from_reader(
            requested.clone(),
            ReaderPage {
                reader: "firecrawl",
                text: "# Read elsewhere\n\nThe page text as the reader returned it.".into(),
                final_url: Some("http://127.0.0.1/internal".into()),
                ..ReaderPage::default()
            },
        )
        .unwrap();
        assert_eq!(
            page.final_url, requested,
            "a reader cannot name a private page"
        );
        assert_eq!(page.title, "Read elsewhere");
        assert_eq!(page_json(&page, &whole_page())["reader"], "firecrawl");
        assert!(page_from_reader(requested, ReaderPage::default()).is_err());
    }
}
