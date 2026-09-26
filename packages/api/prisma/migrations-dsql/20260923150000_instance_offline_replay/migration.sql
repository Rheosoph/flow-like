CREATE TABLE "InstanceOfflineReceipt" (
  "grantId" TEXT NOT NULL,
  "authzVersion" BIGINT NOT NULL,
  "operationId" TEXT NOT NULL,
  digest TEXT NOT NULL,
  "instanceId" TEXT NOT NULL,
  "projectId" TEXT NOT NULL,
  "delegatingUserId" TEXT NOT NULL,
  status TEXT NOT NULL,
  result TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL,
  PRIMARY KEY ("grantId", "authzVersion", "operationId")
);
CREATE INDEX ASYNC "InstanceOfflineReceipt_projectId_createdAt_idx" ON "InstanceOfflineReceipt" ("projectId", "createdAt");
