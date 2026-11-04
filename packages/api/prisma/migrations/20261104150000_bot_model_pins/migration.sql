-- Bot model pins: the speech-to-text, text-to-speech, image generation and vision bits a bot always
-- uses. Nullable like their Aurora DSQL twin; null lets the host choose.
ALTER TABLE "Bot" ADD COLUMN "sttModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "ttsModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "imageModel" TEXT;

ALTER TABLE "Bot" ADD COLUMN "visionModel" TEXT;
