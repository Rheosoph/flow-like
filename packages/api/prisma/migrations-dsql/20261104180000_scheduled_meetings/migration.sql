CREATE TABLE "Meeting" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "conversationId" TEXT NOT NULL,
    "hostId" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "startsAt" TIMESTAMPTZ(3) NOT NULL,
    "endsAt" TIMESTAMPTZ(3) NOT NULL,
    "endedAt" TIMESTAMPTZ(3),
    "guestsAllowed" BOOLEAN NOT NULL DEFAULT true,
    "inviteNonce" TEXT NOT NULL,
    "roomEpoch" INTEGER NOT NULL DEFAULT 0,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE "MeetingParticipant" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "meetingId" TEXT NOT NULL,
    "userId" TEXT,
    "displayName" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "sessionHash" TEXT NOT NULL,
    "lastSeenAt" TIMESTAMPTZ(3) NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE "MeetingMessage" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "meetingId" TEXT NOT NULL,
    "participantId" TEXT NOT NULL,
    "displayName" TEXT NOT NULL,
    "body" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX ASYNC "Meeting_conversationId_startsAt_idx" ON "Meeting"("conversationId", "startsAt");
CREATE INDEX ASYNC "Meeting_hostId_idx" ON "Meeting"("hostId");
CREATE UNIQUE INDEX ASYNC "MeetingParticipant_meetingId_userId_key" ON "MeetingParticipant"("meetingId", "userId");
CREATE INDEX ASYNC "MeetingParticipant_meetingId_status_idx" ON "MeetingParticipant"("meetingId", "status");
CREATE INDEX ASYNC "MeetingParticipant_userId_idx" ON "MeetingParticipant"("userId");
CREATE INDEX ASYNC "MeetingMessage_meetingId_createdAt_id_idx" ON "MeetingMessage"("meetingId", "createdAt", "id");
ALTER TABLE "Meeting" ADD CONSTRAINT "Meeting_conversationId_fkey" FOREIGN KEY ("conversationId") REFERENCES "Conversation"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE ASYNC "Meeting" VALIDATE CONSTRAINT "Meeting_conversationId_fkey";
ALTER TABLE "MeetingParticipant" ADD CONSTRAINT "MeetingParticipant_meetingId_fkey" FOREIGN KEY ("meetingId") REFERENCES "Meeting"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE ASYNC "MeetingParticipant" VALIDATE CONSTRAINT "MeetingParticipant_meetingId_fkey";
ALTER TABLE "MeetingMessage" ADD CONSTRAINT "MeetingMessage_meetingId_fkey" FOREIGN KEY ("meetingId") REFERENCES "Meeting"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE ASYNC "MeetingMessage" VALIDATE CONSTRAINT "MeetingMessage_meetingId_fkey";
