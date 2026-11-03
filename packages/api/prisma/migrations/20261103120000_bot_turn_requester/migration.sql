-- Bot turns record who woke them and whether the conversation is shared, so hosts can narrow the toolset.
ALTER TABLE "BotTurn" ADD COLUMN "requesterId" TEXT;
ALTER TABLE "BotTurn" ADD COLUMN "audience" TEXT NOT NULL DEFAULT 'OWNER';

CREATE INDEX "BotTurn_botId_requesterId_createdAt_idx" ON "BotTurn"("botId", "requesterId", "createdAt");
