//! A bot owner's own web providers for research: hosted search and page-reader APIs paid for with
//! a key in the bot's secrets. Storing a key under its provider's name switches that provider on;
//! nothing else configures it. A key goes to its vendor's API host and nowhere else.
//!
//! Search uses the first provider with a key and falls back to the built-in search when it fails
//! or finds nothing. Readers are tried only after the plain request for a page was refused or
//! came back without text.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use flow_like_types::{reqwest, tokio::sync::Mutex};
use serde_json::{Value, json};
use url::Url;

use super::platform::{
    PageReader, PlatformToolBridge, ReaderPage, SearchProvider, SearchRequest, WebAccess,
    sanitize_search_text, search_result_row,
};
use super::public_web::{
    normalize_public_discovery_url, read_bounded_body_with_limit, source_id_for_url,
};
use super::tool_spec::INTERNET_SEARCH_TOOL;

const MAX_PROVIDER_BYTES: usize = 8 * 1024 * 1024;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const READ_TIMEOUT_MS: u64 = 45_000;
const MAX_TITLE_CHARS: usize = 300;
const MAX_SNIPPET_CHARS: usize = 600;
const MAX_ERROR_CHARS: usize = 200;

/// What one stored secret switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebProviderKey {
    /// The secret's name in the bot's secret store.
    pub secret: &'static str,
    pub provider: &'static str,
    pub searches: bool,
    pub reads: bool,
}

/// Every provider key a bot's research understands, for hosts and UIs that offer them.
pub const WEB_PROVIDER_KEYS: [WebProviderKey; 5] = [
    Provider::Firecrawl.key(),
    Provider::Tavily.key(),
    Provider::Exa.key(),
    Provider::Jina.key(),
    Provider::Brave.key(),
];

/// Readers in the order they are tried: the ones that render pages and get past bot walls first.
const READERS: [Provider; 4] = [
    Provider::Firecrawl,
    Provider::Tavily,
    Provider::Jina,
    Provider::Exa,
];

/// Search providers in the order of preference.
const SEARCHERS: [Provider; 4] = [
    Provider::Tavily,
    Provider::Brave,
    Provider::Exa,
    Provider::Firecrawl,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Provider {
    Firecrawl,
    Tavily,
    Jina,
    Exa,
    Brave,
}

impl Provider {
    const fn name(self) -> &'static str {
        match self {
            Self::Firecrawl => "firecrawl",
            Self::Tavily => "tavily",
            Self::Jina => "jina",
            Self::Exa => "exa",
            Self::Brave => "brave",
        }
    }

    const fn secret(self) -> &'static str {
        match self {
            Self::Firecrawl => "FIRECRAWL_API_KEY",
            Self::Tavily => "TAVILY_API_KEY",
            Self::Jina => "JINA_API_KEY",
            Self::Exa => "EXA_API_KEY",
            Self::Brave => "BRAVE_SEARCH_API_KEY",
        }
    }

    const fn key(self) -> WebProviderKey {
        WebProviderKey {
            secret: self.secret(),
            provider: self.name(),
            searches: !matches!(self, Self::Jina),
            reads: !matches!(self, Self::Brave),
        }
    }
}

/// A provider key. It has no `Debug`, so it cannot end up in a log line by accident.
#[derive(Clone)]
struct ApiKey(Arc<str>);

/// One search hit as a provider returned it, before it is checked and trimmed for the model.
#[derive(Debug, Default, PartialEq, Eq)]
struct Hit {
    title: String,
    url: String,
    snippet: Option<String>,
    date: Option<String>,
}

/// The web providers of one bot turn. Keys are read once, through the host's bridge.
pub struct BotWebProviders {
    bridge: Arc<dyn PlatformToolBridge>,
    keys: Mutex<HashMap<Provider, Option<ApiKey>>>,
    client: reqwest::Client,
}

impl BotWebProviders {
    pub fn new(bridge: Arc<dyn PlatformToolBridge>) -> Self {
        Self {
            bridge,
            keys: Mutex::default(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Flow-Like-Research/1.0")
                .build()
                .unwrap_or_default(),
        }
    }

    /// The research backends of a bot run: its owner's keys, read through `bridge`.
    pub fn access(bridge: Arc<dyn PlatformToolBridge>) -> WebAccess {
        let providers = Arc::new(Self::new(bridge));
        WebAccess {
            search: Some(providers.clone()),
            reader: Some(providers),
        }
    }

    async fn key(&self, provider: Provider) -> Option<ApiKey> {
        let mut keys = self.keys.lock().await;
        if let Some(known) = keys.get(&provider) {
            return known.clone();
        }
        let key = self
            .bridge
            .bot_secret(provider.secret())
            .await
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| ApiKey(value.into()));
        keys.insert(provider, key.clone());
        key
    }

