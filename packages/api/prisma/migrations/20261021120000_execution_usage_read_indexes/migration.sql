-- Match the complete history order and cover the activity aggregate.
CREATE INDEX "ExecutionUsageTracking_userId_createdAt_id_idx"
  ON "ExecutionUsageTracking"("userId", "createdAt", "id", "appId", "status", "microseconds");

-- Find flagged executions without reading every successful execution in the window.
CREATE INDEX "ExecutionUsageTracking_userId_status_createdAt_id_idx"
  ON "ExecutionUsageTracking"("userId", "status", "createdAt", "id");

DROP INDEX "ExecutionUsageTracking_userId_createdAt_idx";
ANALYZE "ExecutionUsageTracking";
