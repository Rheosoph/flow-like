-- Turns a bot only read along with or got a reaction for are marked, so the requester daily cap counts prompted turns only.
ALTER TABLE "BotTurn" ADD COLUMN "unprompted" BOOLEAN;
