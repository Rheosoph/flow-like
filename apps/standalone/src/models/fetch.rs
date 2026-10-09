use anyhow::Context as _;
use flow_like_device_protocol::{DigestAlgorithm, ModelAssetDescriptor, ModelAssetFailure};
use reqwest::{
    StatusCode,
    dns::{Addrs, Name, Resolve, Resolving},
    header::{ACCEPT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE},
};
use sha2::{Digest, Sha256};
use std::{
    error::Error as _,
    fs::File,
    io::{Read, Seek, SeekFrom},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

pub const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const BASE_DEADLINE: Duration = Duration::from_secs(300);
const MINIMUM_BYTES_PER_SECOND: u64 = 32 * 1024;
/// However large the asset, one attempt ends after a day; a source that kept the minimum
/// rate goes on in its next attempt.
const MAX_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);
const RATE_FLOOR: RateFloor = RateFloor {
    window: Duration::from_secs(5 * 60),
    min_bytes: 1024 * 1024,
};
const SYNC_INTERVAL_BYTES: u64 = 64 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 1024 * 1024;

/// Which addresses a fetch may connect to. Production permits global unicast only.
#[derive(Clone, Debug, Default)]
pub struct AddressPolicy {
    exempt: Vec<IpAddr>,
}

impl AddressPolicy {
    pub fn global_only() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn allowing(addresses: impl IntoIterator<Item = IpAddr>) -> Self {
        Self {
            exempt: addresses.into_iter().map(canonical).collect(),
        }
    }

    pub fn permits(&self, address: IpAddr) -> bool {
        let address = canonical(address);
        self.exempt.contains(&address) || is_global(address)
    }
}

fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        IpAddr::V4(_) => address,
    }
}

fn is_global(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => is_global_v6(v6),
    }
}

/// Special-purpose blocks: this network, private, shared (CGNAT), loopback, link-local
/// (cloud metadata included), IETF and documentation ranges, the 6to4 relay, benchmarking,
/// and everything from multicast up to broadcast.
const NON_GLOBAL_V4: [(Ipv4Addr, u32); 14] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 3),
];
const GLOBAL_UNICAST_V6: (Ipv6Addr, u32) = (Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3);
/// The well-known NAT64 prefix carries an IPv4 address that decides instead.
const NAT64_V6: (Ipv6Addr, u32) = (Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96);
/// Inside global unicast: IETF assignments (Teredo included), documentation and 6to4.
const NON_GLOBAL_V6: [(Ipv6Addr, u32); 4] = [
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
    (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
    (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
];

fn within(address: u128, network: u128, prefix: u32, width: u32) -> bool {
    let shift = width - prefix;
    address >> shift == network >> shift
}

fn is_global_v4(address: Ipv4Addr) -> bool {
    let bits = u128::from(u32::from(address));
    !NON_GLOBAL_V4
        .iter()
        .any(|(network, prefix)| within(bits, u128::from(u32::from(*network)), *prefix, 32))
}

fn in_block_v6(address: u128, (network, prefix): (Ipv6Addr, u32)) -> bool {
    within(address, u128::from(network), prefix, 128)
}

fn is_global_v6(address: Ipv6Addr) -> bool {
    let bits = u128::from(address);
    if in_block_v6(bits, NAT64_V6) {
        return is_global_v4(Ipv4Addr::from(bits as u32));
    }
    in_block_v6(bits, GLOBAL_UNICAST_V6)
        && !NON_GLOBAL_V6.iter().any(|block| in_block_v6(bits, *block))
}

#[derive(Debug)]
pub struct AddressRefused {
    pub host: String,
    pub address: IpAddr,
}

impl std::fmt::Display for AddressRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "{} resolves to {}, which is not a public address",
            self.host, self.address
        )
    }
}

impl std::error::Error for AddressRefused {}

/// Name lookups for fetches; the connector never sees an address outside the policy.
struct GuardedResolver {
    policy: AddressPolicy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(resolve_permitted(
            self.policy.clone(),
            name.as_str().to_owned(),
        ))
    }
}

/// Refuses the whole name when any of its addresses is outside the policy.
async fn resolve_permitted(
    policy: AddressPolicy,
    host: String,
) -> Result<Addrs, Box<dyn std::error::Error + Send + Sync>> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
    if let Some(refused) = addresses
        .iter()
        .find(|address| !policy.permits(address.ip()))
    {
        let address = refused.ip();
        return Err(Box::new(AddressRefused { host, address }));
    }
    Ok(Box::new(addresses.into_iter()))
}

pub(crate) enum StreamDigest {
    Sha256(Sha256),
    Blake3(Box<blake3::Hasher>),
}

