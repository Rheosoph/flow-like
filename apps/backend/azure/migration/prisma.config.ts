import { defineConfig } from "prisma/config";

// The datasource URL carries a managed-identity token as its password. It is
// composed by migrate.ts and exists only in the environment of the Prisma
// child process it spawns; nothing here reads a file or a .env. The empty
// fallback keeps `prisma validate` loadable without a database (the image build
// validates the mirrored schema that way), while `db push` on an empty URL
// fails at URL parsing before any connection attempt.
export default defineConfig({
	schema: "prisma/schema/",
	datasource: {
		url: process.env.DATABASE_URL ?? "",
	},
});
