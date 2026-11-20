-- Bot calendar: an event task keeps its TaskEvent as JSON; a bot keeps its calendar wake-up settings and when its connected calendars were last planned.
ALTER TABLE "BotTask" ADD COLUMN "event" JSONB;
ALTER TABLE "Bot" ADD COLUMN "calendarWake" JSONB;
ALTER TABLE "Bot" ADD COLUMN "calendarSyncedAt" TIMESTAMPTZ(3);
