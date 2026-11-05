-- Aurora DSQL migration "device_bot_host"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one
-- and records the migration in _prisma_migrations.
-- tables=0 indexes=0 foreign_keys=0 validations=0 statements=1
-- When a device's bot host last polled the hub (unix milliseconds); written at most once a minute.
-- Apply before deploying the API that reads the column.
ALTER TABLE "ManagedDevice" ADD COLUMN "botHostSeenAt" BIGINT;
