use crate::{Error, Result};
use rustls::{ClientConfig, crypto::CryptoProvider};
use rustls_platform_verifier::Verifier;
use std::sync::{Arc, OnceLock};

static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

/// Product binaries link both ring and aws-lc-rs, so rustls cannot choose a process
/// default. Every TLS client in this crate passes this config explicitly.
pub(crate) fn client_config() -> Result<Arc<ClientConfig>> {
    if let Some(config) = CONFIG.get() {
        return Ok(config.clone());
    }
    let config = Arc::new(build()?);
    Ok(CONFIG.get_or_init(|| config).clone())
}

fn build() -> Result<ClientConfig> {
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let verifier = platform_verifier(provider.clone()).map_err(setup_error)?;
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(setup_error)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
fn platform_verifier(provider: Arc<CryptoProvider>) -> Result<Verifier, rustls::Error> {
    Verifier::new_with_extra_roots(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned(),
        provider,
    )
}

#[cfg(not(all(unix, not(target_vendor = "apple"), not(target_os = "android"))))]
fn platform_verifier(provider: Arc<CryptoProvider>) -> Result<Verifier, rustls::Error> {
    Verifier::new(provider)
}

fn setup_error(error: rustls::Error) -> Error {
    Error::Invalid(format!(
        "Could not set up TLS with the platform roots for the device hub: {error}"
    ))
}
