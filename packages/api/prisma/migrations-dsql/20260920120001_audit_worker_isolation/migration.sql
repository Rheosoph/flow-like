CREATE TABLE "AuditWorkerLease" (
    "id" BIGINT PRIMARY KEY,
    "owner" TEXT,
    "expiresAt" TIMESTAMPTZ(3),
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now()
);
