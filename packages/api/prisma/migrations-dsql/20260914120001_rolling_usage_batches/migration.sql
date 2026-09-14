CREATE INDEX ASYNC "AppRollingUsage_sweptAt_id_idx" ON "AppRollingUsage"("sweptAt", id);
DROP INDEX "AppRollingUsage_sweptAt_idx";
CREATE INDEX ASYNC "AppRollingContribution_counterId_expiresAt_id_idx" ON "AppRollingContribution"("counterId", "expiresAt", id);
DROP INDEX "AppRollingContribution_counterId_expiresAt_idx";
