CREATE TABLE "BotDecisionWait" (
    "botId" TEXT NOT NULL,
    "conversationId" TEXT NOT NULL,
    "participantId" TEXT NOT NULL,
    "triggerMessageId" TEXT NOT NULL,
    "afterSeq" BIGINT NOT NULL,
    "expiresAt" TIMESTAMPTZ(3) NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "BotDecisionWait_pkey" PRIMARY KEY ("botId", "conversationId")
);

CREATE INDEX ASYNC "BotDecisionWait_conversationId_participantId_idx" ON "BotDecisionWait"("conversationId", "participantId");
CREATE INDEX ASYNC "BotDecisionWait_expiresAt_idx" ON "BotDecisionWait"("expiresAt");

ALTER TABLE "BotDecisionWait" ADD CONSTRAINT "BotDecisionWait_botId_fkey" FOREIGN KEY ("botId") REFERENCES "Bot"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE "BotDecisionWait" ADD CONSTRAINT "BotDecisionWait_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;

ALTER TABLE ASYNC "BotDecisionWait" VALIDATE CONSTRAINT "BotDecisionWait_botId_fkey";
ALTER TABLE ASYNC "BotDecisionWait" VALIDATE CONSTRAINT "BotDecisionWait_conversationId_fkey";
