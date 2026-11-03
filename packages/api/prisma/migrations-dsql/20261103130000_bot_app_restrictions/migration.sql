-- What an owner lets a bot do in one attached app: a permission mask (null keeps the owner's full role) and scopes.
ALTER TABLE "BotApp" ADD COLUMN "permissionMask" BIGINT;
ALTER TABLE "BotApp" ADD COLUMN "scopes" JSONB NOT NULL DEFAULT '{}';
