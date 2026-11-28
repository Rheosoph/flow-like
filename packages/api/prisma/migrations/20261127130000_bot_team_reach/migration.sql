-- Bot team reach: lead offers that wait for the bot's owner, people's chats with other owners'
-- bots, and people's classifications per org.
ALTER TABLE "BotTeam" ADD COLUMN "leadOfferBotId" TEXT;
ALTER TABLE "BotTeam" ADD COLUMN "leadOfferBy" TEXT;
ALTER TABLE "BotTeam" ADD COLUMN "leadOfferAt" TIMESTAMPTZ(3);

CREATE TABLE "BotTeamChat" (
    "botId" TEXT NOT NULL,
    "userId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "conversationId" TEXT,
    "limits" JSONB,
    "note" TEXT,
    "requestedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "decidedAt" TIMESTAMPTZ(3),
    "endedAt" TIMESTAMPTZ(3),
    CONSTRAINT "BotTeamChat_pkey" PRIMARY KEY ("botId", "userId")
);

CREATE TABLE "BotOrgClassification" (
    "orgId" TEXT NOT NULL,
    "userId" TEXT NOT NULL,
    "classification" TEXT NOT NULL,
    "setBy" TEXT NOT NULL,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotOrgClassification_pkey" PRIMARY KEY ("orgId", "userId")
);

CREATE UNIQUE INDEX "BotTeamChat_conversationId_key" ON "BotTeamChat"("conversationId");

CREATE INDEX "BotTeamChat_userId_idx" ON "BotTeamChat"("userId");

CREATE INDEX "BotTeamChat_ownerId_status_idx" ON "BotTeamChat"("ownerId", "status");

CREATE INDEX "BotOrgClassification_userId_idx" ON "BotOrgClassification"("userId");

ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_leadOfferBotId_fkey" FOREIGN KEY ("leadOfferBotId") REFERENCES "Bot"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "BotTeamChat" ADD CONSTRAINT "BotTeamChat_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotTeamChat" ADD CONSTRAINT "BotTeamChat_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotTeamChat" ADD CONSTRAINT "BotTeamChat_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "BotOrgClassification" ADD CONSTRAINT "BotOrgClassification_orgId_fkey" FOREIGN KEY ("orgId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotOrgClassification" ADD CONSTRAINT "BotOrgClassification_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;
