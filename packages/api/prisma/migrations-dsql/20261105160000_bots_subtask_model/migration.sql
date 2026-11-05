-- Aurora DSQL migration "bots_subtask_model"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one
-- and records the migration in _prisma_migrations.
-- tables=0 indexes=0 foreign_keys=0 validations=0 statements=1
-- Bot sub-task model: the Bit id the bot's sub-tasks run on. DSQL cannot add a column with a
-- default, so the column is nullable; null uses the bot's own model.
-- Apply before deploying the API that reads the column.
ALTER TABLE "Bot" ADD COLUMN "subtaskModel" TEXT;
