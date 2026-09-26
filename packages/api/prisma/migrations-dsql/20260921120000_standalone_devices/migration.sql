CREATE TABLE "ManagedDevice" (
  id TEXT PRIMARY KEY,
  "ownerId" TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT NOT NULL,
  "authEpoch" BIGINT NOT NULL DEFAULT 1,
  identity TEXT NOT NULL,
  receipt TEXT NOT NULL,
  "registeredAt" BIGINT NOT NULL,
  "lastSeenAt" BIGINT
);
CREATE INDEX ASYNC "ManagedDevice_ownerId_status_idx" ON "ManagedDevice"("ownerId", status);

CREATE TABLE "DeviceEnrollment" (
  id TEXT PRIMARY KEY,
  "deviceId" TEXT NOT NULL,
  "ownerId" TEXT NOT NULL,
  "jwtId" TEXT NOT NULL,
  manifest TEXT NOT NULL,
  status TEXT NOT NULL,
  "expiresAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL,
  "consumedAt" BIGINT
);
CREATE UNIQUE INDEX ASYNC "DeviceEnrollment_deviceId_key" ON "DeviceEnrollment"("deviceId");
CREATE UNIQUE INDEX ASYNC "DeviceEnrollment_jwtId_key" ON "DeviceEnrollment"("jwtId");
CREATE INDEX ASYNC "DeviceEnrollment_ownerId_status_expiresAt_idx" ON "DeviceEnrollment"("ownerId", status, "expiresAt");

CREATE TABLE "DeviceChallenge" (
  "enrollmentId" TEXT PRIMARY KEY,
  id TEXT NOT NULL,
  "nonceHash" TEXT NOT NULL,
  "expiresAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX ASYNC "DeviceChallenge_id_key" ON "DeviceChallenge"(id);

CREATE TABLE "DeviceProofReplay" (
  "deviceId" TEXT NOT NULL,
  "authEpoch" BIGINT NOT NULL,
  "proofId" TEXT NOT NULL,
  "expiresAt" BIGINT NOT NULL,
  PRIMARY KEY ("deviceId", "authEpoch", "proofId")
);
CREATE INDEX ASYNC "DeviceProofReplay_expiresAt_idx" ON "DeviceProofReplay"("expiresAt");
