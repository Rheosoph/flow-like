-- Apps pinned to a bot team, shown in its org chart.
CREATE TABLE "BotTeamApp" (
    "teamId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "pinnedBy" TEXT NOT NULL,
    "approvedBy" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "approvedAt" TIMESTAMPTZ(3),
    CONSTRAINT "BotTeamApp_pkey" PRIMARY KEY ("teamId", "appId")
);

CREATE INDEX ASYNC "BotTeamApp_appId_idx" ON "BotTeamApp"("appId");

ALTER TABLE "BotTeamApp" ADD CONSTRAINT "BotTeamApp_teamId_fkey" FOREIGN KEY ("teamId") REFERENCES "BotTeam"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotTeamApp" ADD CONSTRAINT "BotTeamApp_appId_fkey" FOREIGN KEY ("appId") REFERENCES "App"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotTeamApp" VALIDATE CONSTRAINT "BotTeamApp_teamId_fkey";
ALTER TABLE ASYNC "BotTeamApp" VALIDATE CONSTRAINT "BotTeamApp_appId_fkey";
