-- Remote sinks run as the user who last saved the event instead of a stored PAT.
-- Null on existing rows: their PAT names the run identity once and is then cleared.
-- Leaving an app or deleting the account pauses the sink and clears the column in
-- the API first; the foreign key only guarantees no sink keeps a deleted user.
ALTER TABLE "EventSink" ADD COLUMN "runAsUserId" TEXT;

CREATE INDEX "EventSink_runAsUserId_idx" ON "EventSink"("runAsUserId");

ALTER TABLE "EventSink" ADD CONSTRAINT "EventSink_runAsUserId_fkey" FOREIGN KEY ("runAsUserId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE;
