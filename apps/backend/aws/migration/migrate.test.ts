import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import type pg from "pg";
import {
	ACCEPTED,
	type AppliedRow,
	ConfigError,
	type Environment,
	type Executor,
	FORBIDDEN_SETTINGS,
	JobsPendingError,
	LIST_APPLIED_SQL,
	type LeaseHolder,
	type LocalMigration,
	MigrationError,
	PRISMA_MIGRATIONS_COLUMNS,
	PRISMA_MIGRATIONS_TABLE_SQL,
	RECORD_APPLIED_SQL,
	RECORD_FAILED_SQL,
	RECORD_FINISHED_SQL,
	RECORD_STARTED_SQL,
	type RunOptions,
	applyMigration,
	asyncIndexName,
	awaitingJobs,
	clientConfig,
	composeDatabaseUrl,
	drainClusterJobs,
	grantStatements,
	interruption,
	isAlreadyExistsError,
	isAsyncJobStatement,
	isCreateOrAddStatement,
	isDdlStatement,
	isTransientError,
	listLocalMigrations,
	mayOverlapAsyncJobs,
	migrationChecksum,
	parseConfig,
	pendingMigrations,
	redactDatabaseUrl,
	redactSecret,
	splitStatements,
	startedRecord,
	waitForJobs,
	withRetries,
} from "./migrate";

const ENDPOINT = "abc0def1ghi2jkl3mno4pqr5stu6.dsql.eu-west-1.on.aws";
const ROLE_ARN = "arn:aws:iam::123456789012:role/flow-like-api-runtime";
const TOKEN =
	"abc.dsql.eu-west-1.on.aws/?Action=DbConnectAdmin&X-Amz-Signature=ab%2Fcd+ef=&X-Amz-Expires=900";

function validSettings(): Environment {
	return {
		DSQL_CLUSTER_ENDPOINT: ENDPOINT,
		DSQL_RUNTIME_ROLE_ARN: ROLE_ARN,
	};
}

describe("parseConfig", () => {
	test("accepts the minimal contract and derives the region from the endpoint", () => {
		const config = parseConfig(validSettings());
		expect(config.endpoint).toBe(ENDPOINT);
		expect(config.region).toBe("eu-west-1");
		expect(config.runtimeRoleArn).toBe(ROLE_ARN);
		expect(config.runtimeDbRole).toBe("flow_like_api");
		expect(config.migrationsDir.endsWith("prisma/migrations-dsql")).toBe(true);
	});

	test("accepts a matching DSQL_REGION and rejects a mismatching one", () => {
		expect(
			parseConfig({ ...validSettings(), DSQL_REGION: "eu-west-1" }).region,
		).toBe("eu-west-1");
		expect(() =>
			parseConfig({ ...validSettings(), DSQL_REGION: "us-east-1" }),
		).toThrow(/DSQL_REGION: must match the endpoint's region eu-west-1/);
	});

	test("rejects endpoints that are not bare DSQL hostnames", () => {
		for (const endpoint of [
			"database.example.com",
			`https://${ENDPOINT}`,
			`${ENDPOINT}:5432`,
			"abc.dsql.eu-west-1.on.aws/postgres",
			"ABC.dsql.eu-west-1.on.aws",
			"abc.rds.eu-west-1.amazonaws.com",
			"abc.dsql.on.aws",
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), DSQL_CLUSTER_ENDPOINT: endpoint }),
			).toThrow(ConfigError);
		}
	});

	test("rejects every forbidden setting even when empty", () => {
		expect(FORBIDDEN_SETTINGS).toContain("DATABASE_URL");
		expect(FORBIDDEN_SETTINGS).toContain("PGPASSWORD");
		for (const forbidden of FORBIDDEN_SETTINGS) {
			expect(() =>
				parseConfig({ ...validSettings(), [forbidden]: "" }),
			).toThrow(new RegExp(`^${forbidden} is forbidden`));
		}
	});

	test("requires the endpoint; the runtime role ARN is optional but validated when set", () => {
		expect(() => parseConfig({ DSQL_RUNTIME_ROLE_ARN: ROLE_ARN })).toThrow(
			/DSQL_CLUSTER_ENDPOINT/,
		);
		expect(
			parseConfig({ DSQL_CLUSTER_ENDPOINT: ENDPOINT }).runtimeRoleArn,
		).toBeNull();
		for (const arn of [
			"",
			" ",
			"arn:aws:iam::123:user/x",
			`${ROLE_ARN}'; DROP ROLE admin`,
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), DSQL_RUNTIME_ROLE_ARN: arn }),
			).toThrow(ConfigError);
		}
		expect(() =>
			parseConfig({
				...validSettings(),
				DSQL_CLUSTER_ENDPOINT: ` ${ENDPOINT}`,
			}),
		).toThrow(ConfigError);
	});

	test("validates the runtime database role name", () => {
		expect(
			parseConfig({ ...validSettings(), DSQL_RUNTIME_DB_ROLE: "api_ro" })
				.runtimeDbRole,
		).toBe("api_ro");
		for (const role of [
			"admin",
			"Flow-Like",
			"role name",
			"1abc",
			'x"; DROP',
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), DSQL_RUNTIME_DB_ROLE: role }),
			).toThrow(ConfigError);
		}
	});

	test("resolves a custom migrations directory relative to the job", () => {
		const config = parseConfig({
			...validSettings(),
			DSQL_MIGRATIONS_DIR: "../../../../packages/api/prisma/migrations-dsql",
		});
		expect(config.migrationsDir).toBe(
			resolve(
				import.meta.dir,
				"../../../../packages/api/prisma/migrations-dsql",
			),
		);
	});

	test("job wait budget defaults to two hours and must be a bounded integer", () => {
		expect(parseConfig(validSettings()).jobWaitTimeoutMs).toBe(7_200_000);
		expect(
			parseConfig({ ...validSettings(), DSQL_JOB_WAIT_TIMEOUT_SECS: "600" })
				.jobWaitTimeoutMs,
		).toBe(600_000);
		for (const value of ["0", "59", "86401", "abc", "1.5", "-1", "1e3"]) {
			expect(() =>
				parseConfig({ ...validSettings(), DSQL_JOB_WAIT_TIMEOUT_SECS: value }),
			).toThrow(/DSQL_JOB_WAIT_TIMEOUT_SECS/);
		}
	});
});

