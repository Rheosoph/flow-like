CREATE TABLE "DeviceFleetReader" (
 "deviceId" TEXT NOT NULL, "userId" TEXT NOT NULL, "keyId" TEXT NOT NULL,
 revision BIGINT NOT NULL, "readerJws" TEXT NOT NULL, deleted BOOLEAN NOT NULL,
 PRIMARY KEY ("deviceId", "userId", "keyId")
);
CREATE TABLE "DeviceFleetHead" (
 "deviceId" TEXT PRIMARY KEY, sequence BIGINT NOT NULL, digest TEXT NOT NULL
);
CREATE TABLE "DeviceFleetSnapshot" (
 "deviceId" TEXT NOT NULL, "streamId" TEXT NOT NULL, "userId" TEXT NOT NULL,
 "keyId" TEXT NOT NULL, bundle TEXT NOT NULL, "manifestJws" TEXT NOT NULL, "updatedAt" BIGINT NOT NULL,
 PRIMARY KEY ("deviceId", "streamId")
);
