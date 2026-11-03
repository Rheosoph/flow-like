-- Room invitations: a member is ACTIVE, PENDING (invited) or DECLINED; existing rows are ACTIVE. A bot's room limits live on its membership.
ALTER TABLE "ConversationMember" ADD COLUMN "status" TEXT NOT NULL DEFAULT 'ACTIVE';
ALTER TABLE "ConversationMember" ADD COLUMN "limits" JSONB;
ALTER TABLE "ConversationMember" ADD COLUMN "invitedBy" TEXT;
ALTER TABLE "ConversationMember" ADD COLUMN "invitedAt" TIMESTAMPTZ(3);
ALTER TABLE "ConversationMember" ADD COLUMN "acceptedAt" TIMESTAMPTZ(3);

CREATE INDEX "ConversationMember_principalId_status_idx" ON "ConversationMember"("principalId", "status");
