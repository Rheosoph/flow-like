-- Each member's unread count and newest message, kept by the writes that move them so the
-- conversation list stops counting per request. NULL means not computed yet: the list counts live.
ALTER TABLE "ConversationMember" ADD COLUMN "unreadCount" INTEGER;
ALTER TABLE "ConversationMember" ADD COLUMN "lastEntryId" TEXT;

-- Index-only answers for the realtime poll's per-account aggregates.
CREATE INDEX ASYNC "ConversationMember_principalId_markers_idx" ON "ConversationMember"("principalId", "lastReadAt", "clearedAt", "hiddenAt");
CREATE INDEX ASYNC "Notification_userId_readAt_idx" ON "Notification"("userId", "readAt");
CREATE INDEX ASYNC "BotTurn_ownerId_updatedAt_idx" ON "BotTurn"("ownerId", "updatedAt");
