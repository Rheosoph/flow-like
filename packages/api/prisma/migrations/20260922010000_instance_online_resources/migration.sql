ALTER TABLE "PlacementResourceGrant" ADD COLUMN "onlineAccess" TEXT;
ALTER TABLE "WorkloadInstance" ALTER COLUMN "billingGrantId" DROP NOT NULL;
ALTER TABLE "WorkloadInstance" ALTER COLUMN "billingAuthzVersion" DROP NOT NULL;
