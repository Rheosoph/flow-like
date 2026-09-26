CREATE TABLE "DeviceManagementPolicy" (
  "deviceId" TEXT NOT NULL, version BIGINT NOT NULL, digest TEXT NOT NULL,
  "policyJws" TEXT NOT NULL, "expiresAt" BIGINT NOT NULL, "createdAt" BIGINT NOT NULL,
  PRIMARY KEY ("deviceId", version)
);
CREATE TABLE "DeviceManagementRecipient" (
  "deviceId" TEXT NOT NULL, version BIGINT NOT NULL, "grantId" TEXT NOT NULL,
  "userId" TEXT NOT NULL, "expiresAt" BIGINT NOT NULL,
  PRIMARY KEY ("deviceId", version, "grantId")
);
CREATE INDEX "DeviceManagementRecipient_userId_expiresAt_idx" ON "DeviceManagementRecipient"("userId", "expiresAt");
CREATE TABLE "DeviceManagementApplied" (
  "deviceId" TEXT PRIMARY KEY, version BIGINT NOT NULL, digest TEXT NOT NULL, "appliedAt" BIGINT NOT NULL
);
CREATE TABLE "DeviceControllerVault" (
  "userId" TEXT NOT NULL, "keyId" TEXT NOT NULL, "publicKey" TEXT NOT NULL,
  ciphertext TEXT NOT NULL, revision BIGINT NOT NULL, "updatedAt" BIGINT NOT NULL,
  PRIMARY KEY ("userId", "keyId")
);
