-- Aurora DSQL migration "audit_seals"
-- Hand-written to match prisma/schema/audit.prisma. Every statement runs in its own
-- transaction; apps/backend/aws/migration applies them one by one and waits for the
-- async index jobs.
CREATE TABLE "AuditRecord" (
  id TEXT PRIMARY KEY,
  "chainId" TEXT NOT NULL,
  "timestamp" TIMESTAMPTZ(3) NOT NULL,
  "actorId" TEXT NOT NULL,
  "actorType" TEXT NOT NULL,
  action TEXT NOT NULL,
  "resourceType" TEXT NOT NULL,
  "resourceId" TEXT NOT NULL,
  "ipCommitment" BYTEA,
  "detailsCommitment" BYTEA,
  "actorIp" TEXT,
  "ipSalt" BYTEA,
  details JSONB,
  "detailsSalt" BYTEA,
  mac BYTEA,
  "sealId" TEXT
);
CREATE TABLE "AuditSeal" (
  id TEXT PRIMARY KEY,
  "chainId" TEXT NOT NULL,
  seq BIGINT NOT NULL,
  class TEXT NOT NULL,
  "prevHash" BYTEA NOT NULL,
  "recordCount" INTEGER NOT NULL,
  "recordsRoot" BYTEA NOT NULL,
  "firstAt" TIMESTAMPTZ(3) NOT NULL,
  "lastAt" TIMESTAMPTZ(3) NOT NULL,
  "sealedAt" TIMESTAMPTZ(3) NOT NULL,
  hash BYTEA NOT NULL,
  "epochSeq" BIGINT,
  "epochIndex" INTEGER,
  "epochProof" BYTEA,
  mac BYTEA,
  "ipPending" BOOLEAN NOT NULL DEFAULT false,
  "detailsPending" BOOLEAN NOT NULL DEFAULT false
);
CREATE TABLE "AuditEpoch" (
  seq BIGINT PRIMARY KEY,
  "prevHash" BYTEA NOT NULL,
  "sealCount" INTEGER NOT NULL,
  "sealsRoot" BYTEA NOT NULL,
  "createdAt" TIMESTAMPTZ(3) NOT NULL,
  hash BYTEA NOT NULL,
  kid TEXT NOT NULL,
  signature BYTEA NOT NULL
);
CREATE TABLE "AuditWatermark" (
  "chainId" TEXT PRIMARY KEY,
  seq BIGINT NOT NULL,
  hash BYTEA NOT NULL,
  "prunedAt" TIMESTAMPTZ(3) NOT NULL,
  "batchRoot" BYTEA NOT NULL,
  "batchSize" INTEGER NOT NULL,
  "batchIndex" INTEGER NOT NULL,
  "batchProof" BYTEA NOT NULL,
  kid TEXT NOT NULL,
  signature BYTEA NOT NULL,
  "pendingDelete" BOOLEAN NOT NULL DEFAULT false
);
CREATE TABLE "AuditArchive" (
  period TEXT NOT NULL,
  part INTEGER NOT NULL,
  "objectKey" TEXT NOT NULL,
  sha256 BYTEA NOT NULL,
  "byteSize" BIGINT NOT NULL,
  "firstEpoch" BIGINT,
  "lastEpoch" BIGINT,
  "epochCount" BIGINT NOT NULL,
  "sealCount" BIGINT NOT NULL,
  "recordCount" BIGINT NOT NULL,
  "createdAt" TIMESTAMPTZ(3) NOT NULL,
  kid TEXT,
  signature BYTEA,
  manifest JSONB,
  "uploadedAt" TIMESTAMPTZ(3),
  CONSTRAINT "AuditArchive_pkey" PRIMARY KEY (period, part)
);
CREATE TABLE "AuditHeldChain" (
  "chainId" TEXT PRIMARY KEY,
  seq BIGINT NOT NULL,
  "sealId" TEXT NOT NULL,
  "heldAt" TIMESTAMPTZ(3) NOT NULL,
  reason TEXT NOT NULL
);
CREATE TABLE "AuditExportTarget" (
  "appId" TEXT PRIMARY KEY,
  url TEXT NOT NULL,
  secret TEXT NOT NULL,
  active BOOLEAN NOT NULL DEFAULT true,
  "evidenceCursor" BIGINT NOT NULL DEFAULT 0,
  "activityCursor" BIGINT NOT NULL DEFAULT 0,
  failures INTEGER NOT NULL DEFAULT 0,
  "nextAttemptAt" TIMESTAMPTZ(3),
  "lastDeliveredAt" TIMESTAMPTZ(3),
  "lastError" TEXT,
  "createdAt" TIMESTAMPTZ(3) NOT NULL,
  "updatedAt" TIMESTAMPTZ(3) NOT NULL
);
CREATE INDEX ASYNC "AuditRecord_chainId_timestamp_id_idx" ON "AuditRecord"("chainId","timestamp",id);
CREATE INDEX ASYNC "AuditRecord_sealId_chainId_timestamp_id_idx" ON "AuditRecord"("sealId","chainId","timestamp",id);
CREATE UNIQUE INDEX ASYNC "AuditSeal_chainId_seq_key" ON "AuditSeal"("chainId",seq);
CREATE INDEX ASYNC "AuditSeal_epochSeq_epochIndex_idx" ON "AuditSeal"("epochSeq","epochIndex");
CREATE INDEX ASYNC "AuditSeal_class_lastAt_idx" ON "AuditSeal"(class,"lastAt");
CREATE INDEX ASYNC "AuditSeal_ipPending_lastAt_idx" ON "AuditSeal"("ipPending","lastAt");
CREATE INDEX ASYNC "AuditSeal_detailsPending_lastAt_idx" ON "AuditSeal"("detailsPending","lastAt");
CREATE INDEX ASYNC "AuditEpoch_createdAt_idx" ON "AuditEpoch"("createdAt");
CREATE INDEX ASYNC "AuditWatermark_prunedAt_idx" ON "AuditWatermark"("prunedAt");
CREATE INDEX ASYNC "AuditWatermark_pendingDelete_idx" ON "AuditWatermark"("pendingDelete");
