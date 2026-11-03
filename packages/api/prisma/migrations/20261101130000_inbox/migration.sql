-- Inboxes: conversations, their members, every principal's copy of each conversation event and
-- the public keys envelopes are sealed to. Principals are user subs and bot ids in one column, so
-- nothing references User or Bot; a member row stays after leaving so history keeps its members.
CREATE TABLE "Conversation" (
    "id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "title" TEXT,
    "encryption" TEXT NOT NULL DEFAULT 'PLAIN',
    "directKey" TEXT,
    "createdBy" TEXT NOT NULL,
    "lastActivityAt" TIMESTAMPTZ(3) NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "Conversation_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "ConversationMember" (
    "conversationId" TEXT NOT NULL,
    "principalId" TEXT NOT NULL,
    "principalKind" TEXT NOT NULL,
    "addedBy" TEXT,
    "joinedAt" TIMESTAMPTZ(3) NOT NULL,
    "leftAt" TIMESTAMPTZ(3),
    "lastReadAt" TIMESTAMPTZ(3),

    CONSTRAINT "ConversationMember_pkey" PRIMARY KEY ("conversationId","principalId")
);

CREATE TABLE "InboxEntry" (
    "id" TEXT NOT NULL,
    "inboxId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "messageId" TEXT NOT NULL,
    "senderId" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "envelope" JSONB NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "InboxEntry_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "InboxKey" (
    "principalId" TEXT NOT NULL,
    "keyId" TEXT NOT NULL,
    "algorithm" TEXT NOT NULL,
    "publicKey" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "revokedAt" TIMESTAMPTZ(3),

    CONSTRAINT "InboxKey_pkey" PRIMARY KEY ("principalId","keyId")
);

CREATE UNIQUE INDEX "Conversation_directKey_key" ON "Conversation"("directKey");

CREATE INDEX "ConversationMember_principalId_leftAt_idx" ON "ConversationMember"("principalId", "leftAt");

CREATE INDEX "InboxEntry_inboxId_conversationId_createdAt_idx" ON "InboxEntry"("inboxId", "conversationId", "createdAt");

CREATE INDEX "InboxEntry_inboxId_createdAt_idx" ON "InboxEntry"("inboxId", "createdAt");

CREATE UNIQUE INDEX "InboxEntry_inboxId_messageId_key" ON "InboxEntry"("inboxId", "messageId");

ALTER TABLE "ConversationMember" ADD CONSTRAINT "ConversationMember_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE;
