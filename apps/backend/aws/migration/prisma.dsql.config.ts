import { defineConfig } from "prisma/config";

// Used only by `prisma migrate status`, the truthfulness check migrate.ts runs
// after it has applied the migrations itself. The datasource URL carries a DSQL
// admin token as its password; migrate.ts composes it and it exists only in the
// environment of the Prisma child process. The empty fallback keeps `prisma
// validate` loadable without a database at image build time.
// Both paths are the image's layout by default; the env overrides let the same
// check run from a checkout, where the schema mirror and the migrations live
// under packages/api.
export default defineConfig({
	schema: process.env.DSQL_SCHEMA_DIR ?? "prisma/schema/",
	migrations: {
		path: process.env.DSQL_MIGRATIONS_DIR ?? "prisma/migrations-dsql",
	},
	datasource: {
		url: process.env.DATABASE_URL ?? "",
	},
});
