-- Bot teams: people and bots in nested teams, one lead bot per team, and a team room.
CREATE TABLE "BotTeam" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "purpose" TEXT,
    "parentId" TEXT,
    "leadBotId" TEXT,
    "directAsks" BOOLEAN NOT NULL DEFAULT false,
    "conversationId" TEXT,
    "createdBy" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotTeam_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "BotTeamPerson" (
    "teamId" TEXT NOT NULL,
    "userId" TEXT NOT NULL,
    "role" TEXT NOT NULL DEFAULT 'MEMBER',
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "invitedBy" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "acceptedAt" TIMESTAMPTZ(3),
    CONSTRAINT "BotTeamPerson_pkey" PRIMARY KEY ("teamId", "userId")
);

CREATE TABLE "BotTeamBot" (
    "teamId" TEXT NOT NULL,
    "botId" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "directAgreed" BOOLEAN NOT NULL DEFAULT false,
    "limits" JSONB,
    "invitedBy" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "acceptedAt" TIMESTAMPTZ(3),
    CONSTRAINT "BotTeamBot_pkey" PRIMARY KEY ("teamId", "botId")
);

CREATE UNIQUE INDEX "BotTeam_conversationId_key" ON "BotTeam"("conversationId");

CREATE INDEX "BotTeam_parentId_idx" ON "BotTeam"("parentId");

CREATE INDEX "BotTeamPerson_userId_idx" ON "BotTeamPerson"("userId");

CREATE INDEX "BotTeamBot_botId_idx" ON "BotTeamBot"("botId");

ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_parentId_fkey" FOREIGN KEY ("parentId") REFERENCES "BotTeam"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_leadBotId_fkey" FOREIGN KEY ("leadBotId") REFERENCES "Bot"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "BotTeamPerson" ADD CONSTRAINT "BotTeamPerson_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotTeamPerson" ADD CONSTRAINT "BotTeamPerson_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotTeamBot" ADD CONSTRAINT "BotTeamBot_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "BotTeamBot" ADD CONSTRAINT "BotTeamBot_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE;
