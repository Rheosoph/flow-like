import { describe, expect, spyOn, test } from "bun:test";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import type pg from "pg";
import {
	ACCEPTED,
	AUDIT_EVIDENCE_TABLES,
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
	TABLE_PRIVILEGES,
	acceptedOnRetry,
	applyAuditBoundary,
	applyMigration,
	applyRuntimeGrants,
	asyncIndexName,
	auditBoundary,
	awaitingJobs,
	clientConfig,
	composeDatabaseUrl,
	drainClusterJobs,
	grantAuditWorkerRole,
	grantRuntimeRole,
	grantStatements,
	interruption,
	isAlreadyExistsError,
	isAsyncJobStatement,
	isCreateOrAddStatement,
	isDdlStatement,
	isDoesNotExistError,
	isDropStatement,
	isTransientError,
	listLocalMigrations,
	mayOverlapAsyncJobs,
	migrationChecksum,
	parseConfig,
	pendingMigrations,
	redactDatabaseUrl,
	redactSecret,
	restrictMigrationAccess,
	splitStatements,
	startedRecord,
	waitForJobs,
	withRetries,
} from "./migrate";

const ENDPOINT = "abc0def1ghi2jkl3mno4pqr5stu6.dsql.eu-west-1.on.aws";
const PRIVATE_ENDPOINT = ENDPOINT.replace(".dsql.", ".dsql-fnh4.");
const ROLE_ARN = "arn:aws:iam::123456789012:role/flow-like-api-runtime";
const TRACKER_ROLE_ARN =
	"arn:aws:iam::123456789012:role/flow-like-file-tracker";
const AUDIT_ROLE_ARN = "arn:aws:iam::123456789012:role/flow-like-audit-worker";
const TOKEN =
	"abc.dsql.eu-west-1.on.aws/?Action=DbConnectAdmin&X-Amz-Signature=ab%2Fcd+ef=&X-Amz-Expires=900";

function validSettings(): Environment {
	return {
		DSQL_CLUSTER_ENDPOINT: ENDPOINT,
		DSQL_RUNTIME_ROLE_ARNS: ROLE_ARN,
	};
}

