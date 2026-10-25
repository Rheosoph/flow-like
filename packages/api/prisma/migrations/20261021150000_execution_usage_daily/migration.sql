-- Executions per UTC day, rolled up once the day has closed.
CREATE TABLE "ExecutionUsageDaily" (
    "day" DATE NOT NULL,
    "id" TEXT NOT NULL,
    "userId" TEXT,
    "technicalUserId" TEXT,
    "appId" TEXT,
    "status" TEXT NOT NULL,
    "executions" BIGINT NOT NULL,
    "microseconds" BIGINT NOT NULL,

    CONSTRAINT "ExecutionUsageDaily_pkey" PRIMARY KEY ("day", "id")
);

CREATE TABLE "ExecutionUsageDay" (
    "day" DATE NOT NULL,
    "rolledUpAt" TIMESTAMPTZ(3),
    "claim" TEXT,
    "claimedAt" TIMESTAMPTZ(3),

    CONSTRAINT "ExecutionUsageDay_pkey" PRIMARY KEY ("day")
);

CREATE INDEX "ExecutionUsageDaily_userId_day_idx"
  ON "ExecutionUsageDaily"("userId", "day", "appId", "status", "executions", "microseconds");
CREATE INDEX "ExecutionUsageDaily_appId_idx" ON "ExecutionUsageDaily"("appId");
CREATE INDEX "ExecutionUsageDaily_technicalUserId_idx" ON "ExecutionUsageDaily"("technicalUserId");

ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_technicalUserId_fkey" FOREIGN KEY ("technicalUserId") REFERENCES "TechnicalUser"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_appId_fkey" FOREIGN KEY ("appId") REFERENCES "App"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- Roll up a day from the index alone. The covering index replaces the plain one.
CREATE INDEX "ExecutionUsageTracking_createdAt_rollup_idx"
  ON "ExecutionUsageTracking"("createdAt", "userId", "technicalUserId", "appId", "status", "microseconds");
DROP INDEX "ExecutionUsageTracking_createdAt_idx";
ANALYZE "ExecutionUsageTracking";
