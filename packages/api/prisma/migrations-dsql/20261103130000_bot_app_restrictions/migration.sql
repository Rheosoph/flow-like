-- What an owner lets a bot do in one attached app: a permission mask (null keeps the owner's full role) and scopes.
-- `scopes` is created with the table: DSQL cannot add a NOT NULL or defaulted column.
ALTER TABLE "BotApp" ADD COLUMN "permissionMask" BIGINT;