describe("connection composition", () => {
	test("composeDatabaseUrl encodes the token and uses quaint's verify-full spelling", () => {
		const config = parseConfig(validSettings());
		const raw = composeDatabaseUrl(config, TOKEN);
		const url = new URL(raw);
		expect(url.protocol).toBe("postgresql:");
		expect(url.username).toBe("admin");
		expect(decodeURIComponent(url.password)).toBe(TOKEN);
		expect(url.host).toBe(`${ENDPOINT}:5432`);
		expect(url.pathname).toBe("/postgres");
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("strict");
		expect(url.searchParams.get("application_name")).toBe(
			"flow-like-aws-migration",
		);
		expect(raw).not.toContain(TOKEN);
	});

	test("redaction hides the token in URLs and free text", () => {
		const config = parseConfig(validSettings());
		const url = composeDatabaseUrl(config, TOKEN);
		const redacted = redactDatabaseUrl(url);
		expect(redacted).toBe(
			`postgresql://admin:***@${ENDPOINT}:5432/postgres?${url.split("?")[1]}`,
		);
		expect(redacted).not.toContain(encodeURIComponent(TOKEN));
		expect(redactSecret(`password=${TOKEN} and again ${TOKEN}`, TOKEN)).toBe(
			"password=*** and again ***",
		);
		expect(redactSecret("nothing", "")).toBe("nothing");
	});

	test("clientConfig verifies certificates and never disables TLS", () => {
		const config = parseConfig(validSettings());
		const client = clientConfig(config, TOKEN);
		expect(client.host).toBe(ENDPOINT);
		expect(client.port).toBe(5432);
		expect(client.user).toBe("admin");
		expect(client.database).toBe("postgres");
		expect(client.password).toBe(TOKEN);
		expect(client.ssl).toEqual({ rejectUnauthorized: true });
	});
});

