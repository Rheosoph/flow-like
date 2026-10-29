//! Outbound network policy for model clients built from a Bit's own endpoint.
//!
//! A process that holds a private model's secrets and calls its endpoint for the owner (the
//! API relay, FlowPilot) must not let that endpoint reach the process's own network. The
//! guarded modes refuse IP-literal hosts that break the rule before a request is sent,
//! install a resolver that vets every address a name resolves to at connect time (so
//! rebinding after a check does not help), follow no redirects, and ignore proxy settings,
//! which would move name resolution past the resolver.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use flow_like_types_contracts::egress::{is_host_plane_ip, is_host_plane_name, is_public_ip};
use reqwest::Url;

/// Where a model client built from a Bit's own endpoint may connect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProviderEgress {
    /// No restriction: desktop runs, executors and hosted models.
    #[default]
    Open,
    /// HTTPS only, and every address publicly routable.
    Public,
    /// HTTP or HTTPS and private ranges, for self-hosted deployments. Loopback, link-local,
    /// metadata and other host-plane addresses stay refused.
    Private,
}

/// A destination the egress policy refuses. The message names the host, never the URL:
/// query strings carry keys and signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressRefused(String);

impl fmt::Display for EgressRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EgressRefused {}

impl ProviderEgress {
    /// The mode for private models this server calls for its users: public HTTPS, or
    /// private ranges when the operator sets `CUSTOM_MODEL_EGRESS=private`. Any other
    /// value keeps `Public`; servers never run `Open`.
    pub fn for_server() -> Self {
        Self::server_mode(std::env::var("CUSTOM_MODEL_EGRESS").ok().as_deref())
    }

