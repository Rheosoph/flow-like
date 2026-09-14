CREATE TABLE "CloudDispatchIntent" (
  id TEXT PRIMARY KEY,
  "payerId" TEXT NOT NULL,
  "jobId" TEXT NOT NULL,
  "functionName" TEXT NOT NULL,
  "tenantId" TEXT,
  "payloadPath" TEXT NOT NULL,
  state TEXT NOT NULL,
  "nextAttemptAt" BIGINT NOT NULL,
  "expiresAt" BIGINT NOT NULL,
  attempts BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL
);
CREATE INDEX "CloudDispatchIntent_state_nextAttemptAt_idx" ON "CloudDispatchIntent"(state,"nextAttemptAt");
CREATE INDEX "CloudDispatchIntent_payerId_id_idx" ON "CloudDispatchIntent"("payerId",id);
