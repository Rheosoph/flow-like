CREATE TABLE "DeviceInventoryObservation" (
  "userId" TEXT NOT NULL, "deviceId" TEXT NOT NULL, "keyId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL, revision BIGINT NOT NULL, payload TEXT NOT NULL,
  "updatedAt" BIGINT NOT NULL,
  PRIMARY KEY ("userId", "deviceId", "keyId", "scopeKey")
);