impl StreamDigest {
    pub(crate) fn new(algorithm: DigestAlgorithm) -> Self {
        match algorithm {
            DigestAlgorithm::Sha256 => Self::Sha256(Sha256::new()),
            DigestAlgorithm::Blake3 => Self::Blake3(Box::default()),
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(digest) => digest.update(bytes),
            Self::Blake3(digest) => {
                digest.update(bytes);
            }
        }
    }

    pub(crate) fn hex(&self) -> String {
        match self {
            Self::Sha256(digest) => format!("{:x}", digest.clone().finalize()),
            Self::Blake3(digest) => digest.finalize().to_hex().to_string(),
        }
    }
}

fn hash_into(file: &mut File, limit: u64, digest: &mut StreamDigest) -> std::io::Result<u64> {
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    let mut reader = file.take(limit);
    let mut total = 0u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(total);
        }
        digest.update(&buffer[..count]);
        total += count as u64;
    }
}

/// Hashes the first `size` bytes; `None` when the file is shorter.
fn prefix_digest(
    file: &mut File,
    algorithm: DigestAlgorithm,
    size: u64,
) -> std::io::Result<Option<StreamDigest>> {
    file.seek(SeekFrom::Start(0))?;
    let mut digest = StreamDigest::new(algorithm);
    Ok((hash_into(file, size, &mut digest)? == size).then_some(digest))
}

pub(crate) fn file_digest(
    file: &mut File,
    algorithm: DigestAlgorithm,
    size: u64,
) -> std::io::Result<Option<String>> {
    Ok(prefix_digest(file, algorithm, size)?.map(|digest| digest.hex()))
}

#[derive(Debug)]
pub enum FetchError {
    /// The address, scheme or credentials of the source or a redirect are not allowed.
    Refused(String),
    /// DNS, connect, TLS, timeouts and dropped connections.
    Network(String),
    Status {
        status: u16,
        message: String,
    },
    SizeMismatch(String),
    /// The source stayed under the rate floor or the minimum rate. It is not tried again:
    /// another attempt would hold a download slot just as long.
    TooSlow(String),
    /// `resumed` when the bytes came partly from an earlier transfer, so a fresh
    /// transfer from the same source may still succeed.
    DigestMismatch {
        message: String,
        resumed: bool,
    },
    /// The staging file could not be read or written.
    Local {
        message: String,
        storage_full: bool,
    },
    Cancelled,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Refused(message)
            | Self::Network(message)
            | Self::TooSlow(message)
            | Self::SizeMismatch(message)
            | Self::Status { message, .. }
            | Self::DigestMismatch { message, .. }
            | Self::Local { message, .. } => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("The model asset fetch was cancelled"),
        }
    }
}

impl std::error::Error for FetchError {}

impl FetchError {
    pub fn failure(&self) -> (ModelAssetFailure, Option<u16>) {
        let reason = match self {
            Self::Refused(_) | Self::Network(_) | Self::TooSlow(_) => {
                ModelAssetFailure::EgressBlocked
            }
            Self::Status { status, .. } => return (ModelAssetFailure::HttpStatus, Some(*status)),
            Self::SizeMismatch(_) => ModelAssetFailure::SizeMismatch,
            Self::DigestMismatch { .. } => ModelAssetFailure::DigestMismatch,
            Self::Local {
                storage_full: true, ..
            } => ModelAssetFailure::DiskBudget,
            Self::Local { .. } => ModelAssetFailure::Io,
            Self::Cancelled => ModelAssetFailure::Cancelled,
        };
        (reason, None)
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Status { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            Self::DigestMismatch { resumed, .. } => *resumed,
            _ => false,
        }
    }

    /// No other source can help: the device itself failed or the fetch was stopped.
    pub fn ends_job(&self) -> bool {
        matches!(self, Self::Local { .. } | Self::Cancelled)
    }
}

fn local(context: &str, error: std::io::Error) -> FetchError {
    FetchError::Local {
        storage_full: error.kind() == std::io::ErrorKind::StorageFull,
        message: format!("{context}: {error}"),
    }
}

fn host(url: &Url) -> &str {
    url.host_str().unwrap_or("an unnamed host")
}

fn chain(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        message.push_str(": ");
        message.push_str(&current.to_string());
        cause = current.source();
    }
    message
}

fn refused_address(error: &reqwest::Error) -> Option<String> {
    let mut cause = error.source();
    while let Some(current) = cause {
        if let Some(refused) = current.downcast_ref::<AddressRefused>() {
            return Some(refused.to_string());
        }
        cause = current.source();
    }
    None
}

