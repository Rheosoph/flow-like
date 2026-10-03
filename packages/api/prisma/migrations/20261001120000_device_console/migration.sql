ALTER TABLE "ManagedDevice" ADD COLUMN "revokedAt" BIGINT;
ALTER TABLE "ManagedDevice" ADD COLUMN "displayName" TEXT;
CREATE TABLE "DeviceAuthRejection" (
  "deviceId" TEXT PRIMARY KEY, code TEXT NOT NULL, "skewSeconds" BIGINT,
  count BIGINT NOT NULL DEFAULT 1, "firstAt" BIGINT NOT NULL, "lastAt" BIGINT NOT NULL
);
CREATE TABLE "DeviceCertificateNoticeMute" (
  "userId" TEXT NOT NULL, "deviceId" TEXT NOT NULL, "certificateId" TEXT NOT NULL,
  until BIGINT, "createdAt" BIGINT NOT NULL,
  PRIMARY KEY ("userId", "deviceId", "certificateId")
);
CREATE INDEX "DeviceCertificateNoticeMute_deviceId_idx" ON "DeviceCertificateNoticeMute"("deviceId");
