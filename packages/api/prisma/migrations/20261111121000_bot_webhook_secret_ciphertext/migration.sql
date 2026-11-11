-- Webhook secrets kept encrypted so the HMAC signature can use the real secret.
ALTER TABLE "BotWebhook" ADD COLUMN "secretCiphertext" TEXT;
