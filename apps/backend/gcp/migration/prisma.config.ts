import { defineConfig, env } from "prisma/config";

// The only consumer of this file is `prisma db push` spawned by migrate.ts,
// which is also the only place DATABASE_URL ever exists: it is composed from
// the IAM token in memory and handed to that one child process. `env()` throws
// when the variable is absent, so a stray `bunx prisma ...` typed by hand in
// the container fails to load config instead of connecting to nothing. The
// image build sets a throwaway loopback URL for `prisma validate`, which never
// opens a connection.
export default defineConfig({
	schema: "prisma/schema/",
	datasource: {
		url: env("DATABASE_URL"),
	},
});