describe("splitStatements", () => {
	test("splits on top-level semicolons and drops comment-only fragments", () => {
		const sql = `-- header line one
-- header line two
CREATE TABLE "A" ("id" TEXT NOT NULL, "tags" JSONB NOT NULL DEFAULT '[]');

/* block; comment */
CREATE INDEX ASYNC "A_id_idx" ON "A"("id");
-- trailing comment`;
		const statements = splitStatements(sql);
		expect(statements).toHaveLength(2);
		expect(statements[0]).toMatch(/^-- header line one/);
		expect(statements[0]).toEndWith("DEFAULT '[]')");
		expect(statements[1]).toBe(
			'/* block; comment */\nCREATE INDEX ASYNC "A_id_idx" ON "A"("id")',
		);
	});

	test("keeps semicolons inside quotes, dollar quotes and escaped strings", () => {
		const sql = `INSERT INTO t VALUES ('a;b', 'it''s; fine', E'x\\';y', "col;umn", $$do; nothing$$, $q$a;b$q$);
SELECT 1`;
		const statements = splitStatements(sql);
		expect(statements).toHaveLength(2);
		expect(statements[0]).toContain("'a;b'");
		expect(statements[0]).toContain("$$do; nothing$$");
		expect(statements[0]).toContain("$q$a;b$q$");
		expect(statements[1]).toBe("SELECT 1");
	});

	test("handles missing trailing semicolon and empty input", () => {
		expect(splitStatements("")).toEqual([]);
		expect(splitStatements("-- only comments\n/* here */")).toEqual([]);
		expect(splitStatements("SELECT 1;;;\n\nSELECT 2")).toEqual([
			"SELECT 1",
			"SELECT 2",
		]);
	});

	test("classifies async-job statements and DDL", () => {
		expect(isAsyncJobStatement('CREATE INDEX ASYNC "i" ON "t"("c")')).toBe(
			true,
		);
		expect(
			isAsyncJobStatement('-- note\nCREATE UNIQUE INDEX ASYNC "i" ON "t"("c")'),
		).toBe(true);
		expect(
			isAsyncJobStatement('ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c"'),
		).toBe(true);
		expect(
			isAsyncJobStatement(
				'ALTER TABLE "t" ADD CONSTRAINT "c" FOREIGN KEY ("a") REFERENCES "b"("id") NOT VALID',
			),
		).toBe(false);
		expect(isAsyncJobStatement('CREATE TABLE "t" ("id" TEXT)')).toBe(false);
		expect(isDdlStatement('CREATE TABLE "t" ("id" TEXT)')).toBe(true);
		expect(isDdlStatement("INSERT INTO t VALUES (1)")).toBe(false);
	});

	test("only CREATE TABLE / CREATE INDEX ASYNC may overlap pending async jobs", () => {
		expect(mayOverlapAsyncJobs('CREATE SCHEMA IF NOT EXISTS "public"')).toBe(
			true,
		);
		expect(mayOverlapAsyncJobs('CREATE TABLE "t" ("id" TEXT)')).toBe(true);
		expect(
			mayOverlapAsyncJobs('-- c\nCREATE UNIQUE INDEX ASYNC "i" ON "t"("c")'),
		).toBe(true);
		expect(mayOverlapAsyncJobs('CREATE INDEX ASYNC "i" ON "t"("c")')).toBe(
			true,
		);
		expect(mayOverlapAsyncJobs('CREATE INDEX "i" ON "t"("c")')).toBe(false);
		expect(
			mayOverlapAsyncJobs(
				'ALTER TABLE "t" ADD CONSTRAINT "c" FOREIGN KEY ("a") REFERENCES "b"("id") NOT VALID',
			),
		).toBe(false);
		expect(
			mayOverlapAsyncJobs('ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c"'),
		).toBe(false);
		expect(mayOverlapAsyncJobs('DROP INDEX "i"')).toBe(false);
		expect(mayOverlapAsyncJobs("INSERT INTO t VALUES (1)")).toBe(false);
	});

	test("recognises CREATE/ADD statements and duplicate-object SQLSTATEs", () => {
		expect(isCreateOrAddStatement('CREATE TABLE "t" ("id" TEXT)')).toBe(true);
		expect(isCreateOrAddStatement('CREATE INDEX ASYNC "i" ON "t"("c")')).toBe(
			true,
		);
		expect(
			isCreateOrAddStatement(
				'ALTER TABLE "t" ADD CONSTRAINT "c" FOREIGN KEY ("a") REFERENCES "b"("id") NOT VALID',
			),
		).toBe(true);
		expect(isCreateOrAddStatement('ALTER TABLE "t" ADD COLUMN "x" TEXT')).toBe(
			true,
		);
		expect(
			isCreateOrAddStatement('ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c"'),
		).toBe(false);
		expect(isCreateOrAddStatement('ALTER TABLE "t" DROP COLUMN "x"')).toBe(
			false,
		);
		expect(isCreateOrAddStatement('DROP TABLE "t"')).toBe(false);
		for (const code of ["42P07", "42710", "42701"]) {
			expect(
				isAlreadyExistsError(
					Object.assign(new Error("already exists"), { code }),
				),
			).toBe(true);
		}
		expect(
			isAlreadyExistsError(
				Object.assign(new Error("duplicate key"), { code: "23505" }),
			),
		).toBe(false);
		expect(isAlreadyExistsError(null)).toBe(false);
	});

	test("asyncIndexName spells the index as sys.jobs.object_name does", () => {
		expect(
			asyncIndexName(
				'CREATE UNIQUE INDEX ASYNC "Bit_dependencyTreeHash_key" ON "Bit"("dependencyTreeHash")',
			),
		).toBe("Bit_dependencyTreeHash_key");
		expect(
			asyncIndexName(
				'-- note\nCREATE INDEX ASYNC IF NOT EXISTS "we""ird" ON "t"("c")',
			),
		).toBe('we"ird');
		expect(asyncIndexName("CREATE INDEX ASYNC Plain_Idx ON t(c)")).toBe(
			"plain_idx",
		);
		expect(asyncIndexName('CREATE INDEX "i" ON "t"("c")')).toBeNull();
		expect(
			asyncIndexName('ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c"'),
		).toBeNull();
	});

	test("splits the committed initial migration into its counted statements", () => {
		const dir = resolve(
			import.meta.dir,
			"../../../../packages/api/prisma/migrations-dsql",
		);
		if (!existsSync(dir)) return;
		const names = readdirSync(dir, { withFileTypes: true })
			.filter((e) => e.isDirectory())
			.map((e) => e.name)
			.sort();
		const first = names[0];
		expect(first).toMatch(/^\d{14}_initial$/);
		const sql = readFileSync(
			join(dir, first as string, "migration.sql"),
			"utf8",
		);
		const header = sql.match(
			/^-- tables=(\d+) indexes=(\d+) foreign_keys=(\d+) validations=(\d+) statements=(\d+)$/m,
		);
		expect(header).not.toBeNull();
		const [, tables, indexes, fks, validations, total] = (
			header as RegExpMatchArray
		).map(Number);
		const statements = splitStatements(sql);
		expect(statements).toHaveLength(total as number);
		expect(
			statements.filter((s) => /^(?:--[^\n]*\n)*CREATE TABLE/.test(s)),
		).toHaveLength(tables as number);
		expect(statements.filter(isAsyncJobStatement)).toHaveLength(
			(indexes as number) + (validations as number),
		);
		expect(
			statements.filter((s) => /FOREIGN KEY[\s\S]*NOT VALID$/.test(s)),
		).toHaveLength(fks as number);
		expect(statements.every(isDdlStatement)).toBe(true);
		// Every ADD CONSTRAINT / VALIDATE drains; the CREATE statements never do.
		const firstDrain = statements.findIndex((s) => !mayOverlapAsyncJobs(s));
		expect(firstDrain).toBe((tables as number) + (indexes as number) + 1);
		expect(statements.slice(0, firstDrain).every(mayOverlapAsyncJobs)).toBe(
			true,
		);
		expect(
			statements
				.filter((s) => /^CREATE (?:UNIQUE )?INDEX ASYNC/.test(s))
				.map(asyncIndexName)
				.every((name) => name !== null),
		).toBe(true);
	});
});

