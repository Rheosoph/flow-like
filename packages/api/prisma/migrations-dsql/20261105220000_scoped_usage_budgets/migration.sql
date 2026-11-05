ALTER TABLE "QuotaOperation" ADD COLUMN "botId" TEXT;
ALTER TABLE "QuotaOperation" ADD COLUMN "conversationId" TEXT;
CREATE INDEX ASYNC "QuotaOperation_botId_createdAt_idx" ON "QuotaOperation"("botId", "createdAt");
CREATE INDEX ASYNC "QuotaOperation_botId_conversationId_createdAt_idx" ON "QuotaOperation"("botId", "conversationId", "createdAt");

CREATE TABLE "UsageBudgetPolicy" (
  "id" TEXT NOT NULL PRIMARY KEY,
  "scope" TEXT NOT NULL,
  "subjectId" TEXT NOT NULL,
  "groupId" TEXT,
  "ownerLimits" TEXT,
  "adminLimits" TEXT,
  "updatedAt" BIGINT NOT NULL
);
CREATE INDEX ASYNC "UsageBudgetPolicy_scope_subjectId_idx" ON "UsageBudgetPolicy"("scope", "subjectId");

CREATE TABLE "UsageBudgetPeriod" (
  "id" TEXT NOT NULL PRIMARY KEY,
  "scopeId" TEXT NOT NULL,
  "scope" TEXT NOT NULL,
  "subjectId" TEXT NOT NULL,
  "groupId" TEXT,
  "periodStart" BIGINT NOT NULL,
  "periodEnd" BIGINT NOT NULL,
  "used" TEXT NOT NULL,
  "reserved" TEXT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE INDEX ASYNC "UsageBudgetPeriod_scope_periodStart_subjectId_idx" ON "UsageBudgetPeriod"("scope", "periodStart", "subjectId");
CREATE INDEX ASYNC "UsageBudgetPeriod_scopeId_periodStart_idx" ON "UsageBudgetPeriod"("scopeId", "periodStart");

CREATE TABLE "UsageBudgetReservation" (
  "id" TEXT NOT NULL PRIMARY KEY,
  "operationId" TEXT NOT NULL,
  "periodId" TEXT NOT NULL
);
CREATE INDEX ASYNC "UsageBudgetReservation_operationId_idx" ON "UsageBudgetReservation"("operationId");

ALTER TABLE "UsageInvocation" ADD COLUMN "botId" TEXT;
ALTER TABLE "UsageInvocation" ADD COLUMN "conversationId" TEXT;
CREATE INDEX ASYNC "UsageInvocation_botId_createdAt_idx" ON "UsageInvocation"("botId", "createdAt");
CREATE INDEX ASYNC "UsageInvocation_botId_conversationId_createdAt_idx" ON "UsageInvocation"("botId", "conversationId", "createdAt");
