-- Bot turns record the person whose message started their chain of bot turns, so a chain of bots handing each other the word counts against that person's daily limit on each bot.
ALTER TABLE "BotTurn" ADD COLUMN "originId" TEXT;

CREATE INDEX ASYNC "BotTurn_botId_originId_createdAt_idx" ON "BotTurn"("botId", "originId", "createdAt");
