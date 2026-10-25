-- Executions per UTC day, rolled up once the day has closed. Indexes carry
-- their trailing columns as INCLUDE payload, which Prisma cannot express, so
-- this file is written by hand rather than by scripts/dsql-migration.ts.
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

-- Home activity reads a user's days from this index alone.
CREATE INDEX ASYNC "ExecutionUsageDaily_userId_day_idx"
  ON "ExecutionUsageDaily"("userId", "day")
  INCLUDE ("appId", "status", "executions", "microseconds");
CREATE INDEX ASYNC "ExecutionUsageDaily_appId_idx" ON "ExecutionUsageDaily"("appId");
CREATE INDEX ASYNC "ExecutionUsageDaily_technicalUserId_idx" ON "ExecutionUsageDaily"("technicalUserId");

ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;
ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_technicalUserId_fkey" FOREIGN KEY ("technicalUserId") REFERENCES "TechnicalUser"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;
ALTER TABLE "ExecutionUsageDaily" ADD CONSTRAINT "ExecutionUsageDaily_appId_fkey" FOREIGN KEY ("appId") REFERENCES "App"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "ExecutionUsageDaily" VALIDATE CONSTRAINT "ExecutionUsageDaily_userId_fkey";
ALTER TABLE ASYNC "ExecutionUsageDaily" VALIDATE CONSTRAINT "ExecutionUsageDaily_technicalUserId_fkey";
ALTER TABLE ASYNC "ExecutionUsageDaily" VALIDATE CONSTRAINT "ExecutionUsageDaily_appId_fkey";

-- Roll up a day from the index alone. The runner waits for the build before
-- dropping the plain createdAt index it replaces.
CREATE INDEX ASYNC "ExecutionUsageTracking_createdAt_rollup_idx"
  ON "ExecutionUsageTracking"("createdAt")
  INCLUDE ("userId", "technicalUserId", "appId", "status", "microseconds");
DROP INDEX "ExecutionUsageTracking_createdAt_idx";
ANALYZE "ExecutionUsageTracking";
