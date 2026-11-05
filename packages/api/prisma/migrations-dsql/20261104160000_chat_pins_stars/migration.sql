-- Pinned conversations (per person), pinned messages (per conversation) and starred messages (per person).
CREATE TABLE "PinnedConversation" (
    "principalId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "pinnedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "PinnedConversation_pkey" PRIMARY KEY ("principalId","conversationId")
);

CREATE TABLE "PinnedMessage" (
    "conversationId" TEXT NOT NULL,
    "messageId" TEXT NOT NULL,
    "pinnedBy" TEXT NOT NULL,
    "pinnedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "PinnedMessage_pkey" PRIMARY KEY ("conversationId","messageId")
);

CREATE TABLE "StarredMessage" (
    "principalId" TEXT NOT NULL,
    "messageId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "starredAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "StarredMessage_pkey" PRIMARY KEY ("principalId","messageId")
);

CREATE INDEX ASYNC "PinnedConversation_conversationId_idx" ON "PinnedConversation"("conversationId");

CREATE INDEX ASYNC "PinnedMessage_pinnedBy_idx" ON "PinnedMessage"("pinnedBy");

CREATE INDEX ASYNC "StarredMessage_principalId_starredAt_idx" ON "StarredMessage"("principalId", "starredAt");

CREATE INDEX ASYNC "StarredMessage_principalId_conversationId_starredAt_idx" ON "StarredMessage"("principalId", "conversationId", "starredAt");

CREATE INDEX ASYNC "StarredMessage_conversationId_messageId_idx" ON "StarredMessage"("conversationId", "messageId");

ALTER TABLE "PinnedConversation" ADD CONSTRAINT "PinnedConversation_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "PinnedConversation" VALIDATE CONSTRAINT "PinnedConversation_conversationId_fkey";

ALTER TABLE "PinnedMessage" ADD CONSTRAINT "PinnedMessage_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "PinnedMessage" VALIDATE CONSTRAINT "PinnedMessage_conversationId_fkey";

ALTER TABLE "StarredMessage" ADD CONSTRAINT "StarredMessage_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "StarredMessage" VALIDATE CONSTRAINT "StarredMessage_conversationId_fkey";
