-- Bot webhooks: public URLs that wake a bot. Only the hash of a webhook's secret is stored.
CREATE TABLE "BotWebhook" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "secretHash" TEXT NOT NULL,
    "instruction" TEXT NOT NULL DEFAULT '',
    "enabled" BOOLEAN NOT NULL DEFAULT true,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "lastCalledAt" TIMESTAMPTZ(3),

    CONSTRAINT "BotWebhook_pkey" PRIMARY KEY ("id")
);

CREATE INDEX ASYNC "BotWebhook_botId_idx" ON "BotWebhook"("botId");

ALTER TABLE "BotWebhook" ADD CONSTRAINT "BotWebhook_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

-- ValidateForeignKeys (each returns a sys.jobs id; the runner waits for them)
ALTER TABLE ASYNC "BotWebhook" VALIDATE CONSTRAINT "BotWebhook_botId_fkey";