    /// Sends one request and returns its JSON body, or why the provider did not answer. The key
    /// is taken out of anything the provider echoes back.
    async fn send(
        &self,
        provider: Provider,
        key: &ApiKey,
        request: reqwest::RequestBuilder,
    ) -> Result<Value, String> {
        let name = provider.name();
        let response = request
            .send()
            .await
            .map_err(|error| format!("{name} could not be reached: {}", error.without_url()))?;
        let status = response.status();
        let (body, _) = read_bounded_body_with_limit(response, MAX_PROVIDER_BYTES)
            .await
            .map_err(|error| {
                format!("{name} sent an unreadable answer: {}", error.without_url())
            })?;
        let value = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(value);
        }
        let detail = provider_error(&value).replace(key.0.as_ref(), "[key]");
        Err(match status.as_u16() {
            401 | 403 => format!("{name} rejected the stored key (HTTP {status}){detail}"),
            402 | 432 | 433 => format!("{name} reports no credit left (HTTP {status}){detail}"),
            429 => format!("{name} is rate limiting this key (HTTP 429){detail}"),
            _ => format!("{name} answered HTTP {status}{detail}"),
        })
    }

    /// The request that reads `url` through `provider`; `None` for a provider that reads nothing.
    fn read_request(
        &self,
        provider: Provider,
        key: &ApiKey,
        url: &Url,
    ) -> Option<reqwest::RequestBuilder> {
        let url = url.as_str();
        Some(match provider {
            Provider::Firecrawl => self
                .client
                .post("https://api.firecrawl.dev/v2/scrape")
                .bearer_auth(&key.0)
                .json(&json!({
                    "url": url,
                    "formats": [{ "type": "markdown" }],
                    "onlyMainContent": true,
                    "timeout": READ_TIMEOUT_MS,
                })),
            Provider::Tavily => self
                .client
                .post("https://api.tavily.com/extract")
                .bearer_auth(&key.0)
                .json(&json!({ "urls": [url], "format": "markdown" })),
            Provider::Jina => self
                .client
                .post("https://r.jina.ai/")
                .bearer_auth(&key.0)
                .header(reqwest::header::ACCEPT, "application/json")
                .header("X-Md-Link-Style", "discarded")
                .header("X-Retain-Images", "none")
                .json(&json!({ "url": url })),
            Provider::Exa => self
                .client
                .post("https://api.exa.ai/contents")
                .header("x-api-key", key.0.as_ref())
                .json(&json!({ "urls": [url], "text": true })),
            Provider::Brave => return None,
        })
    }

    async fn read_with(
        &self,
        provider: Provider,
        key: &ApiKey,
        url: &Url,
    ) -> Result<ReaderPage, String> {
        let name = provider.name();
        let request = self
            .read_request(provider, key, url)
            .ok_or_else(|| format!("{name} does not read pages"))?;
        let body = self
            .send(provider, key, request.timeout(READ_TIMEOUT))
            .await?;
        let page = match provider {
            Provider::Firecrawl => firecrawl_page(&body),
            Provider::Tavily => tavily_page(&body),
            Provider::Jina => jina_page(&body),
            Provider::Exa | Provider::Brave => exa_page(&body),
        }?;
        Ok(ReaderPage {
            reader: name,
            ..page
        })
    }

    /// The request that searches through `provider`; `None` for a provider that searches nothing.
    fn search_request(
        &self,
        provider: Provider,
        key: &ApiKey,
        request: &SearchRequest,
    ) -> Option<reqwest::RequestBuilder> {
        Some(match provider {
            Provider::Tavily => self
                .client
                .post("https://api.tavily.com/search")
                .bearer_auth(&key.0)
                .json(&tavily_search(request)),
            Provider::Brave => self
                .client
                .get("https://api.search.brave.com/res/v1/web/search")
                .header("X-Subscription-Token", key.0.as_ref())
                .header(reqwest::header::ACCEPT, "application/json")
                .query(&brave_search(request)),
            Provider::Exa => self
                .client
                .post("https://api.exa.ai/search")
                .header("x-api-key", key.0.as_ref())
                .json(&exa_search(request)),
            Provider::Firecrawl => self
                .client
                .post("https://api.firecrawl.dev/v2/search")
                .bearer_auth(&key.0)
                .json(&firecrawl_search(request)),
            Provider::Jina => return None,
        })
    }

    async fn search_with(
        &self,
        provider: Provider,
        key: &ApiKey,
        request: &SearchRequest,
    ) -> Result<Vec<Hit>, String> {
        let call = self
            .search_request(provider, key, request)
            .ok_or_else(|| format!("{} does not search", provider.name()))?;
        let body = self
            .send(provider, key, call.timeout(SEARCH_TIMEOUT))
            .await?;
        Ok(match provider {
            Provider::Tavily => tavily_hits(&body),
            Provider::Brave => brave_hits(&body),
            Provider::Exa => exa_hits(&body),
            Provider::Firecrawl | Provider::Jina => firecrawl_hits(&body),
        })
    }
}