    fn server_mode(setting: Option<&str>) -> Self {
        match setting.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("private") => Self::Private,
            _ => Self::Public,
        }
    }

    pub fn allows_ip(self, ip: IpAddr) -> bool {
        match self {
            Self::Open => true,
            Self::Public => is_public_ip(ip),
            Self::Private => !is_host_plane_ip(ip),
        }
    }

    /// Checks the scheme and host of `url`. Names pass here and are vetted by the guarded
    /// resolver when a connection is opened.
    pub fn check_url(self, url: &Url) -> Result<(), EgressRefused> {
        if self == Self::Open {
            return Ok(());
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| EgressRefused("Model endpoint URL has no host".to_string()))?;
        match (self, url.scheme()) {
            (Self::Public, "https") | (Self::Private, "http" | "https") => {}
            (Self::Public, scheme) => {
                return Err(EgressRefused(format!(
                    "Model endpoint '{host}' must use https, not '{scheme}'"
                )));
            }
            (_, scheme) => {
                return Err(EgressRefused(format!(
                    "Model endpoint '{host}' must use http or https, not '{scheme}'"
                )));
            }
        }
        let literal = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host)
            .parse::<IpAddr>();
        match literal {
            Ok(ip) if !self.allows_ip(ip) => Err(self.refusal(host)),
            Ok(_) => Ok(()),
            Err(_) if is_host_plane_name(host) => Err(self.refusal(host)),
            Err(_) => Ok(()),
        }
    }

    /// Checks `endpoint` and, for a name, every address it currently resolves to. A
    /// pre-flight for clear errors; connections are vetted again when they are opened.
    pub async fn ensure_endpoint_allowed(self, endpoint: &str) -> Result<(), EgressRefused> {
        if self == Self::Open {
            return Ok(());
        }
        let url = Url::parse(endpoint.trim())
            .map_err(|_| EgressRefused("Model endpoint is not a valid URL".to_string()))?;
        self.check_url(&url)?;
        let host = url.host_str().unwrap_or_default();
        if host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
            return Ok(());
        }
        self.resolve(host).await.map(|_| ())
    }

    /// Installs the policy on a reqwest 0.12 client: the guarded resolver, no redirects, no
    /// proxy, and HTTPS only under `Public`. `Open` returns the builder unchanged.
    pub fn guard(self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if self == Self::Open {
            return builder;
        }
        builder
            .dns_resolver(Arc::new(GuardedResolver(self)))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .https_only(self == Self::Public)
    }

    /// [`Self::guard`] for the reqwest 0.13 clients Rig's providers run on.
    pub(crate) fn guard_rig(
        self,
        builder: reqwest_rig::ClientBuilder,
    ) -> reqwest_rig::ClientBuilder {
        if self == Self::Open {
            return builder;
        }
        builder
            .dns_resolver(Arc::new(GuardedResolver(self)))
            .redirect(reqwest_rig::redirect::Policy::none())
            .no_proxy()
            .https_only(self == Self::Public)
    }

    /// [`Self::check_url`] for a request Rig built. Under `Open` the URI is not parsed.
    pub(crate) fn check_uri(self, uri: &http::Uri) -> Result<(), EgressRefused> {
        if self == Self::Open {
            return Ok(());
        }
        let url = Url::parse(&uri.to_string())
            .map_err(|_| EgressRefused("Model endpoint is not a valid URL".to_string()))?;
        self.check_url(&url)
    }

    async fn resolve(self, host: &str) -> Result<Vec<SocketAddr>, EgressRefused> {
        if is_host_plane_name(host) {
            return Err(self.refusal(host));
        }
        let resolved = tokio::net::lookup_host((host, 0)).await;
        self.vet(host, resolved.map(Iterator::collect))
    }

    /// The addresses `host` resolved to, when this mode allows every one. A name that does not
    /// resolve gets the refusal of one that resolves to a refused address, so the error does not
    /// tell which names this network knows; only the log tells them apart.
    fn vet(
        self,
        host: &str,
        resolved: std::io::Result<Vec<SocketAddr>>,
    ) -> Result<Vec<SocketAddr>, EgressRefused> {
        match resolved {
            Ok(addrs)
                if !addrs.is_empty() && addrs.iter().all(|addr| self.allows_ip(addr.ip())) =>
            {
                Ok(addrs)
            }
            Ok(addrs) => {
                tracing::debug!(
                    host,
                    ?addrs,
                    egress = ?self,
                    "Refused a model endpoint by its addresses"
                );
                Err(self.refusal(host))
            }
            Err(error) => {
                tracing::debug!(
                    host,
                    %error,
                    egress = ?self,
                    "Refused a model endpoint whose name did not resolve"
                );
                Err(self.refusal(host))
            }
        }
    }

    fn refusal(self, host: &str) -> EgressRefused {
        EgressRefused(match self {
            Self::Private => format!(
                "Model endpoint '{host}' does not resolve to an address private models may \
                 reach: this server's own network plane (loopback, link-local or metadata) is \
                 refused"
            ),
            Self::Open | Self::Public => format!(
                "Model endpoint '{host}' does not resolve to a public address; private models \
                 called from this server must use public HTTPS endpoints"
            ),
        })
    }
}

