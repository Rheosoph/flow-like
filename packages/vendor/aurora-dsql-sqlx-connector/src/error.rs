// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[cfg(feature = "occ")]
use crate::occ_retry::OCCType;

pub type Result<T> = std::result::Result<T, DsqlError>;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum DsqlError {
    #[error("configuration error: {0}")]
    ConfigError(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("token error: {0}")]
    TokenError(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("connection error: {0}")]
    ConnectionError(#[source] sqlx::Error),

    #[error("database error: {0}")]
    DatabaseError(#[source] sqlx::Error),

    #[cfg(feature = "occ")]
    #[error("OCC retry exhausted after {attempts} attempts (type: {occ_type:?}): {source}")]
    OCCRetryExhausted {
        attempts: u32,
        occ_type: OCCType,
        #[source]
        source: Box<DsqlError>,
    },
}

#[cfg(feature = "occ")]
impl From<crate::occ_retry::OCCRetryConfigBuilderError> for DsqlError {
    fn from(err: crate::occ_retry::OCCRetryConfigBuilderError) -> Self {
        DsqlError::ConfigError(Box::new(err))
    }
}
