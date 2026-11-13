-- A task's instruction authorizes its runs only when the owner wrote it, or through the owner's
-- message that asked the bot for it. Existing rows stay NULL and authorize nothing.
ALTER TABLE "BotTask" ADD COLUMN "ownerWritten" BOOLEAN;
ALTER TABLE "BotTask" ADD COLUMN "requestConversationId" TEXT;
ALTER TABLE "BotTask" ADD COLUMN "requestMessageId" TEXT;
