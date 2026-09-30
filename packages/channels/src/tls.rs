use std::sync::Arc;

use flow_like_types::{Result, anyhow};

/// Explicit provider and roots: the workspace links both `ring` and `aws-lc-rs` into rustls, so
/// `ClientConfig::builder()` (what tungstenite's `connect_async` and rumqttc's
/// `Transport::wss_with_default_config` use) would panic unless the binary installed a process
/// default first.
pub(crate) fn client_config() -> Result<Arc<rustls::ClientConfig>> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| anyhow!("rustls provider supports no usable TLS version: {error}"))?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}
