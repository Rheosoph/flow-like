ALTER TABLE "BotTask" ADD COLUMN "monitoring" BOOLEAN;
ALTER TABLE "BotTask" ALTER COLUMN "monitoring" SET DEFAULT false;
