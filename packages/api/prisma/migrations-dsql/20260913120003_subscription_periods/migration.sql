ALTER TABLE "User" ADD COLUMN "subscriptionPeriodStart" TIMESTAMPTZ(3);
ALTER TABLE "User" ADD COLUMN "subscriptionPeriodEnd" TIMESTAMPTZ(3);
ALTER TABLE "User" ADD COLUMN "billingPeriodAnchor" TIMESTAMPTZ(3);
ALTER TABLE "User" ADD COLUMN "subscriptionId" TEXT;
ALTER TABLE "User" ADD COLUMN "subscriptionEventCreatedAt" BIGINT;
ALTER TABLE "User" ADD COLUMN "subscriptionEventId" TEXT;
ALTER TABLE "User" ADD COLUMN "subscriptionSyncRevision" BIGINT;
