import { Client } from "pg";

const evidenceTables = [
  "AuditEntry", "AuditRecord", "AuditSeal", "AuditEpoch", "AuditWatermark",
  "AuditArchive", "AuditHeldChain", "AuditWorkerLease",
];
const identifier = (value: string) => `"${value.replaceAll('"', '""')}"`;
const literal = (value: string) => `'${value.replaceAll("'", "''")}'`;
const table = (name: string) => `public.${identifier(name)}`;

function login(url: string | undefined, name: string) {
  if (!url) throw new Error(`${name} is required for database role separation`);
  const parsed = new URL(url);
  const user = decodeURIComponent(parsed.username);
  const password = decodeURIComponent(parsed.password);
  if (!["postgres:", "postgresql:"].includes(parsed.protocol) || !user || !password) {
    throw new Error(`${name} must use a dedicated PostgreSQL login and password`);
  }
  if (parsed.searchParams.has("schema") && parsed.searchParams.get("schema") !== "public") {
    throw new Error("Audit role provisioning requires the public schema");
  }
  if (/\\|\u0000/.test(password) || /\u0000/.test(user)) {
    throw new Error("Database login contains unsupported characters");
  }
  return { user, password, database: parsed.pathname, host: parsed.hostname, port: parsed.port || "5432" };
}

