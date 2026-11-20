-- CalDAV connectors: the calendar a CalDAV connector uses and the calendars its last discovery found. The password is the connector's sealed grant.
ALTER TABLE "BotConnector" ADD COLUMN "calendarUrl" TEXT;
ALTER TABLE "BotConnector" ADD COLUMN "calendars" JSONB;
