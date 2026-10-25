-- The runner waits for each index build before dropping the superseded prefix index.
CREATE INDEX ASYNC "ExecutionUsageTracking_userId_createdAt_id_idx"
  ON "ExecutionUsageTracking"("userId", "createdAt", "id")
  INCLUDE ("appId", "status", "microseconds");

-- Find flagged executions without reading every successful execution in the window.
CREATE INDEX ASYNC "ExecutionUsageTracking_userId_status_createdAt_id_idx"
  ON "ExecutionUsageTracking"("userId", "status", "createdAt", "id");

DROP INDEX "ExecutionUsageTracking_userId_createdAt_idx";
ANALYZE "ExecutionUsageTracking";
