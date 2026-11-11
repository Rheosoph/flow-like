-- technicalUserId names the technical actor of a run: an API key (TechnicalUser.id) or a bot (Bot.id).
-- A bot has no TechnicalUser row, so these foreign keys refused every run a bot started.
ALTER TABLE "ExecutionRun" DROP CONSTRAINT "ExecutionRun_technicalUserId_fkey";
ALTER TABLE "ExecutionUsageTracking" DROP CONSTRAINT "ExecutionUsageTracking_technicalUserId_fkey";
ALTER TABLE "ExecutionUsageDaily" DROP CONSTRAINT "ExecutionUsageDaily_technicalUserId_fkey";
ALTER TABLE "LLMUsageTracking" DROP CONSTRAINT "LLMUsageTracking_technicalUserId_fkey";
ALTER TABLE "EmbeddingUsageTracking" DROP CONSTRAINT "EmbeddingUsageTracking_technicalUserId_fkey";
