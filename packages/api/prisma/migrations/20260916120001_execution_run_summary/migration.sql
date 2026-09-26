ALTER TABLE "ExecutionRun" ADD COLUMN "eventVersion" TEXT;
ALTER TABLE "ExecutionRun" ADD COLUMN "nodes" JSONB;
ALTER TABLE "ExecutionRun" ADD COLUMN "logsCount" BIGINT;
