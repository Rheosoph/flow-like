-- How many turns people other than the owner may start on a bot in a day; null keeps the default.
ALTER TABLE "Bot" ADD COLUMN "othersDailyTurns" INTEGER;
