ALTER TABLE "AppPackage" ADD COLUMN "staleSince" TIMESTAMPTZ(3);

-- Pins that were already stale start their grace period now.
UPDATE "AppPackage" SET "staleSince" = CURRENT_TIMESTAMP WHERE "stale" = true;