fn wants_news(request: &SearchRequest) -> bool {
    request.category == "news"
}

fn tavily_search(request: &SearchRequest) -> Value {
    let mut body = json!({
        "query": request.query,
        "max_results": request.limit,
        "topic": if wants_news(request) { "news" } else { "general" },
    });
    if !request.time_range.is_empty() {
        body["time_range"] = json!(request.time_range);
    }
    body
}

fn brave_search(request: &SearchRequest) -> Vec<(&str, String)> {
    let mut query = vec![
        ("q", request.query.clone()),
        ("count", request.limit.to_string()),
    ];
    if let Some(freshness) = brave_freshness(&request.time_range) {
        query.push(("freshness", freshness.to_string()));
    }
    if let Some(language) = two_letter_language(&request.language) {
        query.push(("search_lang", language));
    }
    query
}

fn exa_search(request: &SearchRequest) -> Value {
    let mut body = json!({
        "query": request.query,
        "numResults": request.limit,
        "contents": { "highlights": true },
    });
    if wants_news(request) {
        body["category"] = json!("news");
    }
    if let Some(days) = time_range_days(&request.time_range) {
        body["startPublishedDate"] =
            json!((chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339());
    }
    body
}

fn firecrawl_search(request: &SearchRequest) -> Value {
    let mut body = json!({
        "query": request.query,
        "limit": request.limit,
        "sources": [{ "type": if wants_news(request) { "news" } else { "web" } }],
    });
    if let Some(tbs) = google_time_filter(&request.time_range) {
        body["tbs"] = json!(tbs);
    }
    body
}

#[async_trait]
impl PageReader for BotWebProviders {
    async fn read(&self, url: &Url) -> Option<Result<ReaderPage, String>> {
        let mut failures = Vec::new();
        for provider in READERS {
            let Some(key) = self.key(provider).await else {
                continue;
            };
            match self.read_with(provider, &key, url).await {
                Ok(page) => return Some(Ok(page)),
                Err(error) => {
                    tracing::warn!(provider = provider.name(), %error, "Web reader failed");
                    failures.push(error);
                }
            }
        }
        (!failures.is_empty()).then(|| Err(failures.join("; ")))
    }
}

#[async_trait]
impl SearchProvider for BotWebProviders {
    async fn search(&self, request: &SearchRequest) -> Option<Value> {
        // The providers are asked for a first page only; paging stays with the built-in search.
        if request.page > 1 {
            return None;
        }
        for provider in SEARCHERS {
            let Some(key) = self.key(provider).await else {
                continue;
            };
            match self.search_with(provider, &key, request).await {
                Ok(hits) => {
                    let results = search_rows(hits, request.limit);
                    if !results.is_empty() {
                        return Some(search_json(provider, request, results));
                    }
                }
                Err(error) => {
                    tracing::warn!(provider = provider.name(), %error, "Web search provider failed");
                }
            }
        }
        None
    }
}

fn search_json(provider: Provider, request: &SearchRequest, results: Vec<Value>) -> Value {
    let mut output = json!({
        "status": "ok",
        "tool": INTERNET_SEARCH_TOOL,
        "query": request.query,
        "category": request.category,
        "provider": provider.name(),
        "page": 1,
        "searched_at": chrono::Utc::now().to_rfc3339(),
        "results": results,
        "citation_eligible": false,
        "untrusted_content": true,
    });
    if !request.time_range.is_empty() {
        output["time_range"] = json!(request.time_range);
    }
    output
}

/// Hits as the model reads them: public URLs only, each once, text trimmed and cleaned.
fn search_rows(hits: Vec<Hit>, limit: usize) -> Vec<Value> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter_map(|hit| {
            let url = normalize_public_discovery_url(&hit.url)?;
            if !seen.insert(url.clone()) {
                return None;
            }
            let domain = Url::parse(&url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string));
            Some(search_result_row(
                source_id_for_url(&url),
                sanitize_search_text(&hit.title, MAX_TITLE_CHARS),
                url,
                domain,
                hit.snippet
                    .map(|snippet| sanitize_search_text(&snippet, MAX_SNIPPET_CHARS)),
                hit.date.map(|date| sanitize_search_text(&date, 64)),
            ))
        })
        .take(limit)
        .collect()
}

