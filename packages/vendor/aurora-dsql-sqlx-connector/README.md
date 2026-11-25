# Aurora DSQL SQLx Connector for Rust

## Overview

A Rust connector for Amazon Aurora DSQL that wraps [SQLx](https://github.com/launchbadge/sqlx) with automatic IAM authentication. The connector handles token generation, SSL configuration, and connection management so you can focus on your application logic.

## Features

- Automatic IAM token generation
- Connection pooling with background token refresh (opt-in `pool` feature)
- Single connection support for simpler use cases
- Connection string parsing support
- OCC retry helpers with exponential backoff and jitter

## Prerequisites

- Rust 1.94 or later
- AWS credentials configured (see [Credentials Resolution](#credentials-resolution) below)
- An Aurora DSQL cluster

For information about creating an Aurora DSQL cluster, see the [Getting started with Aurora DSQL](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/getting-started.html) guide.

### Credentials Resolution

By default, the connector uses the [AWS SDK for Rust default credential chain](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credproviders.html), which resolves credentials in the following order:

1. **Environment variables** (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optionally `AWS_SESSION_TOKEN`)
2. **Shared credentials file** (`~/.aws/credentials`) with optional profile via `AWS_PROFILE` or `profile` config option
3. **Shared config file** (`~/.aws/config`)
4. **IAM role for Amazon EC2/ECS/Lambda** (instance metadata or task role)

The first source that provides valid credentials is used. You can override this by:
- Specifying `profile` in the connection string for a specific AWS profile
- Passing a custom `SharedCredentialsProvider` via the builder for programmatic credential control (e.g., assumed IAM roles in Lambda)

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
aurora-dsql-sqlx-connector = "0.2.1"
```

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `occ` | No | OCC retry helpers (`retry_on_occ`, `is_occ_error`, `OCCType`, `OCCRetryExt` trait for `PgConnection`) |
| `pool` | No | sqlx pool helper with background token refresh |

**Note:** To use `OCCRetryExt` on `PgPool`, enable both `occ` and `pool` features.

For most applications, enable both features:

```toml
[dependencies]
aurora-dsql-sqlx-connector = { version = "0.2.1", features = ["pool", "occ"] }
```

## Configuration Options

These options are parsed from the connection string or set via the builder:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `host` | `string` | (required) | Cluster endpoint or cluster ID |
| `region` | `Option` | (auto-detected) | AWS region; required if host is a cluster ID |
| `user` | `string` | `"admin"` | Database user |
| `database` | `string` | `"postgres"` | Database name |
| `port` | `u16` | `5432` | Database port |
| `profile` | `Option<String>` | `None` | AWS profile name for credentials |
| `credentials_provider` | `Option<SharedCredentialsProvider>` | `None` | Custom credentials provider (builder only) |
| `tokenDurationSecs` | `u64` | `900` (15 minutes) | Token validity duration in seconds |
| `ormPrefix` | `Option<String>` | `None` | ORM prefix for application_name (e.g. `"diesel"` → `"diesel:aurora-dsql-rust-sqlx/{version}"`) |

## Quick Start

Enable the `pool` feature, then:

```rust
use sqlx::Row;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = aurora_dsql_sqlx_connector::pool::connect(
        "postgres://admin@foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws/postgres"
    ).await?;

    let row = sqlx::query("SELECT 'Hello, DSQL!' as greeting")
        .fetch_one(&pool)
        .await?;

    let greeting: &str = row.get("greeting");
    println!("{}", greeting);

    pool.close().await;
    Ok(())
}
```

## Connection String Format

The connector supports PostgreSQL connection string format:

```
postgres://[user@]host[:port]/[database][?param=value&...]
```

Both `postgres://` and `postgresql://` schemes are supported.

**Supported query parameters:**
- `region` — AWS region
- `profile` — AWS profile name
- `tokenDurationSecs` — Token validity duration in seconds
- `ormPrefix` — ORM prefix for application_name

**Region Resolution Priority:**
1. Parse from hostname (e.g., `cluster.dsql.us-east-1.on.aws`)
2. Explicit `?region=...` in connection string
3. AWS SDK default region (`AWS_REGION` env var or `~/.aws/config`)

**Examples:**

```bash
# Full endpoint (region auto-detected from hostname)
postgres://admin@cluster.dsql.us-east-1.on.aws/postgres

# With explicit region
postgres://admin@cluster.dsql.us-east-1.on.aws/postgres?region=us-east-1

# With AWS profile
postgres://admin@cluster.dsql.us-east-1.on.aws/postgres?profile=dev

# Cluster ID (region required)
postgres://admin@foo0bar1baz2quux3quuux4/postgres?region=us-east-1
```

## Advanced Usage

### Host Configuration

The connector supports two host formats:

**Full endpoint** (region auto-detected):
```rust
let opts = DsqlConnectOptions::from_connection_string(
    "postgres://admin@foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws/postgres"
)?;
```

**Cluster ID** (region required):
```rust
let opts = DsqlConnectOptions::from_connection_string(
    "postgres://admin@foo0bar1baz2quux3quuux4/postgres?region=us-east-1"
)?;
```

### Single Connection Usage

For simple scripts or when connection pooling is not needed:

```rust
use sqlx::Row;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut conn = aurora_dsql_sqlx_connector::connection::connect(
        "postgres://admin@foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws/postgres"
    ).await?;

    let row = sqlx::query("SELECT 1 as value")
        .fetch_one(&mut conn)
        .await?;
    let value: i32 = row.get("value");
    println!("Result: {}", value);

    Ok(())
}
```

Each call to `connection::connect()` generates a fresh IAM token. For operations longer than the token duration, create a new connection.

### Pool Configuration

The `pool` feature provides `pool::connect()` helpers that return a standard `sqlx::PgPool` with a background token refresh task that rotates the IAM auth token at 80% of the token duration. This feature requires a tokio runtime. Call `pool.close().await` to stop the background refresh task and release pool resources.

For custom pool settings, pass `PgPoolOptions` to `connect_with()` to get both pool tuning and the background token refresh task:

```rust
use aurora_dsql_sqlx_connector::DsqlConnectOptions;
use sqlx::postgres::PgPoolOptions;

let config = DsqlConnectOptions::from_connection_string(
    "postgres://admin@foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws/postgres"
)?;

let pool = aurora_dsql_sqlx_connector::pool::connect_with(
    &config,
    PgPoolOptions::new().max_connections(20),
).await?;
```

Or use `connect()` for defaults:

```rust
let pool = aurora_dsql_sqlx_connector::pool::connect(
    "postgres://admin@foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws/postgres"
).await?;
```

### Programmatic Configuration

Use `DsqlConnectOptionsBuilder` for programmatic configuration:

```rust
use aurora_dsql_sqlx_connector::{DsqlConnectOptionsBuilder, Region};
use sqlx::postgres::PgConnectOptions;

let pg = PgConnectOptions::new()
    .host("foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws")
    .username("admin")
    .database("postgres");

let opts = DsqlConnectOptionsBuilder::default()
    .pg_connect_options(pg)
    .region(Some(Region::new("us-east-1")))
    .build()?;

let mut conn = aurora_dsql_sqlx_connector::connection::connect_with(&opts).await?;
```

### Custom Credentials Provider

For environments where the default credential chain doesn't apply (e.g., Lambda with assumed roles), pass a custom `SharedCredentialsProvider`:

```rust
use aurora_dsql_sqlx_connector::{DsqlConnectOptionsBuilder, SharedCredentialsProvider};
use aws_credential_types::Credentials;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

let creds = Credentials::new("AKID", "SECRET", Some("SESSION".into()), None, "assumed-role");

let config = DsqlConnectOptionsBuilder::default()
    .pg_connect_options(
        PgConnectOptions::new()
            .host("foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws")
            .username("admin")
            .database("postgres"),
    )
    .credentials_provider(SharedCredentialsProvider::new(creds))
    .build()?;

let pool = aurora_dsql_sqlx_connector::pool::connect_with(
    &config,
    PgPoolOptions::new(),
).await?;
```

## Token Generation

The connector automatically generates IAM authentication tokens:

- **Connection pools**: A background task refreshes the token at 80% of the token duration via `pool.set_connect_options()`. Call `pool.close().await` to stop the refresh task.
- **Single connections**: A fresh token is generated at connection time.
- **Token generation** is a local SigV4 presigning operation with negligible cost.

For the `admin` user, the connector generates admin tokens using `db_connect_admin_auth_token`. For other users, it generates standard tokens using `db_connect_auth_token`.

Token duration defaults to 900 seconds. This can be customized via `tokenDurationSecs` in the connection string.

## OCC Retry

Aurora DSQL uses optimistic concurrency control (OCC). Transactions may fail with OCC errors when concurrent modifications conflict. The connector provides helpers to automatically detect and retry these operations (enable the `occ` feature).

### Using the Extension Trait (Recommended)

Import `OCCRetryExt` to add retry methods to `PgPool` or `PgConnection`:

```rust
use aurora_dsql_sqlx_connector::OCCRetryExt;

// With connection pool
let mut pool = aurora_dsql_sqlx_connector::pool::connect(
    "postgres://admin@cluster.dsql.us-east-1.on.aws/postgres"
).await?;

pool.transaction_with_retry(None, |tx| Box::pin(async move {
    sqlx::query("UPDATE accounts SET balance = balance - 100 WHERE id = $1")
        .bind(account_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
})).await?;

// With single connection
let mut conn = aurora_dsql_sqlx_connector::connection::connect(
    "postgres://admin@cluster.dsql.us-east-1.on.aws/postgres"
).await?;

conn.transaction_with_retry(None, |tx| Box::pin(async move {
    sqlx::query("INSERT INTO users VALUES ($1, $2)")
        .bind(1)
        .bind("alice")
        .execute(&mut **tx)
        .await?;
    Ok(())
})).await?;

// With custom config
use aurora_dsql_sqlx_connector::OCCRetryConfigBuilder;

let config = OCCRetryConfigBuilder::default()
    .max_attempts(5u32)
    .base_delay_ms(10u64)
    .max_delay_ms(50u64)
    .build()?;

pool.transaction_with_retry(Some(&config), |tx| Box::pin(async move {
    sqlx::query("INSERT INTO products VALUES ($1, $2)")
        .bind(1)
        .bind("Widget")
        .execute(&mut **tx)
        .await?;
    Ok(())
})).await?;
```

**Simplifying with `txn!` macro:** Hide `Box::pin` boilerplate:

```rust
use aurora_dsql_sqlx_connector::{txn, OCCRetryExt};

pool.transaction_with_retry(None, |tx| txn!({
    sqlx::query("INSERT INTO users VALUES ($1, $2)")
        .bind(1)
        .bind("alice")
        .execute(&mut **tx)
        .await?;
    Ok(())
})).await?;
```

**Opting Out:** For operations that don't need retry, use sqlx directly:

```rust
// Direct pool usage - no OCC retry
let mut tx = pool.begin().await?;
sqlx::query("SELECT * FROM users").execute(&mut *tx).await?;
tx.commit().await?;
```

### Manual Retry (Advanced)

For custom retry logic or operations that need explicit transaction control:

```rust
use aurora_dsql_sqlx_connector::retry_on_occ;

let config = OCCRetryConfig::default();
retry_on_occ(&config, || async {
    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;
    // Custom logic with full control
    tx.commit().await?;
    Ok(())
}).await?;
```

### Configuration

Customize retry behavior with `OCCRetryConfigBuilder`:

```rust
use aurora_dsql_sqlx_connector::OCCRetryConfigBuilder;

let config = OCCRetryConfigBuilder::default()
    .max_attempts(5u32)          // Default: 3
    .base_delay_ms(10u64)        // Default: 1ms
    .max_delay_ms(50u64)         // Default: 100ms
    .jitter_factor(0.25)         // Default: 0.25 (25%)
    .build()?;
```

**Backoff Strategy:**
- Exponential: `delay = base_delay * 2^(attempt-1)`
- Additive jitter: `jitter = delay * random(0..1) * jitter_factor`
- Capped at `max_delay_ms`

### Retry Logging

The connector uses the `log` crate for retry logging. If your application uses any log implementation (e.g., `env_logger`, `tracing-subscriber`), the connector's logs will be captured automatically.

### OCC Error Types

The connector classifies OCC errors by type for better observability:

| SQLSTATE | OCCType | Description |
|----------|---------|-------------|
| `OC000` | `Data` | Data conflict - concurrent modification of same rows |
| `OC001` | `Schema` | Schema conflict - DDL changes during transaction |
| `40001` | `Unknown` | Generic serialization failure (parsed for embedded OC000/OC001) |

Use `is_occ_error()` to detect and classify errors, or check logs for conflict types. Error codes are included in all retry log messages for better observability.

## Examples

The `example/` directory contains runnable examples with a standalone Cargo project:

| Example | Description |
|---------|-------------|
| [example_preferred](example/src/example_preferred.rs) | Recommended: Pool with concurrent queries and transactional writes |
| [example_no_connection_pool](example/src/alternatives/no_connection_pool/example_no_connection_pool.rs) | Single connection without pooling |

### Running Examples

```bash
export CLUSTER_ENDPOINT=foo0bar1baz2quux3quuux4.dsql.us-east-1.on.aws
cd example

# Run the preferred example (pool-based)
cargo run --bin example_preferred

# Run the no-pool example
cargo run --bin example_no_connection_pool
```

## Additional Resources

- [Amazon Aurora DSQL Documentation](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/what-is-aurora-dsql.html)
- [Aurora DSQL Best Practices](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/best-practices.html)
- [SQLx Documentation](https://docs.rs/sqlx/latest/sqlx/)
- [AWS SDK for Rust](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/welcome.html)

---

Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.

SPDX-License-Identifier: Apache-2.0
