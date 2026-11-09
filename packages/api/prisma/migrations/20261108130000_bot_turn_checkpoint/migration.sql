CREATE TABLE "BotTurnCheckpoint" (
    "turnId" TEXT NOT NULL,
    "revision" BIGINT NOT NULL,
    "claimAttempt" INTEGER NOT NULL,
    "recoveryAttempts" INTEGER NOT NULL DEFAULT 0,
    "startedAtMs" BIGINT NOT NULL,
    "deadlineAtMs" BIGINT NOT NULL,
    "state" JSONB NOT NULL,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,
    CONSTRAINT "BotTurnCheckpoint_pkey" PRIMARY KEY ("turnId")
);

ALTER TABLE "BotTurnCheckpoint" ADD CONSTRAINT "BotTurnCheckpoint_turnId_fkey" FOREIGN KEY ("turnId") REFERENCES "BotTurn"("id") ON DELETE CASCADE ON UPDATE CASCADE;
