use super::*;
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

const NOW: i64 = 1_790_000_000;
const PASSWORD: &[u8] = b"certificate authority password";
const ACCOUNT: &str = "[\"issuer\",\"account\",\"https://api.example.test\",\"profile\"]";
const ID: &str = "bce0a1fe-7055-4131-8e08-ec9b3b4b3975";

fn authority() -> &'static CertificateAuthorityVault {
    static AUTHORITY: OnceLock<CertificateAuthorityVault> = OnceLock::new();
    AUTHORITY.get_or_init(|| {
        create_certificate_authority_vault(
            &CertificateAuthoritySpec {
                account_binding: ACCOUNT.into(),
                authority_id: ID.into(),
                label: "Example organisation".into(),
                dns_suffixes: vec!["example.test".into()],
                ip_addresses: vec!["127.0.0.1".into(), "::1".into()],
                validity_days: 3650,
            },
            PASSWORD,
            NOW,
        )
        .unwrap()
    })
}

fn request(dns: &[&str], ips: &[&str], issuer: bool) -> (KeyPair, CertificateSigningRequest) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(
        dns.iter()
            .chain(ips)
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    params.is_ca = if issuer {
        IsCa::Ca(BasicConstraints::Constrained(0))
    } else {
        IsCa::ExplicitNoCa
    };
    params.key_usages = if issuer {
        vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign]
    } else {
        vec![KeyUsagePurpose::DigitalSignature]
    };
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let csr_pem = params.serialize_request(&key).unwrap().pem().unwrap();
    (
        key,
        CertificateSigningRequest {
            csr_pem,
            dns_names: dns.iter().map(|name| (*name).into()).collect(),
            ip_addresses: ips.iter().map(|ip| (*ip).into()).collect(),
            validity_days: 90,
        },
    )
}

fn sign(request: &CertificateSigningRequest, delegate: bool) -> Result<SignedCertificateChain> {
    sign_with_authority(
        ACCOUNT,
        ID,
        PASSWORD,
        &authority().vault,
        request,
        NOW,
        delegate,
    )
}

fn pem_certificates(chain: &str) -> Vec<CertificateDer<'static>> {
    x509_parser::pem::Pem::iter_from_buffer(chain.as_bytes())
        .map(|pem| CertificateDer::from(pem.unwrap().contents))
        .collect()
}

fn trusted_by_root(chain: &str, server_name: &str, now: i64) -> Result<()> {
    let certificates = pem_certificates(chain);
    let mut roots = RootCertStore::empty();
    roots.add(pem_certificates(&authority().public_bundle.root_certificate_pem).remove(0))?;
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()?;
    verifier.verify_server_cert(
        &certificates[0],
        &certificates[1..],
        &ServerName::try_from(server_name.to_owned())?,
        &[],
        UnixTime::since_unix_epoch(Duration::from_secs(now as u64)),
    )?;
    Ok(())
}

#[test]
fn authority_vaults_are_password_account_identity_and_purpose_bound() {
    let authority = authority();
    let public =
        inspect_certificate_authority_vault(ACCOUNT, ID, PASSWORD, &authority.vault).unwrap();
    assert_eq!(public, authority.public_bundle);
    assert!(
        inspect_certificate_authority_vault("other account", ID, PASSWORD, &authority.vault)
            .is_err()
    );
    assert!(
        inspect_certificate_authority_vault(
            ACCOUNT,
            "cce0a1fe-7055-4131-8e08-ec9b3b4b3975",
            PASSWORD,
            &authority.vault
        )
        .is_err()
    );
    assert!(
        inspect_certificate_authority_vault(ACCOUNT, ID, b"wrong password", &authority.vault)
            .is_err()
    );
    assert!(
        inspect_certificate_authority_vault(ACCOUNT, ID, PASSWORD, &authority.root_vault).is_err()
    );
    let mut damaged = authority.vault.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(inspect_certificate_authority_vault(ACCOUNT, ID, PASSWORD, &damaged).is_err());
    assert!(
        !serde_json::to_string(authority)
            .unwrap()
            .contains("PRIVATE KEY")
    );
}

#[test]
fn backup_inspection_authenticates_root_material_as_well_as_the_issuing_vault() {
    let authority = authority();
    assert_eq!(
        inspect_certificate_authority_backup(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority.vault,
            &authority.root_vault
        )
        .unwrap(),
        authority.public_bundle
    );
    let mut damaged = authority.root_vault.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(
        inspect_certificate_authority_backup(ACCOUNT, ID, PASSWORD, &authority.vault, &damaged)
            .is_err()
    );
    let mut root = open_authority(ACCOUNT, ID, PASSWORD, &authority.root_vault, true).unwrap();
    root.private_key_der.zeroize();
    root.private_key_der = KeyPair::generate().unwrap().serialize_der();
    let mismatched_key = seal_authority(PASSWORD, &root).unwrap();
    assert!(
        inspect_certificate_authority_backup(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority.vault,
            &mismatched_key
        )
        .unwrap_err()
        .to_string()
        .contains("private key")
    );
    assert!(
        renew_certificate_authority_vault(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority.vault,
            &mismatched_key,
            NOW
        )
        .is_err()
    );
    root.public.label = "Swapped root backup".into();
    let mismatched_bundle = seal_authority(PASSWORD, &root).unwrap();
    assert!(
        inspect_certificate_authority_backup(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority.vault,
            &mismatched_bundle
        )
        .unwrap_err()
        .to_string()
        .contains("do not match")
    );
}

