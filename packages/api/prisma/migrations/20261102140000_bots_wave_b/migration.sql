-- Bots wave B: a bot's workspace marker on its app, the embedding model of its memory, event
-- positions in conversations (unique per copy), each bot's own conversation summaries, standing
-- bot-to-bot grants, host presence and the headless turn ledger. New columns are nullable like
-- their Aurora DSQL twin, which cannot add a column with a default; a null "lastSeq" reads as 0.
ALTER TABLE "App" ADD COLUMN "botId" TEXT;

ALTER TABLE "Bot" ADD COLUMN "embeddingModel" TEXT;

ALTER TABLE "Conversation" ADD COLUMN "lastSeq" BIGINT;

ALTER TABLE "InboxEntry" ADD COLUMN "seq" BIGINT;

CREATE TABLE "BotPermission" (
    "id" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "fromBotId" TEXT NOT NULL,
    "toBotId" TEXT NOT NULL,
    "grantedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "grantedBy" TEXT NOT NULL,

    CONSTRAINT "BotPermission_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotHostHeartbeat" (
    "botId" TEXT NOT NULL,
    "hostId" TEXT NOT NULL,
    "hostKind" TEXT NOT NULL,
    "lastSeenAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "BotHostHeartbeat_pkey" PRIMARY KEY ("botId","hostId")
);

CREATE TABLE "BotToolGrant" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "tool" TEXT NOT NULL,
    "scope" TEXT NOT NULL,
    "grantedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "grantedBy" TEXT NOT NULL,

    CONSTRAINT "BotToolGrant_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotTurn" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "ownerId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "triggerMessageId" TEXT NOT NULL,
    "hostKind" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'QUEUED',
    "attempts" INTEGER NOT NULL DEFAULT 0,
    "hostId" TEXT,
    "leaseUntil" TIMESTAMPTZ(3),
    "operationId" TEXT,
    "replyMessageId" TEXT,
    "errorCode" TEXT,
    "hops" INTEGER NOT NULL DEFAULT 0,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "BotTurn_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "ConversationSegment" (
    "id" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "authorId" TEXT NOT NULL,
    "level" INTEGER NOT NULL,
    "seqFrom" BIGINT NOT NULL,
    "seqTo" BIGINT NOT NULL,
    "tFrom" TIMESTAMPTZ(3) NOT NULL,
    "tTo" TIMESTAMPTZ(3) NOT NULL,
    "title" TEXT,
    "summary" TEXT NOT NULL,
    "parentId" TEXT,
    "version" INTEGER NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "ConversationSegment_pkey" PRIMARY KEY ("id")
);

CREATE INDEX "BotPermission_ownerId_idx" ON "BotPermission"("ownerId");

CREATE INDEX "BotPermission_toBotId_idx" ON "BotPermission"("toBotId");

CREATE UNIQUE INDEX "BotPermission_fromBotId_toBotId_key" ON "BotPermission"("fromBotId", "toBotId");

CREATE UNIQUE INDEX "BotToolGrant_botId_tool_scope_key" ON "BotToolGrant"("botId", "tool", "scope");

CREATE INDEX "BotTurn_status_leaseUntil_idx" ON "BotTurn"("status", "leaseUntil");

CREATE INDEX "BotTurn_ownerId_status_idx" ON "BotTurn"("ownerId", "status");

CREATE UNIQUE INDEX "BotTurn_botId_triggerMessageId_key" ON "BotTurn"("botId", "triggerMessageId");

CREATE INDEX "ConversationSegment_conversationId_authorId_level_seqFrom_idx" ON "ConversationSegment"("conversationId", "authorId", "level", "seqFrom");

CREATE INDEX "ConversationSegment_authorId_idx" ON "ConversationSegment"("authorId");

CREATE UNIQUE INDEX "App_botId_key" ON "App"("botId");

CREATE UNIQUE INDEX "InboxEntry_inboxId_conversationId_seq_key" ON "InboxEntry"("inboxId", "conversationId", "seq");

ALTER TABLE "BotPermission" ADD CONSTRAINT "BotPermission_ownerId_fkey" FOREIGN KEY ("ownerId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotPermission" ADD CONSTRAINT "BotPermission_fromBotId_fkey" FOREIGN KEY ("fromBotId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotPermission" ADD CONSTRAINT "BotPermission_toBotId_fkey" FOREIGN KEY ("toBotId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotHostHeartbeat" ADD CONSTRAINT "BotHostHeartbeat_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotToolGrant" ADD CONSTRAINT "BotToolGrant_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotTurn" ADD CONSTRAINT "BotTurn_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotTurn" ADD CONSTRAINT "BotTurn_ownerId_fkey" FOREIGN KEY ("ownerId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "BotTurn" ADD CONSTRAINT "BotTurn_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "ConversationSegment" ADD CONSTRAINT "ConversationSegment_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE;

ALTER TABLE "ConversationSegment" ADD CONSTRAINT "ConversationSegment_authorId_fkey" FOREIGN KEY ("authorId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
