import { expect, test } from "bun:test";
import { Client } from "pg";
import { provisionAuditDatabaseRoles } from "./audit_database_roles";

// Use a disposable PostgreSQL database. This test creates its own schema tables and login roles.
const adminUrl = process.env.AUDIT_ROLE_TEST_DATABASE_URL;
test.skipIf(!adminUrl)("API can append pending records but cannot alter sealed evidence or worker state", async () => {
  const admin = new Client({ connectionString: adminUrl });
  await admin.connect();
  const suffix = `${Date.now()}_${process.pid}`;
  const apiUrl = new URL(adminUrl!);
  apiUrl.username = `api_${suffix}`;
  apiUrl.password = "api-test-password";
  const workerUrl = new URL(adminUrl!);
  workerUrl.username = `worker_${suffix}`;
  workerUrl.password = "worker-test-password";
  const names = ["AuditEntry", "AuditRecord", "AuditSeal", "AuditEpoch", "AuditWatermark", "AuditArchive",
    "AuditHeldChain", "AuditWorkerLease", "AuditExportTarget", "AiActAssessment", "ApplicationData"];
  const api = new Client({ connectionString: apiUrl.toString() });
  const worker = new Client({ connectionString: workerUrl.toString() });
  try {
    for (const name of names) await admin.query(`CREATE TABLE "${name}" (id text PRIMARY KEY, "sealId" text, payload text)`);
    const env = { DATABASE_URL: adminUrl, API_DATABASE_URL: apiUrl.toString(), AUDIT_DATABASE_URL: workerUrl.toString() };
    await provisionAuditDatabaseRoles(admin, env);
    await provisionAuditDatabaseRoles(admin, env);
    await api.connect();
    await worker.connect();
    await api.query('INSERT INTO "AuditRecord" (id, payload) VALUES ($1, $2)', ["pending", "original"]);
    await expect(api.query('INSERT INTO "AuditRecord" (id, "sealId") VALUES ($1, $2)', ["forged", "fake-seal"])).rejects.toMatchObject({ code: "42501" });
    await expect(api.query('UPDATE "AuditRecord" SET payload = $1', ["changed"])).rejects.toMatchObject({ code: "42501" });
    await expect(api.query('DELETE FROM "AuditRecord"')).rejects.toMatchObject({ code: "42501" });
    for (const name of ["AuditSeal", "AuditEpoch", "AuditWatermark", "AuditArchive", "AuditHeldChain", "AuditWorkerLease"]) {
      await expect(api.query(`INSERT INTO "${name}" (id) VALUES ('forged')`)).rejects.toMatchObject({ code: "42501" });
      await expect(api.query(`UPDATE "${name}" SET payload = 'changed'`)).rejects.toMatchObject({ code: "42501" });
      await expect(api.query(`DELETE FROM "${name}"`)).rejects.toMatchObject({ code: "42501" });
    }
    await expect(api.query('ALTER TABLE "AuditEpoch" ADD COLUMN attacker text')).rejects.toMatchObject({ code: "42501" });
    await expect(api.query(`SET ROLE "${decodeURIComponent(new URL(adminUrl!).username)}"`)).rejects.toMatchObject({ code: "42501" });
    await worker.query('UPDATE "AuditRecord" SET "sealId" = $1 WHERE id = $2', ["sealed", "pending"]);
    await worker.query('INSERT INTO "AuditEpoch" (id) VALUES ($1)', ["epoch-1"]);
    await worker.query('INSERT INTO "AuditWorkerLease" (id) VALUES ($1)', ["worker"]);
    expect((await api.query('SELECT "sealId" FROM "AuditRecord" WHERE id = $1', ["pending"])).rows[0].sealId).toBe("sealed");
    await api.query('INSERT INTO "ApplicationData" (id) VALUES ($1)', ["app"]);
    await expect(worker.query('UPDATE "ApplicationData" SET payload = $1', ["changed"])).rejects.toMatchObject({ code: "42501" });
    const inheritedRole = `parent_${suffix}`;
    await admin.query(`CREATE ROLE "${inheritedRole}"`);
    try {
      await admin.query(`GRANT "${inheritedRole}" TO "${apiUrl.username}"`);
      await expect(provisionAuditDatabaseRoles(admin, env)).rejects.toThrow("inherit other roles");
      await admin.query(`REVOKE "${inheritedRole}" FROM "${apiUrl.username}"`);
    } finally {
      await admin.query(`DROP ROLE "${inheritedRole}"`);
    }
    const passwordBefore = (await admin.query("SELECT rolpassword FROM pg_authid WHERE rolname = $1", [apiUrl.username])).rows[0].rolpassword;
    await provisionAuditDatabaseRoles(admin, {
      AUDIT_DB_GRANTS_ONLY: "true", API_DATABASE_ROLE: apiUrl.username, AUDIT_DATABASE_ROLE: workerUrl.username,
    });
    expect((await admin.query("SELECT rolpassword FROM pg_authid WHERE rolname = $1", [apiUrl.username])).rows[0].rolpassword).toBe(passwordBefore);
    const grantOnly = { AUDIT_DB_GRANTS_ONLY: "true", API_DATABASE_ROLE: apiUrl.username, AUDIT_DATABASE_ROLE: workerUrl.username };
    await admin.query('CREATE ROLE cloudsqliamserviceaccount');
    try {
      await admin.query(`GRANT cloudsqliamserviceaccount TO "${apiUrl.username}"`);
      await admin.query(`GRANT cloudsqliamserviceaccount TO "${workerUrl.username}"`);
      await provisionAuditDatabaseRoles(admin, grantOnly);
      await worker.query('SET ROLE cloudsqliamserviceaccount');
      await expect(worker.query('INSERT INTO public."AuditEpoch" (id) VALUES ($1)', ["iam-marker-forgery"])).rejects.toMatchObject({ code: "42501" });
      await worker.query('RESET ROLE');
      await expect(provisionAuditDatabaseRoles(admin, env)).rejects.toThrow("inherit other roles");
      // A marker held only by the worker must receive the same privilege checks.
      await admin.query(`REVOKE cloudsqliamserviceaccount FROM "${apiUrl.username}"`);
      await provisionAuditDatabaseRoles(admin, grantOnly);
      // NOINHERIT does not prevent SET ROLE. Inspect the marker itself too.
      await admin.query('GRANT UPDATE ON "AuditEpoch" TO cloudsqliamserviceaccount');
      await expect(provisionAuditDatabaseRoles(admin, grantOnly)).rejects.toThrow("Inherited audit write privileges");
      await admin.query('REVOKE UPDATE ON "AuditEpoch" FROM cloudsqliamserviceaccount');
      await admin.query('GRANT CREATE ON SCHEMA public TO cloudsqliamserviceaccount');
      await expect(provisionAuditDatabaseRoles(admin, grantOnly)).rejects.toThrow("Inherited database ownership or DDL privileges");
    } finally {
      await admin.query(`REVOKE cloudsqliamserviceaccount FROM "${apiUrl.username}"`);
      await admin.query(`REVOKE cloudsqliamserviceaccount FROM "${workerUrl.username}"`);
      await admin.query('DROP OWNED BY cloudsqliamserviceaccount');
      await admin.query('DROP ROLE cloudsqliamserviceaccount');
    }
    // A stale column grant must not survive a later reconciliation.
    await admin.query(`GRANT INSERT ("sealId") ON "AuditRecord" TO "${apiUrl.username}"`);
    await provisionAuditDatabaseRoles(admin, env);
    await expect(api.query('INSERT INTO "AuditRecord" (id, "sealId") VALUES ($1, $2)', ["stale", "fake-seal"])).rejects.toMatchObject({ code: "42501" });
  } finally {
    await api.end();
    await worker.end();
    for (const name of names.reverse()) await admin.query(`DROP TABLE IF EXISTS "${name}"`);
    for (const role of [apiUrl.username, workerUrl.username]) {
      await admin.query(`DROP OWNED BY "${role}"`);
      await admin.query(`DROP ROLE "${role}"`);
    }
    await admin.end();
  }
}, 30000);
