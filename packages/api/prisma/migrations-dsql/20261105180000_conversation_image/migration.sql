-- Aurora DSQL migration "conversation_image"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one
-- and records the migration in _prisma_migrations.
-- tables=0 indexes=0 foreign_keys=0 validations=0 statements=1
-- A group's picture: the blake3 hex of the PNG stored at `conversations/<id>/image/<image>.png`.
-- Apply before deploying the API that reads the column.
ALTER TABLE "Conversation" ADD COLUMN "image" TEXT;
