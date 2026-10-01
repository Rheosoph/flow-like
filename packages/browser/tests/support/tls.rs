#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

static NEXT_CA: AtomicU64 = AtomicU64::new(1);

pub struct TestCa {
    pub ca_der: Vec<u8>,
    pub leaf_chain: Vec<CertificateDer<'static>>,
    pub leaf_key: PrivateKeyDer<'static>,
}

pub fn test_ca() -> TestCa {
    let ca_key = KeyPair::generate().expect("generate the test CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("test CA params");
    // Each CA needs its own subject: a verifier that finds a trusted CA with the leaf issuer name
    // reports BadSignature instead of UnknownIssuer.
    let serial = NEXT_CA.fetch_add(1, Ordering::Relaxed);
    ca_params.distinguished_name.push(
        DnType::CommonName,
        format!("flow-like-browser Test Root CA {serial}"),
    );
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    set_validity(&mut ca_params, 365);
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .expect("self-sign the test CA");
    let issuer = Issuer::new(ca_params, ca_key);

    let leaf_key = KeyPair::generate().expect("generate the test leaf key");
    let mut leaf_params =
        CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
            .expect("test leaf params");
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    set_validity(&mut leaf_params, 30);
    let leaf = leaf_params
        .signed_by(&leaf_key, &issuer)
        .expect("sign the test leaf");

    TestCa {
        ca_der: ca_cert.der().to_vec(),
        leaf_chain: vec![leaf.der().clone(), ca_cert.der().clone()],
        leaf_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

pub fn server_config(ca: &TestCa) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_no_client_auth()
        .with_single_cert(ca.leaf_chain.clone(), ca.leaf_key.clone_key())
        .expect("the test leaf matches its key");
    Arc::new(config)
}

fn set_validity(params: &mut CertificateParams, days: u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970");
    let epoch = date_time_ymd(1970, 1, 1);
    params.not_before = epoch + (now - DAY);
    params.not_after = epoch + now + DAY * days;
}
