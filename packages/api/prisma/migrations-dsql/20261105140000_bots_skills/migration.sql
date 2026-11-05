-- Bot skills: owner-installed guidance a bot loads before a task it describes. Only ACTIVE skills reach the bot.
CREATE TABLE "BotSkill" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "body" TEXT NOT NULL,
    "source" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "tools" JSONB NOT NULL DEFAULT '[]',
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "BotSkill_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX ASYNC "BotSkill_botId_name_key" ON "BotSkill"("botId", "name");

CREATE INDEX ASYNC "BotSkill_botId_status_idx" ON "BotSkill"("botId", "status");

ALTER TABLE "BotSkill" ADD CONSTRAINT "BotSkill_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

-- ValidateForeignKeys (each returns a sys.jobs id; the runner waits for them)
ALTER TABLE ASYNC "BotSkill" VALIDATE CONSTRAINT "BotSkill_botId_fkey";
