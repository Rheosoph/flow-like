-- NULL shows nothing beyond the description and reviews; the owner opts sections in from the app's settings.
ALTER TABLE "App" ADD COLUMN "showcase" JSONB;
