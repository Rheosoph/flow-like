-- Bots a profile hides from its crew: a custom bot's ID or a built-in bot's system key. NULL hides none.
ALTER TABLE "Profile" ADD COLUMN "hiddenBots" JSONB;
