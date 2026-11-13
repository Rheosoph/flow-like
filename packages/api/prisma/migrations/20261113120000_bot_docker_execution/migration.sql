-- NULL preserves the native execution of existing bots. Docker is always explicit.
ALTER TABLE "Bot" ADD COLUMN "executionMode" TEXT;
