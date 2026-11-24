-- Aurora DSQL migration "file_accounting_updated_at_index"
-- Written to match packages/api/scripts/dsql-migration.ts output for a single index.
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one,
-- waits for the async index/validation jobs and records the migration in _prisma_migrations.
-- tables=0 indexes=1 foreign_keys=0 validations=0 statements=1
CREATE INDEX ASYNC "FileAccountingObject_updatedAt_idx" ON "FileAccountingObject"("updatedAt");
