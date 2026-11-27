-- Bot team threads: a person's thread with a team, where their #Team requests go.
CREATE TABLE "BotTeamThread" (
    "teamId" TEXT NOT NULL,
    "userId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotTeamThread_pkey" PRIMARY KEY ("teamId", "userId")
);

CREATE UNIQUE INDEX ASYNC "BotTeamThread_conversationId_key" ON "BotTeamThread"("conversationId");

CREATE INDEX ASYNC "BotTeamThread_userId_idx" ON "BotTeamThread"("userId");

ALTER TABLE "BotTeamThread" ADD CONSTRAINT "BotTeamThread_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamThread" ADD CONSTRAINT "BotTeamThread_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamThread" ADD CONSTRAINT "BotTeamThread_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotTeamThread" VALIDATE CONSTRAINT "BotTeamThread_teamId_fkey";
ALTER TABLE ASYNC "BotTeamThread" VALIDATE CONSTRAINT "BotTeamThread_userId_fkey";
ALTER TABLE ASYNC "BotTeamThread" VALIDATE CONSTRAINT "BotTeamThread_conversationId_fkey";