fn deadline_for(size: u64) -> Duration {
    BASE_DEADLINE
        .saturating_add(Duration::from_secs(size / MINIMUM_BYTES_PER_SECOND))
        .min(MAX_DEADLINE)
}

/// An attempt that reached its deadline after `streamed` bytes. Only one that kept the
/// minimum rate, so that the day's cap ended it, gets another attempt.
fn expired(label: &str, size: u64, streamed: u64) -> FetchError {
    let limit = deadline_for(size);
    let message = format!("Fetch {label}: not complete within {} s", limit.as_secs());
    if streamed >= MINIMUM_BYTES_PER_SECOND.saturating_mul(limit.as_secs()) {
        FetchError::Network(message)
    } else {
        FetchError::TooSlow(message)
    }
}

/// What a source must send in every window of an attempt, the wait for its response
/// included, so one that trickles bytes cannot hold a download slot until a deadline sized
/// for the whole asset.
#[derive(Clone, Copy, Debug)]
struct RateFloor {
    window: Duration,
    min_bytes: u64,
}

/// The rate floor over consecutive windows of one attempt.
struct RateCheck {
    floor: RateFloor,
    opened: Instant,
    from: u64,
}

impl RateCheck {
    fn new(floor: RateFloor, opened: Instant, from: u64) -> Self {
        Self {
            floor,
            opened,
            from,
        }
    }

    /// False once a window is over with fewer bytes than the floor, or than the rest of
    /// the asset; a window that kept the floor opens the next one.
    fn keeps(&mut self, now: Instant, written: u64, size: u64) -> bool {
        if now.saturating_duration_since(self.opened) < self.floor.window {
            return true;
        }
        let wanted = self.floor.min_bytes.min(size.saturating_sub(self.from));
        if written.saturating_sub(self.from) < wanted {
            return false;
        }
        (self.opened, self.from) = (now, written);
        true
    }

    fn too_slow(&self, label: &str) -> FetchError {
        FetchError::TooSlow(format!(
            "Fetch {label}: the source sent under {} bytes in {} s",
            self.floor.min_bytes,
            self.floor.window.as_secs()
        ))
    }
}

fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let total = match total {
        "*" => None,
        total => Some(total.parse().ok()?),
    };
    Some((start.parse().ok()?, end.parse().ok()?, total))
}

fn continues_at(response: &reqwest::Response, held: u64, size: u64) -> bool {
    response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .is_some_and(|(start, end, total)| {
            start == held && end.checked_add(1) == Some(size) && total.is_none_or(|t| t == size)
        })
}

/// Where the response body starts in the asset: `held` for a continued range, 0 when
/// the source sends the whole asset.
fn body_offset(
    response: &reqwest::Response,
    held: u64,
    size: u64,
    label: &str,
) -> Result<u64, FetchError> {
    let offset = match response.status() {
        StatusCode::OK => 0,
        StatusCode::PARTIAL_CONTENT if held > 0 && continues_at(response, held, size) => held,
        StatusCode::PARTIAL_CONTENT => {
            return Err(FetchError::SizeMismatch(format!(
                "Fetch {label}: the partial response does not continue at byte {held} of {size}"
            )));
        }
        StatusCode::RANGE_NOT_SATISFIABLE if held > 0 => {
            return Err(FetchError::SizeMismatch(format!(
                "Fetch {label}: the source holds fewer than the {size} bytes declared"
            )));
        }
        status => {
            return Err(FetchError::Status {
                status: status.as_u16(),
                message: format!("Fetch {label}: HTTP {}", status.as_u16()),
            });
        }
    };
    let expected = size - offset;
    match response.content_length() {
        Some(length) if length != expected => Err(FetchError::SizeMismatch(format!(
            "Fetch {label}: the response announces {length} bytes, expected {expected}"
        ))),
        _ => Ok(offset),
    }
}

fn redirect_target(response: &reqwest::Response, url: &Url) -> Option<Url> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    url.join(location).ok()
}

enum Resume {
    Complete,
    From(File, StreamDigest, u64),
}

enum Staged {
    Partial(StreamDigest),
    Complete,
    Unusable,
}

/// Re-hashes the `held` staged bytes, so a resumed transfer is verified as a whole.
fn inspect_staged(
    file: &mut File,
    algorithm: DigestAlgorithm,
    held: u64,
    size: u64,
    expected: &str,
) -> std::io::Result<Staged> {
    if held == 0 || held > size {
        return Ok(Staged::Unusable);
    }
    Ok(match prefix_digest(file, algorithm, held)? {
        Some(digest) if held < size => Staged::Partial(digest),
        Some(digest) if digest.hex() == expected => Staged::Complete,
        _ => Staged::Unusable,
    })
}