describe("_prisma_migrations bookkeeping", () => {
	test("checksum is sha256 of the file bytes in lowercase hex", () => {
		expect(migrationChecksum("abc")).toBe(
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
		);
		expect(migrationChecksum("")).toHaveLength(64);
	});

	test("table, insert and update statements match Prisma's row shape", () => {
		for (const column of PRISMA_MIGRATIONS_COLUMNS) {
			expect(PRISMA_MIGRATIONS_TABLE_SQL).toContain(`\n    ${column} `);
		}
		expect(PRISMA_MIGRATIONS_TABLE_SQL).toContain(
			"id                      VARCHAR(36) PRIMARY KEY NOT NULL",
		);
		expect(PRISMA_MIGRATIONS_TABLE_SQL).toContain(
			"checksum                VARCHAR(64) NOT NULL",
		);
		expect(PRISMA_MIGRATIONS_TABLE_SQL).toContain(
			"started_at              TIMESTAMPTZ NOT NULL DEFAULT now()",
		);
		expect(PRISMA_MIGRATIONS_TABLE_SQL).toContain(
			"applied_steps_count     INTEGER NOT NULL DEFAULT 0",
		);
		expect(RECORD_STARTED_SQL).toBe(
			"INSERT INTO _prisma_migrations (id, checksum, started_at, migration_name) VALUES ($1, $2, now(), $3)",
		);
		expect(RECORD_APPLIED_SQL).toContain("applied_steps_count = 1");
		expect(RECORD_APPLIED_SQL).not.toContain("finished_at");
		expect(RECORD_FINISHED_SQL).toContain("finished_at = now()");
		expect(RECORD_FINISHED_SQL).toContain("applied_steps_count = 1");
		expect(RECORD_FAILED_SQL).toContain("SET logs = $2");
		expect(RECORD_FAILED_SQL).not.toContain("finished_at");
		expect(LIST_APPLIED_SQL).toContain("ORDER BY started_at ASC");
	});

	test("startedRecord carries a uuid v4 id, the checksum and the directory name", () => {
		const migration: LocalMigration = {
			name: "20260904054933_initial",
			sql: "CREATE TABLE t (id TEXT);",
			checksum: migrationChecksum("CREATE TABLE t (id TEXT);"),
		};
		const record = startedRecord(migration);
		expect(record.id).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
		);
		expect(record.checksum).toBe(migration.checksum);
		expect(record.migration_name).toBe("20260904054933_initial");
		expect(startedRecord(migration).id).not.toBe(record.id);
	});

	test("listLocalMigrations reads only directories, sorted, with their migration.sql", () => {
		const dir = resolve(
			import.meta.dir,
			"../../../../packages/api/prisma/migrations-dsql",
		);
		if (!existsSync(dir)) return;
		const local = listLocalMigrations(dir);
		expect(local.length).toBeGreaterThan(0);
		expect(local.map((m) => m.name)).toEqual(
			[...local.map((m) => m.name)].sort((a, b) => a.localeCompare(b)),
		);
		expect(local.every((m) => /^\d{14}_[a-z0-9_-]+$/.test(m.name))).toBe(true);
		expect(local[0]?.checksum).toBe(migrationChecksum(local[0]?.sql ?? ""));
		expect(() => listLocalMigrations(join(dir, "does-not-exist"))).toThrow(
			MigrationError,
		);
	});

	const a: LocalMigration = {
		name: "20260101000000_a",
		sql: "A",
		checksum: migrationChecksum("A"),
	};
	const b: LocalMigration = {
		name: "20260102000000_b",
		sql: "B",
		checksum: migrationChecksum("B"),
	};
	const row = (over: Partial<AppliedRow>): AppliedRow => ({
		id: "id",
		checksum: a.checksum,
		migration_name: a.name,
		logs: null,
		finished_at: new Date(),
		rolled_back_at: null,
		applied_steps_count: 1,
		...over,
	});

	test("pendingMigrations returns unapplied migrations in local order", () => {
		expect(pendingMigrations([a, b], [])).toEqual([a, b]);
		expect(pendingMigrations([a, b], [row({})])).toEqual([b]);
		expect(
			pendingMigrations(
				[a, b],
				[row({}), row({ migration_name: b.name, checksum: b.checksum })],
			),
		).toEqual([]);
		expect(
			pendingMigrations([a, b], [row({ rolled_back_at: new Date() })]),
		).toEqual([a, b]);
	});

	test("pendingMigrations refuses failed, edited and locally missing migrations", () => {
		expect(() =>
			pendingMigrations([a, b], [row({ finished_at: null, logs: "boom" })]),
		).toThrow(/failed earlier/);
		expect(() =>
			pendingMigrations(
				[a, b],
				[row({ finished_at: null, applied_steps_count: 0 })],
			),
		).toThrow(/failed earlier/);
		expect(() =>
			pendingMigrations([a, b], [row({ checksum: "0".repeat(64) })]),
		).toThrow(/modified after it was applied/);
		expect(() => pendingMigrations([b], [row({})])).toThrow(/missing locally/);
	});

	test("a row whose statements are committed but whose jobs were never confirmed is resumable, not failed", () => {
		const cut = row({ finished_at: null, applied_steps_count: 1, logs: null });
		expect(awaitingJobs(cut)).toBe(true);
		expect(awaitingJobs(row({ finished_at: null, logs: "job failed" }))).toBe(
			false,
		);
		expect(
			awaitingJobs(row({ finished_at: null, applied_steps_count: 0 })),
		).toBe(false);
		expect(
			awaitingJobs(row({ finished_at: null, rolled_back_at: new Date() })),
		).toBe(false);
		expect(awaitingJobs(row({}))).toBe(false);
		expect(pendingMigrations([a, b], [cut])).toEqual([b]);
		expect(() =>
			pendingMigrations([a, b], [{ ...cut, checksum: "0".repeat(64) }]),
		).toThrow(/modified after it was applied/);
	});
});

