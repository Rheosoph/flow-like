-- Shareable invite links that let anyone signed in join a plain group chat at once.
CREATE TABLE "ConversationInviteLink" (
    "id" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "token" TEXT NOT NULL,
    "name" TEXT,
    "createdBy" TEXT NOT NULL,
    "maxUses" BIGINT NOT NULL DEFAULT -1,
    "countJoined" BIGINT NOT NULL DEFAULT 0,
    "expiresAt" TIMESTAMPTZ(3),
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "ConversationInviteLink_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "ConversationInviteLink_token_key" ON "ConversationInviteLink"("token");

CREATE INDEX "ConversationInviteLink_conversationId_createdAt_idx" ON "ConversationInviteLink"("conversationId", "createdAt");

CREATE INDEX "ConversationInviteLink_createdBy_idx" ON "ConversationInviteLink"("createdBy");

ALTER TABLE "ConversationInviteLink" ADD CONSTRAINT "ConversationInviteLink_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE;
