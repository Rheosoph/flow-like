-- Other people find and invite a bot only when its owner allows it; null counts as off, so existing bots stay hidden without a backfill.
ALTER TABLE "Bot" ADD COLUMN "othersCanInvite" BOOLEAN;
