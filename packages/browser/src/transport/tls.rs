use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use rustls::ClientConfig;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use rustls_platform_verifier::Verifier;

use crate::BrowserError;

const HTTP_1_1: &[u8] = b"http/1.1";

type ConfigCache = Mutex<HashMap<u64, Arc<ClientConfig>>>;

static CONFIGS: OnceLock<ConfigCache> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpProxy {
    System,
    None,
}

pub fn client_config(extra_roots: &[Vec<u8>]) -> crate::Result<Arc<ClientConfig>> {
    let digest = roots_digest(extra_roots);
    let cache = CONFIGS.get_or_init(ConfigCache::default);
    if let Some(config) = lock(cache).get(&digest) {
        return Ok(config.clone());
    }
    let config = Arc::new(build_config(extra_roots)?);
    Ok(lock(cache).entry(digest).or_insert(config).clone())
}

pub fn http_client(
    extra_roots: &[Vec<u8>],
    timeout: Duration,
    proxy: HttpProxy,
) -> crate::Result<reqwest::Client> {
    let config = client_config(extra_roots)?;
    let builder = reqwest::Client::builder()
        .use_preconfigured_tls((*config).clone())
        .timeout(timeout);
    let builder = match proxy {
        HttpProxy::System => builder,
        HttpProxy::None => builder.no_proxy(),
    };
    builder.build().map_err(|error| BrowserError::Connect {
        message: format!(
            "Could not build the HTTP client ({} extra CA): {error}",
            extra_roots.len()
        ),
    })
}

fn lock(cache: &ConfigCache) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<ClientConfig>>> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

fn roots_digest(extra_roots: &[Vec<u8>]) -> u64 {
    let mut hasher = DefaultHasher::new();
    extra_roots.hash(&mut hasher);
    hasher.finish()
}

fn build_config(extra_roots: &[Vec<u8>]) -> crate::Result<ClientConfig> {
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let verifier = platform_verifier(extra_roots, provider.clone())
        .map_err(|error| setup_error(extra_roots.len(), &error))?;
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| setup_error(extra_roots.len(), &error))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP_1_1.to_vec()];
    Ok(config)
}

fn platform_verifier(
    extra_roots: &[Vec<u8>],
    provider: Arc<CryptoProvider>,
) -> Result<Verifier, rustls::Error> {
    let roots: Vec<CertificateDer<'static>> = extra_roots
        .iter()
        .map(|der| CertificateDer::from(der.clone()))
        .chain(bundled_roots())
        .collect();
    if roots.is_empty() {
        Verifier::new(provider)
    } else {
        Verifier::new_with_extra_roots(roots, provider)
    }
}

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
fn bundled_roots() -> impl Iterator<Item = CertificateDer<'static>> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned()
}

#[cfg(not(all(unix, not(target_vendor = "apple"), not(target_os = "android"))))]
fn bundled_roots() -> impl Iterator<Item = CertificateDer<'static>> {
    std::iter::empty()
}

fn setup_error(extra_roots: usize, error: &rustls::Error) -> BrowserError {
    BrowserError::Connect {
        message: format!(
            "Could not set up TLS with the platform roots plus {extra_roots} extra CA: {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_is_exactly_http_1_1() {
        let config = client_config(&[]).expect("default TLS config");
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn configs_are_cached_per_root_set() {
        let first = client_config(&[]).expect("default TLS config");
        let second = client_config(&[]).expect("cached TLS config");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn invalid_extra_roots_fail_with_context() {
        let error = client_config(&[b"not a certificate".to_vec()])
            .expect_err("garbage DER must be rejected");
        let BrowserError::Connect { message } = error else {
            panic!("expected a Connect error, got {error:?}");
        };
        assert!(
            message.contains("platform roots plus 1 extra CA"),
            "{message}"
        );
    }

    #[test]
    fn http_clients_build_with_and_without_the_system_proxy() {
        for proxy in [HttpProxy::System, HttpProxy::None] {
            http_client(&[], Duration::from_secs(5), proxy).expect("HTTP client");
        }
    }
}
