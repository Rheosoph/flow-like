-- A bot's room invitation carries the reason its owner approved; an address without an account gets it as a single-use link by email.
ALTER TABLE "ConversationMember" ADD COLUMN "inviteReason" TEXT;
ALTER TABLE "ConversationInviteLink" ADD COLUMN "email" TEXT;
ALTER TABLE "ConversationInviteLink" ADD COLUMN "reason" TEXT;

-- When a signed-in app last connected; empty until the next connect, which counts as away. No backfill: DSQL caps a transaction at 3,000 rows and db-push targets never run this file.
ALTER TABLE "User" ADD COLUMN "lastSeenAt" TIMESTAMPTZ(3);
