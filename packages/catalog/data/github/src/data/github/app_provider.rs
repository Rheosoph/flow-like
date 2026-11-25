use super::provider::GitHubProvider;
#[cfg(feature = "execute")]
use super::provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};
#[cfg(any(feature = "execute", test))]
use serde::Serialize;

#[cfg(any(feature = "execute", test))]
const JWT_BACKDATE_SECONDS: u64 = 60;
#[cfg(any(feature = "execute", test))]
const JWT_LIFETIME_SECONDS: u64 = 540;

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct AppJwtClaims {
    pub iat: u64,
    pub exp: u64,
    pub iss: Value,
}

/// GitHub accepts the numeric App ID or the string Client ID as issuer; a numeric ID is sent as a number.
#[cfg(any(feature = "execute", test))]
pub(crate) fn app_jwt_claims(app_id: &str, now: u64) -> AppJwtClaims {
    let app_id = app_id.trim();
    let iss = app_id
        .parse::<u64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::from(app_id));
    AppJwtClaims {
        iat: now.saturating_sub(JWT_BACKDATE_SECONDS),
        exp: now + JWT_LIFETIME_SECONDS,
        iss,
    }
}

/// Keys copied from environment variables or secret stores often carry literal `\n` sequences.
#[cfg(any(feature = "execute", test))]
pub(crate) fn normalize_private_key(pem: &str) -> String {
    pem.trim().replace("\\n", "\n")
}

/// API paths that resolve the installation of an App for an owner, most specific first.
#[cfg(any(feature = "execute", test))]
pub(crate) fn installation_lookup_paths(owner: &str, repo: &str) -> Vec<String> {
    let owner = urlencoding::encode(owner.trim());
    let repo = repo.trim();
    if repo.is_empty() {
        vec![
            format!("/orgs/{owner}/installation"),
            format!("/users/{owner}/installation"),
        ]
    } else {
        vec![format!(
            "/repos/{owner}/{}/installation",
            urlencoding::encode(repo)
        )]
    }
}

#[cfg(feature = "execute")]
pub(crate) fn sign_app_jwt(
    app_id: &str,
    private_key_pem: &str,
    now: u64,
) -> flow_like_types::Result<String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    let key = EncodingKey::from_rsa_pem(normalize_private_key(private_key_pem).as_bytes())
        .map_err(|e| flow_like_types::anyhow!("Invalid GitHub App private key (PEM): {}", e))?;
    encode(
        &Header::new(Algorithm::RS256),
        &app_jwt_claims(app_id, now),
        &key,
    )
    .map_err(|e| flow_like_types::anyhow!("Failed to sign GitHub App JWT: {}", e))
}

#[crate::register_node]
#[derive(Default)]
pub struct GitHubAppKeyProviderNode {}

