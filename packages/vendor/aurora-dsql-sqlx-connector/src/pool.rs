// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use aws_sdk_dsql::auth_token::AuthTokenGenerator;

use crate::config::DsqlConnectOptions;
use crate::{DsqlError, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Parse a connection string, create a PgPool, verify connectivity,
/// and spawn a background token refresh task.
pub async fn connect(url: &str) -> Result<PgPool> {
    let config = DsqlConnectOptions::from_connection_string(url)?;
    connect_with(&config, PgPoolOptions::new()).await
}

/// Create a PgPool from pre-built options, verify connectivity,
/// and spawn a background token refresh task.
pub async fn connect_with(
    config: &DsqlConnectOptions,
    pool_options: PgPoolOptions,
) -> Result<PgPool> {
    let sdk_config =
        crate::config::load_aws_config(config.profile(), config.credentials_provider()).await;
    let host = config.resolve_host(&sdk_config)?;
    let region = config.resolve_region(&sdk_config)?;
    let signer =
        crate::token::build_signer(&host, &region, &sdk_config, Some(config.token_duration()))?;

    let user = config.pg_connect_options().get_username();
    let token = crate::token::generate_token(&signer, user, &sdk_config).await?;
    let opts = config.build_connect_options(&sdk_config, &token)?;

    let pool = pool_options
        .connect_with(opts)
        .await
        .map_err(DsqlError::ConnectionError)?;

    spawn_refresh_task(pool.clone(), config.clone(), signer, sdk_config);
    Ok(pool)
}

fn spawn_refresh_task(
    pool: PgPool,
    config: DsqlConnectOptions,
    signer: AuthTokenGenerator,
    sdk_config: aws_config::SdkConfig,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.refresh_interval());
        interval.tick().await; // skip the immediate first tick
        loop {
            tokio::select! {
                _ = pool.close_event() => break,
                _ = interval.tick() => {
                    if let Err(e) = refresh_token(&config, &signer, &sdk_config, &pool).await {
                        log::error!("token refresh failed: {:?}", e);
                    }
                }
            }
        }
    });
}

async fn refresh_token(
    config: &DsqlConnectOptions,
    signer: &AuthTokenGenerator,
    sdk_config: &aws_config::SdkConfig,
    pool: &PgPool,
) -> Result<()> {
    let user = config.pg_connect_options().get_username();
    let token = crate::token::generate_token(signer, user, sdk_config).await?;
    pool.set_connect_options(config.build_connect_options(sdk_config, &token)?);
    Ok(())
}
