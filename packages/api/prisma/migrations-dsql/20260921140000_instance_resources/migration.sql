CREATE TABLE "PlacementResourceGrant" (
  id TEXT PRIMARY KEY,
  "deviceId" TEXT NOT NULL,
  "placementId" TEXT NOT NULL,
  "deploymentId" TEXT NOT NULL,
  "projectId" TEXT NOT NULL,
  "appId" TEXT,
  "delegatingUserId" TEXT NOT NULL,
  "approvedByUserId" TEXT NOT NULL,
  status TEXT NOT NULL,
  "authzVersion" BIGINT NOT NULL DEFAULT 1,
  "modelIds" TEXT NOT NULL,
  "maxInstances" BIGINT NOT NULL,
  "expiresAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL
);
CREATE INDEX ASYNC "PlacementResourceGrant_deviceId_placementId_status_idx" ON "PlacementResourceGrant"("deviceId","placementId",status);
CREATE INDEX ASYNC "PlacementResourceGrant_appId_status_idx" ON "PlacementResourceGrant"("appId",status);

CREATE TABLE "PlacementBillingGrant" (
  id TEXT PRIMARY KEY,
  "grantId" TEXT NOT NULL,
  "payerId" TEXT NOT NULL,
  "approvedByUserId" TEXT NOT NULL,
  status TEXT NOT NULL,
  "authzVersion" BIGINT NOT NULL DEFAULT 1,
  "limitMicros" BIGINT NOT NULL,
  "usedMicros" BIGINT NOT NULL DEFAULT 0,
  "reservedMicros" BIGINT NOT NULL DEFAULT 0,
  "expiresAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL
);
CREATE INDEX ASYNC "PlacementBillingGrant_grantId_status_idx" ON "PlacementBillingGrant"("grantId",status);
CREATE INDEX ASYNC "PlacementBillingGrant_payerId_status_idx" ON "PlacementBillingGrant"("payerId",status);

CREATE TABLE "WorkloadInstance" (
  id TEXT PRIMARY KEY,
  "deviceId" TEXT NOT NULL,
  "grantId" TEXT NOT NULL,
  "billingGrantId" TEXT NOT NULL,
  "workloadKey" TEXT NOT NULL,
  "workloadKeyThumbprint" TEXT NOT NULL,
  "deviceAuthEpoch" BIGINT NOT NULL,
  "grantAuthzVersion" BIGINT NOT NULL,
  "billingAuthzVersion" BIGINT NOT NULL,
  "keyEpoch" BIGINT NOT NULL DEFAULT 1,
  status TEXT NOT NULL,
  "registeredAt" BIGINT NOT NULL,
  "leaseExpiresAt" BIGINT NOT NULL,
  "registrationJws" TEXT NOT NULL
);
CREATE INDEX ASYNC "WorkloadInstance_grantId_status_leaseExpiresAt_idx" ON "WorkloadInstance"("grantId",status,"leaseExpiresAt");
CREATE INDEX ASYNC "WorkloadInstance_deviceId_status_leaseExpiresAt_idx" ON "WorkloadInstance"("deviceId",status,"leaseExpiresAt");

CREATE TABLE "InstanceProofReplay" (
  "instanceId" TEXT NOT NULL,
  "keyEpoch" BIGINT NOT NULL,
  "proofId" TEXT NOT NULL,
  "expiresAt" BIGINT NOT NULL,
  PRIMARY KEY ("instanceId","keyEpoch","proofId")
);
CREATE INDEX ASYNC "InstanceProofReplay_expiresAt_idx" ON "InstanceProofReplay"("expiresAt");

CREATE TABLE "InstanceUsageAdmission" (
  "operationId" TEXT PRIMARY KEY,
  "billingGrantId" TEXT NOT NULL,
  "instanceId" TEXT NOT NULL,
  "payerId" TEXT NOT NULL,
  authority TEXT NOT NULL,
  "requiredModelTier" TEXT NOT NULL,
  "ceilingMicros" BIGINT NOT NULL,
  "usedMicros" BIGINT NOT NULL DEFAULT 0,
  "reservedMicros" BIGINT NOT NULL,
  status TEXT NOT NULL,
  "createdAt" BIGINT NOT NULL
);
CREATE INDEX ASYNC "InstanceUsageAdmission_billingGrantId_status_idx" ON "InstanceUsageAdmission"("billingGrantId",status);
CREATE INDEX ASYNC "InstanceUsageAdmission_instanceId_createdAt_idx" ON "InstanceUsageAdmission"("instanceId","createdAt");

CREATE UNIQUE INDEX ASYNC "WorkloadInstance_workloadKeyThumbprint_key" ON "WorkloadInstance"("workloadKeyThumbprint");
