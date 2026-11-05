-- Bot sub-task model: the Bit id the bot's sub-tasks run on. Nullable like its Aurora DSQL twin;
-- null uses the bot's own model.
ALTER TABLE "Bot" ADD COLUMN "subtaskModel" TEXT;