fn text_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn owned_at(value: &Value, pointer: &str) -> Option<String> {
    text_at(value, pointer).map(str::to_string)
}

/// The message a provider sends with a failed request, whichever field it uses.
fn provider_error(body: &Value) -> String {
    ["/error", "/detail/error", "/message", "/detail"]
        .into_iter()
        .find_map(|pointer| text_at(body, pointer))
        .map(|text| {
            format!(
                ": {}",
                text.chars().take(MAX_ERROR_CHARS).collect::<String>()
            )
        })
        .unwrap_or_default()
}

fn hits_at(body: &Value, pointer: &str, read: impl Fn(&Value) -> Option<Hit>) -> Vec<Hit> {
    body.pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(read)
        .collect()
}

fn hit(row: &Value, title: &str, snippet: &str, date: &str) -> Option<Hit> {
    Some(Hit {
        url: owned_at(row, "/url")?,
        title: owned_at(row, title).unwrap_or_else(|| "Untitled source".into()),
        snippet: owned_at(row, snippet),
        date: owned_at(row, date),
    })
}

fn tavily_hits(body: &Value) -> Vec<Hit> {
    hits_at(body, "/results", |row| {
        hit(row, "/title", "/content", "/published_date")
    })
}

fn brave_hits(body: &Value) -> Vec<Hit> {
    hits_at(body, "/web/results", |row| {
        hit(row, "/title", "/description", "/page_age")
    })
}

fn exa_hits(body: &Value) -> Vec<Hit> {
    hits_at(body, "/results", |row| {
        let highlights = row
            .get("highlights")
            .and_then(Value::as_array)
            .map(|highlights| {
                highlights
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" … ")
            })
            .filter(|text| !text.trim().is_empty());
        let found = hit(row, "/title", "/summary", "/publishedDate")?;
        Some(Hit {
            snippet: highlights.or_else(|| found.snippet.clone()),
            ..found
        })
    })
}

fn firecrawl_hits(body: &Value) -> Vec<Hit> {
    let mut hits = hits_at(body, "/data/news", |row| {
        hit(row, "/title", "/snippet", "/date")
    });
    hits.extend(hits_at(body, "/data/web", |row| {
        hit(row, "/title", "/description", "/date")
    }));
    hits
}

fn page_text(text: Option<&str>, provider: &str) -> Result<String, String> {
    text.map(str::to_string)
        .ok_or_else(|| format!("{provider} returned no page text"))
}

fn firecrawl_page(body: &Value) -> Result<ReaderPage, String> {
    let data = &body["data"];
    if let Some(status) = data
        .pointer("/metadata/statusCode")
        .and_then(Value::as_u64)
        .filter(|status| *status >= 400)
    {
        return Err(format!(
            "firecrawl reached the page, which answered HTTP {status}"
        ));
    }
    // Firecrawl reports several <title> elements as an array.
    let title = owned_at(data, "/metadata/title").or_else(|| owned_at(data, "/metadata/title/0"));
    Ok(ReaderPage {
        text: page_text(text_at(data, "/markdown"), "firecrawl")?,
        title,
        final_url: owned_at(data, "/metadata/url"),
        ..ReaderPage::default()
    })
}

fn tavily_page(body: &Value) -> Result<ReaderPage, String> {
    match text_at(body, "/results/0/raw_content") {
        Some(text) => Ok(ReaderPage {
            text: text.to_string(),
            final_url: owned_at(body, "/results/0/url"),
            ..ReaderPage::default()
        }),
        None => Err(format!(
            "tavily could not extract the page{}",
            text_at(body, "/failed_results/0/error")
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )),
    }
}

