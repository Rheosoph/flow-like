use std::{io::Read, path::Path};

use flow_like_types::{Result, anyhow, bail};

const MAX_CA_BUNDLE_BYTES: u64 = 256 * 1024;

pub(crate) struct Clients {
    pub(crate) jwks: reqwest::Client,
    pub(crate) user_info: reqwest::Client,
}

/// Trust an operator-supplied CA only for requests to the configured identity provider.
/// Loading the file while constructing API state makes invalid trust fail at startup.
pub(crate) fn clients_from_env(_jwks_url: Option<&str>) -> Result<Clients> {
    let path = std::env::var_os("OIDC_CA_CERT_FILE")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    // Preserve the benchmark issuer's loopback-only CA override.
    #[cfg(feature = "benchmark-stack")]
    let path = path.or_else(|| {
        let url = reqwest::Url::parse(_jwks_url?).ok()?;
        matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            .then(|| std::env::var_os("SSL_CERT_FILE"))
            .flatten()
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    });
    clients_from_ca_file(path.as_deref())
}

fn clients_from_ca_file(path: Option<&Path>) -> Result<Clients> {
    let mut jwks = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none());
    // Existing external providers may use HTTP or redirects for user-info.
    // Keep the former Client::new() policy while sharing only the CA trust.
    let mut user_info = reqwest::Client::builder();
    if let Some(path) = path {
        let file = std::fs::File::open(path)
            .map_err(|_| anyhow!("OIDC_CA_CERT_FILE could not be read"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_CA_BUNDLE_BYTES {
            bail!("OIDC_CA_CERT_FILE must be a regular PEM file of at most 256 KiB");
        }
        let mut pem = Vec::new();
        file.take(MAX_CA_BUNDLE_BYTES + 1).read_to_end(&mut pem)?;
        if pem.len() as u64 > MAX_CA_BUNDLE_BYTES {
            bail!("OIDC_CA_CERT_FILE exceeds 256 KiB");
        }
        let certificates = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|_| anyhow!("OIDC_CA_CERT_FILE contains an invalid PEM certificate"))?;
        if certificates.is_empty() {
            bail!("OIDC_CA_CERT_FILE must contain at least one PEM certificate");
        }
        for certificate in certificates {
            jwks = jwks.add_root_certificate(certificate.clone());
            user_info = user_info.add_root_certificate(certificate);
        }
    }
    let build = |builder: reqwest::ClientBuilder| {
        builder
            .build()
            .map_err(|_| anyhow!("Could not initialize OIDC client; check OIDC_CA_CERT_FILE"))
    };
    Ok(Clients {
        jwks: build(jwks)?,
        user_info: build(user_info)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public certificate only. No identity-provider key or credential is a fixture.
    const CA: &str = "-----BEGIN CERTIFICATE-----
MIIDLTCCAhWgAwIBAgIULtNwNRPZZvCiWtoJVJ8ukhIzeacwDQYJKoZIhvcNAQEL
BQAwJjEkMCIGA1UEAwwbRmxvdy1MaWtlIE9JREMgdW5pdCB0ZXN0IENBMB4XDTI2
MTAyOTE0MTEwMloXDTM2MTAyNjE0MTEwMlowJjEkMCIGA1UEAwwbRmxvdy1MaWtl
IE9JREMgdW5pdCB0ZXN0IENBMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKC
AQEAumIAkh/B+Bx3U2Ndf/NRtZRby+UY794Hyq4NSa57KmsBJYETbDHdDeO33YjZ
0DpUxXTxcNYEhQr0PyfmLA27fXcMNYBqGk4S+BlwyHO35k1E0RUkW8emiL4oJYrA
0rR+ggEJSwpFbfenkbD+glnHBA++YnIxplh8iMlfif68Q5WJotym1An/I7ZbCFqN
cqjAlvNqKz5mq4hwdJWUVYiuYcJOsTNyFjLj++nLNMdCAkw0xiZqJ6q3A+pBPWqe
+Dw9W4bO/OV1ctBV557mQy5pr9jVArM0qBfRPy4fxWo6cXCGBtrH1eJ4ZIsUlDsQ
bkZu6dic9GsBUq5dA+8NTwCbpwIDAQABo1MwUTAdBgNVHQ4EFgQURRQ83Wsh/m14
xkmzxtn0ebWdV74wHwYDVR0jBBgwFoAURRQ83Wsh/m14xkmzxtn0ebWdV74wDwYD
VR0TAQH/BAUwAwEB/zANBgkqhkiG9w0BAQsFAAOCAQEArA3N2pujtbmQiuYneN70
6cInMQNaZMhNjEzA0+KQPZcuJkCtrzD6ghCkcWmi1m7DXjC3pbtBLrRWbukiI2jy
E4Ij/Bd9EfNW3W4Jk1gteSN4AjbljimKi49eOcSR+oSDgC4zDzLNaKKFDwBk+Ica
aIAzMLTkrmBIfSWjA+BC9CQaCcK3Wtu3poUWXug/XvW7Wb00tveEFP8de8itFeqJ
WBoEmpSU3SO2s/nRiXEVgA2G40TPXt5M/W7o5hT/6nguFRXZNLLmE6VyDbN/23x5
lyMh2UNH26yfJpHIYnb2gCDizTncYDsVFB+m4Bq00/8BULyLALmEq2qXXH2gdyAZ
kA==
-----END CERTIFICATE-----
";

    #[test]
    fn accepts_public_roots_and_explicit_pem_bundles() {
        assert!(clients_from_ca_file(None).is_ok());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        // Rotation may temporarily require trusting both old and new certificates.
        std::fs::write(&path, CA.repeat(2)).unwrap();
        assert!(clients_from_ca_file(Some(&path)).is_ok());
    }

    #[test]
    fn rejects_missing_empty_malformed_and_oversized_ca_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        assert!(clients_from_ca_file(Some(&path)).is_err());
        assert!(clients_from_ca_file(Some(directory.path())).is_err());
        for contents in [
            "".to_string(),
            "not a certificate".to_string(),
            "-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----\n".to_string(),
            "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n".to_string(),
            "x".repeat(MAX_CA_BUNDLE_BYTES as usize + 1),
        ] {
            std::fs::write(&path, contents).unwrap();
            assert!(clients_from_ca_file(Some(&path)).is_err());
        }
    }

    #[tokio::test]
    async fn custom_trust_does_not_allow_plain_http() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        std::fs::write(&path, CA).unwrap();
        for path in [None, Some(path.as_path())] {
            let error = clients_from_ca_file(path)
                .unwrap()
                .jwks
                .get("http://127.0.0.1:9/jwks")
                .send()
                .await
                .unwrap_err();
            assert!(error.is_builder());
        }
    }
}
