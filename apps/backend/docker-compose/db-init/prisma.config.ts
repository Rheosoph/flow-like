import { defineConfig } from "prisma/config";
export default defineConfig({
  schema: "prisma-postgres-mirror/schema",
  datasource: { url: process.env.DATABASE_URL ?? "" },
});
