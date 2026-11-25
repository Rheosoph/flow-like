/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Meta-providers that augment existing providers with new behavior

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

pub mod credentials;
pub mod region;
pub mod token;

/// A single provider's outcome in a chain, capturing the provider name and its error.
#[derive(Debug)]
pub struct ProviderAttempt<E> {
    name: Cow<'static, str>,
    error: E,
}

impl<E> ProviderAttempt<E> {
    pub(crate) fn new(name: Cow<'static, str>, error: E) -> Self {
        Self { name, error }
    }

    /// The name of the provider that was attempted.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The error returned by this provider.
    pub fn error(&self) -> &E {
        &self.error
    }
}

/// Error returned when all providers in a chain are exhausted without producing credentials or tokens.
///
/// Contains per-provider errors for programmatic inspection via [`ProviderChainError::attempts`].
///
/// To access from a `CredentialsError` or `TokenError`, downcast the error source:
/// ```ignore
/// use aws_config::meta::ProviderChainError;
/// use aws_credential_types::provider::error::CredentialsError;
/// use std::error::Error;
///
/// if let Some(chain_err) = err.source()
///     .and_then(|s| s.downcast_ref::<ProviderChainError<CredentialsError>>())
/// {
///     for attempt in chain_err.attempts() {
///         println!("{}: {}", attempt.name(), attempt.error());
///     }
/// }
/// ```
#[derive(Debug)]
pub struct ProviderChainError<E: Error> {
    attempts: Vec<ProviderAttempt<E>>,
}

impl<E: Error> ProviderChainError<E> {
    pub(crate) fn new(attempts: Vec<ProviderAttempt<E>>) -> Self {
        Self { attempts }
    }

    /// Returns the per-provider errors in chain order.
    pub fn attempts(&self) -> &[ProviderAttempt<E>] {
        &self.attempts
    }
}

impl<E: Error> fmt::Display for ProviderChainError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.attempts.is_empty() {
            return write!(f, "no providers were configured in the chain");
        }
        write!(f, "no credentials found in chain. Attempted:")?;
        for attempt in &self.attempts {
            write!(f, "\n  {}: {}", attempt.name, attempt.error)?;
            let mut source = attempt.error.source();
            while let Some(cause) = source {
                write!(f, ": {cause}")?;
                source = cause.source();
            }
        }
        Ok(())
    }
}

impl<E: Error + 'static> Error for ProviderChainError<E> {}