#[test]
fn service_issuance_does_not_need_root_vault_and_verifies_in_webpki() {
    let (_, request) = request(&["api.example.test"], &["127.0.0.1"], false);
    let signed = sign(&request, false).unwrap();
    assert_eq!(signed.not_after, NOW + 90 * DAY);
    trusted_by_root(&signed.certificate_chain_pem, "api.example.test", NOW).unwrap();
    trusted_by_root(&signed.certificate_chain_pem, "127.0.0.1", NOW).unwrap();
    assert!(trusted_by_root(&signed.certificate_chain_pem, "other.example.test", NOW).is_err());
    assert!(
        sign_service_certificate(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority().root_vault,
            &request,
            NOW
        )
        .is_err()
    );
    let chain = pem_certificates(&signed.certificate_chain_pem);
    let (_, leaf) = X509Certificate::from_der(chain[0].as_ref()).unwrap();
    assert!(!leaf.basic_constraints().unwrap().unwrap().value.ca);
    assert_eq!(leaf.key_usage().unwrap().unwrap().value.flags, 1);
    let eku = leaf.extended_key_usage().unwrap().unwrap();
    assert!(eku.value.server_auth && !eku.value.client_auth && !eku.value.code_signing);
}

#[test]
fn issuer_rejects_unapproved_names_ca_escalation_and_mismatched_approval() {
    let (_, outside) = request(&["example.test.attacker.test"], &[], false);
    assert!(
        sign(&outside, false)
            .unwrap_err()
            .to_string()
            .contains("scope")
    );
    let (_, wrong_ip) = request(&["api.example.test"], &["192.0.2.1"], false);
    assert!(sign(&wrong_ip, false).is_err());
    let (_, mut mismatch) = request(&["api.example.test"], &[], false);
    mismatch.dns_names = vec!["other.example.test".into()];
    assert!(
        sign(&mismatch, false)
            .unwrap_err()
            .to_string()
            .contains("approved")
    );
    let (_, ca) = request(&["api.example.test"], &[], true);
    assert!(sign(&ca, false).is_err());
    let (_, leaf) = request(&["api.example.test"], &[], false);
    assert!(sign(&leaf, true).is_err());
    let (_, overlapping) = request(&["api.example.test", "child.api.example.test"], &[], true);
    assert!(
        sign(&overlapping, true)
            .unwrap_err()
            .to_string()
            .contains("subdomains")
    );
}