fn jina_page(body: &Value) -> Result<ReaderPage, String> {
    Ok(ReaderPage {
        text: page_text(text_at(body, "/data/content"), "jina")?,
        title: owned_at(body, "/data/title"),
        final_url: owned_at(body, "/data/url"),
        published: owned_at(body, "/data/publishedTime"),
        ..ReaderPage::default()
    })
}

fn exa_page(body: &Value) -> Result<ReaderPage, String> {
    if text_at(body, "/statuses/0/status") == Some("error") {
        return Err(format!(
            "exa could not fetch the page{}",
            text_at(body, "/statuses/0/error/tag")
                .map(|tag| format!(": {tag}"))
                .unwrap_or_default()
        ));
    }
    Ok(ReaderPage {
        text: page_text(text_at(body, "/results/0/text"), "exa")?,
        title: owned_at(body, "/results/0/title"),
        final_url: owned_at(body, "/results/0/url"),
        published: owned_at(body, "/results/0/publishedDate"),
        ..ReaderPage::default()
    })
}

fn time_range_days(time_range: &str) -> Option<i64> {
    match time_range {
        "day" => Some(1),
        "week" => Some(7),
        "month" => Some(31),
        "year" => Some(366),
        _ => None,
    }
}

fn brave_freshness(time_range: &str) -> Option<&'static str> {
    match time_range {
        "day" => Some("pd"),
        "week" => Some("pw"),
        "month" => Some("pm"),
        "year" => Some("py"),
        _ => None,
    }
}

fn google_time_filter(time_range: &str) -> Option<&'static str> {
    match time_range {
        "day" => Some("qdr:d"),
        "week" => Some("qdr:w"),
        "month" => Some("qdr:m"),
        "year" => Some("qdr:y"),
        _ => None,
    }
}

