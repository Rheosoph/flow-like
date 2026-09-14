CREATE TABLE "AppRollingUsage" (
    id TEXT PRIMARY KEY,
    "appId" TEXT NOT NULL,
    "userId" TEXT NOT NULL,
    period TEXT NOT NULL,
    cost BIGINT NOT NULL DEFAULT 0,
    tokens BIGINT NOT NULL DEFAULT 0,
    calls BIGINT NOT NULL DEFAULT 0,
    ready BOOLEAN NOT NULL DEFAULT false,
    "backfillCutoff" TIMESTAMPTZ(3) NOT NULL,
    "cursorAt" TIMESTAMPTZ(3) NOT NULL,
    "cursorId" TEXT NOT NULL DEFAULT '',
    "sweptAt" BIGINT NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX "AppRollingUsage_scope_key" ON "AppRollingUsage"("appId", "userId", period);
CREATE INDEX "AppRollingUsage_sweptAt_idx" ON "AppRollingUsage"("sweptAt");
CREATE TABLE "AppRollingContribution" (
    id TEXT PRIMARY KEY,
    "counterId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "sourceId" TEXT NOT NULL,
    "expiresAt" BIGINT NOT NULL,
    cost BIGINT NOT NULL,
    tokens BIGINT NOT NULL,
    calls BIGINT NOT NULL
);
CREATE INDEX "AppRollingContribution_counterId_expiresAt_idx" ON "AppRollingContribution"("counterId", "expiresAt");
CREATE INDEX "AppRollingContribution_appId_idx" ON "AppRollingContribution"("appId");
CREATE INDEX "UsageInvocation_appId_startedAt_id_idx" ON "UsageInvocation"("appId", "startedAt", id);
CREATE INDEX "LLMUsageTracking_appId_createdAt_id_idx" ON "LLMUsageTracking"("appId", "createdAt", id);
CREATE INDEX "EmbeddingUsageTracking_appId_createdAt_id_idx" ON "EmbeddingUsageTracking"("appId", "createdAt", id);
CREATE INDEX "UsageInvocation_status_updatedAt_idx" ON "UsageInvocation"(status, "updatedAt");
