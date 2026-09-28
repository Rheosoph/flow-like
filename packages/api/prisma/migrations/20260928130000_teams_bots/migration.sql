CREATE TABLE "TeamsBotConnection" (
    "id" TEXT PRIMARY KEY,
    "appId" TEXT NOT NULL,
    "eventId" TEXT NOT NULL UNIQUE,
    "data" TEXT NOT NULL,
    "revision" INTEGER NOT NULL DEFAULT 0,
    "managed" BOOLEAN NOT NULL DEFAULT false,
    "updatedAt" BIGINT NOT NULL
);
CREATE INDEX "TeamsBotConnection_appId_idx" ON "TeamsBotConnection" ("appId");
CREATE TABLE "TeamsBotState" (
    "id" TEXT PRIMARY KEY,
    "connectionId" TEXT NOT NULL,
    "data" TEXT NOT NULL,
    "revision" INTEGER NOT NULL DEFAULT 0,
    "expiresAt" BIGINT NOT NULL
);
CREATE INDEX "TeamsBotState_connectionId_idx" ON "TeamsBotState" ("connectionId");
CREATE INDEX "TeamsBotState_expiresAt_idx" ON "TeamsBotState" ("expiresAt");
