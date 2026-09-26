CREATE TABLE "DeviceCertificateInventory" (
  "deviceId" TEXT PRIMARY KEY, revision BIGINT NOT NULL, payload TEXT NOT NULL,
  "updatedAt" BIGINT NOT NULL, "nextCheckAt" BIGINT NOT NULL
);
CREATE INDEX "DeviceCertificateInventory_nextCheckAt_idx" ON "DeviceCertificateInventory"("nextCheckAt");
CREATE TABLE "DeviceCertificateNotice" (
  id TEXT PRIMARY KEY, "deviceId" TEXT NOT NULL, "certificateId" TEXT NOT NULL,
  "certificateRevision" BIGINT NOT NULL, fingerprint TEXT NOT NULL, "notAfter" BIGINT NOT NULL,
  "userId" TEXT NOT NULL, stage TEXT NOT NULL, channel TEXT NOT NULL, status TEXT NOT NULL,
  attempts BIGINT NOT NULL DEFAULT 0, "nextAttemptAt" BIGINT NOT NULL,
  "leaseId" TEXT, "leaseUntil" BIGINT, "completedAt" BIGINT
);
CREATE INDEX "DeviceCertificateNotice_status_nextAttemptAt_idx" ON "DeviceCertificateNotice"(status, "nextAttemptAt");
CREATE INDEX "DeviceCertificateNotice_deviceId_idx" ON "DeviceCertificateNotice"("deviceId");
