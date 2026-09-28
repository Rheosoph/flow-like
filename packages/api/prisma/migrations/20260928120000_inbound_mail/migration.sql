CREATE TABLE "InboundMailAddress" (
    "address" TEXT PRIMARY KEY,
    "appId" TEXT NOT NULL,
    "eventId" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "active" BOOLEAN NOT NULL DEFAULT TRUE,
    "createdAt" BIGINT NOT NULL,
    "updatedAt" BIGINT NOT NULL
);
CREATE INDEX "InboundMailAddress_appId_eventId_idx" ON "InboundMailAddress" ("appId", "eventId");
CREATE TABLE "InboundMailDelivery" (
    "id" TEXT PRIMARY KEY,
    "messageId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "eventId" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "envelope" TEXT,
    "objects" TEXT,
    "attempts" INTEGER NOT NULL DEFAULT 0,
    "nextAttempt" BIGINT NOT NULL,
    "lease" TEXT,
    "receivedAt" BIGINT NOT NULL,
    "expiresAt" BIGINT NOT NULL,
    "runId" TEXT NOT NULL
);
CREATE INDEX "InboundMailDelivery_status_nextAttempt_idx" ON "InboundMailDelivery" ("status", "nextAttempt");
CREATE INDEX "InboundMailDelivery_status_expiresAt_idx" ON "InboundMailDelivery" ("status", "expiresAt");
CREATE INDEX "InboundMailDelivery_messageId_idx" ON "InboundMailDelivery" ("messageId");
CREATE TABLE "MailAutomationSend" (
    "id" TEXT PRIMARY KEY,
    "appId" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "lease" TEXT,
    "leaseUntil" BIGINT NOT NULL,
    "response" TEXT,
    "createdAt" BIGINT NOT NULL,
    "expiresAt" BIGINT NOT NULL
);
CREATE INDEX "MailAutomationSend_expiresAt_idx" ON "MailAutomationSend" ("expiresAt");
CREATE TABLE "MailAutomationQuota" (
    "id" TEXT PRIMARY KEY,
    "count" BIGINT NOT NULL,
    "expiresAt" BIGINT NOT NULL
);
CREATE INDEX "MailAutomationQuota_expiresAt_idx" ON "MailAutomationQuota" ("expiresAt");
