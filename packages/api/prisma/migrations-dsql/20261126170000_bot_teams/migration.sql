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

CREATE UNIQUE INDEX ASYNC "BotTeam_conversationId_key" ON "BotTeam"("conversationId");

CREATE INDEX ASYNC "BotTeam_parentId_idx" ON "BotTeam"("parentId");

CREATE INDEX ASYNC "BotTeamPerson_userId_idx" ON "BotTeamPerson"("userId");

CREATE INDEX ASYNC "BotTeamBot_botId_idx" ON "BotTeamBot"("botId");

ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_parentId_fkey" FOREIGN KEY ("parentId") REFERENCES "BotTeam"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_leadBotId_fkey" FOREIGN KEY ("leadBotId") REFERENCES "Bot"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeam" ADD CONSTRAINT "BotTeam_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE SET NULL ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamPerson" ADD CONSTRAINT "BotTeamPerson_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamPerson" ADD CONSTRAINT "BotTeamPerson_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamBot" ADD CONSTRAINT "BotTeamBot_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamBot" ADD CONSTRAINT "BotTeamBot_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotTeam" VALIDATE CONSTRAINT "BotTeam_parentId_fkey";
ALTER TABLE ASYNC "BotTeam" VALIDATE CONSTRAINT "BotTeam_leadBotId_fkey";
ALTER TABLE ASYNC "BotTeam" VALIDATE CONSTRAINT "BotTeam_conversationId_fkey";
ALTER TABLE ASYNC "BotTeamPerson" VALIDATE CONSTRAINT "BotTeamPerson_teamId_fkey";
ALTER TABLE ASYNC "BotTeamPerson" VALIDATE CONSTRAINT "BotTeamPerson_userId_fkey";
ALTER TABLE ASYNC "BotTeamBot" VALIDATE CONSTRAINT "BotTeamBot_teamId_fkey";
ALTER TABLE ASYNC "BotTeamBot" VALIDATE CONSTRAINT "BotTeamBot_botId_fkey";