describe("grants and retries", () => {
	test("grant script targets the runtime role for existing and future objects", () => {
		const statements = grantStatements(parseConfig(validSettings()));
		expect(statements).toHaveLength(5);
		expect(statements[0]).toBe("GRANT USAGE ON SCHEMA public TO flow_like_api");
		expect(statements[1]).toBe(
			"GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO flow_like_api",
		);
		expect(
			statements.filter((s) =>
				s.startsWith("ALTER DEFAULT PRIVILEGES IN SCHEMA public"),
			),
		).toHaveLength(2);
	});

	test("OCC conflicts and connection loss are transient, everything else is not", () => {
		expect(
			isTransientError(
				Object.assign(
					new Error("change conflicts with another transaction (OC000)"),
					{ code: "40001" },
				),
			),
		).toBe(true);
		expect(
			isTransientError(
				new Error(
					"schema has been updated by another transaction, please retry: (OC001)",
				),
			),
		).toBe(true);
		expect(
			isTransientError(
				Object.assign(new Error("terminating connection"), { code: "57P01" }),
			),
		).toBe(true);
		expect(
			isTransientError(new Error("Connection terminated unexpectedly")),
		).toBe(true);
		expect(
			isTransientError(
				Object.assign(new Error("syntax error"), { code: "42601" }),
			),
		).toBe(false);
		expect(
			isTransientError(
				Object.assign(new Error("duplicate key"), { code: "23505" }),
			),
		).toBe(false);
		expect(isTransientError(null)).toBe(false);
	});

	const noPause = async (): Promise<void> => undefined;

	test("withRetries accepts already-exists on a retry, never on the first attempt", async () => {
		let calls = 0;
		const committedDespiteOcc = async (): Promise<number> => {
			calls++;
			if (calls === 1) throw occError();
			throw duplicateError();
		};
		await expect(
			withRetries("create", committedDespiteOcc, {
				acceptOnRetry: isAlreadyExistsError,
				pause: noPause,
			}),
		).resolves.toBe(ACCEPTED);
		expect(calls).toBe(2);

		calls = 0;
		await expect(
			withRetries(
				"create",
				async () => {
					calls++;
					throw duplicateError();
				},
				{ acceptOnRetry: isAlreadyExistsError, pause: noPause },
			),
		).rejects.toMatchObject({ code: "42P07" });
		expect(calls).toBe(1);
	});

	test("withRetries gives up after maxAttempts and never retries non-transient errors", async () => {
		let calls = 0;
		await expect(
			withRetries(
				"occ",
				async () => {
					calls++;
					throw occError();
				},
				{ maxAttempts: 3, pause: noPause },
			),
		).rejects.toMatchObject({ code: "40001" });
		expect(calls).toBe(3);

		calls = 0;
		await expect(
			withRetries(
				"syntax",
				async () => {
					calls++;
					throw Object.assign(new Error("syntax error"), { code: "42601" });
				},
				{ pause: noPause },
			),
		).rejects.toMatchObject({ code: "42601" });
		expect(calls).toBe(1);
		await expect(
			withRetries("ok", async () => 7, { pause: noPause }),
		).resolves.toBe(7);
	});
});

