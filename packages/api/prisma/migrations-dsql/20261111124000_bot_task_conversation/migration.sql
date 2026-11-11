-- A task scheduled in a group or someone else's chat fires there, sent as the person who asked.
ALTER TABLE "BotTask" ADD COLUMN "conversationId" TEXT;
ALTER TABLE "BotTask" ADD COLUMN "requestedBy" TEXT;
