# Introduction

The file tracker consumes S3 notifications from SQS and maintains app and user storage totals.
Each object size and its aggregate deltas commit in one SQL transaction. A failed transaction
can be replayed, and duplicate or older S3 sequencers leave the totals unchanged. Deleted objects
retain a zero-size accounting row so a delayed notification cannot count them again.

The tracker reads the current S3 object with `HeadObject` after acquiring the SQL object write
intent. This read has an eight-second timeout and runs again after a transaction conflict, so
concurrent messages cannot commit storage observations in the wrong order. Totals cover
current object versions, excluding noncurrent S3 versions, incomplete multipart uploads, and S3
storage overhead. The Lambda role needs `s3:GetObject` on tracked objects and `s3:ListBucket` on
their buckets, which lets a missing object produce a 404 response. An access-denied response is
retried instead of being treated as a deletion.

## Environment

Database, one of two modes:

- **Aurora DSQL** (selected by `DSQL_CLUSTER_ENDPOINT`; same contract as the
  API, `packages/aws-data/src/dsql.rs`): `DSQL_CLUSTER_ENDPOINT`
  (public `<id>.dsql.<region>.on.aws` or PrivateLink
  `<id>.dsql-<service-id>.<region>.on.aws`), optional `DSQL_REGION`, `DSQL_USER` (default
  `admin`; production uses `flow_like_api`), `DSQL_TOKEN_DURATION_SECS`
  (default `3600`, range 1800–604800), `DSQL_MAX_CONNECTIONS` (default `4`). Forbidden alongside
  it, even empty: `DATABASE_URL` and every libpq `PG*` source (`PGPASSWORD`,
  `PGPASSFILE`, `PGSERVICE`, `PGSERVICEFILE`, `PGHOST`, `PGHOSTADDR`,
  `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT`,
  `PGSSLKEY`, `PGOPTIONS`). IAM: `dsql:DbConnect` on the cluster ARN. The
  schema is applied beforehand by `apps/backend/aws/migration`.
- **PostgreSQL / CockroachDB**: `DATABASE_URL` from the environment or from
  SSM Parameter Store under `SECRET_PREFIX`.

See `apps/docs/src/content/docs/self-hosting/aws/database.md`.

## Upgrading an existing tracker

The previous tracker updated DynamoDB object sizes separately from SQL totals. Stop and drain
that version before enabling this one. The old version must never run alongside the new version
or resume after cutover, because its updates would bypass the SQL accounting ledger.

1. Disable the SQS event-source mapping, wait for active old invocations to finish, and retain
   queued messages for the new worker. Pause object writes while reconciling the baseline.
2. Reconcile existing `App.totalSize` and `User.totalSize` against the legacy object inventory.
   Failures in the previous worker may already have caused drift; this migration cannot infer
   lost deltas from incomplete DynamoDB records. Use an S3 inventory when the legacy inventory
   itself is incomplete, and preserve whether a key belongs to an app or a user-owned app.
3. Apply the database migration that creates `FileAccountingObject`. Set `FILES_TABLE_NAME` to
   the legacy DynamoDB table and `FILES_LEGACY_BUCKET_NAME` to the bucket it represented. The old
   keys did not include a bucket, so this explicit bucket is required to avoid importing one
   contribution into several buckets. If the old table mixed buckets with overlapping keys,
   reconcile them before cutover; its records cannot identify their original bucket.
4. Grant the new worker `dynamodb:GetItem` on the legacy table, alongside the S3 permissions
   above. Enable the new worker and resume writes. The legacy table becomes read-only. An
   object's existing contribution is imported once, when its first new event commits; subsequent
   events use only SQL state. Other buckets start with a zero legacy contribution.

Keep the legacy table and its configuration until all baseline rows have been imported or
reconciled. New installations with zero initial totals omit both legacy variables. Do not delete
SQL accounting tombstones during ordinary app/user cleanup, because S3 notifications can arrive
after their owners have been deleted.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Cargo Lambda](https://www.cargo-lambda.info/guide/installation.html)

## Building

To build the project for production, run `cargo lambda build --release`. Remove the `--release` flag to build for development.

Read more about building your lambda function in [the Cargo Lambda documentation](https://www.cargo-lambda.info/commands/build.html).

## Testing

You can run regular Rust unit tests with `cargo test`.

`FLOW_LIKE_TEST_DATABASE_URL` enables the PostgreSQL accounting regression test. It creates a
unique temporary schema and checks overwrite/deletion rollback, legacy import, duplicate
delivery, concurrent duplicates, and out-of-order events. Use a disposable database with
permission to create and drop schemas.

If you want to run integration tests locally, you can use the `cargo lambda watch` and `cargo lambda invoke` commands to do it.

First, run `cargo lambda watch` to start a local server. When you make changes to the code, the server will automatically restart.

Second, you'll need a way to pass the event data to the lambda function.

You can use the existent [event payloads](https://github.com/awslabs/aws-lambda-rust-runtime/tree/main/lambda-events/src/fixtures) in the Rust Runtime repository if your lambda function is using one of the supported event types.

You can use those examples directly with the `--data-example` flag, where the value is the name of the file in the [lambda-events](https://github.com/awslabs/aws-lambda-rust-runtime/tree/main/lambda-events/src/fixtures) repository without the `example_` prefix and the `.json` extension.

```bash
cargo lambda invoke --data-example apigw-request
```

For generic events, where you define the event data structure, you can create a JSON file with the data you want to test with. For example:

```json
{
    "command": "test"
}
```

Then, run `cargo lambda invoke --data-file ./data.json` to invoke the function with the data in `data.json`.


Read more about running the local server in [the Cargo Lambda documentation for the `watch` command](https://www.cargo-lambda.info/commands/watch.html).
Read more about invoking the function in [the Cargo Lambda documentation for the `invoke` command](https://www.cargo-lambda.info/commands/invoke.html).

## Deploying

To deploy the project, run `cargo lambda deploy`. This will create an IAM role and a Lambda function in your AWS account.

Read more about deploying your lambda function in [the Cargo Lambda documentation](https://www.cargo-lambda.info/commands/deploy.html).
