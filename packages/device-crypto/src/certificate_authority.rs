use crate::vault;
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::validate_certificate_id;
use rand_core::{OsRng, RngCore};
use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, CidrSubnet,
    DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, NameConstraints, PKCS_ECDSA_P256_SHA256, PublicKeyData, SanType, SerialNumber,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, net::IpAddr};
use time::OffsetDateTime;
use x509_parser::{
    certification_request::X509CertificationRequest,
    cri_attributes::ParsedCriAttribute,
    extensions::{GeneralName, ParsedExtension},
    prelude::{FromDer, X509Certificate},
};
use zeroize::{Zeroize, Zeroizing};

const DAY: i64 = 86_400;
const MAX_NAMES: usize = 32;
const MAX_PEM_BYTES: usize = 12 * 1024;
const VAULT_CONTEXT: &[u8] = b"flow-like/certificate-authority/v1\0";

#[cfg(test)]
#[path = "certificate_authority_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateAuthoritySpec {
    pub account_binding: String,
    pub authority_id: String,
    pub label: String,
    pub dns_suffixes: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub validity_days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateAuthorityPublic {
    pub account_binding: String,
    pub authority_id: String,
    pub label: String,
    pub dns_suffixes: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub root_certificate_pem: String,
    pub issuer_certificate_pem: String,
    pub sha256_fingerprint: String,
    pub not_before: i64,
    pub not_after: i64,
    pub issuer_not_after: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateAuthorityVault {
    pub public_bundle: CertificateAuthorityPublic,
    pub vault: Vec<u8>,
    pub root_vault: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateSigningRequest {
    pub csr_pem: String,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub validity_days: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCertificateChain {
    pub certificate_chain_pem: String,
    pub not_before: i64,
    pub not_after: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritySecrets {
    version: u32,
    root: bool,
    public: CertificateAuthorityPublic,
    private_key_der: Vec<u8>,
}

impl Drop for AuthoritySecrets {
    fn drop(&mut self) {
        self.private_key_der.zeroize();
    }
}

fn context(account_binding: &str, authority_id: &str, root: bool) -> Result<Vec<u8>> {
    validate_certificate_id(authority_id)?;
    ensure!(
        !account_binding.is_empty() && account_binding.len() <= 4096,
        "Invalid certificate authority account binding"
    );
    // Length-delimited JSON prevents ambiguous account/authority concatenations.
    let mut context = VAULT_CONTEXT.to_vec();
    context.extend(serde_json::to_vec(&(account_binding, authority_id, root))?);
    Ok(context)
}

fn open_authority(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    root: bool,
) -> Result<AuthoritySecrets> {
    let plaintext = vault::open(
        password,
        &context(account_binding, authority_id, root)?,
        ciphertext,
    )?;
    let secrets: AuthoritySecrets =
        serde_json::from_slice(&plaintext).context("Invalid certificate authority vault")?;
    ensure!(
        secrets.version == 1
            && secrets.root == root
            && secrets.public.authority_id == authority_id
            && secrets.public.account_binding == account_binding,
        "Certificate authority vault binding failed"
    );
    Ok(secrets)
}

fn seal_authority(password: &[u8], secrets: &AuthoritySecrets) -> Result<Vec<u8>> {
    let plaintext = Zeroizing::new(serde_json::to_vec(secrets)?);
    vault::seal(
        password,
        &context(
            &secrets.public.account_binding,
            &secrets.public.authority_id,
            secrets.root,
        )?,
        &plaintext,
    )
}

fn checked_time(now: i64) -> Result<OffsetDateTime> {
    ensure!(now > 300, "Invalid certificate issuance time");
    OffsetDateTime::from_unix_timestamp(now).context("Invalid certificate issuance time")
}

fn names(dns: &[String], ips: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    ensure!(
        !dns.is_empty() || !ips.is_empty(),
        "At least one DNS name or IP address is required"
    );
    ensure!(
        dns.len() + ips.len() <= MAX_NAMES,
        "Too many certificate names"
    );
    let mut normalized_dns = BTreeSet::new();
    for name in dns {
        ensure!(
            !name.is_empty()
                && name.len() <= 253
                && name.is_ascii()
                && name.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
                && !name
                    .rsplit('.')
                    .next()
                    .unwrap()
                    .bytes()
                    .all(|c| c.is_ascii_digit()),
            "Use DNS names without wildcards, ports, or trailing dots"
        );
        ensure!(
            normalized_dns.insert(name.to_ascii_lowercase()),
            "Duplicate DNS name"
        );
    }
    let mut normalized_ips = BTreeSet::new();
    for ip in ips {
        let parsed: IpAddr = ip.parse().context("Invalid certificate IP address")?;
        ensure!(
            normalized_ips.insert(parsed.to_string()),
            "Duplicate IP address"
        );
    }
    Ok((
        normalized_dns.into_iter().collect(),
        normalized_ips.into_iter().collect(),
    ))
}

fn in_dns_subtree(name: &str, suffix: &str) -> bool {
    suffix.is_empty()
        || (suffix.starts_with('.')
            && name
                .to_ascii_lowercase()
                .ends_with(&suffix.to_ascii_lowercase()))
        || name.eq_ignore_ascii_case(suffix)
        || name
            .to_ascii_lowercase()
            .strip_suffix(&suffix.to_ascii_lowercase())
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn approved_names(
    public: &CertificateAuthorityPublic,
    dns: &[String],
    ips: &[String],
) -> Result<()> {
    ensure!(
        dns.iter().all(|name| public
            .dns_suffixes
            .iter()
            .any(|suffix| in_dns_subtree(name, suffix)))
            && ips.iter().all(|ip| public.ip_addresses.contains(ip)),
        "Certificate names exceed the authority's approved scope"
    );
    Ok(())
}

fn constraints(dns: &[String], ips: &[String], exact: bool) -> Result<NameConstraints> {
    let mut permitted_subtrees = Vec::new();
    let mut excluded_subtrees = Vec::new();
    if dns.is_empty() {
        excluded_subtrees.push(GeneralSubtree::DnsName(String::new()));
    } else {
        for name in dns {
            if exact {
                ensure!(
                    !dns.iter()
                        .any(|other| other != name && in_dns_subtree(other, name)),
                    "A delegated issuer cannot combine a DNS name with one of its subdomains"
                );
                excluded_subtrees.push(GeneralSubtree::DnsName(format!(".{name}")));
            }
            permitted_subtrees.push(GeneralSubtree::DnsName(name.clone()));
        }
    }
    if ips.is_empty() {
        excluded_subtrees.extend([
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([0; 4], 0)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix([0; 16], 0)),
        ]);
    } else {
        for ip in ips {
            let address: IpAddr = ip.parse()?;
            permitted_subtrees.push(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(
                address,
                if address.is_ipv4() { 32 } else { 128 },
            )));
        }
    }
    Ok(NameConstraints {
        permitted_subtrees,
        excluded_subtrees,
    })
}

fn parameters(label: &str, now: i64, not_after: i64) -> Result<CertificateParams> {
    checked_time(now)?;
    ensure!(not_after > now, "Certificate authority has expired");
    let mut serial = [0u8; 20];
    OsRng.fill_bytes(&mut serial);
    serial[0] &= 0x7f;
    serial[0] |= 1;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, label);
    let mut params = CertificateParams::default();
    params.distinguished_name = dn;
    params.not_before = checked_time(now - 300)?;
    params.not_after = checked_time(not_after)?;
    params.serial_number = Some(SerialNumber::from_slice(&serial));
    params.use_authority_key_identifier_extension = true;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    Ok(params)
}

fn ca_parameters(
    label: &str,
    now: i64,
    not_after: i64,
    path_len: u8,
    scope: NameConstraints,
) -> Result<CertificateParams> {
    let mut params = parameters(label, now, not_after)?;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(path_len));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(scope);
    Ok(params)
}

pub fn create_certificate_authority_vault(
    spec: &CertificateAuthoritySpec,
    password: &[u8],
    now: i64,
) -> Result<CertificateAuthorityVault> {
    context(&spec.account_binding, &spec.authority_id, false)?;
    ensure!(
        !spec.label.trim().is_empty()
            && spec.label.len() <= 64
            && !spec.label.chars().any(char::is_control),
        "Authority label must contain 1 to 64 printable bytes"
    );
    ensure!(
        (365..=3650).contains(&spec.validity_days),
        "Root lifetime must be between 365 and 3650 days"
    );
    let (dns, ips) = names(&spec.dns_suffixes, &spec.ip_addresses)?;
    let not_after = now
        .checked_add(i64::from(spec.validity_days) * DAY)
        .context("Certificate time overflow")?;
    let root_key = Zeroizing::new(KeyPair::generate()?);
    let root_params = ca_parameters(
        &format!("{} root", spec.label),
        now,
        not_after,
        2,
        constraints(&dns, &ips, false)?,
    )?;
    let root_certificate = root_params.self_signed(&*root_key)?;
    let issuer_not_after = (now + 365 * DAY).min(not_after);
    let issuer_key = Zeroizing::new(KeyPair::generate()?);
    let issuer_params = ca_parameters(
        &format!("{} service issuer", spec.label),
        now,
        issuer_not_after,
        1,
        constraints(&dns, &ips, false)?,
    )?;
    let issuer_certificate =
        issuer_params.signed_by(&*issuer_key, &Issuer::from_params(&root_params, &*root_key))?;
    let public = CertificateAuthorityPublic {
        account_binding: spec.account_binding.clone(),
        authority_id: spec.authority_id.clone(),
        label: spec.label.clone(),
        dns_suffixes: dns,
        ip_addresses: ips,
        root_certificate_pem: root_certificate.pem(),
        issuer_certificate_pem: issuer_certificate.pem(),
        sha256_fingerprint: format!("{:x}", Sha256::digest(root_certificate.der())),
        not_before: now - 300,
        not_after,
        issuer_not_after,
    };
    wrap_keys(
        password,
        public,
        root_key.serialize_der(),
        issuer_key.serialize_der(),
    )
}

fn wrap_keys(
    password: &[u8],
    public: CertificateAuthorityPublic,
    root_key: Vec<u8>,
    issuer_key: Vec<u8>,
) -> Result<CertificateAuthorityVault> {
    let root = AuthoritySecrets {
        version: 1,
        root: true,
        public: public.clone(),
        private_key_der: root_key,
    };
    let issuer = AuthoritySecrets {
        version: 1,
        root: false,
        public: public.clone(),
        private_key_der: issuer_key,
    };
    Ok(CertificateAuthorityVault {
        public_bundle: public,
        vault: seal_authority(password, &issuer)?,
        root_vault: seal_authority(password, &root)?,
    })
}

pub fn inspect_certificate_authority_vault(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
) -> Result<CertificateAuthorityPublic> {
    Ok(
        open_authority(account_binding, authority_id, password, ciphertext, false)?
            .public
            .clone(),
    )
}

fn validate_authority_backup(issuer: &AuthoritySecrets, root: &AuthoritySecrets) -> Result<()> {
    ensure!(
        issuer.public == root.public,
        "Root and issuing authority backups do not match"
    );
    let public = &issuer.public;
    let (dns, ips) = names(&public.dns_suffixes, &public.ip_addresses)?;
    ensure!(
        dns == public.dns_suffixes && ips == public.ip_addresses,
        "Authority scope is not canonical"
    );
    let scope = constraints(&dns, &ips, false)?;
    let root_der = single_pem(&public.root_certificate_pem, "CERTIFICATE")?;
    let issuer_der = single_pem(&public.issuer_certificate_pem, "CERTIFICATE")?;
    let mut parsed = Vec::new();
    for (der, private_key, path_length) in [
        (&root_der, &root.private_key_der, 2),
        (&issuer_der, &issuer.private_key_der, 1),
    ] {
        let (rest, cert) = X509Certificate::from_der(der)
            .map_err(|_| anyhow::anyhow!("Invalid authority certificate"))?;
        ensure!(rest.is_empty(), "Trailing data in authority certificate");
        cert.extensions_map()?;
        let basic = cert
            .basic_constraints()?
            .context("Authority has no CA constraints")?;
        ensure!(
            basic.critical
                && basic.value.ca
                && basic.value.path_len_constraint == Some(path_length),
            "Invalid authority path length"
        );
        let usage = cert.key_usage()?.context("Authority has no key usage")?;
        ensure!(
            usage.value.key_cert_sign() && usage.value.flags & !0x60 == 0,
            "Invalid authority key usage"
        );
        let eku = cert
            .extended_key_usage()?
            .context("Authority has no TLS purpose restriction")?;
        ensure!(
            eku.value.server_auth
                && !eku.value.any
                && !eku.value.client_auth
                && !eku.value.code_signing
                && !eku.value.email_protection
                && !eku.value.time_stamping
                && !eku.value.ocsp_signing
                && eku.value.other.is_empty(),
            "Authority must be restricted to TLS server authentication"
        );
        let restrictions = cert
            .name_constraints()?
            .context("Authority has no name constraints")?;
        ensure!(
            restrictions.critical
                && constraint_tokens(
                    restrictions
                        .value
                        .permitted_subtrees
                        .as_deref()
                        .unwrap_or_default()
                )? == expected_constraint_tokens(&scope.permitted_subtrees)?
                && constraint_tokens(
                    restrictions
                        .value
                        .excluded_subtrees
                        .as_deref()
                        .unwrap_or_default()
                )? == expected_constraint_tokens(&scope.excluded_subtrees)?,
            "Authority scope does not match its certificates"
        );
        let key = Zeroizing::new(KeyPair::try_from(private_key.as_slice())?);
        ensure!(
            key.algorithm() == &PKCS_ECDSA_P256_SHA256
                && key.subject_public_key_info() == cert.public_key().raw,
            "Authority certificate and private key do not match"
        );
        parsed.push(cert);
    }
    ensure!(
        parsed[0].subject() == parsed[0].issuer() && parsed[1].issuer() == parsed[0].subject(),
        "Authority chain issuer does not match"
    );
    parsed[0]
        .verify_signature(None)
        .context("Invalid root signature")?;
    parsed[1]
        .verify_signature(Some(parsed[0].public_key()))
        .context("Invalid issuing authority signature")?;
    ensure!(
        public.sha256_fingerprint == format!("{:x}", Sha256::digest(&root_der))
            && public.not_before == parsed[0].validity().not_before.timestamp()
            && public.not_after == parsed[0].validity().not_after.timestamp()
            && public.issuer_not_after == parsed[1].validity().not_after.timestamp()
            && public.issuer_not_after <= public.not_after
            && parsed[1].validity().not_before.timestamp() >= public.not_before,
        "Authority metadata does not match its certificates"
    );
    Ok(())
}

pub fn inspect_certificate_authority_backup(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    root_ciphertext: &[u8],
) -> Result<CertificateAuthorityPublic> {
    let issuer = open_authority(account_binding, authority_id, password, ciphertext, false)?;
    let root = open_authority(
        account_binding,
        authority_id,
        password,
        root_ciphertext,
        true,
    )?;
    validate_authority_backup(&issuer, &root)?;
    Ok(issuer.public.clone())
}

pub fn rewrap_certificate_authority_vault(
    account_binding: &str,
    authority_id: &str,
    current_password: &[u8],
    new_password: &[u8],
    ciphertext: &[u8],
    root_ciphertext: &[u8],
) -> Result<CertificateAuthorityVault> {
    let issuer = open_authority(
        account_binding,
        authority_id,
        current_password,
        ciphertext,
        false,
    )?;
    let root = open_authority(
        account_binding,
        authority_id,
        current_password,
        root_ciphertext,
        true,
    )?;
    validate_authority_backup(&issuer, &root)?;
    Ok(CertificateAuthorityVault {
        public_bundle: issuer.public.clone(),
        vault: seal_authority(new_password, &issuer)?,
        root_vault: seal_authority(new_password, &root)?,
    })
}

pub fn renew_certificate_authority_vault(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    root_ciphertext: &[u8],
    now: i64,
) -> Result<CertificateAuthorityVault> {
    let issuer = open_authority(account_binding, authority_id, password, ciphertext, false)?;
    let root = open_authority(
        account_binding,
        authority_id,
        password,
        root_ciphertext,
        true,
    )?;
    validate_authority_backup(&issuer, &root)?;
    let mut public = issuer.public.clone();
    ensure!(now >= public.not_before, "Root authority is not yet valid");
    let not_after = now
        .checked_add(365 * DAY)
        .context("Certificate time overflow")?
        .min(public.not_after);
    let root_key = Zeroizing::new(KeyPair::try_from(root.private_key_der.as_slice())?);
    let root_issuer = Issuer::from_ca_cert_pem(&public.root_certificate_pem, &*root_key)?;
    let issuer_key = Zeroizing::new(KeyPair::generate()?);
    let params = ca_parameters(
        &format!("{} service issuer", public.label),
        now,
        not_after,
        1,
        constraints(&public.dns_suffixes, &public.ip_addresses, false)?,
    )?;
    public.issuer_certificate_pem = params.signed_by(&*issuer_key, &root_issuer)?.pem();
    public.issuer_not_after = not_after;
    wrap_keys(
        password,
        public,
        root.private_key_der.clone(),
        issuer_key.serialize_der(),
    )
}

fn single_pem(pem: &str, label: &str) -> Result<Vec<u8>> {
    let trimmed = pem.trim();
    ensure!(
        trimmed.len() <= MAX_PEM_BYTES
            && trimmed.starts_with(&format!("-----BEGIN {label}-----"))
            && trimmed.ends_with(&format!("-----END {label}-----")),
        "Invalid PEM header or size"
    );
    let (rest, decoded) = x509_parser::pem::parse_x509_pem(trimmed.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid PEM encoding"))?;
    ensure!(
        decoded.label == label && rest.iter().all(u8::is_ascii_whitespace),
        "Expected exactly one PEM block"
    );
    Ok(decoded.contents)
}

fn checked_csr(
    request: &CertificateSigningRequest,
    issuer: bool,
) -> Result<(CertificateSigningRequestParams, Vec<String>, Vec<String>)> {
    ensure!(
        (1..=if issuer { 365 } else { 397 }).contains(&request.validity_days),
        "Invalid certificate lifetime"
    );
    let (dns, ips) = names(&request.dns_names, &request.ip_addresses)?;
    let der = single_pem(&request.csr_pem, "CERTIFICATE REQUEST")?;
    let (rest, parsed) = X509CertificationRequest::from_der(&der)
        .map_err(|_| anyhow::anyhow!("Invalid certificate signing request"))?;
    ensure!(
        rest.is_empty(),
        "Trailing data in certificate signing request"
    );
    let mut attributes = BTreeSet::new();
    for attribute in parsed.certification_request_info.attributes() {
        ensure!(
            attributes.insert(attribute.oid.to_id_string()),
            "Duplicate CSR attribute"
        );
        let ParsedCriAttribute::ExtensionRequest(requested) = attribute.parsed_attribute() else {
            bail!("Unsupported CSR attribute");
        };
        let mut extensions = BTreeSet::new();
        for extension in &requested.extensions {
            ensure!(
                extensions.insert(extension.oid.to_id_string()),
                "Duplicate CSR extension"
            );
        }
    }
    let csr = CertificateSigningRequestParams::from_der(&der.as_slice().into())
        .context("Invalid or unsupported signed CSR")?;
    ensure!(
        csr.public_key.algorithm() == &PKCS_ECDSA_P256_SHA256,
        "Device certificate requests must use ECDSA P-256"
    );
    ensure!(
        csr.params
            .extended_key_usages
            .iter()
            .all(|usage| *usage == ExtendedKeyUsagePurpose::ServerAuth),
        "Only TLS server authentication may be requested"
    );
    if issuer {
        ensure!(
            csr.params.is_ca == IsCa::Ca(BasicConstraints::Constrained(0)),
            "Device issuer must request CA path length zero"
        );
        ensure!(
            csr.params
                .key_usages
                .contains(&KeyUsagePurpose::KeyCertSign)
                && csr.params.key_usages.iter().all(|usage| matches!(
                    usage,
                    KeyUsagePurpose::KeyCertSign | KeyUsagePurpose::CrlSign
                )),
            "Invalid device issuer key usage"
        );
    } else {
        ensure!(
            matches!(csr.params.is_ca, IsCa::NoCa | IsCa::ExplicitNoCa),
            "A service request cannot become a certificate authority"
        );
        ensure!(
            csr.params
                .key_usages
                .iter()
                .all(|usage| *usage == KeyUsagePurpose::DigitalSignature),
            "Invalid service key usage"
        );
    }
    let mut requested_dns = Vec::new();
    let mut requested_ips = Vec::new();
    for name in &csr.params.subject_alt_names {
        match name {
            SanType::DnsName(name) => requested_dns.push(name.as_str().to_owned()),
            SanType::IpAddress(address) => requested_ips.push(address.to_string()),
            _ => bail!("Only DNS and IP subject alternative names are supported"),
        }
    }
    let requested = names(&requested_dns, &requested_ips)?;
    ensure!(
        requested == (dns.clone(), ips.clone()),
        "CSR names do not match the explicitly approved names"
    );
    Ok((csr, dns, ips))
}

fn signed_request(
    mut csr: CertificateSigningRequestParams,
    request: &CertificateSigningRequest,
    dns: &[String],
    ips: &[String],
    now: i64,
    parent_expiry: i64,
    issuer: &Issuer<'_, impl rcgen::SigningKey>,
    delegate: bool,
    chain: &str,
) -> Result<SignedCertificateChain> {
    let not_after = now
        .checked_add(i64::from(request.validity_days) * DAY)
        .context("Certificate time overflow")?
        .min(parent_expiry);
    let name = dns
        .first()
        .or_else(|| ips.first())
        .context("Certificate has no approved name")?;
    let mut params = if delegate {
        ca_parameters(
            &format!("{name} service issuer"),
            now,
            not_after,
            0,
            constraints(dns, ips, true)?,
        )?
    } else {
        let mut params = parameters(name, now, not_after)?;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params
    };
    params.subject_alt_names = dns
        .iter()
        .map(|name| Ok(SanType::DnsName(name.clone().try_into()?)))
        .chain(ips.iter().map(|ip| Ok(SanType::IpAddress(ip.parse()?))))
        .collect::<Result<Vec<_>>>()?;
    csr.params = params;
    let certificate_chain_pem = format!("{}{}", csr.signed_by(issuer)?.pem(), chain);
    ensure!(
        certificate_chain_pem.len() <= MAX_PEM_BYTES,
        "Certificate chain exceeds the management limit"
    );
    Ok(SignedCertificateChain {
        certificate_chain_pem,
        not_before: now - 300,
        not_after,
    })
}

fn sign_with_authority(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    request: &CertificateSigningRequest,
    now: i64,
    delegate: bool,
) -> Result<SignedCertificateChain> {
    let (csr, dns, ips) = checked_csr(request, delegate)?;
    let secrets = open_authority(account_binding, authority_id, password, ciphertext, false)?;
    approved_names(&secrets.public, &dns, &ips)?;
    ensure!(
        now >= secrets.public.not_before,
        "Certificate authority is not yet valid"
    );
    let key = Zeroizing::new(KeyPair::try_from(secrets.private_key_der.as_slice())?);
    let issuer = Issuer::from_ca_cert_pem(&secrets.public.issuer_certificate_pem, &*key)?;
    let chain = format!(
        "{}{}",
        secrets.public.issuer_certificate_pem, secrets.public.root_certificate_pem
    );
    signed_request(
        csr,
        request,
        &dns,
        &ips,
        now,
        secrets
            .public
            .issuer_not_after
            .min(secrets.public.not_after),
        &issuer,
        delegate,
        &chain,
    )
}

pub fn sign_service_certificate(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    request: &CertificateSigningRequest,
    now: i64,
) -> Result<SignedCertificateChain> {
    sign_with_authority(
        account_binding,
        authority_id,
        password,
        ciphertext,
        request,
        now,
        false,
    )
}

pub fn sign_device_certificate_issuer(
    account_binding: &str,
    authority_id: &str,
    password: &[u8],
    ciphertext: &[u8],
    request: &CertificateSigningRequest,
    now: i64,
) -> Result<SignedCertificateChain> {
    sign_with_authority(
        account_binding,
        authority_id,
        password,
        ciphertext,
        request,
        now,
        true,
    )
}

fn certificate_chain(pem: &str) -> Result<Vec<Vec<u8>>> {
    ensure!(
        pem.len() <= MAX_PEM_BYTES,
        "Issuer chain exceeds the management limit"
    );
    let mut remaining = pem.trim().as_bytes();
    let mut chain = Vec::new();
    while !remaining.is_empty() {
        ensure!(
            remaining.starts_with(b"-----BEGIN CERTIFICATE-----"),
            "Issuer chain must contain only certificates"
        );
        let (rest, decoded) = x509_parser::pem::parse_x509_pem(remaining)
            .map_err(|_| anyhow::anyhow!("Invalid issuer chain PEM"))?;
        ensure!(
            decoded.label == "CERTIFICATE",
            "Invalid issuer chain PEM label"
        );
        chain.push(decoded.contents);
        ensure!(
            chain.len() <= 3,
            "Expected device issuer, organisation issuer, and root"
        );
        remaining = rest.trim_ascii();
    }
    ensure!(
        chain.len() == 3,
        "Expected device issuer, organisation issuer, and root"
    );
    Ok(chain)
}

fn subnet_token(bytes: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    format!("ip:{}", STANDARD.encode(bytes))
}

fn expected_constraint_tokens(subtrees: &[GeneralSubtree]) -> Result<BTreeSet<String>> {
    subtrees
        .iter()
        .map(|subtree| {
            Ok(match subtree {
                GeneralSubtree::DnsName(name) => format!("dns:{}", name.to_ascii_lowercase()),
                GeneralSubtree::IpAddress(CidrSubnet::V4(address, mask)) => {
                    subnet_token(&[address.as_slice(), mask.as_slice()].concat())
                }
                GeneralSubtree::IpAddress(CidrSubnet::V6(address, mask)) => {
                    subnet_token(&[address.as_slice(), mask.as_slice()].concat())
                }
                _ => bail!("Unsupported issuer name constraint"),
            })
        })
        .collect()
}

fn constraint_tokens(
    subtrees: &[x509_parser::extensions::GeneralSubtree<'_>],
) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for subtree in subtrees {
        let value = match &subtree.base {
            GeneralName::DNSName(name) => format!("dns:{}", name.to_ascii_lowercase()),
            GeneralName::IPAddress(bytes) if matches!(bytes.len(), 8 | 32) => subnet_token(bytes),
            _ => bail!("Unsupported issuer name constraint"),
        };
        ensure!(result.insert(value), "Duplicate issuer name constraint");
    }
    Ok(result)
}

fn ip_matches_subnet(ip: &str, subnet: &[u8]) -> Result<bool> {
    let bytes = match ip.parse::<IpAddr>()? {
        IpAddr::V4(address) => address.octets().to_vec(),
        IpAddr::V6(address) => address.octets().to_vec(),
    };
    if subnet.len() != 2 * bytes.len() {
        return Ok(false);
    }
    let (network, mask) = subnet.split_at(bytes.len());
    Ok(bytes
        .iter()
        .zip(network)
        .zip(mask)
        .all(|((address, network), mask)| address & mask == network & mask))
}

fn check_chain_names(cert: &X509Certificate<'_>, dns: &[String], ips: &[String]) -> Result<()> {
    let extension = cert
        .name_constraints()?
        .context("Issuer has no name constraints")?;
    ensure!(
        extension.critical,
        "Issuer name constraints must be critical"
    );
    let permitted = extension
        .value
        .permitted_subtrees
        .as_deref()
        .unwrap_or_default();
    let excluded = extension
        .value
        .excluded_subtrees
        .as_deref()
        .unwrap_or_default();
    constraint_tokens(permitted)?;
    constraint_tokens(excluded)?;
    for name in dns {
        let permit_dns: Vec<_> = permitted
            .iter()
            .filter_map(|subtree| match &subtree.base {
                GeneralName::DNSName(value) => Some(*value),
                _ => None,
            })
            .collect();
        ensure!(
            permit_dns.is_empty() || permit_dns.iter().any(|suffix| in_dns_subtree(name, suffix)),
            "DNS name is outside the issuer chain's permitted scope"
        );
        ensure!(!excluded.iter().any(|subtree| matches!(&subtree.base, GeneralName::DNSName(suffix) if in_dns_subtree(name,suffix))), "DNS name is excluded by the issuer chain");
    }
    for ip in ips {
        let permit_ip: Vec<_> = permitted
            .iter()
            .filter_map(|subtree| match &subtree.base {
                GeneralName::IPAddress(bytes) => Some(*bytes),
                _ => None,
            })
            .collect();
        let allowed = permit_ip
            .iter()
            .map(|subnet| ip_matches_subnet(ip, subnet))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            allowed.is_empty() || allowed.contains(&true),
            "IP address is outside the issuer chain's permitted scope"
        );
        for subtree in excluded {
            if let GeneralName::IPAddress(bytes) = &subtree.base {
                ensure!(
                    !ip_matches_subnet(ip, bytes)?,
                    "IP address is excluded by the issuer chain"
                );
            }
        }
    }
    Ok(())
}

/// Validate the complete delegated authority before storing it or using its key.
/// The encrypted management command supplies the approved exact DNS/IP scope.
pub fn validate_device_issuer_chain(
    chain_pem: &str,
    key_pem: &str,
    dns_names: &[String],
    ip_addresses: &[String],
    now: i64,
) -> Result<(i64, i64)> {
    checked_time(now)?;
    let (dns, ips) = names(dns_names, ip_addresses)?;
    let expected = constraints(&dns, &ips, true)?;
    let chain = certificate_chain(chain_pem)?;
    let parsed = chain
        .iter()
        .map(|der| {
            let (rest, cert) = X509Certificate::from_der(der)
                .map_err(|_| anyhow::anyhow!("Invalid issuer certificate"))?;
            ensure!(rest.is_empty(), "Trailing data in issuer certificate");
            Ok(cert)
        })
        .collect::<Result<Vec<_>>>()?;
    for (index, certificate) in parsed.iter().enumerate() {
        certificate.extensions_map()?;
        for extension in certificate.extensions() {
            ensure!(
                !matches!(
                    extension.parsed_extension(),
                    ParsedExtension::ParseError { .. }
                ) && !(extension.critical && extension.parsed_extension().unsupported()),
                "Invalid or unsupported critical issuer extension"
            );
        }
        let basic = certificate
            .basic_constraints()?
            .context("Issuer has no CA constraints")?;
        ensure!(
            basic.critical
                && basic.value.ca
                && basic.value.path_len_constraint == Some(index as u32),
            "Invalid issuer CA path length"
        );
        let usage = certificate
            .key_usage()?
            .context("Issuer has no key usage")?;
        ensure!(
            usage.value.key_cert_sign() && usage.value.flags & !0x60 == 0,
            "Issuer may only sign certificates and revocation lists"
        );
        let eku = certificate
            .extended_key_usage()?
            .context("Issuer has no TLS purpose restriction")?;
        ensure!(
            eku.value.server_auth
                && !eku.value.any
                && !eku.value.client_auth
                && !eku.value.code_signing
                && !eku.value.email_protection
                && !eku.value.time_stamping
                && !eku.value.ocsp_signing
                && eku.value.other.is_empty(),
            "Issuer must be restricted to TLS server authentication"
        );
        check_chain_names(certificate, &dns, &ips)?;
        ensure!(
            certificate.validity().not_before.timestamp() <= now
                && certificate.validity().not_after.timestamp() > now,
            "Issuer chain is expired or not yet valid"
        );
        let signer = parsed.get(index + 1).unwrap_or(certificate);
        ensure!(
            certificate.issuer() == signer.subject(),
            "Issuer chain order is invalid"
        );
        certificate
            .verify_signature(Some(signer.public_key()))
            .map_err(|_| anyhow::anyhow!("Issuer chain signature is invalid"))?;
    }
    let leaf_constraints = parsed[0].name_constraints()?.unwrap();
    ensure!(
        constraint_tokens(
            leaf_constraints
                .value
                .permitted_subtrees
                .as_deref()
                .unwrap_or_default()
        )? == expected_constraint_tokens(&expected.permitted_subtrees)?
            && constraint_tokens(
                leaf_constraints
                    .value
                    .excluded_subtrees
                    .as_deref()
                    .unwrap_or_default()
            )? == expected_constraint_tokens(&expected.excluded_subtrees)?,
        "Delegated issuer does not match the approved exact DNS/IP scope"
    );
    ensure!(
        key_pem.len() <= MAX_PEM_BYTES,
        "Issuer key exceeds the size limit"
    );
    let key = Zeroizing::new(KeyPair::from_pem(key_pem)?);
    ensure!(
        key.algorithm() == &PKCS_ECDSA_P256_SHA256
            && key.subject_public_key_info() == parsed[0].public_key().raw,
        "Issuer certificate and private key do not match"
    );
    Ok((
        parsed
            .iter()
            .map(|cert| cert.validity().not_before.timestamp())
            .max()
            .unwrap(),
        parsed
            .iter()
            .map(|cert| cert.validity().not_after.timestamp())
            .min()
            .unwrap(),
    ))
}

/// Device-local signing never exports the issuer or service private key.
pub fn sign_csr_with_device_issuer(
    chain_pem: &str,
    key_pem: &str,
    request: &CertificateSigningRequest,
    now: i64,
) -> Result<SignedCertificateChain> {
    let (csr, dns, ips) = checked_csr(request, false)?;
    let (_, expires) = validate_device_issuer_chain(chain_pem, key_pem, &dns, &ips, now)?;
    let chain = certificate_chain(chain_pem)?;
    let key = Zeroizing::new(KeyPair::from_pem(key_pem)?);
    let issuer = Issuer::from_ca_cert_der(&chain[0].as_slice().into(), &*key)?;
    signed_request(
        csr, request, &dns, &ips, now, expires, &issuer, false, chain_pem,
    )
}
