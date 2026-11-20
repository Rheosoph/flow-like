-- Bot calendar feed: one private iCalendar subscription per bot. Only the token's hash is used for lookup.
CREATE TABLE "BotCalendarFeed" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "tokenEncrypted" TEXT NOT NULL,
    "includeEvents" BOOLEAN NOT NULL DEFAULT true,
    "includeTasks" BOOLEAN NOT NULL DEFAULT true,
    "includeMemories" BOOLEAN NOT NULL DEFAULT false,
    "includeNotes" BOOLEAN NOT NULL DEFAULT false,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,
    "lastFetchedAt" TIMESTAMPTZ(3),

    CONSTRAINT "BotCalendarFeed_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "BotCalendarFeed_botId_key" ON "BotCalendarFeed"("botId");
CREATE UNIQUE INDEX "BotCalendarFeed_tokenHash_key" ON "BotCalendarFeed"("tokenHash");

ALTER TABLE "BotCalendarFeed" ADD CONSTRAINT "BotCalendarFeed_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
