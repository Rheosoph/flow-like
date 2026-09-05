-- Saved home overrides belong to a user profile. Published defaults belong to the hub.
ALTER TABLE "Profile" ADD COLUMN "homeLayout" JSONB;
ALTER TABLE "Profile" ADD COLUMN "homeDefaultId" TEXT;

CREATE TABLE "HomeDefault" (
    "id" TEXT NOT NULL,
    "layout" JSONB NOT NULL,
    "revision" TEXT NOT NULL,
    CONSTRAINT "HomeDefault_pkey" PRIMARY KEY ("id")
);

-- Profiles originally installed from a template retain its ID.
UPDATE "Profile" SET "homeDefaultId" = "id"
WHERE "id" IN (SELECT "id" FROM "TemplateProfile");
