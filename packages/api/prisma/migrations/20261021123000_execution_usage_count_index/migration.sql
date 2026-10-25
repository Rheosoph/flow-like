-- Exact counts read fewer bytes from this index than the covering activity index.
CREATE INDEX "ExecutionUsageTracking_userId_createdAt_idx"
  ON "ExecutionUsageTracking"("userId", "createdAt");
ANALYZE "ExecutionUsageTracking";