// ---------------------------------------------------------------------------
// applyMigration / waitForJobs against a scripted cluster
// ---------------------------------------------------------------------------

interface PgError extends Error {
	code?: string;
}

function occError(): PgError {
	return Object.assign(
		new Error(
			"schema has been updated by another transaction, please retry: (OC001)",
		),
		{ code: "40001" },
	);
}

function duplicateError(): PgError {
	return Object.assign(new Error('relation "A_id_idx" already exists'), {
		code: "42P07",
	});
}

interface FakeJob {
	status: string;
	object_name: string;
	details: string | null;
}

interface Call {
	readonly sql: string;
	readonly values: unknown[];
}

function queryResult<R extends pg.QueryResultRow>(
	rows: pg.QueryResultRow[],
): pg.QueryResult<R> {
	return {
		command: "",
		rowCount: rows.length,
		oid: 0,
		fields: [],
		rows: rows as R[],
	};
}

// Mirrors Session.run (withRetries around one query) over an in-memory
// sys.jobs: async statements submit a processing job, wait_for_job completes
// it (or fails it for object names listed in failJobs), and close() unblocks
// a wait that was told to hang.
class FakeSession implements Executor {
	readonly calls: Call[] = [];
	readonly jobs = new Map<string, FakeJob>();
	readonly failures = new Map<string, PgError[]>();
	readonly failJobs = new Set<string>();
	blockWaits = false;
	closed = 0;
	private counter = 0;
	private unblock: (() => void) | null = null;

	addJob(objectName: string): string {
		const id = `job${++this.counter}`;
		this.jobs.set(id, {
			status: "processing",
			object_name: objectName,
			details: null,
		});
		return id;
	}

	waits(): unknown[] {
		return this.calls
			.filter((call) => call.sql.startsWith("CALL sys.wait_for_job"))
			.map((call) => call.values[0]);
	}

	indexOf(needle: string): number {
		return this.calls.findIndex((call) => call.sql.includes(needle));
	}

	indexOfWait(jobId: string): number {
		return this.calls.findIndex(
			(call) =>
				call.sql.startsWith("CALL sys.wait_for_job") &&
				call.values[0] === jobId,
		);
	}

	async run<R extends pg.QueryResultRow = pg.QueryResultRow>(
		sql: string,
		values: unknown[] = [],
		label = sql,
		options: RunOptions = {},
	): Promise<pg.QueryResult<R>> {
		const outcome = await withRetries(label, () => this.query<R>(sql, values), {
			...options,
			pause: async () => undefined,
		});
		return outcome === ACCEPTED ? queryResult<R>([]) : outcome;
	}

	async close(): Promise<void> {
		this.closed++;
		this.unblock?.();
		this.unblock = null;
	}

	private jobRows(filter: (job: FakeJob) => boolean): pg.QueryResultRow[] {
		return [...this.jobs]
			.filter(([, job]) => filter(job))
			.map(([job_id, job]) => ({
				job_id,
				status: job.status,
				details: job.details,
				job_type: "INDEX_BUILD",
				object_name: job.object_name,
			}));
	}

