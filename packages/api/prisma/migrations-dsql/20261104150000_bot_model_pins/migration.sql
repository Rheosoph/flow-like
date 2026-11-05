-- Aurora DSQL migration "bot_model_pins"
-- Every statement runs in its own transaction; apps/backend/aws/migration applies them one by one
-- and records the migration in _prisma_migrations.
-- tables=0 indexes=0 foreign_keys=0 validations=0 statements=4
-- Bot model pins: the speech-to-text, text-to-speech, image generation and vision bits a bot always
-- uses. DSQL cannot add a column with a default, so every column is nullable; null lets the host
-- choose.
-- Apply before deploying the API that reads the columns.
ALTER TABLE "Bot" ADD COLUMN "sttModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "ttsModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "imageModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "visionModel" TEXT;
