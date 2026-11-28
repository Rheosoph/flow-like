/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_credential_types::{
    provider::{self, error::CredentialsError, future, ProvideCredentials},
    Credentials,
};
use aws_smithy_types::error::display::DisplayErrorContext;
use std::borrow::Cow;
use std::fmt::Debug;
use tracing::Instrument;

use crate::meta::{ProviderAttempt, ProviderChainError};

/// Credentials provider that checks a series of inner providers
///
/// Each provider will be evaluated in order:
/// * If a provider returns valid [`Credentials`] they will be returned immediately.
///   No other credential providers will be used.
/// * Otherwise, if a provider returns [`CredentialsError::CredentialsNotLoaded`], the next provider will be checked.
/// * Finally, if a provider returns any other error condition, an error will be returned immediately.
///
/// # Examples
///
/// ```no_run
/// # fn example() {
/// use aws_config::meta::credentials::CredentialsProviderChain;
/// use aws_config::environment::credentials::EnvironmentVariableCredentialsProvider;
/// use aws_config::profile::ProfileFileCredentialsProvider;
///
/// let provider = CredentialsProviderChain::first_try("Environment", EnvironmentVariableCredentialsProvider::new())
///     .or_else("Profile", ProfileFileCredentialsProvider::builder().build());
/// # }
/// ```
pub struct CredentialsProviderChain {
    providers: Vec<(Cow<'static, str>, Box<dyn ProvideCredentials>)>,
}

impl Debug for CredentialsProviderChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialsProviderChain")
            .field(
                "providers",
                &self
                    .providers
                    .iter()
                    .map(|provider| &provider.0)
                    .collect::<Vec<&Cow<'static, str>>>(),
            )
            .finish()
    }
}

impl CredentialsProviderChain {
    /// Create a `CredentialsProviderChain` that begins by evaluating this provider
    pub fn first_try(
        name: impl Into<Cow<'static, str>>,
        provider: impl ProvideCredentials + 'static,
    ) -> Self {
        CredentialsProviderChain {
            providers: vec![(name.into(), Box::new(provider))],
        }
    }

    /// Add a fallback provider to the credentials provider chain
    pub fn or_else(
        mut self,
        name: impl Into<Cow<'static, str>>,
        provider: impl ProvideCredentials + 'static,
    ) -> Self {
        self.providers.push((name.into(), Box::new(provider)));
        self
    }

    /// Add a fallback to the default provider chain
    #[cfg(any(feature = "default-https-client", feature = "rustls"))]
    pub async fn or_default_provider(self) -> Self {
        self.or_else(
            "DefaultProviderChain",
            crate::default_provider::credentials::default_provider().await,
        )
    }

    /// Creates a credential provider chain that starts with the default provider
    #[cfg(any(feature = "default-https-client", feature = "rustls"))]
    pub async fn default_provider() -> Self {
        Self::first_try(
            "DefaultProviderChain",
            crate::default_provider::credentials::default_provider().await,
        )
    }

    async fn credentials(&self) -> provider::Result {
        let mut attempts = Vec::with_capacity(self.providers.len());
        for (name, provider) in &self.providers {
            let span = tracing::debug_span!("credentials_provider_chain", provider = %name);
            match provider.provide_credentials().instrument(span).await {
                Ok(credentials) => {
                    tracing::debug!(provider = %name, "loaded credentials");
                    return Ok(credentials);
                }
                Err(err @ CredentialsError::CredentialsNotLoaded(_)) => {
                    tracing::debug!(provider = %name, context = %DisplayErrorContext(&err), "provider in chain did not provide credentials");
                    attempts.push(ProviderAttempt::new(name.clone(), err));
                }
                Err(err) => {
                    tracing::warn!(provider = %name, error = %DisplayErrorContext(&err), "provider failed to provide credentials");
                    return Err(err);
                }
            }
        }
        Err(CredentialsError::not_loaded(ProviderChainError::new(
            attempts,
        )))
    }
}

