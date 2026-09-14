CREATE TABLE "QuotaWarningState" (
    id TEXT PRIMARY KEY,
    "payerId" TEXT NOT NULL,
    resource TEXT NOT NULL,
    "periodKey" TEXT NOT NULL,
    "highestThreshold" INTEGER NOT NULL DEFAULT 0,
    episode BIGINT NOT NULL DEFAULT 0,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now()
);
CREATE INDEX "QuotaWarningState_payerId_idx" ON "QuotaWarningState"("payerId");