describe("parseConfig", () => {
	test("accepts the minimal contract and derives the region from the endpoint", () => {
		const config = parseConfig(validSettings());
		expect(config.endpoint).toBe(ENDPOINT);
		expect(config.region).toBe("eu-west-1");
		expect(config.runtimeRoleArns).toEqual([ROLE_ARN]);
		expect(config.runtimeDbRole).toBe("flow_like_api");
		expect(config.migrationsDir.endsWith("prisma/migrations-dsql")).toBe(true);
	});

	test("accepts PrivateLink endpoints and derives their region", () => {
		const config = parseConfig({
			...validSettings(),
			DSQL_CLUSTER_ENDPOINT: PRIVATE_ENDPOINT,
		});
		expect(config.endpoint).toBe(PRIVATE_ENDPOINT);
		expect(config.region).toBe("eu-west-1");
	});

	test("accepts a matching DSQL_REGION and rejects a mismatching one", () => {
		for (const endpoint of [ENDPOINT, PRIVATE_ENDPOINT]) {
			const settings = { ...validSettings(), DSQL_CLUSTER_ENDPOINT: endpoint };
			expect(
				parseConfig({ ...settings, DSQL_REGION: "eu-west-1" }).region,
			).toBe("eu-west-1");
			expect(() =>
				parseConfig({ ...settings, DSQL_REGION: "us-east-1" }),
			).toThrow(/DSQL_REGION: must match the endpoint's region eu-west-1/);
		}
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
			"abc.rds.eu-west-1.on.aws",
			"abc.dsql-.eu-west-1.on.aws",
			"abc.dsql--fnh4.eu-west-1.on.aws",
			"abc.dsql-fnh4-.eu-west-1.on.aws",
			"abc.dsql-fnh4.us-east-12.on.aws",
			`abc.dsql-${"a".repeat(59)}.eu-west-1.on.aws`,
			`https://${PRIVATE_ENDPOINT}`,
			`${PRIVATE_ENDPOINT}:5432`,
			`${PRIVATE_ENDPOINT}/postgres`,
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

	test("requires the endpoint; the runtime role ARNs are optional but validated when set", () => {
		expect(() => parseConfig({ DSQL_RUNTIME_ROLE_ARNS: ROLE_ARN })).toThrow(
			/DSQL_CLUSTER_ENDPOINT/,
		);
		expect(
			parseConfig({ DSQL_CLUSTER_ENDPOINT: ENDPOINT }).runtimeRoleArns,
		).toEqual([]);
		for (const arn of [
			"",
			" ",
			"arn:aws:iam::123:user/x",
			`${ROLE_ARN}'; DROP ROLE admin`,
			`${ROLE_ARN},arn:aws:iam::123:user/x`,
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), DSQL_RUNTIME_ROLE_ARNS: arn }),
			).toThrow(ConfigError);
		}
		expect(() =>
			parseConfig({
				...validSettings(),
				DSQL_CLUSTER_ENDPOINT: ` ${ENDPOINT}`,
			}),
		).toThrow(ConfigError);
	});

	test("DSQL_RUNTIME_ROLE_ARNS is a comma list: spaces around commas are fine, empty and duplicate entries are not", () => {
		const list = (value: string) =>
			parseConfig({ ...validSettings(), DSQL_RUNTIME_ROLE_ARNS: value })
				.runtimeRoleArns;
		expect(list(ROLE_ARN)).toEqual([ROLE_ARN]);
		expect(list(`${ROLE_ARN},${TRACKER_ROLE_ARN}`)).toEqual([
			ROLE_ARN,
			TRACKER_ROLE_ARN,
		]);
		expect(list(`${ROLE_ARN}, ${TRACKER_ROLE_ARN}`)).toEqual([
			ROLE_ARN,
			TRACKER_ROLE_ARN,
		]);
		expect(list(`${TRACKER_ROLE_ARN} ,  ${ROLE_ARN}`)).toEqual([
			TRACKER_ROLE_ARN,
			ROLE_ARN,
		]);
		for (const value of [
			`${ROLE_ARN},`,
			`,${ROLE_ARN}`,
			`${ROLE_ARN},,${TRACKER_ROLE_ARN}`,
			`${ROLE_ARN}, ,${TRACKER_ROLE_ARN}`,
		]) {
			expect(() => list(value)).toThrow(
				/DSQL_RUNTIME_ROLE_ARNS: must be a comma-separated list without empty entries/,
			);
		}
		expect(() => list(`${ROLE_ARN},${ROLE_ARN}`)).toThrow(
			new RegExp(`DSQL_RUNTIME_ROLE_ARNS: lists ${ROLE_ARN} more than once`),
		);
		expect(() => list(`${ROLE_ARN}, ${TRACKER_ROLE_ARN} ,${ROLE_ARN}`)).toThrow(
			/more than once/,
		);
	});

	test("an ARN listed for the runtime role cannot also be the audit worker's", () => {
		expect(
			parseConfig({
				...validSettings(),
				DSQL_RUNTIME_ROLE_ARNS: `${ROLE_ARN},${TRACKER_ROLE_ARN}`,
				DSQL_AUDIT_ROLE_ARN: AUDIT_ROLE_ARN,
			}).auditRoleArn,
		).toBe(AUDIT_ROLE_ARN);
		for (const value of [
			TRACKER_ROLE_ARN,
			`${ROLE_ARN},${TRACKER_ROLE_ARN}`,
			`${AUDIT_ROLE_ARN}, ${TRACKER_ROLE_ARN}`,
		]) {
			expect(() =>
				parseConfig({
					...validSettings(),
					DSQL_RUNTIME_ROLE_ARNS: value,
					DSQL_AUDIT_ROLE_ARN: TRACKER_ROLE_ARN,
				}),
			).toThrow(
				/DSQL_AUDIT_ROLE_ARN: must be the audit worker's own IAM role, not one listed in DSQL_RUNTIME_ROLE_ARNS/,
			);
		}
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

	test("keeps the audit worker role and IAM identity apart from the runtime", () => {
		const defaults = parseConfig(validSettings());
		expect(defaults.auditDbRole).toBe("flow_like_audit_worker");
		expect(defaults.auditRoleArn).toBeNull();
		expect(
			parseConfig({ ...validSettings(), DSQL_AUDIT_ROLE_ARN: AUDIT_ROLE_ARN })
				.auditRoleArn,
		).toBe(AUDIT_ROLE_ARN);
		for (const settings of [
			{ DSQL_AUDIT_ROLE_ARN: "flow-like-audit-worker" },
			{ DSQL_AUDIT_ROLE_ARN: ROLE_ARN },
			{ DSQL_AUDIT_DB_ROLE: "flow_like_api" },
			{ DSQL_AUDIT_DB_ROLE: "admin" },
			{ DSQL_RUNTIME_DB_ROLE: "flow_like_audit_worker" },
		]) {
			expect(() => parseConfig({ ...validSettings(), ...settings })).toThrow(
				ConfigError,
			);
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
	test("preserves the PrivateLink hostname and TLS checks in both clients", () => {
		const config = parseConfig({
			...validSettings(),
			DSQL_CLUSTER_ENDPOINT: PRIVATE_ENDPOINT,
		});
		const client = clientConfig(config, TOKEN);
		expect(client.host).toBe(PRIVATE_ENDPOINT);
		expect(client.ssl).toEqual({ rejectUnauthorized: true });
		const url = new URL(composeDatabaseUrl(config, TOKEN));
		expect(url.hostname).toBe(PRIVATE_ENDPOINT);
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("strict");
	});

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

	test("accepts does-not-exist only for a retried DROP", () => {
		expect(isDropStatement('DROP INDEX "AppRollingUsage_sweptAt_idx"')).toBe(
			true,
		);
		expect(isDropStatement('ALTER TABLE "t" DROP COLUMN "x"')).toBe(true);
		expect(isDropStatement('ALTER TABLE "t" DROP CONSTRAINT "c"')).toBe(true);
		expect(isDropStatement('CREATE INDEX ASYNC "i" ON "t"("c")')).toBe(false);
		expect(
			isDropStatement('ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c"'),
		).toBe(false);
		for (const code of ["42P01", "42704", "42703"]) {
			expect(
				isDoesNotExistError(
					Object.assign(new Error("does not exist"), { code }),
				),
			).toBe(true);
		}
		expect(isDoesNotExistError(null)).toBe(false);

		const missing = Object.assign(new Error("index does not exist"), {
			code: "42704",
		});
		const duplicate = Object.assign(new Error("already exists"), {
			code: "42P07",
		});
		expect(acceptedOnRetry('DROP INDEX "i"', missing)).toBe(true);
		expect(acceptedOnRetry('DROP INDEX "i"', duplicate)).toBe(false);
		expect(
			acceptedOnRetry('CREATE INDEX ASYNC "i" ON "t"("c")', duplicate),
		).toBe(true);
		expect(acceptedOnRetry('CREATE INDEX ASYNC "i" ON "t"("c")', missing)).toBe(
			false,
		);
	});

	test("a DROP that committed but reported OC001 is accepted on the retry", async () => {
		let attempts = 0;
		const result = await withRetries(
			"drop",
			async () => {
				attempts++;
				if (attempts === 1)
					throw Object.assign(new Error("change conflicts (OC001)"), {
						code: "40001",
					});
				throw Object.assign(new Error("index does not exist"), {
					code: "42704",
				});
			},
			{
				acceptOnRetry: (error) => acceptedOnRetry('DROP INDEX "i"', error),
				pause: async () => undefined,
			},
		);
		expect(result).toBe(ACCEPTED);
		expect(attempts).toBe(2);
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
	test("grant script covers sequences without default table access", () => {
		const statements = grantStatements(parseConfig(validSettings()));
		expect(statements).toHaveLength(2);
		expect(statements.some((s) => s.includes("TABLES"))).toBe(false);
		expect(
			statements.filter((s) =>
				s.startsWith("ALTER DEFAULT PRIVILEGES IN SCHEMA public"),
			),
		).toHaveLength(1);
	});

	test("runtime bootstrap uses inherited public schema access without changing the system schema", async () => {
		const calls: Call[] = [];
		const session: Executor = {
			async run<R extends pg.QueryResultRow>(
				sql: string,
				values: unknown[] = [],
			) {
				calls.push({ sql, values });
				if (/^GRANT .* ON SCHEMA public\b/.test(sql)) {
					throw new Error("feature not supported on system entity");
				}
				return queryResult<R>(
					sql.includes("has_schema_privilege") ? [{ can_use: true }] : [],
				);
			},
			async close() {},
		};
		await grantRuntimeRole(session, parseConfig(validSettings()), [ROLE_ARN]);
		expect(
			calls.find((call) => call.sql.includes("has_schema_privilege"))?.values,
		).toEqual(["flow_like_api"]);
		expect(calls.map((call) => call.sql)).toContain(
			`AWS IAM GRANT flow_like_api TO '${ROLE_ARN}'`,
		);
		expect(
			calls
				.map((call) => call.sql)
				.filter((sql) => /^(GRANT|ALTER DEFAULT PRIVILEGES)\b/.test(sql)),
		).toEqual(grantStatements(parseConfig(validSettings())));
	});

	test("runtime bootstrap fails before granting access when inherited schema usage is missing", async () => {
		const calls: string[] = [];
		const session: Executor = {
			async run<R extends pg.QueryResultRow>(sql: string) {
				calls.push(sql);
				return queryResult<R>(
					sql.includes("has_schema_privilege")
						? [{ can_use: false }]
						: [{ exists: 1 }],
				);
			},
			async close() {},
		};
		await expect(
			grantRuntimeRole(session, parseConfig(validSettings()), [ROLE_ARN]),
		).rejects.toThrow(/lacks inherited USAGE on schema public/);
		expect(
			calls.some((sql) =>
				/^(AWS IAM GRANT|GRANT|ALTER DEFAULT PRIVILEGES)\b/.test(sql),
			),
		).toBe(false);
	});

	// A catalog of roles and IAM mappings that CREATE ROLE and AWS IAM GRANT
	// update, so a second probe for the same role or mapping finds it.
	function mappingSession(options: {
		mappings?: [role: string, arn: string][];
	}): Executor & { calls: string[] } {
		const calls: string[] = [];
		const roles = new Set<string>();
		const mappings = new Set(
			(options.mappings ?? []).map(([role, arn]) => `${role}|${arn}`),
		);
		return {
			calls,
			async run<R extends pg.QueryResultRow>(
				sql: string,
				values: unknown[] = [],
			) {
				calls.push(sql);
				if (sql.startsWith("SELECT 1 FROM pg_roles")) {
					return queryResult<R>(
						roles.has(values[0] as string) ? [{ one: 1 }] : [],
					);
				}
				if (sql.includes("has_schema_privilege")) {
					return queryResult<R>([{ can_use: true }]);
				}
				if (sql.includes("sys.iam_pg_role_mappings")) {
					return queryResult<R>(
						mappings.has(`${values[0]}|${values[1]}`) ? [{ one: 1 }] : [],
					);
				}
				const created = sql.match(/^CREATE ROLE (\S+) WITH LOGIN$/);
				if (created) roles.add(created[1] as string);
				const granted = sql.match(/^AWS IAM GRANT (\S+) TO '([^']+)'$/);
				if (granted) mappings.add(`${granted[1]}|${granted[2]}`);
				return queryResult<R>([]);
			},
			async close() {},
		};
	}

	test("one run maps every runtime IAM role in order and reconciles the grants once", async () => {
		const config = parseConfig({
			...validSettings(),
			DSQL_RUNTIME_ROLE_ARNS: `${ROLE_ARN}, ${TRACKER_ROLE_ARN}`,
		});
		const session = mappingSession({});
		await grantRuntimeRole(session, config, config.runtimeRoleArns);
		const apiGrant = session.calls.indexOf(
			`AWS IAM GRANT flow_like_api TO '${ROLE_ARN}'`,
		);
		const trackerGrant = session.calls.indexOf(
			`AWS IAM GRANT flow_like_api TO '${TRACKER_ROLE_ARN}'`,
		);
		const firstDcl = session.calls.findIndex((sql) =>
			/^(GRANT|ALTER DEFAULT PRIVILEGES)\b/.test(sql),
		);
		expect(apiGrant).toBeGreaterThan(-1);
		expect(trackerGrant).toBeGreaterThan(apiGrant);
		expect(firstDcl).toBeGreaterThan(trackerGrant);
		expect(
			session.calls.filter((sql) => sql.startsWith("CREATE ROLE")),
		).toEqual(["CREATE ROLE flow_like_api WITH LOGIN"]);
		expect(
			session.calls.filter((sql) =>
				/^(GRANT|ALTER DEFAULT PRIVILEGES)\b/.test(sql),
			),
		).toEqual(grantStatements(config));
		expect(
			session.calls.filter((sql) =>
				sql.startsWith("SELECT n.nspname AS schema"),
			),
		).toHaveLength(1);
	});

	test("the audit exclusivity check applies to every listed runtime ARN", async () => {
		const config = parseConfig({
			...validSettings(),
			DSQL_RUNTIME_ROLE_ARNS: `${ROLE_ARN},${TRACKER_ROLE_ARN}`,
		});
		const session = mappingSession({
			mappings: [["flow_like_audit_worker", TRACKER_ROLE_ARN]],
		});
		await expect(
			grantRuntimeRole(session, config, config.runtimeRoleArns),
		).rejects.toThrow(
			new RegExp(
				`${TRACKER_ROLE_ARN} is already mapped to database role flow_like_audit_worker`,
			),
		);
		expect(session.calls).toContain(
			`AWS IAM GRANT flow_like_api TO '${ROLE_ARN}'`,
		);
		expect(session.calls).not.toContain(
			`AWS IAM GRANT flow_like_api TO '${TRACKER_ROLE_ARN}'`,
		);
		expect(
			session.calls.some((sql) =>
				/^(GRANT|ALTER DEFAULT PRIVILEGES)\b/.test(sql),
			),
		).toBe(false);
	});

	test("audit evidence tables mirror the PostgreSQL role split", () => {
		const script = readFileSync(
			resolve(import.meta.dir, "../../shared/audit_database_roles.ts"),
			"utf8",
		);
		const list = script.match(/const evidenceTables = \[([^\]]*)\]/)?.[1];
		expect(list).toBeDefined();
		expect([...(list ?? "").matchAll(/"(\w+)"/g)].map((m) => m[1])).toEqual([
			...AUDIT_EVIDENCE_TABLES,
		]);
	});

	test("audit boundary model: worker first, API appends records and never sees the lease", () => {
		const grants = auditBoundary(parseConfig(validSettings()));
		const firstApi = grants.findIndex((g) => g.role === "flow_like_api");
		expect(
			grants
				.slice(0, firstApi)
				.every((g) => g.role === "flow_like_audit_worker"),
		).toBe(true);
		expect(grants.every((g) => g.exact)).toBe(true);
		const api = (table: string) =>
			grants.find(
				(g) => g.role === "flow_like_api" && g.table === `public."${table}"`,
			)?.wanted;
		expect(api("AuditRecord")).toEqual(["SELECT", "INSERT"]);
		expect(api("AuditWorkerLease")).toEqual([]);
		expect(api("AuditSeal")).toEqual(["SELECT"]);
		expect(api("AuditExportTarget")).toBeUndefined();
	});

	const DCL = /^(GRANT|REVOKE|ALTER DEFAULT PRIVILEGES)\b/;
	const APP_TABLES = [
		...AUDIT_EVIDENCE_TABLES,
		"AiActAssessment",
		"AuditExportTarget",
		"App",
		"_prisma_migrations",
		"_prisma_custom_metadata",
		"_flow_migration_lock",
	];
	type Acl = Map<string, Set<string>>;
	type DefaultAcl = { schema: string | null; object_type: string; acl: string };
	const aclKey = (role: unknown, table: unknown) => `${role}|${table}`;

	// admin's default privileges: the runtime role holds everything, the worker nothing.
	function defaultAcl(): Acl {
		return new Map(
			APP_TABLES.map((table) => [
				aclKey("flow_like_api", `public."${table}"`),
				new Set(["SELECT", "INSERT", "UPDATE", "DELETE"]),
			]),
		);
	}

	// A catalog whose GRANT/REVOKE statements take effect, so the verification
	// pass reads what the reconciliation wrote.
	function aclSession(options: {
		roles: string[];
		acl: Acl;
		ignoreRevokes?: boolean;
		defaultAcls?: DefaultAcl[];
	}): Executor & { calls: string[] } {
		const calls: string[] = [];
		const defaults = options.defaultAcls ?? [
			{ schema: "public", object_type: "S", acl: "{flow_like_api=rU/admin}" },
		];
		return {
			calls,
			async run<R extends pg.QueryResultRow>(
				sql: string,
				values: unknown[] = [],
			) {
				calls.push(sql);
				if (sql.startsWith("SELECT rolname FROM pg_roles")) {
					return queryResult<R>(
						options.roles
							.filter((role) => values.includes(role))
							.map((rolname) => ({ rolname })),
					);
				}
				if (sql.startsWith("SELECT c.relname FROM pg_class")) {
					return queryResult<R>(APP_TABLES.map((relname) => ({ relname })));
				}
				if (sql.startsWith("SELECT n.nspname AS schema")) {
					return queryResult<R>(defaults);
				}
				const revokeDefaults = sql.match(
					/^ALTER DEFAULT PRIVILEGES( IN SCHEMA public)? REVOKE ALL ON TABLES FROM flow_like_api$/,
				);
				if (revokeDefaults && !options.ignoreRevokes) {
					for (const entry of defaults) {
						if (
							entry.object_type === "r" &&
							entry.schema === (revokeDefaults[1] ? "public" : null)
						)
							entry.acl = "{admin=arwd/admin}";
					}
				}
				if (
					sql ===
					"ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO flow_like_api"
				) {
					defaults.push({
						schema: "public",
						object_type: "S",
						acl: "{flow_like_api=rU/admin}",
					});
				}
				if (sql.includes("has_table_privilege")) {
					const held = options.acl.get(aclKey(values[0], values[1]));
					return queryResult<R>([
						Object.fromEntries(
							TABLE_PRIVILEGES.map((p) => [p, held?.has(p) === true]),
						),
					]);
				}
				const grant = sql.match(/^GRANT (.+) ON TABLE (.+) TO (\S+)$/);
				if (grant) {
					const key = aclKey(grant[3], grant[2]);
					const held = options.acl.get(key) ?? new Set<string>();
					for (const p of (grant[1] ?? "").split(", ")) held.add(p);
					options.acl.set(key, held);
				}
				const revoke = sql.match(/^REVOKE (.+) ON TABLE (.+) FROM (\S+)$/);
				if (revoke && !options.ignoreRevokes) {
					const held = options.acl.get(aclKey(revoke[3], revoke[2]));
					for (const p of (revoke[1] ?? "").split(", ")) held?.delete(p);
				}
				return queryResult<R>([]);
			},
			async close() {},
		};
	}

	test("runtime grants repair metadata access, preserve evidence and become a no-op", async () => {
		const config = parseConfig(validSettings());
		const acl = defaultAcl();
		acl.delete(aclKey("flow_like_api", 'public."App"'));
		const session = aclSession({
			roles: ["flow_like_api"],
			acl,
		});
		await applyRuntimeGrants(session, config);
		expect(session.calls.filter((sql) => DCL.test(sql))).toEqual([
			'REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLE public."_prisma_migrations" FROM flow_like_api',
			'REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLE public."_prisma_custom_metadata" FROM flow_like_api',
			'REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLE public."_flow_migration_lock" FROM flow_like_api',
			'GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE public."App" TO flow_like_api',
		]);
		for (const name of APP_TABLES.filter((table) => table.startsWith("_"))) {
			expect(acl.get(aclKey("flow_like_api", `public."${name}"`))?.size).toBe(
				0,
			);
		}
		expect(acl.get(aclKey("flow_like_api", 'public."AuditRecord"'))).toEqual(
			new Set(["SELECT", "INSERT", "UPDATE", "DELETE"]),
		);
		const again = aclSession({
			roles: ["flow_like_api"],
			acl,
		});
		await applyRuntimeGrants(again, config);
		expect(again.calls.some((sql) => DCL.test(sql))).toBe(false);
	});

	test("runtime grants revoke both global and schema table defaults once and retain sequence access", async () => {
		const config = parseConfig(validSettings());
		const acl = new Map<string, Set<string>>();
		const defaultAcls: DefaultAcl[] = [
			{
				schema: null,
				object_type: "r",
				acl: "{admin=arwd/admin,flow_like_api=arwd/admin}",
			},
			{ schema: "public", object_type: "r", acl: "{flow_like_api=a/admin}" },
			{ schema: "public", object_type: "S", acl: "{flow_like_api=rU/admin}" },
		];
		const session = aclSession({ roles: ["flow_like_api"], acl, defaultAcls });
		await applyRuntimeGrants(session, config);
		const statements = session.calls.filter((sql) => DCL.test(sql));
		expect(statements.slice(0, 2)).toEqual([
			"ALTER DEFAULT PRIVILEGES REVOKE ALL ON TABLES FROM flow_like_api",
			"ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON TABLES FROM flow_like_api",
		]);
		expect(
			statements.some((sql) =>
				/SEQUENCES|_prisma|_flow_migration_lock/.test(sql),
			),
		).toBe(false);
		expect(acl.get(aclKey("flow_like_api", 'public."App"'))).toEqual(
			new Set(["SELECT", "INSERT", "UPDATE", "DELETE"]),
		);
		const again = aclSession({ roles: ["flow_like_api"], acl, defaultAcls });
		await applyRuntimeGrants(again, config);
		expect(again.calls.some((sql) => DCL.test(sql))).toBe(false);
	});

	test("sequence defaults are restored independently of table defaults and remain a no-op afterwards", async () => {
		const config = parseConfig(validSettings());
		const acl = defaultAcl();
		for (const name of APP_TABLES.filter((table) => table.startsWith("_"))) {
			acl.delete(aclKey("flow_like_api", `public."${name}"`));
		}
		const defaultAcls: DefaultAcl[] = [
			{
				schema: "public",
				object_type: "r",
				acl: "{other_flow_like_api=arwd/admin,flow_like_apix=arwd/admin}",
			},
			{ schema: "public", object_type: "S", acl: "{flow_like_api=U/admin}" },
		];
		const session = aclSession({ roles: ["flow_like_api"], acl, defaultAcls });
		await applyRuntimeGrants(session, config);
		expect(session.calls.filter((sql) => DCL.test(sql))).toEqual(
			grantStatements(config),
		);
		const again = aclSession({ roles: ["flow_like_api"], acl, defaultAcls });
		await applyRuntimeGrants(again, config);
		expect(again.calls.some((sql) => DCL.test(sql))).toBe(false);
	});

	test("metadata restriction fails if inherited or PUBLIC table access survives the revoke", async () => {
		const session = aclSession({
			roles: ["flow_like_api"],
			acl: defaultAcl(),
			ignoreRevokes: true,
		});
		await expect(
			restrictMigrationAccess(session, parseConfig(validSettings())),
		).rejects.toThrow(
			/migration privilege boundary not in effect: flow_like_api still holds SELECT, INSERT, UPDATE, DELETE on public\."_prisma_migrations"/,
		);
	});

	test("metadata restriction fails if a default privilege revoke does not take effect", async () => {
		const session = aclSession({
			roles: ["flow_like_api"],
			acl: defaultAcl(),
			ignoreRevokes: true,
			defaultAcls: [
				{ schema: null, object_type: "r", acl: "{flow_like_api=arwd/admin}" },
			],
		});
		await expect(
			restrictMigrationAccess(session, parseConfig(validSettings())),
		).rejects.toThrow(
			/still holds default table privileges for the migration owner/,
		);
	});

	test("audit boundary waits for the worker role before touching the API", async () => {
		const session = aclSession({ roles: ["flow_like_api"], acl: defaultAcl() });
		await applyAuditBoundary(session, parseConfig(validSettings()));
		expect(session.calls.some((sql) => DCL.test(sql))).toBe(false);
	});

	test("audit boundary grants the worker first, revokes only DSQL privileges, then goes quiet", async () => {
		const config = parseConfig(validSettings());
		const acl = defaultAcl();
		const roles = ["flow_like_api", "flow_like_audit_worker"];
		const session = aclSession({ roles, acl });
		await applyAuditBoundary(session, config);
		const dcl = session.calls.filter((sql) => DCL.test(sql));
		const workerGrants = AUDIT_EVIDENCE_TABLES.length + 2;
		expect(dcl).toHaveLength(workerGrants + AUDIT_EVIDENCE_TABLES.length);
		expect(
			dcl
				.slice(0, workerGrants)
				.every((sql) => /^GRANT .* TO flow_like_audit_worker$/.test(sql)),
		).toBe(true);
		expect(
			dcl
				.slice(workerGrants)
				.every((sql) => /^REVOKE .* FROM flow_like_api$/.test(sql)),
		).toBe(true);
		expect(dcl).toContain(
			'GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE public."AuditSeal" TO flow_like_audit_worker',
		);
		expect(dcl).toContain(
			'GRANT SELECT, UPDATE ON TABLE public."AuditExportTarget" TO flow_like_audit_worker',
		);
		expect(dcl).toContain(
			'REVOKE INSERT, UPDATE, DELETE ON TABLE public."AuditEntry" FROM flow_like_api',
		);
		expect(dcl).toContain(
			'REVOKE UPDATE, DELETE ON TABLE public."AuditRecord" FROM flow_like_api',
		);
		expect(dcl).toContain(
			'REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLE public."AuditWorkerLease" FROM flow_like_api',
		);
		expect(dcl.some((sql) => /TRUNCATE|TRIGGER/.test(sql))).toBe(false);
		expect(acl.get(aclKey("flow_like_api", 'public."AuditRecord"'))).toEqual(
			new Set(["SELECT", "INSERT"]),
		);
		expect(acl.get(aclKey("flow_like_api", 'public."App"'))).toEqual(
			new Set(["SELECT", "INSERT", "UPDATE", "DELETE"]),
		);
		const again = aclSession({ roles, acl });
		await applyAuditBoundary(again, config);
		expect(again.calls.some((sql) => DCL.test(sql))).toBe(false);
	});

	// Lines the job printed through makeLog while `work` ran; makeLog resolves
	// console.log at call time, so the spy sees every line.
	async function capturedLog(work: () => Promise<unknown>): Promise<string[]> {
		const spy = spyOn(console, "log").mockImplementation(() => undefined);
		try {
			await work();
			return spy.mock.calls.map((call) => String(call[0]));
		} finally {
			spy.mockRestore();
		}
	}
	const VERIFIED_LINE =
		"[aws-migration] AUDIT_BOUNDARY_VERIFIED api=flow_like_api worker=flow_like_audit_worker";

	test("a verified boundary prints exactly one AUDIT_BOUNDARY_VERIFIED line", async () => {
		const config = parseConfig(validSettings());
		const roles = ["flow_like_api", "flow_like_audit_worker"];
		const acl = defaultAcl();
		const lines = await capturedLog(() =>
			applyAuditBoundary(aclSession({ roles, acl }), config),
		);
		expect(lines.filter((line) => line === VERIFIED_LINE)).toHaveLength(1);
		expect(
			lines.filter((line) => line.includes("AUDIT_BOUNDARY_VERIFIED")),
		).toHaveLength(1);
		expect(
			lines.some((line) => line.includes("audit privilege boundary verified:")),
		).toBe(true);
		expect(lines.at(-1)).toBe(VERIFIED_LINE);

		const quiet = await capturedLog(() =>
			applyAuditBoundary(aclSession({ roles, acl }), config),
		);
		expect(quiet.filter((line) => line === VERIFIED_LINE)).toHaveLength(1);
	});

	test("no AUDIT_BOUNDARY_VERIFIED line without the worker role or when the boundary fails", async () => {
		const config = parseConfig(validSettings());
		const missingWorker = await capturedLog(() =>
			applyAuditBoundary(
				aclSession({ roles: ["flow_like_api"], acl: defaultAcl() }),
				config,
			),
		);
		expect(
			missingWorker.some((line) => line.includes("AUDIT_BOUNDARY_VERIFIED")),
		).toBe(false);
		expect(missingWorker.some((line) => line.includes("does not exist"))).toBe(
			true,
		);

		const failed = await capturedLog(() =>
			applyAuditBoundary(
				aclSession({
					roles: ["flow_like_api", "flow_like_audit_worker"],
					acl: defaultAcl(),
					ignoreRevokes: true,
				}),
				config,
			).catch(() => undefined),
		);
		expect(
			failed.some((line) => line.includes("AUDIT_BOUNDARY_VERIFIED")),
		).toBe(false);
	});

	test("audit boundary fails when a REVOKE does not take effect", async () => {
		const session = aclSession({
			roles: ["flow_like_api", "flow_like_audit_worker"],
			acl: defaultAcl(),
			ignoreRevokes: true,
		});
		await expect(
			applyAuditBoundary(session, parseConfig(validSettings())),
		).rejects.toThrow(
			/flow_like_api still holds INSERT, UPDATE, DELETE on public\."AuditEntry"/,
		);
	});

	test("audit boundary fails when the worker reaches an application table", async () => {
		const acl = defaultAcl();
		acl.set(
			aclKey("flow_like_audit_worker", 'public."App"'),
			new Set(["SELECT"]),
		);
		const session = aclSession({
			roles: ["flow_like_api", "flow_like_audit_worker"],
			acl,
		});
		await expect(
			applyAuditBoundary(session, parseConfig(validSettings())),
		).rejects.toThrow(
			/holds SELECT on App; the worker must not reach application tables/,
		);
	});

	test("audit worker mapping refuses an IAM role that already logs in as the API", async () => {
		const calls: Call[] = [];
		const session: Executor = {
			async run<R extends pg.QueryResultRow>(
				sql: string,
				values: unknown[] = [],
			) {
				calls.push({ sql, values });
				if (sql.includes("has_schema_privilege")) {
					return queryResult<R>([{ can_use: true }]);
				}
				const sharedWithApi =
					sql.includes("sys.iam_pg_role_mappings") &&
					values[0] === "flow_like_api";
				return queryResult<R>(sharedWithApi ? [{ exists: 1 }] : []);
			},
			async close() {},
		};
		const config = parseConfig({
			...validSettings(),
			DSQL_AUDIT_ROLE_ARN: AUDIT_ROLE_ARN,
		});
		await expect(
			grantAuditWorkerRole(session, config, AUDIT_ROLE_ARN),
		).rejects.toThrow(/already mapped to database role flow_like_api/);
		expect(calls.some((call) => call.sql.startsWith("AWS IAM GRANT"))).toBe(
			false,
		);
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
