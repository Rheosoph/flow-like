-- Uploaders can have a version purged by the maintenance sweep once its retention ends.
-- Null keeps existing versions until someone purges them.
ALTER TABLE "LockedItemVersion" ADD COLUMN "deleteAfterRetention" BOOLEAN;

CREATE INDEX "LockedItemVersion_status_deleteAfterRetention_retainUntil_idx"
  ON "LockedItemVersion"("status", "deleteAfterRetention", "retainUntil");
