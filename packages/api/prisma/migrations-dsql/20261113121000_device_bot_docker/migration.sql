-- Docker readiness is reported by the authenticated device agent.
ALTER TABLE "ManagedDevice" ADD COLUMN "botDocker" JSONB;
