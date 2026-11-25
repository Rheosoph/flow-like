// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::config::DsqlConnectOptions;
use crate::{DsqlError, Result};
use sqlx::Connection;

/// Parse a connection string and connect to DSQL with a fresh IAM token.
pub async fn connect(url: &str) -> Result<sqlx::PgConnection> {
    let config = DsqlConnectOptions::from_connection_string(url)?;
    connect_with(&config).await
}

/// Connect to DSQL using pre-built options with a fresh IAM token.
pub async fn connect_with(config: &DsqlConnectOptions) -> Result<sqlx::PgConnection> {
    let opts = config.authenticated_pg_options().await?;
    sqlx::PgConnection::connect_with(&opts)
        .await
        .map_err(DsqlError::ConnectionError)
}