fn resume_point(
    mut file: File,
    algorithm: DigestAlgorithm,
    size: u64,
    expected: &str,
) -> std::io::Result<Resume> {
    let held = file.metadata()?.len();
    match inspect_staged(&mut file, algorithm, held, size, expected)? {
        Staged::Partial(digest) => return Ok(Resume::From(file, digest, held)),
        Staged::Complete => return Ok(Resume::Complete),
        Staged::Unusable => {}
    }
    file.set_len(0)?;
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(Resume::From(file, StreamDigest::new(algorithm), 0))
}

async fn resume(file: File, asset: &ModelAssetDescriptor) -> Result<Resume, FetchError> {
    let label = format!("Resume model asset {}", asset.digest.store_key());
    let (algorithm, size, expected) =
        (asset.digest.algorithm, asset.size, asset.digest.hex.clone());
    tokio::task::spawn_blocking(move || resume_point(file, algorithm, size, &expected))
        .await
        .map_err(|error| FetchError::Local {
            message: format!("{label}: {error}"),
            storage_full: false,
        })?
        .map_err(|error| local(&label, error))
}

/// One response streamed into the staging file. Every exit flushes the file first, so
/// no write of an abandoned transfer can land after the next one starts.
struct Download {
    response: reqwest::Response,
    writer: tokio::fs::File,
    digest: StreamDigest,
    resumed: bool,
    /// Where this response's bytes start in the asset.
    offset: u64,
    written: u64,
    unsynced: u64,
    size: u64,
    label: String,
}

impl Download {
    async fn start(
        response: reqwest::Response,
        file: File,
        digest: StreamDigest,
        held: u64,
        asset: &ModelAssetDescriptor,
    ) -> Result<Self, FetchError> {
        let label = format!(
            "model asset {} from {}",
            asset.digest.store_key(),
            host(response.url())
        );
        let offset = body_offset(&response, held, asset.size, &label)?;
        let mut writer = tokio::fs::File::from_std(file);
        let digest = if offset == held {
            digest
        } else {
            writer
                .set_len(0)
                .await
                .map_err(|error| local(&format!("Restart {label}"), error))?;
            writer
                .seek(SeekFrom::Start(0))
                .await
                .map_err(|error| local(&format!("Restart {label}"), error))?;
            StreamDigest::new(asset.digest.algorithm)
        };
        Ok(Self {
            response,
            writer,
            digest,
            resumed: offset > 0,
            offset,
            written: offset,
            unsynced: 0,
            size: asset.size,
            label,
        })
    }

    /// Reads time out after a minute without bytes, so the rate floor is checked at least
    /// that often.
    async fn stream(
        &mut self,
        cancel: &CancellationToken,
        deadline: Instant,
        mut rate: RateCheck,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<(), FetchError> {
        let expiry = tokio::time::sleep_until(deadline);
        tokio::pin!(expiry);
        loop {
            let chunk = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(FetchError::Cancelled),
                () = &mut expiry => {
                    return Err(expired(&self.label, self.size, self.written - self.offset));
                }
                chunk = self.response.chunk() => chunk,
            };
            let chunk = chunk.map_err(|error| {
                FetchError::Network(format!(
                    "Fetch {}: stopped after {} of {} bytes: {}",
                    self.label,
                    self.written,
                    self.size,
                    chain(&error.without_url())
                ))
            })?;
            let Some(chunk) = chunk else {
                return Ok(());
            };
            self.write(&chunk).await?;
            progress(self.written);
            if !rate.keeps(Instant::now(), self.written, self.size) {
                return Err(rate.too_slow(&self.label));
            }
        }
    }

    async fn write(&mut self, chunk: &[u8]) -> Result<(), FetchError> {
        let written = self
            .written
            .checked_add(chunk.len() as u64)
            .filter(|written| *written <= self.size)
            .ok_or_else(|| {
                FetchError::SizeMismatch(format!(
                    "Fetch {}: the source sends more than the {} bytes declared",
                    self.label, self.size
                ))
            })?;
        let context = format!("Write {}", self.label);
        self.writer
            .write_all(chunk)
            .await
            .map_err(|error| local(&context, error))?;
        self.digest.update(chunk);
        self.written = written;
        self.unsynced += chunk.len() as u64;
        if self.unsynced >= SYNC_INTERVAL_BYTES {
            self.writer
                .sync_data()
                .await
                .map_err(|error| local(&context, error))?;
            self.unsynced = 0;
        }
        Ok(())
    }

