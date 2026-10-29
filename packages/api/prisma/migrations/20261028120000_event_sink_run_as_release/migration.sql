-- When a release paused the sink because the user it ran as lost their standing in the
-- app (left, role or account changed, legacy token refused). Such a pause stops only the
-- runs that use that user; runs a Flow-Like person triggers continue. Turning the sink on
-- or off clears it, so a person's switch-off stops every run. Null on existing rows: a
-- sink that is off keeps stopping every run until someone turns it back on.
ALTER TABLE "EventSink" ADD COLUMN "runAsReleasedAt" TIMESTAMPTZ(3);
