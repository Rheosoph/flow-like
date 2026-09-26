-- Financial identities remain available after users and products are deleted.
CREATE TABLE "PaymentAccountBinding" (
  "id" TEXT PRIMARY KEY,
  "userId" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "purpose" TEXT NOT NULL,
  "activeAccountId" TEXT,
  "candidateAccountId" TEXT,
  "generation" BIGINT NOT NULL DEFAULT 0,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentAccountBinding_userId_livemode_purpose_key" ON "PaymentAccountBinding"("userId","livemode","purpose");
CREATE INDEX "PaymentAccountBinding_activeAccountId_idx" ON "PaymentAccountBinding"("activeAccountId");
CREATE INDEX "PaymentAccountBinding_candidateAccountId_idx" ON "PaymentAccountBinding"("candidateAccountId");

CREATE TABLE "ConnectedAccount" (
  "id" TEXT PRIMARY KEY,
  "userId" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "purpose" TEXT NOT NULL,
  "generation" BIGINT NOT NULL,
  "stripeAccountId" TEXT,
  "creationParams" JSONB NOT NULL,
  "state" TEXT NOT NULL DEFAULT 'creating',
  "country" TEXT NOT NULL,
  "defaultCurrency" TEXT,
  "business" JSONB,
  "requirements" JSONB,
  "capabilities" JSONB,
  "chargesEnabled" BOOLEAN NOT NULL DEFAULT false,
  "payoutsEnabled" BOOLEAN NOT NULL DEFAULT false,
  "canAcceptPayments" BOOLEAN NOT NULL DEFAULT false,
  "canSell" BOOLEAN NOT NULL DEFAULT false,
  "disabledReason" TEXT,
  "retiredAt" BIGINT,
  "retiredReason" TEXT,
  "consentId" TEXT,
  "resumeNonceHash" TEXT,
  "resumeExpiresAt" BIGINT,
  "resumeUsedAt" BIGINT,
  "syncedAt" BIGINT,
  "syncRevision" BIGINT NOT NULL DEFAULT 0,
  "syncLeaseUntil" BIGINT,
  "syncNextAt" BIGINT NOT NULL DEFAULT 0,
  "syncAttempts" INTEGER NOT NULL DEFAULT 0,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "ConnectedAccount_platformAccountId_livemode_stripeAccountId_key" ON "ConnectedAccount"("platformAccountId","livemode","stripeAccountId");
CREATE UNIQUE INDEX "ConnectedAccount_userId_livemode_purpose_generation_key" ON "ConnectedAccount"("userId","livemode","purpose","generation");
CREATE INDEX "ConnectedAccount_userId_livemode_retiredAt_idx" ON "ConnectedAccount"("userId","livemode","retiredAt");
CREATE INDEX "ConnectedAccount_state_syncedAt_idx" ON "ConnectedAccount"("state","syncedAt");

CREATE TABLE "AppPaymentSettings" (
  "appId" TEXT PRIMARY KEY,
  "ownerUserId" TEXT NOT NULL,
  "paymentsEnabled" BOOLEAN NOT NULL DEFAULT false,
  "refundsFromFlows" BOOLEAN NOT NULL DEFAULT false,
  "maxPaymentAmount" JSONB,
  "refundsDailyCap" JSONB,
  "adminBlockedAt" BIGINT,
  "adminBlockedReason" TEXT,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "updatedBy" TEXT NOT NULL,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);

CREATE TABLE "PaymentsBlock" (
  "userId" TEXT PRIMARY KEY,
  "reason" TEXT NOT NULL,
  "blockedBy" TEXT NOT NULL,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);

CREATE TABLE "PaymentRequest" (
  "id" TEXT PRIMARY KEY,
  "runId" TEXT NOT NULL,
  "nodeId" TEXT NOT NULL,
  "nonce" TEXT NOT NULL,
  "requestDigest" TEXT NOT NULL,
  "appId" TEXT NOT NULL,
  "boardId" TEXT,
  "eventId" TEXT,
  "payerUserId" TEXT NOT NULL,
  "payeeUserId" TEXT NOT NULL,
  "connectedAccountId" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "applicationFeeAmount" BIGINT NOT NULL,
  "feeBps" INTEGER NOT NULL,
  "productName" TEXT NOT NULL,
  "description" TEXT NOT NULL,
  "reference" TEXT,
  "snapshot" JSONB NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'CREATED',
  "reason" TEXT,
  "cancelRequested" BOOLEAN NOT NULL DEFAULT false,
  "acceptedAttemptId" TEXT,
  "runRevision" BIGINT,
  "settingsRevision" BIGINT,
  "bindingRevision" BIGINT,
  "expiresAt" BIGINT NOT NULL,
  "nextCheckAt" BIGINT NOT NULL,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentRequest_runId_nodeId_nonce_key" ON "PaymentRequest"("runId","nodeId","nonce");
CREATE INDEX "PaymentRequest_runId_status_idx" ON "PaymentRequest"("runId","status");
CREATE INDEX "PaymentRequest_appId_createdAt_idx" ON "PaymentRequest"("appId","createdAt");
CREATE INDEX "PaymentRequest_payerUserId_appId_reference_idx" ON "PaymentRequest"("payerUserId","appId","reference");
CREATE INDEX "PaymentRequest_payeeUserId_createdAt_idx" ON "PaymentRequest"("payeeUserId","createdAt");
CREATE INDEX "PaymentRequest_status_nextCheckAt_idx" ON "PaymentRequest"("status","nextCheckAt");
CREATE INDEX "PaymentRequest_status_expiresAt_idx" ON "PaymentRequest"("status","expiresAt");

CREATE TABLE "PaymentOrder" (
  "id" TEXT PRIMARY KEY,
  "kind" TEXT NOT NULL,
  "userId" TEXT NOT NULL,
  "itemId" TEXT NOT NULL,
  "payeeUserId" TEXT NOT NULL,
  "connectedAccountId" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "openKey" TEXT,
  "status" TEXT NOT NULL DEFAULT 'CREATED',
  "chargeType" TEXT NOT NULL,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "applicationFeeAmount" BIGINT NOT NULL,
  "feeBps" INTEGER NOT NULL,
  "snapshot" JSONB NOT NULL,
  "consentId" TEXT,
  "acceptedAttemptId" TEXT,
  "cancelRequested" BOOLEAN NOT NULL DEFAULT false,
  "withdrawnAt" BIGINT,
  "expiresAt" BIGINT NOT NULL,
  "nextCheckAt" BIGINT NOT NULL,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentOrder_openKey_key" ON "PaymentOrder"("openKey");
CREATE INDEX "PaymentOrder_userId_createdAt_idx" ON "PaymentOrder"("userId","createdAt");
CREATE INDEX "PaymentOrder_payeeUserId_createdAt_idx" ON "PaymentOrder"("payeeUserId","createdAt");
CREATE INDEX "PaymentOrder_kind_itemId_createdAt_idx" ON "PaymentOrder"("kind","itemId","createdAt");
CREATE INDEX "PaymentOrder_status_nextCheckAt_idx" ON "PaymentOrder"("status","nextCheckAt");
CREATE INDEX "PaymentOrder_status_expiresAt_idx" ON "PaymentOrder"("status","expiresAt");

CREATE TABLE "LegacyCheckout" (
  "id" TEXT PRIMARY KEY,
  "openKey" TEXT,
  "kind" TEXT NOT NULL,
  "userId" TEXT NOT NULL,
  "itemId" TEXT NOT NULL,
  "parameters" JSONB NOT NULL,
  "requestDigest" TEXT NOT NULL,
  "stripeSessionId" TEXT,
  "checkoutUrl" TEXT,
  "status" TEXT NOT NULL DEFAULT 'OPENING',
  "failureReason" TEXT,
  "expiresAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "LegacyCheckout_openKey_key" ON "LegacyCheckout"("openKey");
CREATE INDEX "LegacyCheckout_status_expiresAt_idx" ON "LegacyCheckout"("status","expiresAt");
CREATE INDEX "LegacyCheckout_userId_createdAt_idx" ON "LegacyCheckout"("userId","createdAt");
CREATE INDEX "LegacyCheckout_stripeSessionId_idx" ON "LegacyCheckout"("stripeSessionId");

CREATE TABLE "PaymentAttempt" (
  "id" TEXT PRIMARY KEY,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "attempt" INTEGER NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "connectedAccountId" TEXT,
  "livemode" BOOLEAN NOT NULL,
  "operationId" TEXT NOT NULL,
  "stripeSessionId" TEXT,
  "stripePaymentIntentId" TEXT,
  "stripeChargeId" TEXT,
  "checkoutUrl" TEXT,
  "payerUserId" TEXT,
  "payeeUserId" TEXT NOT NULL,
  "appId" TEXT,
  "packageId" TEXT,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "applicationFeeAmount" BIGINT NOT NULL,
  "feeBps" INTEGER NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'OPENING',
  "capturedAmount" BIGINT NOT NULL DEFAULT 0,
  "refundedAmount" BIGINT NOT NULL DEFAULT 0,
  "reservedRefundAmount" BIGINT NOT NULL DEFAULT 0,
  "orphaned" BOOLEAN NOT NULL DEFAULT false,
  "hydrationStatus" TEXT,
  "snapshot" JSONB NOT NULL,
  "expiresAt" BIGINT,
  "nextCheckAt" BIGINT,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentAttempt_sourceType_sourceId_attempt_key" ON "PaymentAttempt"("sourceType","sourceId","attempt");
CREATE UNIQUE INDEX "PaymentAttempt_9706843f2745_key" ON "PaymentAttempt"("platformAccountId","scopeKey","livemode","stripeSessionId");
CREATE UNIQUE INDEX "PaymentAttempt_b8a3abb70bb8_key" ON "PaymentAttempt"("platformAccountId","scopeKey","livemode","stripePaymentIntentId");
CREATE UNIQUE INDEX "PaymentAttempt_7786bca3b3a0_key" ON "PaymentAttempt"("platformAccountId","scopeKey","livemode","stripeChargeId");
CREATE INDEX "PaymentAttempt_status_nextCheckAt_idx" ON "PaymentAttempt"("status","nextCheckAt");
CREATE INDEX "PaymentAttempt_operationId_idx" ON "PaymentAttempt"("operationId");
CREATE INDEX "PaymentAttempt_payeeUserId_createdAt_idx" ON "PaymentAttempt"("payeeUserId","createdAt");
CREATE INDEX "PaymentAttempt_payerUserId_createdAt_idx" ON "PaymentAttempt"("payerUserId","createdAt");
CREATE INDEX "PaymentAttempt_appId_createdAt_idx" ON "PaymentAttempt"("appId","createdAt");
CREATE INDEX "PaymentAttempt_packageId_createdAt_idx" ON "PaymentAttempt"("packageId","createdAt");

CREATE TABLE "StripeOperation" (
  "id" TEXT PRIMARY KEY,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "operation" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "connectedAccountId" TEXT,
  "livemode" BOOLEAN NOT NULL,
  "requestParams" JSONB NOT NULL,
  "requestDigest" TEXT NOT NULL,
  "idempotencyKey" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'PENDING',
  "response" JSONB,
  "stripeObjectId" TEXT,
  "requestId" TEXT,
  "error" JSONB,
  "firstAttemptAt" BIGINT NOT NULL,
  "lastAttemptAt" BIGINT NOT NULL,
  "nextAttemptAt" BIGINT NOT NULL,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "leaseOwner" TEXT,
  "leaseExpiresAt" BIGINT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "StripeOperation_01a7fa2fb2c7_key" ON "StripeOperation"("platformAccountId","scopeKey","livemode","idempotencyKey");
CREATE INDEX "StripeOperation_status_nextAttemptAt_idx" ON "StripeOperation"("status","nextAttemptAt");
CREATE INDEX "StripeOperation_sourceType_sourceId_idx" ON "StripeOperation"("sourceType","sourceId");
CREATE INDEX "StripeOperation_requestId_idx" ON "StripeOperation"("requestId");

CREATE TABLE "PaymentWebhookInbox" (
  "id" TEXT PRIMARY KEY,
  "endpoint" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "stripeEventId" TEXT NOT NULL,
  "eventType" TEXT NOT NULL,
  "payload" JSONB NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'PENDING',
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "nextAttemptAt" BIGINT NOT NULL,
  "leaseOwner" TEXT,
  "leaseExpiresAt" BIGINT,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "quarantineReason" TEXT,
  "lastError" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentWebhookInbox_5b980572a4ec_key" ON "PaymentWebhookInbox"("endpoint","platformAccountId","scopeKey","livemode","stripeEventId");
CREATE INDEX "PaymentWebhookInbox_status_nextAttemptAt_idx" ON "PaymentWebhookInbox"("status","nextAttemptAt");

CREATE TABLE "PaymentOutbox" (
  "id" TEXT PRIMARY KEY,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "effect" TEXT NOT NULL,
  "payload" JSONB NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'PENDING',
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "nextAttemptAt" BIGINT NOT NULL,
  "leaseOwner" TEXT,
  "leaseExpiresAt" BIGINT,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "lastError" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE INDEX "PaymentOutbox_status_nextAttemptAt_idx" ON "PaymentOutbox"("status","nextAttemptAt");
CREATE INDEX "PaymentOutbox_sourceType_sourceId_idx" ON "PaymentOutbox"("sourceType","sourceId");

CREATE TABLE "PaymentRefund" (
  "id" TEXT PRIMARY KEY,
  "attemptId" TEXT NOT NULL,
  "commandId" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "stripeRefundId" TEXT,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'RESERVED',
  "reason" TEXT NOT NULL,
  "requestedBy" TEXT NOT NULL,
  "operationId" TEXT,
  "refundApplicationFee" BOOLEAN NOT NULL DEFAULT false,
  "reverseTransfer" BOOLEAN NOT NULL DEFAULT false,
  "failureReason" TEXT,
  "pendingReason" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentRefund_commandId_key" ON "PaymentRefund"("commandId");
CREATE UNIQUE INDEX "PaymentRefund_1ab912bc982b_key" ON "PaymentRefund"("platformAccountId","scopeKey","livemode","stripeRefundId");
CREATE INDEX "PaymentRefund_attemptId_status_idx" ON "PaymentRefund"("attemptId","status");
CREATE INDEX "PaymentRefund_status_updatedAt_idx" ON "PaymentRefund"("status","updatedAt");

CREATE TABLE "PaymentAdjustment" (
  "id" TEXT PRIMARY KEY,
  "attemptId" TEXT NOT NULL,
  "purpose" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "parentObjectId" TEXT,
  "stripeObjectId" TEXT,
  "operationId" TEXT NOT NULL,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'PENDING',
  "error" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentAdjustment_operationId_key" ON "PaymentAdjustment"("operationId");
CREATE UNIQUE INDEX "PaymentAdjustment_ade2a1ddd843_key" ON "PaymentAdjustment"("platformAccountId","scopeKey","livemode","stripeObjectId");
CREATE INDEX "PaymentAdjustment_attemptId_purpose_idx" ON "PaymentAdjustment"("attemptId","purpose");
CREATE INDEX "PaymentAdjustment_status_updatedAt_idx" ON "PaymentAdjustment"("status","updatedAt");

CREATE TABLE "PaymentLedgerEntry" (
  "id" TEXT PRIMARY KEY,
  "attemptId" TEXT,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "platformAccountId" TEXT NOT NULL,
  "scopeKey" TEXT NOT NULL,
  "livemode" BOOLEAN NOT NULL,
  "perspective" TEXT NOT NULL,
  "kind" TEXT NOT NULL,
  "amount" BIGINT NOT NULL,
  "currency" TEXT NOT NULL,
  "payeeUserId" TEXT,
  "payerUserId" TEXT,
  "appId" TEXT,
  "packageId" TEXT,
  "stripeObjectId" TEXT NOT NULL,
  "stripeBalanceTransactionId" TEXT,
  "stripeEventId" TEXT,
  "occurredAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentLedgerEntry_c3ec49d08e8a_key" ON "PaymentLedgerEntry"("platformAccountId","scopeKey","livemode","stripeObjectId","perspective","kind");
CREATE INDEX "PaymentLedgerEntry_payeeUserId_occurredAt_idx" ON "PaymentLedgerEntry"("payeeUserId","occurredAt");
CREATE INDEX "PaymentLedgerEntry_appId_occurredAt_idx" ON "PaymentLedgerEntry"("appId","occurredAt");
CREATE INDEX "PaymentLedgerEntry_packageId_occurredAt_idx" ON "PaymentLedgerEntry"("packageId","occurredAt");
CREATE INDEX "PaymentLedgerEntry_sourceType_sourceId_idx" ON "PaymentLedgerEntry"("sourceType","sourceId");
CREATE INDEX "PaymentLedgerEntry_attemptId_idx" ON "PaymentLedgerEntry"("attemptId");

CREATE TABLE "AccessGrant" (
  "id" TEXT PRIMARY KEY,
  "userId" TEXT NOT NULL,
  "itemKind" TEXT NOT NULL,
  "itemId" TEXT NOT NULL,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'ACTIVE',
  "reason" TEXT,
  "grantedBy" TEXT,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "AccessGrant_userId_itemKind_itemId_sourceType_sourceId_key" ON "AccessGrant"("userId","itemKind","itemId","sourceType","sourceId");
CREATE INDEX "AccessGrant_userId_itemKind_itemId_status_idx" ON "AccessGrant"("userId","itemKind","itemId","status");
CREATE INDEX "AccessGrant_sourceType_sourceId_idx" ON "AccessGrant"("sourceType","sourceId");

CREATE TABLE "PaymentEntitlement" (
  "id" TEXT PRIMARY KEY,
  "userId" TEXT NOT NULL,
  "itemKind" TEXT NOT NULL,
  "itemId" TEXT NOT NULL,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "blocked" BOOLEAN NOT NULL DEFAULT false,
  "blockReason" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentEntitlement_userId_itemKind_itemId_key" ON "PaymentEntitlement"("userId","itemKind","itemId");

CREATE TABLE "LegalConsent" (
  "id" TEXT PRIMARY KEY,
  "userId" TEXT NOT NULL,
  "kind" TEXT NOT NULL,
  "subjectType" TEXT,
  "subjectId" TEXT,
  "textVersion" TEXT NOT NULL,
  "textHash" TEXT NOT NULL,
  "locale" TEXT NOT NULL,
  "accepted" BOOLEAN NOT NULL,
  "evidence" JSONB,
  "createdAt" BIGINT NOT NULL
);
CREATE INDEX "LegalConsent_userId_kind_createdAt_idx" ON "LegalConsent"("userId","kind","createdAt");
CREATE INDEX "LegalConsent_subjectType_subjectId_idx" ON "LegalConsent"("subjectType","subjectId");

CREATE TABLE "PaymentLimitCounter" (
  "key" TEXT PRIMARY KEY,
  "count" BIGINT NOT NULL DEFAULT 0,
  "amount" BIGINT NOT NULL DEFAULT 0,
  "currency" TEXT,
  "windowEnd" BIGINT NOT NULL,
  "revision" BIGINT NOT NULL DEFAULT 0,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE INDEX "PaymentLimitCounter_windowEnd_idx" ON "PaymentLimitCounter"("windowEnd");

CREATE TABLE "PaymentLimitReservation" (
  "id" TEXT PRIMARY KEY,
  "counterKey" TEXT NOT NULL,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "amount" BIGINT NOT NULL,
  "count" BIGINT NOT NULL DEFAULT 1,
  "status" TEXT NOT NULL DEFAULT 'RESERVED',
  "expiresAt" BIGINT NOT NULL,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentLimitReservation_counterKey_sourceType_sourceId_key" ON "PaymentLimitReservation"("counterKey","sourceType","sourceId");
CREATE INDEX "PaymentLimitReservation_status_expiresAt_idx" ON "PaymentLimitReservation"("status","expiresAt");
CREATE INDEX "PaymentLimitReservation_counterKey_status_idx" ON "PaymentLimitReservation"("counterKey","status");

CREATE TABLE "PaymentLegacyArchive" (
  "id" TEXT PRIMARY KEY,
  "sourceType" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "canonicalId" TEXT,
  "originalRow" JSONB NOT NULL,
  "evidence" JSONB,
  "status" TEXT NOT NULL DEFAULT 'REVIEW',
  "reviewedBy" TEXT,
  "createdAt" BIGINT NOT NULL,
  "updatedAt" BIGINT NOT NULL
);
CREATE UNIQUE INDEX "PaymentLegacyArchive_sourceType_sourceId_key" ON "PaymentLegacyArchive"("sourceType","sourceId");
CREATE INDEX "PaymentLegacyArchive_status_createdAt_idx" ON "PaymentLegacyArchive"("status","createdAt");
CREATE INDEX "PaymentLegacyArchive_canonicalId_idx" ON "PaymentLegacyArchive"("canonicalId");
ALTER TABLE "AppPurchase" ADD COLUMN "paymentOrderId" TEXT;
ALTER TABLE "AppPurchase" ADD COLUMN "chargeType" TEXT;
CREATE INDEX "AppPurchase_paymentOrderId_idx" ON "AppPurchase"("paymentOrderId");
ALTER TABLE "AppPurchase" DROP CONSTRAINT "AppPurchase_userId_fkey";
ALTER TABLE "AppPurchase" DROP CONSTRAINT "AppPurchase_appId_fkey";
ALTER TABLE "WasmPackagePurchase" ADD COLUMN "paymentOrderId" TEXT;
ALTER TABLE "WasmPackagePurchase" ADD COLUMN "chargeType" TEXT;
CREATE INDEX "WasmPackagePurchase_paymentOrderId_idx" ON "WasmPackagePurchase"("paymentOrderId");
ALTER TABLE "WasmPackagePurchase" DROP CONSTRAINT "WasmPackagePurchase_userId_fkey";
ALTER TABLE "WasmPackagePurchase" DROP CONSTRAINT "WasmPackagePurchase_packageId_fkey";
ALTER TABLE "AppPurchase" DROP CONSTRAINT "AppPurchase_discountId_fkey";
ALTER TABLE "JoinQueue" ADD COLUMN "approvedAt" BIGINT;
ALTER TABLE "JoinQueue" ADD COLUMN "approvedBy" TEXT;
