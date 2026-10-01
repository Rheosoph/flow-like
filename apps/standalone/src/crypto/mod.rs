pub use flow_like_device_crypto::{CryptoError, Result, mls, mls_store, noise};

/// aws-lc-rs for every TLS endpoint: it offers hybrid post-quantum key exchange (X25519MLKEM768).
pub fn tls_provider() -> rustls::crypto::CryptoProvider {
    rustls::crypto::aws_lc_rs::default_provider()
}
