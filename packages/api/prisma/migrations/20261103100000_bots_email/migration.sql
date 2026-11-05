-- Bot email: a bot's mail threads and the RFC Message-IDs that thread later mails into them. A
-- bot's address lives in "InboundMailAddress" (kind 'bot'), so it needs no table here.
CREATE TABLE "BotMailThread" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "subject" TEXT NOT NULL,
    "participants" TEXT NOT NULL,
    "lastMessageAt" TIMESTAMPTZ(3) NOT NULL,
    "lastSnippet" TEXT,
    "messageCount" INTEGER NOT NULL DEFAULT 0,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "BotMailThread_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotMailMessage" (
    "id" TEXT NOT NULL,
    "threadId" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "messageId" TEXT NOT NULL,
    "direction" TEXT NOT NULL,
    "entryMessageId" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "BotMailMessage_pkey" PRIMARY KEY ("id")
);

CREATE INDEX "BotMailThread_botId_lastMessageAt_idx" ON "BotMailThread"("botId", "lastMessageAt");

CREATE INDEX "BotMailThread_ownerId_idx" ON "BotMailThread"("ownerId");

CREATE INDEX "BotMailMessage_threadId_idx" ON "BotMailMessage"("threadId");

CREATE UNIQUE INDEX "BotMailMessage_botId_messageId_key" ON "BotMailMessage"("botId", "messageId");

ALTER TABLE "BotMailThread" ADD CONSTRAINT "BotMailThread_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotMailMessage" ADD CONSTRAINT "BotMailMessage_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotMailMessage" ADD CONSTRAINT "BotMailMessage_threadId_fkey" FOREIGN KEY ("threadId") REFERENCES "BotMailThread"("id") ON DELETE CASCADE ON UPDATE CASCADE;
