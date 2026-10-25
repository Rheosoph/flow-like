-- Bytes of each owner's stored archives, maintained with every DeviceArchive insert and delete.
CREATE TABLE "DeviceArchiveUsage" (
    "ownerId" TEXT NOT NULL,
    "usedBytes" BIGINT NOT NULL,

    CONSTRAINT "DeviceArchiveUsage_pkey" PRIMARY KEY ("ownerId")
);

-- Owner sums, per-device usage and expired-row batches read this index alone.
CREATE INDEX "DeviceArchive_ownerUsage_idx"
  ON "DeviceArchive"("ownerId", "expiresAt", "deviceId", "archiveId", "sizeBytes", "createdAt");
DROP INDEX "DeviceArchive_ownerId_expiresAt_idx";
DROP INDEX "DeviceArchive_ownerId_expiresAt_sizeBytes_idx";