type ResolveError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy)]
struct GuardedResolver(ProviderEgress);

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let (egress, host) = (self.0, name.as_str().to_owned());
        Box::pin(async move {
            let addrs = egress
                .resolve(&host)
                .await
                .map_err(|error| -> ResolveError { Box::new(error) })?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl reqwest_rig::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest_rig::dns::Name) -> reqwest_rig::dns::Resolving {
        let (egress, host) = (self.0, name.as_str().to_owned());
        Box::pin(async move {
            let addrs = egress
                .resolve(&host)
                .await
                .map_err(|error| -> ResolveError { Box::new(error) })?;
            Ok(Box::new(addrs.into_iter()) as reqwest_rig::dns::Addrs)
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn check(egress: ProviderEgress, raw: &str) -> Result<(), EgressRefused> {
        egress.check_url(&Url::parse(raw).unwrap())
    }

    /// Accepts connections on loopback, answers each with `response` and counts them.
    pub(crate) async fn answer_every_connection(
        response: &'static str,
    ) -> (SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = served.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buffer = [0u8; 8192];
                let _ = socket.read(&mut buffer).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (addr, served)
    }

    #[test]
    fn public_refuses_private_shared_and_host_plane_addresses() {
        for raw in [
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "100.100.100.200",
            "0.0.0.0",
            "fd00::1",
            "fd00:ec2::254",
            "fd20:ce::254",
            "fe80::1",
            "::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!ProviderEgress::Public.allows_ip(ip(raw)), "{raw}");
        }
        for raw in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(ProviderEgress::Public.allows_ip(ip(raw)), "{raw}");
        }
    }

    #[test]
    fn private_allows_private_ranges_but_not_the_host_plane() {
        for raw in [
            "10.0.0.1",
            "10.20.30.40",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "fd00::1",
            "93.184.216.34",
        ] {
            assert!(ProviderEgress::Private.allows_ip(ip(raw)), "{raw}");
        }
        for raw in [
            "127.0.0.1",
            "169.254.169.254",
            "169.254.170.2",
            "168.63.129.16",
            "100.100.100.200",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00:ec2::254",
            "fd00:ec2::23",
            "fd20:ce::254",
            "::ffff:127.0.0.1",
            "::ffff:100.100.100.200",
        ] {
            assert!(!ProviderEgress::Private.allows_ip(ip(raw)), "{raw}");
        }
    }

    #[test]
    fn public_requires_https_and_a_public_literal_or_name() {
        assert!(check(ProviderEgress::Public, "https://api.example.com/v1").is_ok());
        assert!(check(ProviderEgress::Public, "https://93.184.216.34/v1").is_ok());
        for raw in [
            "http://api.example.com/v1",
            "ftp://api.example.com/v1",
            "https://127.0.0.1/v1",
            "https://2130706433/v1",
            "https://169.254.169.254/latest/meta-data/",
            "https://10.0.0.1/v1",
            "https://100.64.0.1/v1",
            "https://[fd00::1]/v1",
            "https://[::1]:8443/v1",
            "https://localhost/v1",
            "https://metadata.google.internal/v1",
        ] {
            assert!(check(ProviderEgress::Public, raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn private_allows_plain_http_on_private_networks() {
        for raw in [
            "http://10.0.0.5:8000/v1",
            "http://192.168.1.20:11434",
            "https://vllm.internal.example/v1",
            "http://[fd00::5]:8000/v1",
        ] {
            assert!(check(ProviderEgress::Private, raw).is_ok(), "{raw}");
        }
        for raw in [
            "http://127.0.0.1:11434",
            "http://localhost:1234",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.100.100.200/latest/meta-data/",
            "http://[fd20:ce::254]/computeMetadata/v1/",
            "http://[fd00:ec2::23]/v1/credentials",
            "http://[::1]:8000/v1",
            "http://instance-data/",
            "file:///etc/passwd",
        ] {
            assert!(check(ProviderEgress::Private, raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn open_checks_nothing() {
        for raw in [
            "http://127.0.0.1:11434",
            "http://169.254.169.254/",
            "ftp://x/",
        ] {
            assert!(check(ProviderEgress::Open, raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn servers_use_public_unless_the_operator_opts_into_private() {
        for (setting, expected) in [
            (None, ProviderEgress::Public),
            (Some(""), ProviderEgress::Public),
            (Some("open"), ProviderEgress::Public),
            (Some("public"), ProviderEgress::Public),
            (Some("privat"), ProviderEgress::Public),
            (Some("private"), ProviderEgress::Private),
            (Some(" Private "), ProviderEgress::Private),
        ] {
            assert_eq!(
                ProviderEgress::server_mode(setting),
                expected,
                "{setting:?}"
            );
        }
    }

    #[test]
    fn refusals_never_echo_the_url() {
        let error = check(
            ProviderEgress::Public,
            "http://generativelanguage.example/v1?key=AIzaSecret",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("generativelanguage.example"), "{error}");
        assert!(!error.contains("AIzaSecret"), "{error}");
    }

    #[tokio::test]
    async fn names_on_the_host_plane_are_refused_before_resolution() {
        for egress in [ProviderEgress::Public, ProviderEgress::Private] {
            assert!(
                egress
                    .ensure_endpoint_allowed("https://localhost/v1")
                    .await
                    .is_err()
            );
            assert!(egress.resolve("metadata.google.internal").await.is_err());
        }
        assert!(
            ProviderEgress::Public
                .ensure_endpoint_allowed("https://10.0.0.1/v1")
                .await
                .is_err()
        );
        assert!(
            ProviderEgress::Private
                .ensure_endpoint_allowed("http://10.0.0.1:8000/v1")
                .await
                .is_ok()
        );
        assert!(
            ProviderEgress::Open
                .ensure_endpoint_allowed("not a url")
                .await
                .is_ok()
        );
    }

    #[test]
    fn a_name_that_does_not_resolve_is_refused_like_one_on_a_refused_address() {
        let host = "db.svc.cluster.local";
        let at = |raw: &str| SocketAddr::new(ip(raw), 0);
        for egress in [ProviderEgress::Public, ProviderEgress::Private] {
            let refused = egress
                .vet(host, Ok(vec![at("10.0.0.1"), at("169.254.169.254")]))
                .unwrap_err();
            let unknown = egress
                .vet(host, Err(std::io::ErrorKind::NotFound.into()))
                .unwrap_err();
            let empty = egress.vet(host, Ok(Vec::new())).unwrap_err();
            assert_eq!(unknown, refused, "{egress:?}");
            assert_eq!(empty, refused, "{egress:?}");
            assert!(refused.to_string().contains(host), "{refused}");
        }
        assert!(
            ProviderEgress::Private
                .vet(host, Ok(vec![at("10.0.0.1")]))
                .is_ok()
        );
    }

    #[tokio::test]
    async fn the_resolver_vets_every_address() {
        assert!(ProviderEgress::Public.resolve("127.0.0.1").await.is_err());
        assert!(ProviderEgress::Public.resolve("10.0.0.1").await.is_err());
        assert!(ProviderEgress::Private.resolve("10.0.0.1").await.is_ok());
        assert!(
            ProviderEgress::Private
                .resolve("169.254.169.254")
                .await
                .is_err()
        );
        assert!(ProviderEgress::Private.resolve("::1").await.is_err());
    }

    #[tokio::test]
    async fn guarded_clients_resolve_through_the_policy() {
        let (addr, served) = answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
        )
        .await;
        for egress in [ProviderEgress::Public, ProviderEgress::Private] {
            let client = egress.guard(reqwest::Client::builder()).build().unwrap();
            let result = client
                .get(format!("http://localhost.:{}/", addr.port()))
                .send()
                .await;
            assert!(result.is_err(), "{egress:?}");
        }
        assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn guarded_clients_never_follow_redirects() {
        let (addr, served) = answer_every_connection(
            "HTTP/1.1 302 Found\r\nlocation: http://models.test/next\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        let builder = reqwest::Client::builder().resolve("models.test", addr);
        let client = ProviderEgress::Private.guard(builder).build().unwrap();
        let response = client
            .get(format!("http://models.test:{}/v1", addr.port()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guarded_clients_ignore_proxies() {
        let (proxy, proxied) = answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nproxy",
        )
        .await;
        let (target, served) = answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-length: 6\r\nconnection: close\r\n\r\ntarget",
        )
        .await;
        let builder = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
            .resolve("models.test", target);
        let client = ProviderEgress::Private.guard(builder).build().unwrap();
        let body = client
            .get(format!("http://models.test:{}/v1", target.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "target");
        assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(proxied.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
