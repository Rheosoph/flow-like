ALTER TABLE "ConversationMember" ADD COLUMN "hiddenAt" TIMESTAMPTZ(3);
ALTER TABLE "ConversationMember" ADD COLUMN "clearedAt" TIMESTAMPTZ(3);
ALTER TABLE "ConversationMember" ADD COLUMN "clearedSeq" BIGINT;
ALTER TABLE "InboxEntry" ADD COLUMN "cleared" BOOLEAN;
ALTER TABLE "ConversationMember" ADD COLUMN "clearedLegacyAt" TIMESTAMPTZ(3);