/** Apply the privilege boundary after schema migration, using the schema owner's connection. */
export async function provisionAuditDatabaseRoles(client: Client, env = process.env) {
  const grantsOnly = env.AUDIT_DB_GRANTS_ONLY === "true";
  const session = await client.query("SELECT current_user AS owner, current_database() AS database");
  const owner = grantsOnly
    ? { user: session.rows[0].owner, database: "/" + session.rows[0].database, password: "", host: "", port: "" }
    : login(env.DATABASE_URL, "DATABASE_URL");
  function roleName(name: string | undefined, variable: string) {
    if (!name || /\u0000/.test(name)) throw new Error(`${variable} is required in grant-only mode`);
    return { user: name, password: "", database: owner.database, host: owner.host, port: owner.port };
  }
  const api = grantsOnly ? roleName(env.API_DATABASE_ROLE, "API_DATABASE_ROLE") : login(env.API_DATABASE_URL, "API_DATABASE_URL");
  const worker = grantsOnly ? roleName(env.AUDIT_DATABASE_ROLE, "AUDIT_DATABASE_ROLE") : login(env.AUDIT_DATABASE_URL, "AUDIT_DATABASE_URL");
  if (new Set([owner.user, api.user, worker.user]).size !== 3 || (!grantsOnly && new Set([owner.password, api.password, worker.password]).size !== 3)) {
    throw new Error("Migration, API and audit worker require distinct database users and passwords");
  }
  if ([api, worker].some(role => role.database !== owner.database || role.host !== owner.host || role.port !== owner.port)) {
    throw new Error("Migration, API and audit worker URLs must target the same database endpoint");
  }
  const version = await client.query("SELECT version() AS version");
  const cockroach = String(version.rows[0].version).includes("CockroachDB");
  const tables = await client.query("SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public'");
  const available = new Set<string>(tables.rows.map(row => row.tablename));
  const authenticationRoles = new Set<string>();
  for (const name of [...evidenceTables, "AuditExportTarget", "AiActAssessment"]) {
    if (!available.has(name)) throw new Error(`Apply the audit schema before granting roles: missing ${name}`);
  }
  for (const role of [api, worker]) {
    const existing = await client.query("SELECT * FROM pg_catalog.pg_roles WHERE rolname = $1", [role.user]);
    if (existing.rows.length) {
      const flags = existing.rows[0];
      if (["rolsuper", "rolcreaterole", "rolcreatedb", "rolreplication", "rolbypassrls"].some(key => flags[key] === true)) {
        throw new Error("API and worker database roles must not have administrative privileges");
      }
      const memberships = await client.query(`WITH RECURSIVE inherited(oid) AS (
        SELECT m.roleid FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles r ON r.oid = m.member WHERE r.rolname = $1
        UNION SELECT m.roleid FROM pg_catalog.pg_auth_members m JOIN inherited i ON i.oid = m.member
      ) SELECT r.* FROM inherited i JOIN pg_catalog.pg_roles r ON r.oid = i.oid`, [role.user]);
      // Cloud SQL uses these non-group system roles to recognize IAM logins.
      // Inspect their effective rights below; their names alone do not establish isolation.
      const allowedMemberships = grantsOnly && !cockroach
        && memberships.rows.every(parent => ["cloudsqliamserviceaccount", "cloudsqliamuser"].includes(parent.rolname)
          && !["rolsuper", "rolcreaterole", "rolcreatedb", "rolreplication", "rolbypassrls"].some(key => parent[key] === true));
      if (allowedMemberships) for (const parent of memberships.rows) authenticationRoles.add(parent.rolname);
      const owned = await client.query("SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_roles r ON r.oid = c.relowner WHERE r.rolname = $1 LIMIT 1", [role.user]);
      const schemas = await client.query("SELECT 1 FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles r ON r.oid = n.nspowner WHERE r.rolname = $1 LIMIT 1", [role.user]);
      const databases = await client.query("SELECT 1 FROM pg_catalog.pg_database d JOIN pg_catalog.pg_roles r ON r.oid = d.datdba WHERE r.rolname = $1 LIMIT 1", [role.user]);
      if ((memberships.rows.length && !allowedMemberships) || owned.rows.length || schemas.rows.length || databases.rows.length) {
        throw new Error("API and worker roles must not own database objects or inherit other roles");
      }
    } else {
      if (grantsOnly) throw new Error("Grant-only mode requires existing API and audit worker database roles");
      await client.query(`CREATE ROLE ${identifier(role.user)} WITH LOGIN PASSWORD ${literal(role.password)}${cockroach ? "" : " NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS"}`);
    }
    if (!grantsOnly) await client.query(`ALTER ROLE ${identifier(role.user)} WITH LOGIN PASSWORD ${literal(role.password)}`);
  }

  await client.query("BEGIN");
  try {
    await client.query("REVOKE ALL ON SCHEMA public FROM PUBLIC");
    for (const role of [api, worker]) {
      const name = identifier(role.user);
      await client.query(`REVOKE ALL ON SCHEMA public FROM ${name}`);
      await client.query(`GRANT USAGE ON SCHEMA public TO ${name}`);
      for (const target of available) await client.query(`REVOKE ALL ON TABLE ${table(target)} FROM ${name}`);
    }
    for (const target of evidenceTables) await client.query(`REVOKE ALL ON TABLE ${table(target)} FROM PUBLIC`);
    for (const target of available) {
      if (!evidenceTables.includes(target) && !target.startsWith("_prisma")) {
        await client.query(`GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE ${table(target)} TO ${identifier(api.user)}`);
      }
    }
    for (const target of evidenceTables) {
      if (target !== "AuditWorkerLease") await client.query(`GRANT SELECT ON TABLE ${table(target)} TO ${identifier(api.user)}`);
      await client.query(`GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE ${table(target)} TO ${identifier(worker.user)}`);
    }
    // PostgreSQL column grants survive a table-level REVOKE, so clear them explicitly.
    if (!cockroach) {
      for (const target of evidenceTables) {
        const columns = await client.query("SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position", [target]);
        const names = columns.rows.map(row => identifier(row.column_name)).join(", ");
        await client.query(`REVOKE INSERT (${names}), UPDATE (${names}), REFERENCES (${names}) ON TABLE ${table(target)} FROM PUBLIC, ${identifier(api.user)}`);
      }
    }
    // CockroachDB 24.2 grants INSERT at table scope. Existing records and all signed
    // state still remain read-only to the API; the verifier rejects invented links.
    if (cockroach) {
      await client.query(`GRANT INSERT ON TABLE ${table("AuditRecord")} TO ${identifier(api.user)}`);
    } else {
      const columns = await client.query("SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'AuditRecord' AND column_name <> 'sealId' ORDER BY ordinal_position");
      await client.query(`GRANT INSERT (${columns.rows.map(row => identifier(row.column_name)).join(", ")}) ON TABLE ${table("AuditRecord")} TO ${identifier(api.user)}`);
    }
    await client.query(`GRANT SELECT ON TABLE ${table("AiActAssessment")} TO ${identifier(worker.user)}`);
    await client.query(`GRANT SELECT, UPDATE ON TABLE ${table("AuditExportTarget")} TO ${identifier(worker.user)}`);
    if (!cockroach) {
      await client.query(`GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO ${identifier(api.user)}`);
      await client.query(`REVOKE CREATE ON DATABASE ${identifier(decodeURIComponent(owner.database.slice(1)))} FROM PUBLIC, ${identifier(api.user)}, ${identifier(worker.user)}`);
      for (const user of [api.user, worker.user, ...authenticationRoles]) {
        const create = await client.query("SELECT has_schema_privilege($1, 'public', 'CREATE') OR has_database_privilege($1, current_database(), 'CREATE') AS allowed", [user]);
        const ownership = await client.query(`SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_roles r ON r.oid = c.relowner WHERE r.rolname = $1)
          OR EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles r ON r.oid = n.nspowner WHERE r.rolname = $1)
          OR EXISTS (SELECT 1 FROM pg_catalog.pg_database d JOIN pg_catalog.pg_roles r ON r.oid = d.datdba WHERE r.rolname = $1) AS allowed`, [user]);
        if (create.rows[0].allowed || ownership.rows[0].allowed) throw new Error("Inherited database ownership or DDL privileges defeat audit role separation");
      }
      for (const user of [api.user, ...authenticationRoles]) {
        for (const target of evidenceTables) {
          const pending = user === api.user && target === "AuditRecord";
          const operations = "UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER" + (pending ? "" : ",INSERT");
          const columns = "UPDATE,REFERENCES" + (pending ? "" : ",INSERT");
          const forbidden = await client.query("SELECT has_table_privilege($1, $2, $3) OR has_any_column_privilege($1, $2, $4) AS allowed", [user, table(target), operations, columns]);
          if (forbidden.rows[0].allowed) throw new Error("Inherited audit write privileges defeat API and worker separation");
        }
      }
      const sealedInsert = await client.query("SELECT has_column_privilege($1, 'public.\"AuditRecord\"', 'sealId', 'INSERT') AS allowed", [api.user]);
      if (sealedInsert.rows[0].allowed) throw new Error("Inherited seal linkage privileges defeat API and worker separation");
    }
    await client.query("COMMIT");
  } catch (error) {
    await client.query("ROLLBACK");
    throw error;
  }
}

if (import.meta.main) {
  const client = new Client({ connectionString: process.env.DATABASE_URL });
  try {
    await client.connect();
    await provisionAuditDatabaseRoles(client);
    console.log("API and audit worker database privileges applied");
  } catch {
    // Database errors can contain credential-bearing SQL. Keep it out of deployment logs.
    console.error("Audit database role provisioning failed; verify migration ownership, distinct logins and the audit schema");
    process.exitCode = 1;
  } finally {
    await client.end();
  }
}
