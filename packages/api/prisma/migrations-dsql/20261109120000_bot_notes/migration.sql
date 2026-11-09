-- Bot notes: keyed working state a bot keeps in one conversation, and the live status line of a running turn.
ALTER TABLE "BotTurn" ADD COLUMN "statusText" TEXT;

CREATE TABLE "BotNote" (
    "botId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "key" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "status" TEXT,
    "body" TEXT NOT NULL DEFAULT '',
    "state" TEXT NOT NULL DEFAULT 'OPEN',
    "version" INTEGER NOT NULL,
    "classification" TEXT,
    "pii" BOOLEAN NOT NULL DEFAULT false,
    "updatedBy" TEXT NOT NULL,
    "history" JSONB NOT NULL DEFAULT '[]',
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotNote_pkey" PRIMARY KEY ("botId", "conversationId", "key")
);

CREATE INDEX ASYNC "BotNote_conversationId_idx" ON "BotNote"("conversationId");

ALTER TABLE "BotNote" ADD CONSTRAINT "BotNote_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotNote" ADD CONSTRAINT "BotNote_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotNote" VALIDATE CONSTRAINT "BotNote_botId_fkey";
ALTER TABLE ASYNC "BotNote" VALIDATE CONSTRAINT "BotNote_conversationId_fkey";
