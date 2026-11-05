-- Files a conversation member uploaded into a conversation: platform storage under the conversation, not an app.
CREATE TABLE "ConversationFile" (
    "id" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "uploaderId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "mime" TEXT NOT NULL,
    "bytes" BIGINT NOT NULL,
    "sha256" TEXT,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "messageId" TEXT,
    "deletedAt" TIMESTAMPTZ(3),
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "ConversationFile_pkey" PRIMARY KEY ("id")
);

CREATE INDEX ASYNC "ConversationFile_conversationId_messageId_idx" ON "ConversationFile"("conversationId", "messageId");

CREATE INDEX ASYNC "ConversationFile_uploaderId_idx" ON "ConversationFile"("uploaderId");

CREATE INDEX ASYNC "ConversationFile_deletedAt_idx" ON "ConversationFile"("deletedAt");

CREATE INDEX ASYNC "ConversationFile_status_createdAt_idx" ON "ConversationFile"("status", "createdAt");

ALTER TABLE "ConversationFile" ADD CONSTRAINT "ConversationFile_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "ConversationFile" VALIDATE CONSTRAINT "ConversationFile_conversationId_fkey";
