-- NULL keeps every existing app open to bots; the owner switches it on in the app's settings.
ALTER TABLE "App" ADD COLUMN "refuseBots" BOOLEAN;
