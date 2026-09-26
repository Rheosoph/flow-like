-- Aurora DSQL migration "explore_layout"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one,
-- waits for the async index/validation jobs and records the migration in _prisma_migrations.
-- tables=4 indexes=2 foreign_keys=0 validations=0 statements=6
CREATE TABLE "ExploreLayout" (
    "edition" TEXT NOT NULL,
    "revision" TEXT NOT NULL,
    "publishedAt" TIMESTAMPTZ(3),
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "ExploreLayout_pkey" PRIMARY KEY ("edition")
);

CREATE TABLE "ExploreSlot" (
    "edition" TEXT NOT NULL,
    "key" TEXT NOT NULL,
    "area" TEXT NOT NULL,
    "position" INTEGER NOT NULL,

    CONSTRAINT "ExploreSlot_pkey" PRIMARY KEY ("edition", "key")
);

CREATE TABLE "ExplorePlacement" (
    "edition" TEXT NOT NULL,
    "id" TEXT NOT NULL,
    "slotKey" TEXT NOT NULL,
    "position" INTEGER NOT NULL,
    "kind" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "enabled" BOOLEAN NOT NULL DEFAULT false,
    "startsAt" TIMESTAMPTZ(3),
    "endsAt" TIMESTAMPTZ(3),
    "audience" JSONB NOT NULL DEFAULT '[]',
    "content" JSONB NOT NULL DEFAULT '{}',
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "ExplorePlacement_pkey" PRIMARY KEY ("edition", "id")
);

CREATE TABLE "ExplorePlacementItem" (
    "edition" TEXT NOT NULL,
    "placementId" TEXT NOT NULL,
    "position" INTEGER NOT NULL,
    "itemKind" TEXT NOT NULL,
    "itemId" TEXT NOT NULL,
    "overrides" JSONB NOT NULL DEFAULT '{}',

    CONSTRAINT "ExplorePlacementItem_pkey" PRIMARY KEY ("edition", "placementId", "position")
);

CREATE INDEX ASYNC "ExplorePlacement_edition_slotKey_position_idx" ON "ExplorePlacement"("edition", "slotKey", "position");

CREATE INDEX ASYNC "ExplorePlacementItem_edition_itemKind_itemId_idx" ON "ExplorePlacementItem"("edition", "itemKind", "itemId");
