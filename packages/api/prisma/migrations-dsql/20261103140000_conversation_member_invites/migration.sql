-- Room invitations: a member is ACTIVE, PENDING (invited) or DECLINED; existing rows are ACTIVE. A bot's room limits live on its membership.
-- `status` is created with the table: DSQL cannot add a NOT NULL or defaulted column.
ALTER TABLE "ConversationMember" ADD COLUMN "limits" JSONB;
ALTER TABLE "ConversationMember" ADD COLUMN "invitedBy" TEXT;
ALTER TABLE "ConversationMember" ADD COLUMN "invitedAt" TIMESTAMPTZ(3);
ALTER TABLE "ConversationMember" ADD COLUMN "acceptedAt" TIMESTAMPTZ(3);

CREATE INDEX ASYNC "ConversationMember_principalId_status_idx" ON "ConversationMember"("principalId", "status");
