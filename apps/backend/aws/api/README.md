# AWS API Lambda

The Flow-Like API packaged for AWS Lambda (`lambda_http`, streaming responses).
Build with `cargo lambda build --release -p flow-like-aws-api`.

## Environment

Database, one of two modes:

- **Aurora DSQL** (selected by `DSQL_CLUSTER_ENDPOINT`; contract in
  `packages/aws-data/src/dsql.rs`):
  - `DSQL_CLUSTER_ENDPOINT`: `<id>.dsql.<region>.on.aws`
  - `DSQL_REGION`: optional, must match the endpoint
  - `DSQL_USER`: database role, default `admin`; production uses
    `flow_like_api`, which the migration job creates and binds to this
    function's IAM role
  - `DSQL_TOKEN_DURATION_SECS`: token lifetime in seconds, default `3600` (1800–604800)
  - `DSQL_MAX_CONNECTIONS`: default `4`
  - Forbidden alongside it, even empty: `DATABASE_URL`, `PGPASSWORD`,
    `PGPASSFILE`, `PGSERVICE`, `PGSERVICEFILE`, `PGHOST`, `PGHOSTADDR`,
    `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`, `PGSSLROOTCERT`,
    `PGSSLCERT`, `PGSSLKEY`, `PGOPTIONS`.
  - IAM: `dsql:DbConnect` on the cluster ARN (never `dsql:DbConnectAdmin`).
  - The schema must already be applied by `apps/backend/aws/migration`.
- **PostgreSQL / CockroachDB**: `DATABASE_URL` from SSM Parameter Store under
  `SECRET_PREFIX` (or the environment).

Other settings: `SECRET_PREFIX` (SSM prefix for secrets), `CDN_BUCKET_NAME`,
`CDN_BUCKET_ENDPOINT`, `CDN_BUCKET_ACCESS_KEY_ID`, `SENTRY_ENDPOINT`.

See `apps/docs/src/content/docs/self-hosting/aws/database.md` for the full
DSQL setup, IAM policies and the migration workflow.
