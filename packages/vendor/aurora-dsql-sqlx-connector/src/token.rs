// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{DsqlError, Result};
use aws_sdk_dsql::auth_token::{AuthTokenGenerator, Config};

/// Build a reusable `AuthTokenGenerator` for the given DSQL endpoint.
///
/// The returned signer can be passed to [`generate_token`] multiple times
/// without rebuilding it from scratch.
pub(crate) fn build_signer(
    host: &str,
    region: &aws_config::Region,
    sdk_config: &aws_config::SdkConfig,
    token_duration_secs: Option<u64>,
) -> Result<AuthTokenGenerator> {
    let credentials_provider = sdk_config
        .credentials_provider()
        .ok_or_else(|| DsqlError::TokenError("No credentials provider found".into()))?;

    let mut builder = Config::builder()
        .hostname(host)
        .region(region.clone())
        .credentials(credentials_provider);

    if let Some(duration) = token_duration_secs {
        builder = builder.expires_in(duration);
    }

    Ok(AuthTokenGenerator::new(
        builder.build().map_err(DsqlError::TokenError)?,
    ))
}

/// Generate an IAM auth token using a pre-built signer.
pub(crate) async fn generate_token(
    signer: &AuthTokenGenerator,
    user: &str,
    sdk_config: &aws_config::SdkConfig,
) -> Result<String> {
    let token = if user == "admin" {
        signer.db_connect_admin_auth_token(sdk_config).await
    } else {
        signer.db_connect_auth_token(sdk_config).await
    }
    .map_err(DsqlError::TokenError)?;

    Ok(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_credential_types::provider::SharedCredentialsProvider;
    use aws_credential_types::Credentials;

    const TEST_HOST: &str = "example.dsql.us-east-1.on.aws";

    async fn fake_signer(duration: Option<u64>) -> (AuthTokenGenerator, aws_config::SdkConfig) {
        let creds = Credentials::new("fake_key", "fake_secret", None, None, "test");
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .credentials_provider(SharedCredentialsProvider::new(creds))
            .load()
            .await;
        let region = aws_config::Region::new("us-east-1");
        let signer = build_signer(TEST_HOST, &region, &sdk_config, duration)
            .expect("signer creation should succeed");
        (signer, sdk_config)
    }

    #[tokio::test]
    async fn test_generate_token_admin_user() {
        let (signer, sdk_config) = fake_signer(None).await;

        let token = generate_token(&signer, "admin", &sdk_config)
            .await
            .expect("token generation should succeed with fake credentials");

        assert!(!token.is_empty(), "Token should not be empty");
    }

    #[tokio::test]
    async fn test_generate_token_non_admin_user() {
        let (signer, sdk_config) = fake_signer(None).await;

        let token = generate_token(&signer, "regular_user", &sdk_config)
            .await
            .expect("token generation should succeed with fake credentials");

        assert!(!token.is_empty(), "Token should not be empty");
    }

    #[tokio::test]
    async fn test_generate_token_with_custom_duration() {
        let (signer, sdk_config) = fake_signer(Some(600)).await;

        let token = generate_token(&signer, "admin", &sdk_config)
            .await
            .expect("token generation should succeed with custom duration");

        assert!(!token.is_empty(), "Token should not be empty");
    }

    #[test]
    fn test_build_signer_requires_credentials_provider() {
        // Build an SdkConfig without credentials
        let sdk_config = aws_config::SdkConfig::builder()
            .region(aws_config::Region::new("us-east-1"))
            .build();
        let region = aws_config::Region::new("us-east-1");

        let result = build_signer("example.dsql.us-east-1.on.aws", &region, &sdk_config, None);

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("credentials"),
            "Expected credentials error, got: {}",
            msg
        );
    }
}
