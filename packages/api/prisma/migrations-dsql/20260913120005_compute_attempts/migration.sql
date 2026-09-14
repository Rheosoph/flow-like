CREATE TABLE "ComputeAttempt" (
    id TEXT PRIMARY KEY,
    "functionName" TEXT NOT NULL,
    "requestId" TEXT NOT NULL,
    "operationId" TEXT,
    "payerId" TEXT,
    role TEXT NOT NULL,
    "costClass" TEXT NOT NULL,
    "memoryMb" INTEGER NOT NULL,
    architecture TEXT NOT NULL,
    region TEXT NOT NULL,
    "measuredDurationMs" BIGINT,
    "billedDurationMs" BIGINT,
    "costMicroUsd" BIGINT,
    "evidenceRank" INTEGER NOT NULL DEFAULT 0,
    evidence TEXT NOT NULL,
    "rateVersion" TEXT NOT NULL,
    status TEXT NOT NULL,
    "startedAt" TIMESTAMPTZ(3) NOT NULL,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX ASYNC "ComputeAttempt_functionName_requestId_key" ON "ComputeAttempt"("functionName", "requestId");
CREATE INDEX ASYNC "ComputeAttempt_operationId_startedAt_idx" ON "ComputeAttempt"("operationId", "startedAt");
CREATE INDEX ASYNC "ComputeAttempt_payerId_startedAt_idx" ON "ComputeAttempt"("payerId", "startedAt");
CREATE INDEX ASYNC "ComputeAttempt_status_startedAt_idx" ON "ComputeAttempt"(status, "startedAt");
CREATE TABLE "ComputeAttemptRevision" (
    id TEXT PRIMARY KEY,
    "attemptId" TEXT NOT NULL,
    evidence TEXT NOT NULL,
    payload TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now()
);
CREATE INDEX ASYNC "ComputeAttemptRevision_attemptId_createdAt_idx" ON "ComputeAttemptRevision"("attemptId", "createdAt");
CREATE INDEX ASYNC "ComputeAttempt_status_id_idx" ON "ComputeAttempt"(status, id);
CREATE INDEX ASYNC "ComputeAttempt_evidenceRank_id_idx" ON "ComputeAttempt"("evidenceRank", id);
