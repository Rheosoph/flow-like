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

CREATE UNIQUE INDEX ASYNC "BotChannelLink_botId_provider_chatKey_key" ON "BotChannelLink"("botId", "provider", "chatKey");
CREATE INDEX ASYNC "BotChannelLink_ownerId_idx" ON "BotChannelLink"("ownerId");
CREATE INDEX ASYNC "BotChannelLink_conversationId_idx" ON "BotChannelLink"("conversationId");

ALTER TABLE "BotChannelLink" ADD CONSTRAINT "BotChannelLink_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotChannelLink" ADD CONSTRAINT "BotChannelLink_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotChannelLink" VALIDATE CONSTRAINT "BotChannelLink_botId_fkey";
ALTER TABLE ASYNC "BotChannelLink" VALIDATE CONSTRAINT "BotChannelLink_conversationId_fkey";
