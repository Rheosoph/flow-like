-- Skills a package version ships for bots (PackageSkillEntry[] with embedded instructions), reviewed with the version.
ALTER TABLE "WasmPackageVersion" ADD COLUMN "skills" JSONB;