	private async query<R extends pg.QueryResultRow>(
		sql: string,
		values: unknown[],
	): Promise<pg.QueryResult<R>> {
		this.calls.push({ sql, values });
		const queued = this.failures.get(sql);
		if (queued && queued.length > 0) throw queued.shift();
		if (sql.startsWith("CALL sys.wait_for_job")) {
			if (this.blockWaits) {
				await new Promise<void>((resolve) => {
					this.unblock = resolve;
				});
				throw Object.assign(new Error("Connection terminated unexpectedly"), {
					code: "08006",
				});
			}
			const job = this.jobs.get(values[0] as string);
			if (job) {
				if (this.failJobs.has(job.object_name)) {
					job.status = "failed";
					job.details = "Found duplicate key while validating index for UCVs";
				} else job.status = "completed";
			}
			return queryResult<R>([{ wait_for_job: "succeeded" }]);
		}
		if (sql.includes("FROM sys.jobs WHERE object_name = $1")) {
			return queryResult<R>(
				this.jobRows((job) => job.object_name === values[0]).map(
					({ job_id }) => ({ job_id }),
				),
			);
		}
		if (sql.includes("FROM sys.jobs WHERE status IN")) {
			return queryResult<R>(
				this.jobRows(
					(job) => job.status === "processing" || job.status === "submitted",
				),
			);
		}
		if (sql.includes("FROM sys.jobs"))
			return queryResult<R>(this.jobRows(() => true));
		if (isAsyncJobStatement(sql)) {
			const id = this.addJob(`public.${asyncIndexName(sql) ?? "validation"}`);
			return queryResult<R>([{ job_id: id }]);
		}
		return queryResult<R>([]);
	}
}

class FakeLease implements LeaseHolder {
	touches = 0;
	async touch(): Promise<void> {
		this.touches++;
	}
}

function migrationOf(sql: string): LocalMigration {
	return {
		name: "20260904054933_initial",
		sql,
		checksum: migrationChecksum(sql),
	};
}

const FK_MIGRATION = `CREATE TABLE "A" ("id" TEXT NOT NULL, CONSTRAINT "A_pkey" PRIMARY KEY ("id"));
CREATE TABLE "B" ("id" TEXT NOT NULL, "aId" TEXT NOT NULL, CONSTRAINT "B_pkey" PRIMARY KEY ("id"));
CREATE UNIQUE INDEX ASYNC "A_id_key" ON "A"("id");
CREATE INDEX ASYNC "B_aId_idx" ON "B"("aId");
ALTER TABLE "B" ADD CONSTRAINT "B_aId_fkey" FOREIGN KEY ("aId") REFERENCES "A"("id") ON DELETE CASCADE ON UPDATE CASCADE NOT VALID;
ALTER TABLE ASYNC "B" VALIDATE CONSTRAINT "B_aId_fkey";
`;

