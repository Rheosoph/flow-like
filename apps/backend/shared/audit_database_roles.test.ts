import { describe, expect, spyOn, test } from "bun:test";
import { Client } from "pg";
import { provisionAuditDatabaseRoles } from "./audit_database_roles";

const GRANTS_ONLY_ENV = {
	AUDIT_DB_GRANTS_ONLY: "true",
	API_DATABASE_ROLE: "flowlike_api",
	AUDIT_DATABASE_ROLE: "flowlike_audit_worker",
};
const SCHEMA_REVOKE = "REVOKE ALL ON SCHEMA public FROM PUBLIC";
const DATABASE_REVOKE =
	'REVOKE CREATE ON DATABASE "flow_like" FROM PUBLIC, "flowlike_api", "flowlike_audit_worker"';

interface FakeCatalog {
	ownsSchema: boolean;
	ownsDatabase: boolean;
	/** Answer for every has_*_privilege probe of the verification step. */
	privileged?: boolean;
}

// A PostgreSQL catalog as the grant-only branch reads it: the migration
// identity is `migration`, both runtime roles exist with no memberships and no
// owned objects, and every audit table is present. DCL is recorded, not run.
function fakeClient(catalog: FakeCatalog) {
	const statements: string[] = [];
	const tables = [
		"AuditEntry",
		"AuditRecord",
		"AuditSeal",
		"AuditEpoch",
		"AuditWatermark",
		"AuditArchive",
		"AuditHeldChain",
		"AuditWorkerLease",
		"AuditExportTarget",
		"AiActAssessment",
		"App",
	];
	const roles = new Set([
		GRANTS_ONLY_ENV.API_DATABASE_ROLE,
		GRANTS_ONLY_ENV.AUDIT_DATABASE_ROLE,
	]);
	async function query(text: string, values: unknown[] = []) {
		statements.push(text);
		if (text.startsWith("SELECT current_user"))
			return { rows: [{ owner: "migration", database: "flow_like" }] };
		if (text.startsWith("SELECT version()"))
			return { rows: [{ version: "PostgreSQL 16.4" }] };
		if (text.includes("FROM pg_catalog.pg_tables"))
			return { rows: tables.map((tablename) => ({ tablename })) };
		if (text.includes("WHERE n.nspname = 'public'"))
			return { rows: [{ owns: catalog.ownsSchema }] };
		if (text.includes("WHERE d.datname = current_database()"))
			return { rows: [{ owns: catalog.ownsDatabase }] };
		if (text.startsWith("SELECT * FROM pg_catalog.pg_roles")) {
			return {
				rows: roles.has(String(values[0]))
					? [
							{
								rolname: values[0],
								rolsuper: false,
								rolcreaterole: false,
								rolcreatedb: false,
								rolreplication: false,
								rolbypassrls: false,
							},
						]
					: [],
			};
		}
		if (text.includes("information_schema.columns")) {
			return {
				rows: text.includes("column_name <> 'sealId'")
					? [{ column_name: "id" }]
					: [{ column_name: "id" }, { column_name: "sealId" }],
			};
		}
		if (text.includes("AS allowed"))
			return { rows: [{ allowed: catalog.privileged === true }] };
		return { rows: [] };
	}
	return { client: { query } as unknown as Client, statements };
}

function capturedWarnings(): { lines: string[]; restore: () => void } {
	const lines: string[] = [];
	const spy = spyOn(console, "warn").mockImplementation(
		(...args: unknown[]) => {
			lines.push(String(args[0]));
		},
	);
	return { lines, restore: () => spy.mockRestore() };
}

describe("grant-only mode without ownership of public or the database", () => {
	test("skips the two revokes it cannot perform, says so, and keeps the table boundary and its verification", async () => {
		const { client, statements } = fakeClient({
			ownsSchema: false,
			ownsDatabase: false,
		});
		const warnings = capturedWarnings();
		try {
			await provisionAuditDatabaseRoles(client, GRANTS_ONLY_ENV);
		} finally {
			warnings.restore();
		}
		expect(statements).not.toContain(SCHEMA_REVOKE);
		expect(statements).not.toContain(DATABASE_REVOKE);
		expect(warnings.lines).toEqual([
			'audit role provisioning: skipped REVOKE ALL ON SCHEMA public FROM PUBLIC because "migration" does not own schema public',
			'audit role provisioning: skipped REVOKE CREATE ON DATABASE "flow_like" because "migration" does not own the database',
		]);
		expect(statements).toContain(
			'REVOKE ALL ON TABLE public."AuditRecord" FROM PUBLIC',
		);
		expect(statements).toContain(
			'REVOKE ALL ON TABLE public."AuditSeal" FROM "flowlike_api"',
		);
		expect(statements).toContain(
			'GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE public."AuditSeal" TO "flowlike_audit_worker"',
		);
		expect(statements).toContain(
			'GRANT INSERT ("id") ON TABLE public."AuditRecord" TO "flowlike_api"',
		);
		expect(
			statements.filter((sql) =>
				sql.includes("has_table_privilege($1, $2, $3)"),
			).length,
		).toBe(8);
		expect(statements).toContain("COMMIT");
		expect(statements).not.toContain("ROLLBACK");
	});

	test("still fails when a role holds audit write privileges it cannot revoke", async () => {
		const { client, statements } = fakeClient({
			ownsSchema: false,
			ownsDatabase: false,
			privileged: true,
		});
		const warnings = capturedWarnings();
		try {
			await expect(
				provisionAuditDatabaseRoles(client, GRANTS_ONLY_ENV),
			).rejects.toThrow("defeat audit role separation");
		} finally {
			warnings.restore();
		}
		expect(statements).toContain("ROLLBACK");
		expect(statements).not.toContain("COMMIT");
	});

	test("an owner revokes as before without a warning", async () => {
		const { client, statements } = fakeClient({
			ownsSchema: true,
			ownsDatabase: true,
		});
		const warnings = capturedWarnings();
		try {
			await provisionAuditDatabaseRoles(client, GRANTS_ONLY_ENV);
		} finally {
			warnings.restore();
		}
		expect(statements).toContain(SCHEMA_REVOKE);
		expect(statements).toContain(DATABASE_REVOKE);
		expect(warnings.lines).toEqual([]);
		expect(statements).toContain("COMMIT");
	});
});

