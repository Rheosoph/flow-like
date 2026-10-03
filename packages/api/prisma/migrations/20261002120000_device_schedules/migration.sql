CREATE TABLE "DeviceScheduleClaim" (
  "appId" TEXT NOT NULL, "eventId" TEXT NOT NULL,
  "deviceId" TEXT, "placementId" TEXT, "releasedBy" TEXT, "releasedAt" BIGINT,
  "grantId" TEXT, "claimedAt" BIGINT, "seenAt" BIGINT, "resumeAt" BIGINT,
  PRIMARY KEY ("appId", "eventId")
);
CREATE INDEX "DeviceScheduleClaim_grantId_idx" ON "DeviceScheduleClaim"("grantId");
