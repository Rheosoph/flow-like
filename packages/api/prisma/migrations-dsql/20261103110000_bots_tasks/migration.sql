-- Bot tasks: work a bot schedules for itself (once or cron). A sweep fires due tasks exactly once.
CREATE TABLE "BotTask" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "runAt" TIMESTAMPTZ(3),
    "cron" TEXT,
    "timezone" TEXT,
    "instruction" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "nextRunAt" TIMESTAMPTZ(3),
    "lastRunAt" TIMESTAMPTZ(3),
    "enabled" BOOLEAN NOT NULL DEFAULT true,
    "createdBy" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "BotTask_pkey" PRIMARY KEY ("id")
);

CREATE INDEX ASYNC "BotTask_botId_idx" ON "BotTask"("botId");

CREATE INDEX ASYNC "BotTask_enabled_nextRunAt_idx" ON "BotTask"("enabled", "nextRunAt");

ALTER TABLE "BotTask" ADD CONSTRAINT "BotTask_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

-- ValidateForeignKeys (each returns a sys.jobs id; the runner waits for them)
ALTER TABLE ASYNC "BotTask" VALIDATE CONSTRAINT "BotTask_botId_fkey";