    async fn finish(
        mut self,
        streamed: Result<(), FetchError>,
        expected: &str,
    ) -> Result<(), FetchError> {
        let context = format!("Store {}", self.label);
        self.writer
            .flush()
            .await
            .map_err(|error| local(&context, error))?;
        self.writer
            .sync_all()
            .await
            .map_err(|error| local(&context, error))?;
        if let Err(error) = streamed {
            if let FetchError::SizeMismatch(_) = error {
                self.discard().await;
            }
            return Err(error);
        }
        if self.written < self.size {
            return Err(FetchError::Network(format!(
                "Fetch {}: the source ended after {} of {} bytes",
                self.label, self.written, self.size
            )));
        }
        let actual = self.digest.hex();
        if actual == expected {
            return Ok(());
        }
        self.discard().await;
        Err(FetchError::DigestMismatch {
            message: format!(
                "Fetch {}: the bytes hash to {actual}, not the pinned {expected}",
                self.label
            ),
            resumed: self.resumed,
        })
    }

    async fn discard(&mut self) {
        let result = match self.writer.set_len(0).await {
            Ok(()) => self.writer.sync_all().await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!("Discard the staged bytes of {}: {error}", self.label);
        }
    }
}

/// HTTPS-only downloader for model assets with an address policy on every hop.
#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    policy: AddressPolicy,
    floor: RateFloor,
}

impl Fetcher {
    pub fn new(policy: AddressPolicy) -> anyhow::Result<Self> {
        Self::build(policy, |builder| builder)
    }

    #[cfg(test)]
    pub(crate) fn trusting(policy: AddressPolicy, certificate_pem: &str) -> anyhow::Result<Self> {
        let certificate = reqwest::Certificate::from_pem(certificate_pem.as_bytes())?;
        Self::build(policy, |builder| builder.add_root_certificate(certificate))
    }

    #[cfg(test)]
    pub(crate) fn with_rate_floor(mut self, window: Duration, min_bytes: u64) -> Self {
        self.floor = RateFloor { window, min_bytes };
        self
    }

