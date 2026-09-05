import { Client } from "pg";
import { runPrePush } from "./prisma/pre-push";

const url = process.env.DATABASE_URL;
if (!url) throw new Error("DATABASE_URL is required");
const timeout = Number(process.env.MIGRATION_LOCK_TIMEOUT_SECONDS ?? "120");
if (!Number.isSafeInteger(timeout) || timeout < 1) throw new Error("Invalid migration lock timeout");
const deadline = Date.now() + timeout * 1000;
let client: Client;
for (;;) {
  client = new Client({ connectionString: url, connectionTimeoutMillis: 5000 });
  try { await client.connect(); break; }
  catch {
    await client.end().catch(() => {});
    if (Date.now() >= deadline) throw new Error("Database did not become available");
    await Bun.sleep(1000);
  }
}
try {
  for (;;) {
    const result = await client.query("SELECT pg_try_advisory_lock(714629015) AS acquired");
    if (result.rows[0].acquired) break;
    if (Date.now() >= deadline) throw new Error("Database migration lock timed out");
    await Bun.sleep(1000);
  }
  // The session lock remains held throughout all schema changes.
  await runPrePush(client);
  const child = Bun.spawn(["bun", "node_modules/prisma/build/index.js", "db", "push", "--schema=prisma-postgres-mirror/schema"], {
    stdout: "inherit", stderr: "inherit", env: process.env,
  });
  const code = await child.exited;
  if (code !== 0) throw new Error(`Schema update failed with exit code ${code}`);
  console.log("Database schema applied");
} finally {
  await client.end();
}
