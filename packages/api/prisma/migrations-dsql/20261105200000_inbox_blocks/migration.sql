-- Aurora DSQL migration "inbox_blocks"
-- Apply before deploying the API that reads inbox blocks.
-- tables=1 indexes=1 foreign_keys=0 validations=0 statements=2
CREATE TABLE "InboxBlock" (
    "ownerId" TEXT NOT NULL,
    "principalId" TEXT NOT NULL,
    "principalKind" TEXT NOT NULL,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "InboxBlock_pkey" PRIMARY KEY ("ownerId", "principalId")
);

CREATE INDEX ASYNC "InboxBlock_principalId_idx" ON "InboxBlock"("principalId");

