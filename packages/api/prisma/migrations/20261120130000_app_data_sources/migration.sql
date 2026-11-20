-- External databases an app's queries read as `<alias>.<table>`; the credentials are stored sealed.
CREATE TABLE "AppDataSource" (
    "id" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "alias" TEXT NOT NULL,
    "driver" TEXT NOT NULL,
    "config" JSONB NOT NULL DEFAULT '{}',
    "secret" TEXT,
    "sealedBy" TEXT,
    "secretFields" JSONB NOT NULL DEFAULT '[]',
    "createdBy" TEXT NOT NULL,
    "lastCheckedAt" TIMESTAMPTZ(3),
    "lastCheckStatus" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "AppDataSource_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "AppDataSource_appId_alias_key" ON "AppDataSource"("appId", "alias");

ALTER TABLE "AppDataSource" ADD CONSTRAINT "AppDataSource_appId_fkey" FOREIGN KEY ("appId") REFERENCES "App"("id") ON DELETE CASCADE ON UPDATE CASCADE;
