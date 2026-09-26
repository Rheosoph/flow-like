CREATE TABLE "DeviceArchive" (
  "deviceId" TEXT NOT NULL, "archiveId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
  scope TEXT NOT NULL, kind TEXT NOT NULL, sequence BIGINT NOT NULL,
  digest TEXT NOT NULL, bundle TEXT NOT NULL, "sizeBytes" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL, "expiresAt" BIGINT NOT NULL,
  PRIMARY KEY ("deviceId", "archiveId"), UNIQUE ("deviceId", scope, kind, sequence)
);
CREATE INDEX "DeviceArchive_ownerId_expiresAt_idx" ON "DeviceArchive"("ownerId", "expiresAt");
CREATE INDEX "DeviceArchive_expiresAt_idx" ON "DeviceArchive"("expiresAt");
CREATE TABLE "DeviceArchiveHead" (
  "deviceId" TEXT NOT NULL, scope TEXT NOT NULL, kind TEXT NOT NULL,
  sequence BIGINT NOT NULL, digest TEXT NOT NULL,
  PRIMARY KEY ("deviceId", scope, kind)
);