    /// Proxies stay off: a proxy would resolve names itself, past the address policy.
    fn build(
        policy: AddressPolicy,
        configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(GuardedResolver {
                policy: policy.clone(),
            }))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(IDLE_TIMEOUT)
            .user_agent(concat!("flow-like-standalone/", env!("CARGO_PKG_VERSION")));
        let client = configure(builder)
            .build()
            .context("Build the model asset HTTP client")?;
        Ok(Self {
            client,
            policy,
            floor: RATE_FLOOR,
        })
    }

    /// Streams `source` into the staging `file` of `asset`, continuing after the bytes it
    /// already holds. Succeeds only when the file holds exactly the pinned bytes.
    pub async fn fetch(
        &self,
        source: &str,
        asset: &ModelAssetDescriptor,
        file: File,
        cancel: &CancellationToken,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<(), FetchError> {
        let key = asset.digest.store_key();
        let (file, digest, held) = match resume(file, asset).await? {
            Resume::Complete => {
                progress(asset.size);
                return Ok(());
            }
            Resume::From(file, digest, held) => (file, digest, held),
        };
        progress(held);
        let started = Instant::now();
        let deadline = started + deadline_for(asset.size);
        let silence = deadline.min(started + self.floor.window);
        let what = format!("model asset {key}");
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FetchError::Cancelled),
            () = tokio::time::sleep_until(silence) => {
                return Err(FetchError::TooSlow(format!(
                    "Fetch model asset {key}: no response within {} s",
                    silence.duration_since(started).as_secs()
                )));
            }
            response = self.request(source, held, &what) => response?,
        };
        let mut download = Download::start(response, file, digest, held, asset).await?;
        let rate = RateCheck::new(self.floor, started, download.written);
        let streamed = download.stream(cancel, deadline, rate, progress).await;
        download.finish(streamed, &asset.digest.hex).await
    }

    /// A small HTTPS resource, such as an image a chat request links, under the address policy
    /// and redirect checks of asset downloads: its content type and at most `limit` bytes.
    pub async fn fetch_small(
        &self,
        source: &str,
        what: &str,
        limit: u64,
    ) -> Result<(Option<String>, Vec<u8>), FetchError> {
        let mut response = self.request(source, 0, what).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Status {
                status: status.as_u16(),
                message: format!("Fetch {what}: the server answered {status}"),
            });
        }
        let too_large = || FetchError::SizeMismatch(format!("Fetch {what}: over {limit} bytes"));
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(too_large());
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            FetchError::Network(format!("Fetch {what}: {}", chain(&error.without_url())))
        })? {
            if (bytes.len() + chunk.len()) as u64 > limit {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((content_type, bytes))
    }

    async fn request(
        &self,
        source: &str,
        offset: u64,
        what: &str,
    ) -> Result<reqwest::Response, FetchError> {
        let mut url = Url::parse(source).map_err(|error| {
            FetchError::Refused(format!("Fetch {what}: invalid source: {error}"))
        })?;
        let mut redirects = 0;
        loop {
            self.admit(&url, what)?;
            let response = self.send(&url, offset, what).await?;
            let Some(next) = redirect_target(&response, &url) else {
                return Ok(response);
            };
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(FetchError::Status {
                    status: response.status().as_u16(),
                    message: format!(
                        "Fetch {what} from {}: more than {MAX_REDIRECTS} redirects",
                        host(&url)
                    ),
                });
            }
            url = next;
        }
    }

    /// The resolver never sees IP literals, so they are checked here on every hop.
    fn admit(&self, url: &Url, what: &str) -> Result<(), FetchError> {
        let refuse = |reason: String| {
            Err(FetchError::Refused(format!(
                "Fetch {what} from {}: {reason}",
                host(url)
            )))
        };
        if url.scheme() != "https" {
            return refuse(format!("{} is not HTTPS", url.scheme()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return refuse("the address carries credentials".into());
        }
        let address = match url.host() {
            Some(Host::Domain(_)) => return Ok(()),
            Some(Host::Ipv4(address)) => IpAddr::V4(address),
            Some(Host::Ipv6(address)) => IpAddr::V6(address),
            None => return refuse("the address has no host".into()),
        };
        if self.policy.permits(address) {
            Ok(())
        } else {
            refuse(format!("{address} is not a public address"))
        }
    }

    async fn send(
        &self,
        url: &Url,
        offset: u64,
        what: &str,
    ) -> Result<reqwest::Response, FetchError> {
        let mut request = self
            .client
            .get(url.clone())
            .header(ACCEPT_ENCODING, "identity");
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        request.send().await.map_err(|error| {
            let context = format!("Fetch {what} from {}", host(url));
            match refused_address(&error) {
                Some(refused) => FetchError::Refused(format!("{context}: {refused}")),
                None => FetchError::Network(format!("{context}: {}", chain(&error.without_url()))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::test_server::{Origin, asset, pattern, serving, trickling};
    use anyhow::Result;
    use axum::{
        Router,
        extract::Path as RoutePath,
        http::{StatusCode as HttpStatus, header},
        response::{IntoResponse, Response},
        routing::get,
    };
    use std::{io::Write, path::Path};

    fn staged(directory: &Path, bytes: &[u8]) -> Result<File> {
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(directory.join("asset.part"))?;
        file.write_all(bytes)?;
        Ok(file)
    }

    fn redirect(location: String) -> Response {
        (HttpStatus::FOUND, [(header::LOCATION, location)]).into_response()
    }

    const REFUSED: [&str; 30] = [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "100.127.255.255",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.0.8",
        "192.0.2.1",
        "192.168.1.1",
        "198.18.0.1",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::127.0.0.1",
        "64:ff9b::7f00:1",
        "100::1",
        "2001::1",
        "2001:db8::1",
        "2002:7f00:1::1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "fec0::1",
        "ff02::1",
    ];
    const PERMITTED: [&str; 8] = [
        "1.1.1.1",
        "8.8.8.8",
        "100.128.0.1",
        "172.32.0.1",
        "::ffff:1.1.1.1",
        "64:ff9b::101:101",
        "2606:4700::1111",
        "2a00:1450:4001::1",
    ];

    #[test]
    fn only_global_addresses_pass_the_production_policy() {
        let policy = AddressPolicy::global_only();
        for refused in REFUSED {
            assert!(!policy.permits(refused.parse().unwrap()), "{refused}");
        }
        for permitted in PERMITTED {
            assert!(policy.permits(permitted.parse().unwrap()), "{permitted}");
        }
        let test = AddressPolicy::allowing(["127.0.0.1".parse().unwrap()]);
        assert!(test.permits("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!test.permits("127.0.0.2".parse().unwrap()));
    }

    #[test]
    fn content_ranges_must_continue_the_staged_bytes() {
        assert_eq!(parse_content_range("bytes 5-9/10"), Some((5, 9, Some(10))));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("items 5-9/10"), None);
        assert_eq!(parse_content_range("bytes 5/10"), None);
        assert_eq!(deadline_for(64 * 1024 * 1024).as_secs(), 300 + 2048);
    }

    #[tokio::test]
    async fn downloads_reject_offset_overflow_before_writing() -> Result<()> {
        let mut download = Download {
            response: axum::http::Response::new("").into(),
            writer: tokio::fs::File::from_std(tempfile::tempfile()?),
            digest: StreamDigest::new(DigestAlgorithm::Sha256),
            resumed: true,
            offset: u64::MAX,
            written: u64::MAX,
            unsynced: 0,
            size: u64::MAX,
            label: "model asset".into(),
        };
        assert!(matches!(
            download.write(b"x").await,
            Err(FetchError::SizeMismatch(_))
        ));
        assert_eq!(download.writer.metadata().await?.len(), 0);
        Ok(())
    }

    #[test]
    fn an_attempt_lasts_a_day_at_most_and_a_slow_source_gets_no_other() {
        let large_asset = 64 * 1024_u64.pow(3);
        assert_eq!(deadline_for(large_asset), MAX_DEADLINE);
        let kept_rate = MINIMUM_BYTES_PER_SECOND * MAX_DEADLINE.as_secs();
        assert!(expired("asset", large_asset, kept_rate).is_retryable());
        let slower = expired("asset", large_asset, kept_rate - 1);
        assert!(matches!(slower, FetchError::TooSlow(_)) && !slower.is_retryable());
        let gibibyte = 1 << 30;
        assert!(!expired("asset", gibibyte, gibibyte - 1).is_retryable());
    }

    #[test]
    fn every_window_must_bring_the_floor_or_the_rest_of_the_asset() {
        let floor = RATE_FLOOR;
        let opened = Instant::now();
        let at = |seconds| opened + Duration::from_secs(seconds);
        let mebibyte = 1024 * 1024;
        let mut rate = RateCheck::new(floor, opened, 0);
        assert!(rate.keeps(at(299), 1, u64::MAX));
        assert!(rate.keeps(at(300), mebibyte, u64::MAX));
        assert!(rate.keeps(at(599), mebibyte + 1, u64::MAX));
        assert!(!rate.keeps(at(600), 2 * mebibyte - 1, u64::MAX));
        let tail = |written| RateCheck::new(floor, opened, 1000).keeps(at(301), written, 1100);
        assert!(!tail(1099));
        assert!(tail(1100));
    }

    /// The bytes under test, served by `origin`, and one staging file to fetch into.
    struct Bench {
        origin: Origin,
        bytes: Arc<Vec<u8>>,
        asset: ModelAssetDescriptor,
        directory: tempfile::TempDir,
    }

    impl Bench {
        async fn start(size: usize, routes: fn(Arc<Vec<u8>>) -> Router) -> Result<Self> {
            let bytes = Arc::new(pattern(size));
            Ok(Self {
                origin: Origin::start(routes(Arc::clone(&bytes))).await,
                asset: asset(&bytes, Vec::new()),
                bytes,
                directory: tempfile::tempdir()?,
            })
        }

        fn staged_path(&self) -> std::path::PathBuf {
            self.directory.path().join("asset.part")
        }

        async fn fetch(&self, fetcher: &Fetcher, source: &str, staged_bytes: &[u8]) -> FetchResult {
            let file = staged(self.directory.path(), staged_bytes).expect("a staging file");
            let cancel = CancellationToken::new();
            fetcher
                .fetch(source, &self.asset, file, &cancel, &|_| ())
                .await
        }
    }

    type FetchResult = std::result::Result<(), FetchError>;

    fn urlencode(value: &str) -> String {
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
    }

    fn hop_routes(bytes: Arc<Vec<u8>>) -> Router {
        Router::new().route("/blob", serving(bytes)).route(
            "/hop/{target}",
            get(|RoutePath(target): RoutePath<String>| async move { redirect(target) }),
        )
    }

    fn chain_routes(bytes: Arc<Vec<u8>>) -> Router {
        Router::new().route("/blob", serving(bytes)).route(
            "/chain/{left}",
            get(|RoutePath(left): RoutePath<u32>| async move {
                redirect(match left {
                    0 => "/blob".to_owned(),
                    left => format!("/chain/{}", left - 1),
                })
            }),
        )
    }

    fn trickle_routes(bytes: Arc<Vec<u8>>) -> Router {
        Router::new().route(
            "/trickle",
            get(move || {
                let bytes = Arc::clone(&bytes);
                async move { trickling(&bytes, Duration::from_millis(50)) }
            }),
        )
    }

    fn resume_routes(bytes: Arc<Vec<u8>>) -> Router {
        let whole = Arc::clone(&bytes);
        Router::new().route("/ranged", serving(bytes)).route(
            "/whole",
            get(move || {
                let whole = Arc::clone(&whole);
                async move { whole.to_vec() }
            }),
        )
    }

    #[tokio::test]
    async fn the_production_policy_refuses_private_literals_and_names() -> Result<()> {
        let bench = Bench::start(4096, hop_routes).await?;
        let port = bench.origin.port;
        let production =
            Fetcher::trusting(AddressPolicy::global_only(), &bench.origin.certificate)?;
        for source in [
            bench.origin.url("/blob"),
            format!("https://localhost:{port}/blob"),
            format!("https://[::ffff:7f00:1]:{port}/blob"),
            "https://169.254.169.254/latest/meta-data".to_owned(),
            format!("http://127.0.0.1:{port}/blob"),
        ] {
            let error = bench.fetch(&production, &source, b"").await.unwrap_err();
            assert!(matches!(error, FetchError::Refused(_)), "{source}: {error}");
        }
        assert_eq!(bench.origin.hits.total(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn redirects_to_private_or_plain_targets_are_refused() -> Result<()> {
        let bench = Bench::start(4096, hop_routes).await?;
        let port = bench.origin.port;
        let fetcher = bench.origin.fetcher();
        for target in [
            format!("https://127.0.0.2:{port}/blob"),
            "https://10.0.0.1/blob".to_owned(),
            format!("http://127.0.0.1:{port}/blob"),
        ] {
            let source = bench.origin.url(&format!("/hop/{}", urlencode(&target)));
            let error = bench.fetch(&fetcher, &source, b"").await.unwrap_err();
            assert!(matches!(error, FetchError::Refused(_)), "{target}: {error}");
        }
        assert!(bench.origin.hits.to("/blob").is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn redirects_are_followed_at_most_five_times() -> Result<()> {
        let bench = Bench::start(4096, chain_routes).await?;
        let fetcher = bench.origin.fetcher();
        bench
            .fetch(&fetcher, &bench.origin.url("/chain/4"), b"")
            .await?;
        assert_eq!(std::fs::read(bench.staged_path())?, *bench.bytes);
        let source = bench.origin.url("/chain/5");
        let error = bench.fetch(&fetcher, &source, b"").await.unwrap_err();
        let too_many = matches!(error, FetchError::Status { status: 302, .. });
        assert!(too_many, "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn staged_bytes_resume_with_a_range_or_restart_when_it_is_ignored() -> Result<()> {
        let bench = Bench::start(300_000, resume_routes).await?;
        let fetcher = bench.origin.fetcher();
        let prefix = &bench.bytes[..100_000];
        bench
            .fetch(&fetcher, &bench.origin.url("/ranged"), prefix)
            .await?;
        assert_eq!(std::fs::read(bench.staged_path())?, *bench.bytes);
        let hits = bench.origin.hits.to("/ranged");
        assert_eq!(hits[0].range.as_deref(), Some("bytes=100000-"));
        assert_eq!(hits[0].accept_encoding.as_deref(), Some("identity"));

        bench
            .fetch(&fetcher, &bench.origin.url("/whole"), prefix)
            .await?;
        assert_eq!(std::fs::read(bench.staged_path())?, *bench.bytes);

        bench
            .fetch(&fetcher, &bench.origin.url("/missing"), &bench.bytes)
            .await?;
        assert!(bench.origin.hits.to("/missing").is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_trickling_source_ends_its_attempt_as_too_slow() -> Result<()> {
        let bench = Bench::start(4096, trickle_routes).await?;
        let floor = Duration::from_secs(1);
        let fetcher = bench.origin.fetcher().with_rate_floor(floor, 64);
        let source = bench.origin.url("/trickle");
        let fetched = tokio::time::timeout(10 * floor, bench.fetch(&fetcher, &source, b"")).await;
        let error = fetched
            .context("the trickle held the attempt")?
            .unwrap_err();
        let too_slow = matches!(error, FetchError::TooSlow(_));
        assert!(too_slow && !error.is_retryable(), "{error}");
        assert_eq!(error.failure(), (ModelAssetFailure::EgressBlocked, None));
        let staged = std::fs::metadata(bench.staged_path())?.len();
        assert!(staged < 64, "{staged} bytes staged");
        Ok(())
    }

    #[tokio::test]
    async fn a_corrupt_staged_prefix_fails_as_a_retryable_mismatch() -> Result<()> {
        let bench = Bench::start(300_000, resume_routes).await?;
        let fetcher = bench.origin.fetcher();
        let mut corrupt = bench.bytes[..100_000].to_vec();
        corrupt[10] ^= 1;
        let source = bench.origin.url("/ranged");
        let error = bench.fetch(&fetcher, &source, &corrupt).await.unwrap_err();
        let resumed = matches!(error, FetchError::DigestMismatch { resumed: true, .. });
        assert!(resumed && error.is_retryable(), "{error}");
        assert_eq!(std::fs::metadata(bench.staged_path())?.len(), 0);
        Ok(())
    }
}