/// `de` from `de-DE`; nothing for `auto` or anything that is not a language code.
fn two_letter_language(language: &str) -> Option<String> {
    let code = language.split('-').next()?;
    (code.len() == 2
        && code
            .chars()
            .all(|character| character.is_ascii_alphabetic()))
    .then(|| code.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Secrets {
        stored: Vec<(&'static str, &'static str)>,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl PlatformToolBridge for Secrets {
        async fn call(&self, tool: &str, _: Value) -> String {
            panic!("web providers call no tool: {tool}")
        }

        async fn bot_secret(&self, name: &str) -> Option<String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.stored
                .iter()
                .find(|(stored, _)| *stored == name)
                .map(|(_, value)| value.to_string())
        }
    }

    fn request(args: Value) -> SearchRequest {
        SearchRequest::parse(&args).unwrap()
    }

    #[tokio::test]
    async fn a_bot_without_keys_uses_no_provider_and_asks_for_each_key_once() {
        let secrets = Arc::new(Secrets::default());
        let providers = BotWebProviders::new(secrets.clone());
        let url = Url::parse("https://example.com/blocked").unwrap();

        assert!(providers.read(&url).await.is_none());
        assert!(
            providers
                .search(&request(json!({ "query": "bmw" })))
                .await
                .is_none()
        );
        let after_first = secrets.reads.load(Ordering::SeqCst);
        assert_eq!(after_first, WEB_PROVIDER_KEYS.len());

        assert!(providers.read(&url).await.is_none());
        assert_eq!(secrets.reads.load(Ordering::SeqCst), after_first);
    }

    #[tokio::test]
    async fn a_blank_key_counts_as_no_key_and_keys_are_trimmed() {
        let providers = BotWebProviders::new(Arc::new(Secrets {
            stored: vec![("TAVILY_API_KEY", "  "), ("EXA_API_KEY", " exa-key\n")],
            ..Secrets::default()
        }));
        assert!(providers.key(Provider::Tavily).await.is_none());
        assert_eq!(
            providers.key(Provider::Exa).await.unwrap().0.as_ref(),
            "exa-key"
        );
    }

    #[test]
    fn every_key_names_one_provider_and_what_it_does() {
        let secrets: HashSet<_> = WEB_PROVIDER_KEYS.iter().map(|key| key.secret).collect();
        assert_eq!(secrets.len(), WEB_PROVIDER_KEYS.len());
        for key in WEB_PROVIDER_KEYS {
            assert!(flow_like_types::bot_message::is_secret_name(key.secret));
            assert!(key.searches || key.reads);
        }
        for provider in READERS {
            assert!(provider.key().reads, "{provider:?}");
        }
        for provider in SEARCHERS {
            assert!(provider.key().searches, "{provider:?}");
        }
    }

    #[test]
    fn firecrawl_and_tavily_pages_are_read_from_their_documented_shape() {
        let firecrawl = firecrawl_page(&json!({
            "success": true,
            "data": {
                "markdown": "# Outlook cut\n\nBody.",
                "metadata": {
                    "title": ["Outlook cut", "Share"],
                    "sourceURL": "https://example.com/a",
                    "url": "https://www.example.com/a",
                    "statusCode": 200
                }
            }
        }))
        .unwrap();
        assert_eq!(firecrawl.text, "# Outlook cut\n\nBody.");
        assert_eq!(firecrawl.title.as_deref(), Some("Outlook cut"));
        assert_eq!(
            firecrawl.final_url.as_deref(),
            Some("https://www.example.com/a")
        );
        let blocked = firecrawl_page(&json!({
            "success": true,
            "data": { "markdown": "Access denied", "metadata": { "statusCode": 403 } }
        }));
        assert!(blocked.unwrap_err().contains("HTTP 403"));

        let tavily = tavily_page(&json!({
            "results": [{ "url": "https://example.com/a", "raw_content": "Article text." }],
            "failed_results": []
        }))
        .unwrap();
        assert_eq!(tavily.text, "Article text.");
        let failed = tavily_page(&json!({
            "results": [],
            "failed_results": [{ "url": "https://example.com/a", "error": "Failed to retrieve content" }]
        }));
        assert!(failed.unwrap_err().contains("Failed to retrieve content"));
    }

    #[test]
    fn jina_and_exa_pages_carry_title_and_date_and_report_a_missing_page() {
        let jina = jina_page(&json!({
            "data": { "title": "A page", "url": "https://example.com/a", "content": "Text." }
        }))
        .unwrap();
        assert_eq!(
            (jina.text.as_str(), jina.title.as_deref()),
            ("Text.", Some("A page"))
        );
        assert!(jina_page(&json!({ "data": { "content": " " } })).is_err());

        let exa = exa_page(&json!({
            "results": [{
                "title": "A page",
                "url": "https://example.com/a",
                "publishedDate": "2026-11-04T00:00:00.000Z",
                "text": "Text."
            }],
            "statuses": [{ "id": "https://example.com/a", "status": "success", "source": "crawled" }]
        }))
        .unwrap();
        assert_eq!(exa.published.as_deref(), Some("2026-11-04T00:00:00.000Z"));
        let unreachable = exa_page(&json!({
            "results": [],
            "statuses": [{ "id": "x", "status": "error", "error": { "tag": "CRAWL_NOT_FOUND", "httpStatusCode": 404 } }]
        }));
        assert!(unreachable.unwrap_err().contains("CRAWL_NOT_FOUND"));
    }

    #[test]
    fn search_answers_are_read_from_each_vendor_s_documented_shape() {
        let tavily = tavily_hits(&json!({
            "results": [{
                "title": "BMW cuts outlook",
                "url": "https://example.com/bmw",
                "content": "Second warning this year.",
                "score": 0.81,
                "published_date": "Tue, 03 Nov 2026 17:00:00 GMT"
            }]
        }));
        assert_eq!(
            tavily,
            vec![Hit {
                title: "BMW cuts outlook".into(),
                url: "https://example.com/bmw".into(),
                snippet: Some("Second warning this year.".into()),
                date: Some("Tue, 03 Nov 2026 17:00:00 GMT".into()),
            }]
        );

        let brave = brave_hits(&json!({
            "web": { "results": [
                { "title": "A", "url": "https://example.com/a", "description": "About A." },
                { "title": "No link" }
            ] }
        }));
        assert_eq!(brave.len(), 1);
        assert_eq!(brave[0].snippet.as_deref(), Some("About A."));

        let exa = exa_hits(&json!({
            "results": [{
                "title": "B",
                "url": "https://example.com/b",
                "publishedDate": "2026-11-04T00:00:00.000Z",
                "highlights": ["First.", "Second."]
            }]
        }));
        assert_eq!(exa[0].snippet.as_deref(), Some("First. … Second."));
        assert_eq!(exa[0].date.as_deref(), Some("2026-11-04T00:00:00.000Z"));

        let firecrawl = firecrawl_hits(&json!({
            "success": true,
            "data": {
                "web": [{ "url": "https://example.com/w", "title": "W", "description": "Web." }],
                "news": [{ "url": "https://example.com/n", "title": "N", "snippet": "News.", "date": "2026-11-04" }]
            }
        }));
        assert_eq!(
            firecrawl
                .iter()
                .map(|hit| hit.title.as_str())
                .collect::<Vec<_>>(),
            ["N", "W"]
        );
    }

    #[test]
    fn hits_reach_the_model_public_deduplicated_and_trimmed() {
        let rows = search_rows(
            vec![
                Hit {
                    title: "Kept\u{202e}".into(),
                    url: "https://EXAMPLE.com/a#part".into(),
                    snippet: Some("x".repeat(2_000)),
                    date: Some("2026-11-04".into()),
                },
                Hit {
                    title: "Same page".into(),
                    url: "https://example.com/a".into(),
                    ..Hit::default()
                },
                Hit {
                    title: "Private".into(),
                    url: "http://127.0.0.1/admin".into(),
                    ..Hit::default()
                },
                Hit {
                    title: "Second".into(),
                    url: "https://example.com/b".into(),
                    ..Hit::default()
                },
                Hit {
                    title: "Beyond the limit".into(),
                    url: "https://example.com/c".into(),
                    ..Hit::default()
                },
            ],
            2,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["url"], "https://example.com/a");
        assert_eq!(rows[0]["title"], "Kept");
        assert_eq!(rows[0]["publishedDate"], "2026-11-04");
        assert!(rows[0]["content"].as_str().unwrap().chars().count() <= MAX_SNIPPET_CHARS);
        assert_eq!(rows[1]["url"], "https://example.com/b");
        assert!(rows[1].get("content").is_none());

        let output = search_json(
            Provider::Tavily,
            &request(json!({ "query": "bmw", "category": "news", "time_range": "week" })),
            rows,
        );
        assert_eq!(output["provider"], "tavily");
        assert_eq!(output["tool"], INTERNET_SEARCH_TOOL);
        assert_eq!(
            (&output["category"], &output["time_range"]),
            (&json!("news"), &json!("week"))
        );
    }

    #[test]
    fn provider_errors_are_short_and_request_filters_map_to_each_vendor() {
        assert_eq!(
            provider_error(
                &json!({ "detail": { "error": "Unauthorized: missing or invalid API key." } })
            ),
            ": Unauthorized: missing or invalid API key."
        );
        assert_eq!(
            provider_error(&json!({ "error": "x".repeat(900) }))
                .chars()
                .count(),
            202
        );
        assert_eq!(provider_error(&Value::Null), "");

        assert_eq!(brave_freshness("week"), Some("pw"));
        assert_eq!(google_time_filter("day"), Some("qdr:d"));
        assert_eq!(time_range_days("month"), Some(31));
        assert_eq!(brave_freshness(""), None);
        assert_eq!(two_letter_language("de-DE").as_deref(), Some("de"));
        assert_eq!(two_letter_language("auto"), None);

        let news = request(json!({
            "query": "bmw",
            "category": "news",
            "time_range": "week",
            "language": "de-DE",
            "limit": 5,
        }));
        assert_eq!(
            tavily_search(&news),
            json!({ "query": "bmw", "max_results": 5, "topic": "news", "time_range": "week" })
        );
        assert_eq!(
            brave_search(&news),
            vec![
                ("q", "bmw".to_string()),
                ("count", "5".to_string()),
                ("freshness", "pw".to_string()),
                ("search_lang", "de".to_string()),
            ]
        );
        assert_eq!(
            firecrawl_search(&news),
            json!({ "query": "bmw", "limit": 5, "sources": [{ "type": "news" }], "tbs": "qdr:w" })
        );
        let exa = exa_search(&news);
        assert_eq!(exa["category"], "news");
        assert!(exa["startPublishedDate"].is_string());

        let plain = request(json!({ "query": "bmw" }));
        assert_eq!(
            tavily_search(&plain),
            json!({ "query": "bmw", "max_results": 8, "topic": "general" })
        );
        assert_eq!(brave_search(&plain).len(), 2);
        assert!(exa_search(&plain).get("category").is_none());
        assert!(firecrawl_search(&plain).get("tbs").is_none());
    }
}