describe("applyMigration", () => {
	test("drains the index jobs before the first ALTER and finishes only after the validation job", async () => {
		const fake = new FakeSession();
		const lease = new FakeLease();
		await applyMigration(fake, lease, migrationOf(FK_MIGRATION), 60_000);

		expect(fake.waits()).toEqual(["job1", "job2", "job3"]);
		expect(fake.indexOf('CREATE INDEX ASYNC "B_aId_idx"')).toBeLessThan(
			fake.indexOfWait("job1"),
		);
		expect(fake.indexOfWait("job2")).toBeLessThan(
			fake.indexOf('ALTER TABLE "B" ADD CONSTRAINT'),
		);
		expect(fake.indexOf('ALTER TABLE "B" ADD CONSTRAINT')).toBeLessThan(
			fake.indexOf('ALTER TABLE ASYNC "B" VALIDATE'),
		);
		expect(fake.indexOf('ALTER TABLE ASYNC "B" VALIDATE')).toBeLessThan(
			fake.indexOf(RECORD_APPLIED_SQL),
		);
		expect(fake.indexOf(RECORD_APPLIED_SQL)).toBeLessThan(
			fake.indexOfWait("job3"),
		);
		expect(fake.indexOfWait("job3")).toBeLessThan(
			fake.indexOf(RECORD_FINISHED_SQL),
		);
		expect(fake.indexOf(RECORD_STARTED_SQL)).toBe(0);
		expect(fake.indexOf(RECORD_FAILED_SQL)).toBe(-1);
		expect([...fake.jobs.values()].every((j) => j.status === "completed")).toBe(
			true,
		);
	});

	test("a retried CREATE INDEX ASYNC that already exists is accepted and its job recovered from sys.jobs", async () => {
		const fake = new FakeSession();
		const statement = 'CREATE INDEX ASYNC "A_id_idx" ON "A"("id")';
		const committed = fake.addJob("public.A_id_idx");
		fake.failures.set(statement, [occError(), duplicateError()]);
		await applyMigration(
			fake,
			new FakeLease(),
			migrationOf(`${statement};`),
			60_000,
		);

		expect(fake.calls.filter((c) => c.sql === statement)).toHaveLength(2);
		const lookup = fake.calls.find((c) =>
			c.sql.includes("FROM sys.jobs WHERE object_name = $1"),
		);
		expect(lookup?.values).toEqual(["public.A_id_idx"]);
		expect(fake.waits()).toEqual([committed]);
		expect(fake.indexOf(RECORD_FINISHED_SQL)).toBeGreaterThan(-1);
		expect(fake.indexOf(RECORD_FAILED_SQL)).toBe(-1);
	});

	test("already exists on the first attempt is a real failure", async () => {
		const fake = new FakeSession();
		const statement = 'CREATE INDEX ASYNC "A_id_idx" ON "A"("id")';
		fake.failures.set(statement, [duplicateError()]);
		await expect(
			applyMigration(
				fake,
				new FakeLease(),
				migrationOf(`${statement};`),
				60_000,
			),
		).rejects.toThrow(/#1\/1: CREATE INDEX ASYNC "A_id_idx".*already exists/);
		const failed = fake.calls.find((c) => c.sql === RECORD_FAILED_SQL);
		expect(failed?.values[1]).toMatch(/^statement 1\/1 failed \(42P07\)/);
		expect(fake.indexOf(RECORD_APPLIED_SQL)).toBe(-1);
		expect(fake.indexOf(RECORD_FINISHED_SQL)).toBe(-1);
	});

	test("a failed job is recorded in logs and finished_at stays NULL", async () => {
		const fake = new FakeSession();
		fake.failJobs.add("public.A_id_key");
		await expect(
			applyMigration(fake, new FakeLease(), migrationOf(FK_MIGRATION), 60_000),
		).rejects.toThrow(/async DDL job\(s\) failed/);
		const failed = fake.calls.find((c) => c.sql === RECORD_FAILED_SQL);
		expect(failed?.values[1]).toMatch(
			/job1 public\.A_id_key: Found duplicate key/,
		);
		expect(fake.indexOf(RECORD_FINISHED_SQL)).toBe(-1);
		expect(fake.indexOf('ALTER TABLE "B" ADD CONSTRAINT')).toBe(-1);
	});

	test("a signal during the final wait leaves the migration resumable and unblocks the session", async () => {
		const fake = new FakeSession();
		const statement = 'CREATE INDEX ASYNC "A_id_idx" ON "A"("id")';
		fake.blockWaits = true;
		const timer = setTimeout(() => {
			interruption.signal = "SIGTERM";
		}, 50);
		try {
			await expect(
				applyMigration(
					fake,
					new FakeLease(),
					migrationOf(`${statement};`),
					60_000,
				),
			).rejects.toThrow(JobsPendingError);
		} finally {
			clearTimeout(timer);
			interruption.signal = null;
		}
		expect(fake.closed).toBeGreaterThan(0);
		expect(fake.indexOf(RECORD_APPLIED_SQL)).toBeGreaterThan(-1);
		expect(fake.indexOf(RECORD_FAILED_SQL)).toBe(-1);
		expect(fake.indexOf(RECORD_FINISHED_SQL)).toBe(-1);
		const resumed: AppliedRow = {
			id: "id",
			checksum: migrationChecksum(`${statement};`),
			migration_name: "20260904054933_initial",
			logs: null,
			finished_at: null,
			rolled_back_at: null,
			applied_steps_count: 1,
		};
		expect(awaitingJobs(resumed)).toBe(true);
	});
});

describe("waitForJobs", () => {
	test("gives up after the wait budget and names the pending jobs", async () => {
		const fake = new FakeSession();
		await expect(
			waitForJobs(fake, new FakeLease(), ["j1", "j2"], 0),
		).rejects.toThrow(
			/gave up after 0 s \(DSQL_JOB_WAIT_TIMEOUT_SECS\) with 2 async job\(s\) still pending: j1, j2/,
		);
		expect(fake.calls).toHaveLength(0);
	});

	test("a signal while sys.wait_for_job blocks is honoured within the watchdog interval", async () => {
		const fake = new FakeSession();
		const job = fake.addJob("public.Slow_idx");
		fake.blockWaits = true;
		const timer = setTimeout(() => {
			interruption.signal = "SIGINT";
		}, 50);
		const started = Date.now();
		try {
			await expect(
				waitForJobs(fake, new FakeLease(), [job], 60_000),
			).rejects.toThrow(
				new RegExp(
					`interrupted by SIGINT with 1 async job\\(s\\) still pending: ${job}`,
				),
			);
		} finally {
			clearTimeout(timer);
			interruption.signal = null;
		}
		expect(Date.now() - started).toBeLessThan(5_000);
		expect(fake.closed).toBe(1);
	});

	test("drainClusterJobs waits for jobs left behind by an earlier run and is a no-op otherwise", async () => {
		const idle = new FakeSession();
		await drainClusterJobs(idle, new FakeLease(), 60_000);
		expect(idle.calls).toHaveLength(1);
		expect(idle.waits()).toEqual([]);

		const busy = new FakeSession();
		const leftover = busy.addJob("public.Old_idx");
		await drainClusterJobs(busy, new FakeLease(), 60_000);
		expect(busy.waits()).toEqual([leftover]);
		expect(busy.jobs.get(leftover)?.status).toBe("completed");
	});
});
