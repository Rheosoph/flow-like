-- Manual and publish suite runs remember who started them, so a queued pickup
-- dispatches as that user instead of the event's run-as user. Null for scheduled
-- runs and for rows queued before this column existed; the API errors a manual or
-- publish pickup without one.
-- Apply before deploying the API that reads the column.
ALTER TABLE "RegressionSuiteRun" ADD COLUMN "actorUserId" TEXT;

-- The migration runner waits for the index build before the next statement.
CREATE INDEX ASYNC "RegressionSuiteRun_actorUserId_idx" ON "RegressionSuiteRun"("actorUserId");

ALTER TABLE "RegressionSuiteRun" ADD CONSTRAINT "RegressionSuiteRun_actorUserId_fkey" FOREIGN KEY ("actorUserId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "RegressionSuiteRun" VALIDATE CONSTRAINT "RegressionSuiteRun_actorUserId_fkey";
