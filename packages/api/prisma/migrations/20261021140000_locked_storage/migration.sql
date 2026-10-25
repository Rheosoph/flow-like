-- Locked storage: write-once files in the locked bucket and their immutable versions.
CREATE TABLE "LockedItem" (
    "id" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "scope" TEXT NOT NULL DEFAULT 'PROJECT',
    "ownerId" TEXT,
    "pathKey" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "folder" TEXT NOT NULL DEFAULT '',
    "contentType" TEXT,
    "latestVersion" INTEGER NOT NULL DEFAULT 0,
    "latestSize" BIGINT NOT NULL DEFAULT 0,
    "liveVersions" INTEGER NOT NULL DEFAULT 0,
    "liveSize" BIGINT NOT NULL DEFAULT 0,
    "metadata" JSONB,
    "createdBy" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL,

    CONSTRAINT "LockedItem_pkey" PRIMARY KEY ("id")
);

CREATE TABLE "LockedItemVersion" (
    "id" TEXT NOT NULL,
    "itemId" TEXT NOT NULL,
    "appId" TEXT NOT NULL,
    "version" INTEGER,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "objectKey" TEXT NOT NULL,
    "providerVersionId" TEXT,
    "fileName" TEXT NOT NULL,
    "contentType" TEXT NOT NULL,
    "size" BIGINT NOT NULL,
    "sha256" TEXT NOT NULL,
    "checksumVerified" BOOLEAN NOT NULL DEFAULT false,
    "retentionMode" TEXT NOT NULL,
    "retainUntil" TIMESTAMPTZ(3) NOT NULL,
    "enforcement" TEXT NOT NULL,
    "legalHold" BOOLEAN NOT NULL DEFAULT false,
    "comment" TEXT,
    "metadata" JSONB,
    "uploadedBy" TEXT NOT NULL,
    "uploadExpiresAt" TIMESTAMPTZ(3) NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "lockedAt" TIMESTAMPTZ(3),
    "purgedAt" TIMESTAMPTZ(3),
    "purgedBy" TEXT,

    CONSTRAINT "LockedItemVersion_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "LockedItem_appId_pathKey_key" ON "LockedItem"("appId", "pathKey");

CREATE INDEX "LockedItem_appId_scope_folder_idx" ON "LockedItem"("appId", "scope", "folder");

CREATE INDEX "LockedItem_appId_ownerId_idx" ON "LockedItem"("appId", "ownerId");

CREATE UNIQUE INDEX "LockedItemVersion_itemId_version_key" ON "LockedItemVersion"("itemId", "version");

CREATE INDEX "LockedItemVersion_itemId_status_idx" ON "LockedItemVersion"("itemId", "status");

CREATE INDEX "LockedItemVersion_status_uploadExpiresAt_idx" ON "LockedItemVersion"("status", "uploadExpiresAt");

CREATE INDEX "LockedItemVersion_appId_status_retainUntil_idx" ON "LockedItemVersion"("appId", "status", "retainUntil");
