CREATE TABLE "QuotaPeriod" (
    id TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    "periodStart" BIGINT NOT NULL,
    "periodEnd" BIGINT NOT NULL,
    used TEXT NOT NULL,
    reserved TEXT NOT NULL,
    "updatedAt" BIGINT NOT NULL,
    "trackingStartedAt" BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX "QuotaPeriod_payerId_periodEnd_idx" ON "QuotaPeriod"("payerId", "periodEnd");
CREATE TABLE "QuotaAccount" (
    "payerId" TEXT PRIMARY KEY,
    active BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE "QuotaOperation" (
    id TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    "periodId" TEXT NOT NULL,
    "actorId" TEXT,
    "appId" TEXT,
    "modelId" TEXT,
    provider TEXT,
    kind TEXT NOT NULL,
    "fundingClass" TEXT NOT NULL,
    "executionMode" TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'legacy',
    "entitlementVersion" TEXT NOT NULL DEFAULT 'legacy',
    status TEXT NOT NULL,
    ceiling TEXT NOT NULL,
    used TEXT NOT NULL,
    reserved TEXT NOT NULL,
    deadline BIGINT NOT NULL,
    generation BIGINT NOT NULL DEFAULT 0,
    "ownerId" TEXT,
    "cancelRequested" BOOLEAN NOT NULL DEFAULT FALSE,
    "createdAt" BIGINT NOT NULL,
    "updatedAt" BIGINT NOT NULL,
    "payloadsDeletedAt" BIGINT,
    "nextReceiptCheckAt" BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX "QuotaOperation_status_kind_nextReceiptCheckAt_id_idx" ON "QuotaOperation"(status, kind, "nextReceiptCheckAt", id);
CREATE INDEX "QuotaOperation_payerId_id_idx" ON "QuotaOperation"("payerId", id);
CREATE INDEX "QuotaOperation_status_deadline_idx" ON "QuotaOperation"(status, deadline);
CREATE INDEX "QuotaOperation_status_updatedAt_idx" ON "QuotaOperation"(status, "updatedAt");
CREATE INDEX "QuotaOperation_status_payloadsDeletedAt_deadline_idx" ON "QuotaOperation"(status, "payloadsDeletedAt", deadline);
CREATE INDEX "QuotaOperation_appId_id_idx" ON "QuotaOperation"("appId", id);
CREATE TABLE "QuotaEvent" (
    id TEXT PRIMARY KEY,
    "operationId" TEXT NOT NULL,
    "payerId" TEXT NOT NULL,
    actual TEXT NOT NULL,
    detail TEXT NOT NULL,
    "createdAt" BIGINT NOT NULL
);
CREATE INDEX "QuotaEvent_operationId_createdAt_idx" ON "QuotaEvent"("operationId", "createdAt");
CREATE TABLE "QuotaDailyUsage" (
    id TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    day TEXT NOT NULL,
    "appId" TEXT,
    "modelId" TEXT,
    provider TEXT,
    "fundingClass" TEXT NOT NULL,
    "executionMode" TEXT NOT NULL,
    used TEXT NOT NULL,
    "updatedAt" BIGINT NOT NULL
);
CREATE INDEX "QuotaDailyUsage_payerId_day_id_idx" ON "QuotaDailyUsage"("payerId", day, id);
