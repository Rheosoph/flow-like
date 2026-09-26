-- Aurora DSQL migration "app_package_license_lapse"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one,
-- waits for the async index/validation jobs and records the migration in _prisma_migrations.
-- tables=0 indexes=0 foreign_keys=0 validations=0 statements=2
ALTER TABLE "AppPackage" ADD COLUMN "staleSince" TIMESTAMPTZ(3);

-- Pins that were already stale start their grace period now.
UPDATE "AppPackage" SET "staleSince" = CURRENT_TIMESTAMP WHERE "stale" = true;
