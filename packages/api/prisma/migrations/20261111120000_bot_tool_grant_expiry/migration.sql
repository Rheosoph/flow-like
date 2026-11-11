-- Standing bot tool grants lapse 90 days after their last use.
ALTER TABLE "BotToolGrant" ADD COLUMN "expiresAt" TIMESTAMPTZ(3);
