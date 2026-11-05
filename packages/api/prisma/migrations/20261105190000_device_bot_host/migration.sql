-- When a device's bot host last polled the hub (unix milliseconds); written at most once a minute.
ALTER TABLE "ManagedDevice" ADD COLUMN "botHostSeenAt" BIGINT;
