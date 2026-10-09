use std::{fmt, time::Duration};

use reqwest::{
    Client as HttpClient, Method, Response, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid client configuration: {0}")]
    Configuration(String),
    #[error("HTTP {status}: {message}")]
    Api {
        status: u16,
        message: String,
        body: Value,
        request_id: Option<String>,
        retry_after: Option<String>,
    },
    #[error("HTTP transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Invalid API response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid server stream: {0}")]
    Stream(String),
    #[error("Execution {run_id} ended with status {status}: {message}")]
    Execution {
        run_id: String,
        status: String,
        message: String,
    },
    #[error("Timed out waiting for execution")]
    Timeout,
}

impl Error {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
    pub fn is_auth(&self) -> bool {
        matches!(self.status(), Some(401 | 403))
    }
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }
}

#[derive(Clone)]
pub enum Auth {
    PersonalAccessToken(String),
    ApiKey(String),
    Bearer(String),
    Anonymous,
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PersonalAccessToken(_) => "PersonalAccessToken([redacted])",
            Self::ApiKey(_) => "ApiKey([redacted])",
            Self::Bearer(_) => "Bearer([redacted])",
            Self::Anonymous => "Anonymous",
        })
    }
}

impl Auth {
    pub fn from_token(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        if token.starts_with("pat_") {
            Ok(Self::PersonalAccessToken(token))
        } else if token.starts_with("flk_") {
            Ok(Self::ApiKey(token))
        } else {
            Err(Error::Configuration("Expected a pat_ or flk_ token".into()))
        }
    }

    pub fn from_env() -> Result<Self> {
        let pat = std::env::var("FLOW_LIKE_PAT")
            .ok()
            .filter(|value| !value.is_empty());
        let api_key = std::env::var("FLOW_LIKE_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        let token = match (pat, api_key) {
            (Some(_), Some(_)) => {
                return Err(Error::Configuration(
                    "Set only one of FLOW_LIKE_PAT and FLOW_LIKE_API_KEY".into(),
                ));
            }
            (Some(token), None) | (None, Some(token)) => token,
            (None, None) => {
                return Err(Error::Configuration(
                    "Set FLOW_LIKE_PAT or FLOW_LIKE_API_KEY".into(),
                ));
            }
        };
        Self::from_token(token)
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let (name, token) = match self {
            Self::Anonymous => return Ok(headers),
            Self::ApiKey(token) => ("x-api-key", token.clone()),
            Self::PersonalAccessToken(token) => ("authorization", token.clone()),
            Self::Bearer(token) => ("authorization", format!("Bearer {token}")),
        };
        let mut value = HeaderValue::from_str(&token)
            .map_err(|_| Error::Configuration("Invalid credential header".into()))?;
        value.set_sensitive(true);
        headers.insert(name, value);
        Ok(headers)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RequestOptions {
    pub body: Option<Value>,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
}

impl RequestOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn json(mut self, body: impl Into<Value>) -> Self {
        self.body = Some(body.into());
        self
    }
    pub fn query(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.query.push((key.into(), value.to_string()));
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
}

#[derive(Clone)]
pub struct Client {
    pub(crate) http: HttpClient,
    base_url: Url,
    auth: HeaderMap,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

pub struct ClientBuilder {
    base_url: String,
    auth: Auth,
    timeout: Duration,
    connect_timeout: Duration,
    user_agent: String,
    board_format_version: u32,
}

impl ClientBuilder {
    /// Board JSON remains opaque to this client. Set the format understood by your caller.
    pub fn board_format_version(mut self, version: u32) -> Self {
        self.board_format_version = version;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }
    pub fn build(self) -> Result<Client> {
        if self.board_format_version == 0 {
            return Err(Error::Configuration(
                "Board format version must be positive".into(),
            ));
        }
        let mut base_url = Url::parse(&self.base_url)
            .map_err(|_| Error::Configuration("Invalid base URL".into()))?;
        validate_url(&base_url)?;
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(Error::Configuration(
                "Base URL cannot contain query parameters or a fragment".into(),
            ));
        }
        let path = base_url.path().trim_end_matches('/');
        let path = if path.ends_with("/api/v1") {
            format!("{path}/")
        } else {
            format!("{path}/api/v1/")
        };
        base_url.set_path(&path);
        let http = HttpClient::builder()
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(self.user_agent)
            .build()?;
        let mut auth = self.auth.headers()?;
        auth.insert(
            "x-flow-like-board-format",
            HeaderValue::from(self.board_format_version),
        );
        Ok(Client {
            http,
            base_url,
            auth,
        })
    }
}

pub(crate) fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Configuration(
            "Expected an HTTP(S) URL without embedded credentials".into(),
        ));
    }
    Ok(())
}

impl Client {
    pub fn builder(base_url: impl Into<String>, auth: Auth) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.into(),
            auth,
            timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(10),
            user_agent: format!("flow-like-platform-rust/{}", env!("CARGO_PKG_VERSION")),
            board_format_version: 2,
        }
    }
    pub fn new(base_url: impl Into<String>, auth: Auth) -> Result<Self> {
        Self::builder(base_url, auth).build()
    }
    pub fn from_env() -> Result<Self> {
        Self::new(
            std::env::var("FLOW_LIKE_BASE_URL")
                .map_err(|_| Error::Configuration("Set FLOW_LIKE_BASE_URL".into()))?,
            Auth::from_env()?,
        )
    }
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub(crate) fn url(&self, segments: &[&str]) -> Result<Url> {
        if segments
            .iter()
            .any(|s| s.is_empty() || *s == "." || *s == "..")
        {
            return Err(Error::Configuration(
                "Path segments cannot be empty, '.' or '..'".into(),
            ));
        }
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Configuration("Invalid base URL".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// Calls an API route with each dynamic path component encoded as one segment.
    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        segments: &[&str],
        options: &RequestOptions,
    ) -> Result<T> {
        let response = self.request_raw(method, segments, options).await?;
        decode(response).await
    }

    pub async fn request_raw(
        &self,
        method: Method,
        segments: &[&str],
        options: &RequestOptions,
    ) -> Result<Response> {
        if options.headers.contains_key(AUTHORIZATION) || options.headers.contains_key("x-api-key")
        {
            return Err(Error::Configuration(
                "Configure authentication on the client, not RequestOptions".into(),
            ));
        }
        let mut request = self
            .http
            .request(method, self.url(segments)?)
            .headers(self.auth.clone())
            .headers(options.headers.clone())
            .query(&options.query);
        if let Some(body) = &options.body {
            request = request.json(body);
        }
        if let Some(timeout) = options.timeout {
            request = request.timeout(timeout);
        }
        checked(request.send().await?).await
    }
}

pub(crate) async fn checked(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let text = response.text().await?;
    let body = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| Value::String(text.clone()));
    let message = body
        .get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or(&text)
        .to_owned();
    Err(Error::Api {
        status,
        message,
        body,
        request_id,
        retry_after,
    })
}

pub(crate) async fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
    let bytes = response.bytes().await?;
    Ok(serde_json::from_slice(if bytes.is_empty() {
        b"null"
    } else {
        &bytes
    })?)
}