impl GitHubAppKeyProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GitHubAppKeyProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_provider_app_key",
            "GitHub (App Key)",
            "Authenticate as a GitHub App installation using the App ID and private key. Signs a short-lived JWT, resolves the installation (by ID, or by organization, user or repository) and exchanges it for an installation token that expires after one hour.",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "providerAppKey");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Request an installation token",
            VariableType::Execution,
        );

        node.add_input_pin(
            "app_id",
            "App ID",
            "GitHub App ID or Client ID",
            VariableType::String,
        );

        node.add_input_pin(
            "private_key",
            "Private Key",
            "Private key of the GitHub App in PEM format",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "installation_id",
            "Installation ID",
            "Installation ID. Leave at 0 to look the installation up by owner (and repository)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "owner",
            "Owner",
            "Organization or user the App is installed on. Used when no installation ID is set",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "repo",
            "Repository",
            "Optional repository name to look up a repository-level installation",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.github.com")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when the installation token was issued",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when signing, the installation lookup or the token request fails",
            VariableType::Execution,
        );

        node.add_output_pin(
            "provider",
            "Provider",
            "GitHub provider authenticated with the installation token",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "expires_at",
            "Expires At",
            "Expiry of the installation token (ISO 8601)",
            VariableType::String,
        );

        node.add_output_pin(
            "resolved_installation_id",
            "Installation",
            "Installation ID the token was issued for",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(9)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let app_id: String = context.evaluate_pin("app_id").await?;
        let private_key: String = context.evaluate_pin("private_key").await?;
        let installation_id: i64 = context.evaluate_pin("installation_id").await.unwrap_or(0);
        let owner: String = context.evaluate_pin("owner").await.unwrap_or_default();
        let repo: String = context.evaluate_pin("repo").await.unwrap_or_default();
        let base_url: String = context
            .evaluate_pin("base_url")
            .await
            .unwrap_or_else(|_| "https://api.github.com".to_string());

        if app_id.trim().is_empty() || private_key.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "GitHub App ID and private key are required"
            ));
        }
        if installation_id <= 0 && owner.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "Set an installation ID or the owner the GitHub App is installed on"
            ));
        }

        match issue_installation_token(
            context,
            &app_id,
            &private_key,
            installation_id,
            &owner,
            &repo,
            &base_url,
        )
        .await
        {
            Ok((token, expires_at, resolved_id)) => {
                let provider = GitHubProvider {
                    provider_id: GITHUB_PROVIDER_ID.to_string(),
                    access_token: token,
                    base_url,
                };
                context.set_pin_value("provider", json!(provider)).await?;
                context
                    .set_pin_value("expires_at", json!(expires_at))
                    .await?;
                context
                    .set_pin_value("resolved_installation_id", json!(resolved_id))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(
                    &format!("GitHub App authentication failed: {}", e),
                    LogLevel::Error,
                );
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub App authentication requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn issue_installation_token(
    context: &ExecutionContext,
    app_id: &str,
    private_key: &str,
    installation_id: i64,
    owner: &str,
    repo: &str,
    base_url: &str,
) -> flow_like_types::Result<(String, String, i64)> {
    use flow_like::flow::execution::egress::GuardedHttpClient;
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| flow_like_types::anyhow!("System clock is before the Unix epoch: {}", e))?
        .as_secs();
    let jwt = sign_app_jwt(app_id, private_key, now)?;
    let app = GitHubProvider {
        provider_id: GITHUB_PROVIDER_ID.to_string(),
        access_token: jwt,
        base_url: base_url.to_string(),
    };
    let client = GuardedHttpClient::new(context.execution_environment())?;

    let installation_id = if installation_id > 0 {
        installation_id
    } else {
        let mut last_error = String::new();
        let mut found = None;
        for path in installation_lookup_paths(owner, repo) {
            let url = app.api_url(&path);
            let response = client
                .get(&url)?
                .header("Authorization", app.auth_header())
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
                .header("User-Agent", "flow-like")
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                let body: Value = response.json().await?;
                found = body["id"].as_i64();
                break;
            }
            last_error = format!(
                "{} {}: {}",
                status,
                path,
                response.text().await.unwrap_or_default()
            );
        }
        found.ok_or_else(|| {
            flow_like_types::anyhow!(
                "No installation of the GitHub App found for '{}' ({})",
                owner,
                last_error
            )
        })?
    };

    let url = app.api_url(&format!(
        "/app/installations/{installation_id}/access_tokens"
    ));
    let response = client
        .post(&url)?
        .header("Authorization", app.auth_header())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", "flow-like")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(flow_like_types::anyhow!(
            "Installation token request for installation {} failed with {}: {}",
            installation_id,
            status,
            text
        ));
    }
    let body: Value = response.json().await?;
    let token = body["token"]
        .as_str()
        .ok_or_else(|| flow_like_types::anyhow!("Installation token response has no token"))?
        .to_string();
    let expires_at = body["expires_at"].as_str().unwrap_or_default().to_string();
    Ok((token, expires_at, installation_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_app_id_is_sent_as_number_and_client_id_as_string() {
        let claims = app_jwt_claims(" 123456 ", 1_000);
        assert_eq!(claims.iss, json!(123456));
        assert_eq!(claims.iat, 940);
        assert_eq!(claims.exp, 1_540);

        let claims = app_jwt_claims("Iv23liABC", 1_000);
        assert_eq!(claims.iss, json!("Iv23liABC"));
    }

    #[test]
    fn jwt_lifetime_stays_below_github_limit() {
        let claims = app_jwt_claims("1", 10_000);
        assert!(claims.exp - claims.iat <= 600);
        assert!(claims.iat < 10_000);
    }

    #[test]
    fn escaped_newlines_in_keys_are_restored() {
        let pem = "  -----BEGIN RSA PRIVATE KEY-----\\nabc\\n-----END RSA PRIVATE KEY-----\n";
        assert_eq!(
            normalize_private_key(pem),
            "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----"
        );
    }

    #[test]
    fn installation_lookup_prefers_repository_then_org_then_user() {
        assert_eq!(
            installation_lookup_paths("acme", "api"),
            vec!["/repos/acme/api/installation".to_string()]
        );
        assert_eq!(
            installation_lookup_paths("acme", ""),
            vec![
                "/orgs/acme/installation".to_string(),
                "/users/acme/installation".to_string()
            ]
        );
    }

    #[test]
    fn node_offers_lookup_by_owner_by_default() {
        let node = GitHubAppKeyProviderNode::new().get_node();
        let pin = node.get_pin_by_name("installation_id").unwrap();
        let value: Value =
            flow_like_types::json::from_slice(pin.default_value.as_deref().unwrap()).unwrap();
        assert_eq!(value, json!(0));
        assert!(node.get_pin_by_name("private_key").is_some());
    }

    #[cfg(feature = "execute")]
    mod signing {
        use super::super::*;
        use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
        use rsa::{
            RsaPrivateKey,
            pkcs1::EncodeRsaPrivateKey,
            pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
        };
        use std::sync::OnceLock;

        struct TestKeys {
            pkcs1: String,
            pkcs8: String,
            public: String,
        }

        /// Generated per test run because `*.pem` is gitignored and no private key belongs in the repo.
        fn keys() -> &'static TestKeys {
            static KEYS: OnceLock<TestKeys> = OnceLock::new();
            KEYS.get_or_init(|| {
                let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
                TestKeys {
                    pkcs1: key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string(),
                    pkcs8: key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
                    public: key
                        .to_public_key()
                        .to_public_key_pem(LineEnding::LF)
                        .unwrap(),
                }
            })
        }

        #[derive(serde::Deserialize)]
        struct Claims {
            iat: u64,
            exp: u64,
            iss: u64,
        }

        fn verify(token: &str, now: u64) -> Claims {
            let header = decode_header(token).unwrap();
            assert_eq!(header.alg, Algorithm::RS256);
            let mut validation = Validation::new(Algorithm::RS256);
            validation.validate_exp = false;
            validation.required_spec_claims.clear();
            let claims = decode::<Claims>(
                token,
                &DecodingKey::from_rsa_pem(keys().public.as_bytes()).unwrap(),
                &validation,
            )
            .unwrap()
            .claims;
            assert_eq!(claims.iat, now - JWT_BACKDATE_SECONDS);
            assert_eq!(claims.exp, now + JWT_LIFETIME_SECONDS);
            claims
        }

        #[test]
        fn signs_rs256_jwt_with_github_pkcs1_key() {
            let now = 1_750_000_000;
            assert!(keys().pkcs1.starts_with("-----BEGIN RSA PRIVATE KEY-----"));
            let token = sign_app_jwt("4242", &keys().pkcs1, now).unwrap();
            assert_eq!(verify(&token, now).iss, 4242);
        }

        #[test]
        fn signs_with_pkcs8_key_and_escaped_newlines() {
            let now = 1_750_000_000;
            let escaped = keys().pkcs8.replace('\n', "\\n");
            let token = sign_app_jwt("7", &escaped, now).unwrap();
            assert_eq!(verify(&token, now).iss, 7);
        }

        #[test]
        fn rejects_invalid_private_key() {
            let error = sign_app_jwt("1", "not a key", 1).unwrap_err();
            assert!(error.to_string().contains("Invalid GitHub App private key"));
        }
    }
}
