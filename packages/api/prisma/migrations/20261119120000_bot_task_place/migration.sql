-- Place tasks: the hub fires them when the owner's phone reports crossing a circle. NULL on time tasks.
ALTER TABLE "BotTask" ADD COLUMN "placeLatitude" DOUBLE PRECISION;
ALTER TABLE "BotTask" ADD COLUMN "placeLongitude" DOUBLE PRECISION;
ALTER TABLE "BotTask" ADD COLUMN "placeRadiusM" DOUBLE PRECISION;
ALTER TABLE "BotTask" ADD COLUMN "placeOn" TEXT;
ALTER TABLE "BotTask" ADD COLUMN "placeLabel" TEXT;
ALTER TABLE "BotTask" ADD COLUMN "placeRepeat" BOOLEAN DEFAULT false;
ALTER TABLE "BotTask" ADD COLUMN "placeUntil" TIMESTAMPTZ(3);

CREATE INDEX "BotTask_ownerId_kind_enabled_idx" ON "BotTask"("ownerId", "kind", "enabled");
