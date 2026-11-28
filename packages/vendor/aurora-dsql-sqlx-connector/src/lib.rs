// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(docsrs, feature(doc_cfg))]

//! Aurora DSQL connector for SQLx.

mod config;
pub mod connection;
mod error;
#[cfg(feature = "occ")]
mod occ_retry;
#[cfg(feature = "pool")]
pub mod pool;
mod token;
pub(crate) mod util;

pub use aws_config::Region;
pub use aws_credential_types::provider::SharedCredentialsProvider;
pub use config::{DsqlConnectOptions, DsqlConnectOptionsBuilder};
pub use error::{DsqlError, Result};
#[cfg(feature = "occ")]
pub use occ_retry::{
    is_occ_error, retry_on_occ, OCCRetryConfig, OCCRetryConfigBuilder, OCCRetryExt, OCCType,
};