// Use a disposable PostgreSQL database. This test creates its own schema tables and login roles.
const adminUrl = process.env.AUDIT_ROLE_TEST_DATABASE_URL;
test.skipIf(!adminUrl)(
	"API can append pending records but cannot alter sealed evidence or worker state",
	async () => {
		const admin = new Client({ connectionString: adminUrl });
		await admin.connect();
		const suffix = `${Date.now()}_${process.pid}`;
		const apiUrl = new URL(adminUrl!);
		apiUrl.username = `api_${suffix}`;
		apiUrl.password = "api-test-password";
		const workerUrl = new URL(adminUrl!);
		workerUrl.username = `worker_${suffix}`;
		workerUrl.password = "worker-test-password";
		const names = [
			"AuditEntry",
			"AuditRecord",
			"AuditSeal",
			"AuditEpoch",
			"AuditWatermark",
			"AuditArchive",
			"AuditHeldChain",
			"AuditWorkerLease",
			"AuditExportTarget",
			"AiActAssessment",
			"ApplicationData",
		];
		const api = new Client({ connectionString: apiUrl.toString() });
		const worker = new Client({ connectionString: workerUrl.toString() });
		try {
			for (const name of names)
				await admin.query(
					`CREATE TABLE "${name}" (id text PRIMARY KEY, "sealId" text, payload text)`,
				);
			const env = {
				DATABASE_URL: adminUrl,
				API_DATABASE_URL: apiUrl.toString(),
				AUDIT_DATABASE_URL: workerUrl.toString(),
			};
			await provisionAuditDatabaseRoles(admin, env);
			await provisionAuditDatabaseRoles(admin, env);
			await api.connect();
			await worker.connect();
			await api.query(
				'INSERT INTO "AuditRecord" (id, payload) VALUES ($1, $2)',
				["pending", "original"],
			);
			await expect(
				api.query('INSERT INTO "AuditRecord" (id, "sealId") VALUES ($1, $2)', [
					"forged",
					"fake-seal",
				]),
			).rejects.toMatchObject({ code: "42501" });
			await expect(
				api.query('UPDATE "AuditRecord" SET payload = $1', ["changed"]),
			).rejects.toMatchObject({ code: "42501" });
			await expect(
				api.query('DELETE FROM "AuditRecord"'),
			).rejects.toMatchObject({ code: "42501" });
			for (const name of [
				"AuditSeal",
				"AuditEpoch",
				"AuditWatermark",
				"AuditArchive",
				"AuditHeldChain",
				"AuditWorkerLease",
			]) {
				await expect(
					api.query(`INSERT INTO "${name}" (id) VALUES ('forged')`),
				).rejects.toMatchObject({ code: "42501" });
				await expect(
					api.query(`UPDATE "${name}" SET payload = 'changed'`),
				).rejects.toMatchObject({ code: "42501" });
				await expect(api.query(`DELETE FROM "${name}"`)).rejects.toMatchObject({
					code: "42501",
				});
			}
			await expect(
				api.query('ALTER TABLE "AuditEpoch" ADD COLUMN attacker text'),
			).rejects.toMatchObject({ code: "42501" });
			await expect(
				api.query(
					`SET ROLE "${decodeURIComponent(new URL(adminUrl!).username)}"`,
				),
			).rejects.toMatchObject({ code: "42501" });
			await worker.query(
				'UPDATE "AuditRecord" SET "sealId" = $1 WHERE id = $2',
				["sealed", "pending"],
			);
			await worker.query('INSERT INTO "AuditEpoch" (id) VALUES ($1)', [
				"epoch-1",
			]);
			await worker.query('INSERT INTO "AuditWorkerLease" (id) VALUES ($1)', [
				"worker",
			]);
			expect(
				(
					await api.query('SELECT "sealId" FROM "AuditRecord" WHERE id = $1', [
						"pending",
					])
				).rows[0].sealId,
			).toBe("sealed");
			await api.query('INSERT INTO "ApplicationData" (id) VALUES ($1)', [
				"app",
			]);
			await expect(
				worker.query('UPDATE "ApplicationData" SET payload = $1', ["changed"]),
			).rejects.toMatchObject({ code: "42501" });
			const inheritedRole = `parent_${suffix}`;
			await admin.query(`CREATE ROLE "${inheritedRole}"`);
			try {
				await admin.query(`GRANT "${inheritedRole}" TO "${apiUrl.username}"`);
				await expect(provisionAuditDatabaseRoles(admin, env)).rejects.toThrow(
					"inherit other roles",
				);
				await admin.query(
					`REVOKE "${inheritedRole}" FROM "${apiUrl.username}"`,
				);
			} finally {
				await admin.query(`DROP ROLE "${inheritedRole}"`);
			}
			const passwordBefore = (
				await admin.query(
					"SELECT rolpassword FROM pg_authid WHERE rolname = $1",
					[apiUrl.username],
				)
			).rows[0].rolpassword;
			await provisionAuditDatabaseRoles(admin, {
				AUDIT_DB_GRANTS_ONLY: "true",
				API_DATABASE_ROLE: apiUrl.username,
				AUDIT_DATABASE_ROLE: workerUrl.username,
			});
			expect(
				(
					await admin.query(
						"SELECT rolpassword FROM pg_authid WHERE rolname = $1",
						[apiUrl.username],
					)
				).rows[0].rolpassword,
			).toBe(passwordBefore);
			const grantOnly = {
				AUDIT_DB_GRANTS_ONLY: "true",
				API_DATABASE_ROLE: apiUrl.username,
				AUDIT_DATABASE_ROLE: workerUrl.username,
			};
			await admin.query("CREATE ROLE cloudsqliamserviceaccount");
			try {
				await admin.query(
					`GRANT cloudsqliamserviceaccount TO "${apiUrl.username}"`,
				);
				await admin.query(
					`GRANT cloudsqliamserviceaccount TO "${workerUrl.username}"`,
				);
				await provisionAuditDatabaseRoles(admin, grantOnly);
				await worker.query("SET ROLE cloudsqliamserviceaccount");
				await expect(
					worker.query('INSERT INTO public."AuditEpoch" (id) VALUES ($1)', [
						"iam-marker-forgery",
					]),
				).rejects.toMatchObject({ code: "42501" });
				await worker.query("RESET ROLE");
				await expect(provisionAuditDatabaseRoles(admin, env)).rejects.toThrow(
					"inherit other roles",
				);
				// A marker held only by the worker must receive the same privilege checks.
				await admin.query(
					`REVOKE cloudsqliamserviceaccount FROM "${apiUrl.username}"`,
				);
				await provisionAuditDatabaseRoles(admin, grantOnly);
				// NOINHERIT does not prevent SET ROLE. Inspect the marker itself too.
				await admin.query(
					'GRANT UPDATE ON "AuditEpoch" TO cloudsqliamserviceaccount',
				);
				await expect(
					provisionAuditDatabaseRoles(admin, grantOnly),
				).rejects.toThrow("Inherited audit write privileges");
				await admin.query(
					'REVOKE UPDATE ON "AuditEpoch" FROM cloudsqliamserviceaccount',
				);
				await admin.query(
					"GRANT CREATE ON SCHEMA public TO cloudsqliamserviceaccount",
				);
				await expect(
					provisionAuditDatabaseRoles(admin, grantOnly),
				).rejects.toThrow("Inherited database ownership or DDL privileges");
			} finally {
				await admin.query(
					`REVOKE cloudsqliamserviceaccount FROM "${apiUrl.username}"`,
				);
				await admin.query(
					`REVOKE cloudsqliamserviceaccount FROM "${workerUrl.username}"`,
				);
				await admin.query("DROP OWNED BY cloudsqliamserviceaccount");
				await admin.query("DROP ROLE cloudsqliamserviceaccount");
			}
			// A stale column grant must not survive a later reconciliation.
			await admin.query(
				`GRANT INSERT ("sealId") ON "AuditRecord" TO "${apiUrl.username}"`,
			);
			await provisionAuditDatabaseRoles(admin, env);
			await expect(
				api.query('INSERT INTO "AuditRecord" (id, "sealId") VALUES ($1, $2)', [
					"stale",
					"fake-seal",
				]),
			).rejects.toMatchObject({ code: "42501" });
		} finally {
			await api.end();
			await worker.end();
			for (const name of names.reverse())
				await admin.query(`DROP TABLE IF EXISTS "${name}"`);
			for (const role of [apiUrl.username, workerUrl.username]) {
				await admin.query(`DROP OWNED BY "${role}"`);
				await admin.query(`DROP ROLE "${role}"`);
			}
			await admin.end();
		}
	},
	30000,
);
