-- Bots and conversations moved from a desktop get ids minted here; the id they had on the
-- device stays only as an import key, so a repeated move finds what the first one created.
-- Keys are scoped to the owner (bots) or the creator (conversations). Unique indexes treat
-- NULLs as distinct, so rows made in the account never collide.
ALTER TABLE "Bot" ADD COLUMN "importKey" TEXT;

ALTER TABLE "Conversation" ADD COLUMN "importKey" TEXT;

CREATE UNIQUE INDEX "Bot_ownerId_importKey_key" ON "Bot"("ownerId", "importKey");

CREATE UNIQUE INDEX "Conversation_createdBy_importKey_key" ON "Conversation"("createdBy", "importKey");
