-- Uploaders can have a version purged by the maintenance sweep once its retention ends.
-- Null keeps existing versions until someone purges them; DSQL cannot add a column with a default.
ALTER TABLE "LockedItemVersion" ADD COLUMN "deleteAfterRetention" BOOLEAN;

-- The migration runner waits for the index build before the next statement.
CREATE INDEX ASYNC "LockedItemVersion_status_deleteAfterRetention_retainUntil_idx"
  ON "LockedItemVersion"("status", "deleteAfterRetention", "retainUntil");
