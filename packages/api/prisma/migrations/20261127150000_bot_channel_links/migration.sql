-- Bot channel links: the Microsoft Teams and Discord chats a bot is part of, each with its guest room.
CREATE TABLE "BotChannelLink" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "provider" TEXT NOT NULL,
    "chatKey" TEXT NOT NULL,
    "chatTitle" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "meta" TEXT NOT NULL DEFAULT '{}',
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "lastSeenAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotChannelLink_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotChannelSender" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotChannelSender_pkey" PRIMARY KEY ("botId", "id")
);

CREATE UNIQUE INDEX "BotChannelLink_botId_provider_chatKey_key" ON "BotChannelLink"("botId", "provider", "chatKey");
CREATE INDEX "BotChannelLink_ownerId_idx" ON "BotChannelLink"("ownerId");
CREATE INDEX "BotChannelLink_conversationId_idx" ON "BotChannelLink"("conversationId");
CREATE INDEX "BotChannelSender_id_idx" ON "BotChannelSender"("id");

ALTER TABLE "BotChannelLink" ADD CONSTRAINT "BotChannelLink_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotChannelLink" ADD CONSTRAINT "BotChannelLink_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotChannelSender" ADD CONSTRAINT "BotChannelSender_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
