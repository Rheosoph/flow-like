-- Bot turns record who woke them and whether the conversation is shared, so hosts can narrow the toolset.
-- `audience` is created with the table: DSQL cannot add a NOT NULL or defaulted column.
ALTER TABLE "BotTurn" ADD COLUMN "requesterId" TEXT;

CREATE INDEX ASYNC "BotTurn_botId_requesterId_createdAt_idx" ON "BotTurn"("botId", "requesterId", "createdAt");
