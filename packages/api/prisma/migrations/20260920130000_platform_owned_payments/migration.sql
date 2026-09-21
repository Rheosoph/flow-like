-- Platform-owned payments have no connected account; historical routes remain unchanged.
ALTER TABLE "PaymentOrder" ALTER COLUMN "connectedAccountId" DROP NOT NULL;
ALTER TABLE "PaymentRequest" ALTER COLUMN "connectedAccountId" DROP NOT NULL;
