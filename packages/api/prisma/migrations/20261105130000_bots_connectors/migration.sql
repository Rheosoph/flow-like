-- Bot connectors: the MCP servers and OAuth accounts a bot's owner attached. The OAuth grant is stored sealed.
CREATE TABLE "BotConnector" (
    "id" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "providerId" TEXT,
    "url" TEXT,
    "authKind" TEXT NOT NULL DEFAULT 'NONE',
    "authRef" TEXT,
    "grantEncrypted" TEXT,
    "scopes" JSONB NOT NULL DEFAULT '[]',
    "defaultPolicy" TEXT NOT NULL DEFAULT 'ask',
    "toolPolicies" JSONB NOT NULL DEFAULT '{}',
    "toolsSnapshot" JSONB NOT NULL DEFAULT '[]',
    "confirmedHashes" JSONB NOT NULL DEFAULT '{}',
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "BotConnector_pkey" PRIMARY KEY ("id")
);

CREATE INDEX "BotConnector_botId_idx" ON "BotConnector"("botId");

ALTER TABLE "BotConnector" ADD CONSTRAINT "BotConnector_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