#[test]
fn csr_signature_extra_blocks_privileged_usage_and_wildcards_are_rejected() {
    let (key, mut valid) = request(&["api.example.test"], &[], false);
    valid.csr_pem.push_str(&valid.csr_pem.clone());
    assert!(checked_csr(&valid, false).is_err());
    let mut params = CertificateParams::new(vec!["api.example.test".into()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    valid.csr_pem = params.serialize_request(&key).unwrap().pem().unwrap();
    assert!(checked_csr(&valid, false).is_err());
    let (_, mut wildcard) = request(&["*.example.test"], &[], false);
    assert!(checked_csr(&wildcard, false).is_err());
    wildcard.dns_names = vec!["api.example.test".into()];
    assert!(checked_csr(&wildcard, false).is_err());
    let (_, mut tampered) = request(&["api.example.test"], &[], false);
    let mut der = single_pem(&tampered.csr_pem, "CERTIFICATE REQUEST").unwrap();
    let last = der.len() - 1;
    der[last] ^= 1;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    tampered.csr_pem = format!(
        "-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----\n",
        STANDARD.encode(der)
    );
    assert!(checked_csr(&tampered, false).is_err());
}

#[test]
fn device_can_renew_locally_but_cannot_change_names_or_exceed_issuer_expiry() {
    let (issuer_key, mut issuer_request) = request(&["api.example.test"], &["127.0.0.1"], true);
    issuer_request.validity_days = 30;
    let issuer = sign(&issuer_request, true).unwrap();
    validate_device_issuer_chain(
        &issuer.certificate_chain_pem,
        &issuer_key.serialize_pem(),
        &issuer_request.dns_names,
        &issuer_request.ip_addresses,
        NOW,
    )
    .unwrap();
    let (_, leaf_request) = request(&["api.example.test"], &["127.0.0.1"], false);
    let leaf = sign_csr_with_device_issuer(
        &issuer.certificate_chain_pem,
        &issuer_key.serialize_pem(),
        &leaf_request,
        NOW + DAY,
    )
    .unwrap();
    assert_eq!(leaf.not_after, issuer.not_after);
    trusted_by_root(&leaf.certificate_chain_pem, "api.example.test", NOW + DAY).unwrap();
    trusted_by_root(&leaf.certificate_chain_pem, "127.0.0.1", NOW + DAY).unwrap();
    assert!(
        sign_csr_with_device_issuer(
            &issuer.certificate_chain_pem,
            &issuer_key.serialize_pem(),
            &leaf_request,
            issuer.not_after
        )
        .is_err()
    );
    let (_, broadened) = request(&["child.api.example.test"], &["127.0.0.1"], false);
    assert!(
        sign_csr_with_device_issuer(
            &issuer.certificate_chain_pem,
            &issuer_key.serialize_pem(),
            &broadened,
            NOW
        )
        .is_err()
    );
    let other_key = KeyPair::generate().unwrap();
    assert!(
        validate_device_issuer_chain(
            &issuer.certificate_chain_pem,
            &other_key.serialize_pem(),
            &issuer_request.dns_names,
            &issuer_request.ip_addresses,
            NOW
        )
        .is_err()
    );
}

fn malicious_leaf(issuer_pem: &str, issuer_key: &KeyPair, name: &str) -> String {
    let chain = certificate_chain(issuer_pem).unwrap();
    let issuer = Issuer::from_ca_cert_der(&chain[0].as_slice().into(), issuer_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let mut params = parameters(name, NOW, NOW + DAY).unwrap();
    params.subject_alt_names = CertificateParams::new(vec![name.into()])
        .unwrap()
        .subject_alt_names;
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    format!(
        "{}{}",
        params.signed_by(&key, &issuer).unwrap().pem(),
        issuer_pem
    )
}

#[test]
fn webpki_enforces_exact_dns_delegation_and_excludes_ip_when_dns_only() {
    let (key, request) = request(&["api.example.test"], &[], true);
    let issuer = sign(&request, true).unwrap();
    let allowed = malicious_leaf(&issuer.certificate_chain_pem, &key, "api.example.test");
    trusted_by_root(&allowed, "api.example.test", NOW).unwrap();
    for blocked in [
        "child.api.example.test",
        "other.example.test",
        "127.0.0.1",
        "::1",
    ] {
        let chain = malicious_leaf(&issuer.certificate_chain_pem, &key, blocked);
        assert!(
            trusted_by_root(&chain, blocked, NOW).is_err(),
            "unexpectedly trusted {blocked}"
        );
    }
}

#[test]
fn webpki_enforces_exact_ips_and_excludes_dns_when_ip_only() {
    let (key, request) = request(&[], &["127.0.0.1", "::1"], true);
    let issuer = sign(&request, true).unwrap();
    for allowed in ["127.0.0.1", "::1"] {
        trusted_by_root(
            &malicious_leaf(&issuer.certificate_chain_pem, &key, allowed),
            allowed,
            NOW,
        )
        .unwrap();
    }
    for blocked in ["127.0.0.2", "::2", "api.example.test"] {
        assert!(
            trusted_by_root(
                &malicious_leaf(&issuer.certificate_chain_pem, &key, blocked),
                blocked,
                NOW
            )
            .is_err()
        );
    }
}

#[test]
fn renewing_and_rewrapping_preserve_root_trust_and_require_matching_root_backup() {
    let authority = authority();
    let renewed = renew_certificate_authority_vault(
        ACCOUNT,
        ID,
        PASSWORD,
        &authority.vault,
        &authority.root_vault,
        NOW + DAY,
    )
    .unwrap();
    assert_eq!(
        renewed.public_bundle.root_certificate_pem,
        authority.public_bundle.root_certificate_pem
    );
    assert_ne!(
        renewed.public_bundle.issuer_certificate_pem,
        authority.public_bundle.issuer_certificate_pem
    );
    assert_eq!(
        renewed.public_bundle.issuer_not_after,
        authority.public_bundle.issuer_not_after + DAY
    );
    assert!(
        rewrap_certificate_authority_vault(
            ACCOUNT,
            ID,
            PASSWORD,
            b"new certificate password",
            &renewed.vault,
            &authority.root_vault
        )
        .is_err()
    );
    let rewrapped = rewrap_certificate_authority_vault(
        ACCOUNT,
        ID,
        PASSWORD,
        b"new certificate password",
        &renewed.vault,
        &renewed.root_vault,
    )
    .unwrap();
    assert_eq!(
        inspect_certificate_authority_vault(
            ACCOUNT,
            ID,
            b"new certificate password",
            &rewrapped.vault
        )
        .unwrap(),
        renewed.public_bundle
    );
    assert!(inspect_certificate_authority_vault(ACCOUNT, ID, PASSWORD, &rewrapped.vault).is_err());
    assert!(
        renew_certificate_authority_vault(
            ACCOUNT,
            ID,
            PASSWORD,
            &authority.vault,
            &authority.root_vault,
            authority.public_bundle.not_after
        )
        .is_err()
    );
}
