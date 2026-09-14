CREATE TABLE "AccountCapacity" (
    "payerId" TEXT PRIMARY KEY,
    "storageBytes" BIGINT NOT NULL DEFAULT 0,
    "reservedStorageBytes" BIGINT NOT NULL DEFAULT 0,
    "projectCount" BIGINT NOT NULL DEFAULT 0,
    "initialized" BOOLEAN NOT NULL DEFAULT false,
    "baselineAppId" TEXT NOT NULL DEFAULT '',
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX "AccountCapacity_initialized_updatedAt_payerId_idx" ON "AccountCapacity"(initialized,"updatedAt","payerId");
CREATE TABLE "ProjectCapacity" (
    "appId" TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    "counted" BOOLEAN NOT NULL,
    "publicForkSourceId" TEXT,
    "active" BOOLEAN NOT NULL DEFAULT true,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX "ProjectCapacity_payerId_active_idx" ON "ProjectCapacity"("payerId", "active");
ALTER TABLE "FileAccountingObject" ADD COLUMN "payerId" TEXT;

CREATE TABLE "StorageUploadGrant" (
    "id" TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "bucket" TEXT NOT NULL,
    "objectKey" TEXT NOT NULL,
    "maxBytes" BIGINT NOT NULL,
    "reservedBytes" BIGINT NOT NULL,
    "expiresAt" TIMESTAMPTZ(3) NOT NULL,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX "StorageUploadGrant_expiresAt_idx" ON "StorageUploadGrant"("expiresAt");
CREATE INDEX "StorageUploadGrant_appId_idx" ON "StorageUploadGrant"("appId");
