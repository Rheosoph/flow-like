-- Bots, the apps their owners attach to them and their encrypted secrets. Unique indexes treat
-- NULLs as distinct, so "Bot_ownerId_systemKey_key" holds one general and one flux bot per owner
-- while custom bots (null "systemKey") never collide.
CREATE TABLE "Bot" (
    "id" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "systemKey" TEXT,
    "presetId" TEXT,
    "name" TEXT,
    "description" TEXT,
    "instructions" TEXT,
    "appearance" JSONB NOT NULL,
    "modelPolicy" JSONB NOT NULL DEFAULT '{}',
    "classification" TEXT,
    "pii" BOOLEAN NOT NULL DEFAULT false,
    "capabilities" JSONB NOT NULL DEFAULT '[]',
    "profileId" TEXT,
    "location" TEXT NOT NULL DEFAULT 'SERVER',
    "deviceId" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,
    "deletedAt" TIMESTAMPTZ(3),

    CONSTRAINT "Bot_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotApp" (
    "botId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "addedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "BotApp_pkey" PRIMARY KEY ("botId","appId")
);

CREATE TABLE "BotSecret" (
    "botId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "encryption" TEXT NOT NULL,
    "ciphertext" TEXT NOT NULL,
    "description" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,
    "lastUsedAt" TIMESTAMPTZ(3),

    CONSTRAINT "BotSecret_pkey" PRIMARY KEY ("botId","name")
);

CREATE INDEX "Bot_ownerId_deletedAt_idx" ON "Bot"("ownerId", "deletedAt");

CREATE UNIQUE INDEX "Bot_ownerId_systemKey_key" ON "Bot"("ownerId", "systemKey");

CREATE INDEX "BotApp_appId_idx" ON "BotApp"("appId");

ALTER TABLE "Bot" ADD CONSTRAINT "Bot_ownerId_fkey" FOREIGN KEY ("ownerId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotApp" ADD CONSTRAINT "BotApp_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotApp" ADD CONSTRAINT "BotApp_appId_fkey" FOREIGN KEY ("appId") REFERENCES "App"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotSecret" ADD CONSTRAINT "BotSecret_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