impl ProvideCredentials for CredentialsProviderChain {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(self.credentials())
    }

    fn fallback_on_interrupt(&self) -> Option<Credentials> {
        for (_, provider) in &self.providers {
            if let creds @ Some(_) = provider.fallback_on_interrupt() {
                return creds;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aws_credential_types::{
        credential_fn::provide_credentials_fn,
        provider::{error::CredentialsError, future, ProvideCredentials},
        Credentials,
    };
    use aws_smithy_async::future::timeout::Timeout;

    use crate::meta::credentials::CredentialsProviderChain;

    #[derive(Debug)]
    struct FallbackCredentials(Credentials);

    impl ProvideCredentials for FallbackCredentials {
        fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
        where
            Self: 'a,
        {
            future::ProvideCredentials::new(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(self.0.clone())
            })
        }

        fn fallback_on_interrupt(&self) -> Option<Credentials> {
            Some(self.0.clone())
        }
    }

    #[tokio::test]
    async fn fallback_credentials_should_be_returned_from_provider2_on_timeout_while_provider2_was_providing_credentials(
    ) {
        let chain = CredentialsProviderChain::first_try(
            "provider1",
            provide_credentials_fn(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Err(CredentialsError::not_loaded(
                    "no providers in chain provided credentials",
                ))
            }),
        )
        .or_else("provider2", FallbackCredentials(Credentials::for_tests()));

        // Let the first call to `provide_credentials` succeed.
        let expected = chain.provide_credentials().await.unwrap();

        // Let the second call fail with an external timeout.
        let timeout = Timeout::new(
            chain.provide_credentials(),
            tokio::time::sleep(Duration::from_millis(300)),
        );
        match timeout.await {
            Ok(_) => panic!("provide_credentials completed before timeout future"),
            Err(_err) => match chain.fallback_on_interrupt() {
                Some(actual) => assert_eq!(actual, expected),
                None => panic!(
                    "provide_credentials timed out and no credentials returned from fallback_on_interrupt"
                ),
            },
        };
    }

    #[tokio::test]
    async fn fallback_credentials_should_be_returned_from_provider2_on_timeout_while_provider1_was_providing_credentials(
    ) {
        let chain = CredentialsProviderChain::first_try(
            "provider1",
            provide_credentials_fn(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Err(CredentialsError::not_loaded(
                    "no providers in chain provided credentials",
                ))
            }),
        )
        .or_else("provider2", FallbackCredentials(Credentials::for_tests()));

        // Let the first call to `provide_credentials` succeed.
        let expected = chain.provide_credentials().await.unwrap();

        // Let the second call fail with an external timeout.
        let timeout = Timeout::new(
            chain.provide_credentials(),
            tokio::time::sleep(Duration::from_millis(100)),
        );
        match timeout.await {
            Ok(_) => panic!("provide_credentials completed before timeout future"),
            Err(_err) => match chain.fallback_on_interrupt() {
                Some(actual) => assert_eq!(actual, expected),
                None => panic!(
                    "provide_credentials timed out and no credentials returned from fallback_on_interrupt"
                ),
            },
        };
    }

    #[tokio::test]
    async fn error_message_includes_per_provider_summary() {
        let chain = CredentialsProviderChain::first_try(
            "Environment",
            provide_credentials_fn(|| async { Err(CredentialsError::not_loaded("not set")) }),
        )
        .or_else(
            "Profile",
            provide_credentials_fn(|| async {
                Err(CredentialsError::not_loaded("profile 'deploy' not found"))
            }),
        )
        .or_else(
            "IMDS",
            provide_credentials_fn(|| async {
                Err(CredentialsError::not_loaded(
                    "could not communicate with IMDS",
                ))
            }),
        );

        let err = chain.provide_credentials().await.expect_err("should fail");
        assert!(matches!(err, CredentialsError::CredentialsNotLoaded(_)));
        let source = std::error::Error::source(&err)
            .expect("error should have a source")
            .to_string();
        assert!(
            source.contains("no credentials found in chain. Attempted:"),
            "missing header in: {source}"
        );
        assert!(
            source.contains("Environment:"),
            "missing Environment in: {source}"
        );
        assert!(source.contains("Profile:"), "missing Profile in: {source}");
        assert!(source.contains("IMDS:"), "missing IMDS in: {source}");
    }

    #[tokio::test]
    async fn hard_fail_stops_chain() {
        let chain = CredentialsProviderChain::first_try(
            "Failing",
            provide_credentials_fn(|| async {
                Err(CredentialsError::provider_error("503 Service Unavailable"))
            }),
        )
        .or_else(
            "Working",
            provide_credentials_fn(|| async { Ok(Credentials::for_tests()) }),
        );

        let err = chain
            .provide_credentials()
            .await
            .expect_err("should hard-fail, not fall through");
        assert!(
            matches!(err, CredentialsError::ProviderError(_)),
            "expected ProviderError, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn empty_chain_error_message() {
        let chain = CredentialsProviderChain { providers: vec![] };
        let err = chain.provide_credentials().await.expect_err("should fail");
        assert!(matches!(err, CredentialsError::CredentialsNotLoaded(_)));
        let source = std::error::Error::source(&err)
            .expect("error should have a source")
            .to_string();
        assert!(
            source.contains("no providers were configured in the chain"),
            "unexpected message: {source}"
        );
    }

    #[tokio::test]
    async fn programmatic_access_via_downcast() {
        use crate::meta::ProviderChainError;

        let chain = CredentialsProviderChain::first_try(
            "Environment",
            provide_credentials_fn(|| async { Err(CredentialsError::not_loaded("not set")) }),
        )
        .or_else(
            "Profile",
            provide_credentials_fn(|| async {
                Err(CredentialsError::not_loaded("no profile defined"))
            }),
        );

        let err = chain.provide_credentials().await.expect_err("should fail");
        let source = std::error::Error::source(&err).expect("should have source");
        let chain_err = source
            .downcast_ref::<ProviderChainError<CredentialsError>>()
            .expect("should downcast to ProviderChainError");
        assert_eq!(chain_err.attempts().len(), 2);
        assert_eq!(chain_err.attempts()[0].name(), "Environment");
        assert_eq!(chain_err.attempts()[1].name(), "Profile");
    }
}
