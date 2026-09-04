#!/usr/bin/env bun
// One-shot Aurora DSQL migration job.
//
// Applies packages/api/prisma/migrations-dsql to the cluster as the `admin`
// role, authenticated with an IAM token from @aws-sdk/dsql-signer. The
// environment contract and the forbidden list are the ones
// flow_like_aws_data::dsql enforces for the API and file-tracker Lambdas
// (packages/aws-data/src/dsql.rs), so a job revision cannot be configured in a
// way its siblings would refuse. No password exists anywhere: the token becomes
// the PostgreSQL password of connections that exist only in this process and,
// for the final `prisma migrate status`, in the environment of that child.
//
// Why this job applies statements itself instead of `prisma migrate deploy`:
// the schema engine Prisma 7.3.0 ships (prisma-engines 9d6ad21c, 2026-01-20)
// sends each migration.sql as ONE simple-query batch; PostgreSQL wraps a
// multi-statement batch in an implicit transaction, and DSQL allows exactly one
// DDL statement per transaction. Newer engines split the batch with a
// PostgreSQL parser, which does not know `CREATE INDEX ASYNC` and falls back to
// the whole batch. So every statement here runs as its own autocommit
// statement, and the migration is recorded in `_prisma_migrations` exactly as
// Prisma records it (uuid id, sha256 checksum, started_at/finished_at,
// applied_steps_count) so that `prisma migrate status` - run at the end as an
// independent check - stays truthful. One deliberate refinement: a migration's
// `finished_at` is set only after the async index/validation jobs it submitted
// have completed, so a row that is finished describes a schema that is usable.
//
// Observed on a live cluster: while hundreds of index jobs run, a single
// session still gets `40001 … (OC001)` on a few percent of DDL statements
// (retried here), a retried statement may find that the failed attempt was
// committed after all ("already exists" is accepted on the second attempt), and
// `ADD CONSTRAINT … FOREIGN KEY` fails with "no unique constraint matching
// given keys" while the referenced unique index is still building - so every
// statement other than CREATE TABLE / CREATE INDEX ASYNC first waits for the
// jobs the migration has submitted so far.

import { createHash, randomUUID } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { hostname } from "node:os";
import { join, resolve } from "node:path";
import { DsqlSigner } from "@aws-sdk/dsql-signer";
import type pg from "pg";
import {
	ADMIN_USER,
	ConfigError,
	DsqlSession,
	type DsqlTarget,
	type Environment,
	type Executor,
	MigrationError,
	TOKEN_EXPIRES_IN_SECONDS,
	clientConfig as composeClientConfig,
	composeDatabaseUrl as composeUrl,
	envReader,
	interruption,
	invalid,
	isAlreadyExistsError,
	isTransientError,
	makeLog,
	parseDsqlTarget,
	redactDatabaseUrl,
	redactSecret,
	sleep,
} from "./shared";

export {
	ACCEPTED,
	ConfigError,
	ENDPOINT_ENV,
	FORBIDDEN_SETTINGS,
	MigrationError,
	REGION_ENV,
	interruption,
	isAlreadyExistsError,
	isConnectionError,
	isTransientError,
	redactDatabaseUrl,
	redactSecret,
	withRetries,
} from "./shared";
export type {
	Environment,
	Executor,
	RetryOptions,
	RunOptions,
} from "./shared";

const LOG_PREFIX = "[aws-migration]";
const APPLICATION_NAME = "flow-like-aws-migration";
const DEFAULT_RUNTIME_DB_ROLE = "flow_like_api";
const DEFAULT_MIGRATIONS_DIR = "prisma/migrations-dsql";
const LEASE_TABLE = "_flow_migration_lock";
const LEASE_MINUTES = 30;
const LEASE_RENEW_MS = 5 * 60_000;
const JOB_POLL_MS = 5_000;
const WATCHDOG_MS = 500;
const DEFAULT_JOB_WAIT_TIMEOUT_SECS = 2 * 60 * 60;
const MIN_JOB_WAIT_TIMEOUT_SECS = 60;
const MAX_JOB_WAIT_TIMEOUT_SECS = 24 * 60 * 60;

export const RUNTIME_ROLE_ARN_ENV = "DSQL_RUNTIME_ROLE_ARN";
export const RUNTIME_DB_ROLE_ENV = "DSQL_RUNTIME_DB_ROLE";
export const MIGRATIONS_DIR_ENV = "DSQL_MIGRATIONS_DIR";
export const JOB_WAIT_TIMEOUT_ENV = "DSQL_JOB_WAIT_TIMEOUT_SECS";

const ROLE_ARN_PATTERN =
	/^arn:aws(?:-[a-z]+)*:iam::\d{12}:role\/[\w+=,.@/-]{1,512}$/;
const IDENTIFIER_PATTERN = /^[a-z_][a-z0-9_]{0,62}$/;
const UUID_PATTERN =
	/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export interface MigrationConfig extends DsqlTarget {
	// null: no runtime role is bound yet (dev clusters); the grant step is skipped.
	readonly runtimeRoleArn: string | null;
	readonly runtimeDbRole: string;
	readonly migrationsDir: string;
	// Upper bound for one wait on async jobs (per migration, and for the
	// cluster-wide drains); the jobs themselves keep running past it.
	readonly jobWaitTimeoutMs: number;
}

// Thrown when a wait on async jobs is cut short (signal or wait budget). The
// jobs keep running on the cluster and nothing is wrong with the schema, so a
// migration whose statements are all committed stays resumable.
export class JobsPendingError extends MigrationError {
	override readonly name = "JobsPendingError";
}

export function parseConfig(env: Environment): MigrationConfig {
	const target = parseDsqlTarget(env);
	const { optional } = envReader(env);

	const runtimeRoleArn = optional(RUNTIME_ROLE_ARN_ENV) ?? null;
	if (runtimeRoleArn !== null && !ROLE_ARN_PATTERN.test(runtimeRoleArn)) {
		invalid(
			RUNTIME_ROLE_ARN_ENV,
			"must be an IAM role ARN (arn:aws:iam::<account>:role/<name>)",
		);
	}

	const runtimeDbRole =
		optional(RUNTIME_DB_ROLE_ENV) ?? DEFAULT_RUNTIME_DB_ROLE;
	if (!IDENTIFIER_PATTERN.test(runtimeDbRole) || runtimeDbRole === ADMIN_USER) {
		invalid(
			RUNTIME_DB_ROLE_ENV,
			"must be a lowercase PostgreSQL identifier other than admin",
		);
	}

	const migrationsDir = resolve(
		import.meta.dir,
		optional(MIGRATIONS_DIR_ENV) ?? DEFAULT_MIGRATIONS_DIR,
	);

	const jobWaitTimeout = optional(JOB_WAIT_TIMEOUT_ENV);
	const jobWaitTimeoutSecs =
		jobWaitTimeout === undefined
			? DEFAULT_JOB_WAIT_TIMEOUT_SECS
			: Number(jobWaitTimeout);
	if (
		jobWaitTimeout !== undefined &&
		(!/^\d+$/.test(jobWaitTimeout) ||
			jobWaitTimeoutSecs < MIN_JOB_WAIT_TIMEOUT_SECS ||
			jobWaitTimeoutSecs > MAX_JOB_WAIT_TIMEOUT_SECS)
	) {
		invalid(
			JOB_WAIT_TIMEOUT_ENV,
			`must be an integer number of seconds between ${MIN_JOB_WAIT_TIMEOUT_SECS} and ${MAX_JOB_WAIT_TIMEOUT_SECS}`,
		);
	}

	return {
		...target,
		runtimeRoleArn,
		runtimeDbRole,
		migrationsDir,
		jobWaitTimeoutMs: jobWaitTimeoutSecs * 1_000,
	};
}

export function composeDatabaseUrl(config: DsqlTarget, token: string): string {
	return composeUrl(config, token, APPLICATION_NAME);
}

export function clientConfig(
	config: DsqlTarget,
	token: string,
): pg.ClientConfig {
	return composeClientConfig(config, token, APPLICATION_NAME);
}

// ---------------------------------------------------------------------------
// Statement splitting and classification
// ---------------------------------------------------------------------------

// Splits SQL on top-level semicolons, honoring '…' (with '' and E'\'' escapes),
// "…", $tag$…$tag$, -- comments and nestable /* */ comments. Comment-only
// fragments are dropped; the trailing semicolon is removed.
export function splitStatements(sql: string): string[] {
	const statements: string[] = [];
	let start = 0;
	let i = 0;
	const n = sql.length;

	const push = (end: number) => {
		const text = sql.slice(start, end).trim();
		if (text !== "" && stripComments(text).trim() !== "") statements.push(text);
	};

	while (i < n) {
		const ch = sql[i] as string;
		const next = sql[i + 1];
		if (ch === "-" && next === "-") {
			const eol = sql.indexOf("\n", i);
			i = eol === -1 ? n : eol + 1;
		} else if (ch === "/" && next === "*") {
			let depth = 1;
			i += 2;
			while (i < n && depth > 0) {
				if (sql[i] === "/" && sql[i + 1] === "*") {
					depth++;
					i += 2;
				} else if (sql[i] === "*" && sql[i + 1] === "/") {
					depth--;
					i += 2;
				} else i++;
			}
		} else if (ch === "'") {
			const escaped =
				i > 0 &&
				/[eE]/.test(sql[i - 1] as string) &&
				!/\w/.test(sql[i - 2] ?? " ");
			i++;
			while (i < n) {
				if (escaped && sql[i] === "\\") i += 2;
				else if (sql[i] === "'" && sql[i + 1] === "'") i += 2;
				else if (sql[i] === "'") {
					i++;
					break;
				} else i++;
			}
		} else if (ch === '"') {
			i++;
			while (i < n) {
				if (sql[i] === '"' && sql[i + 1] === '"') i += 2;
				else if (sql[i] === '"') {
					i++;
					break;
				} else i++;
			}
		} else if (ch === "$") {
			const tag = sql.slice(i).match(/^\$([A-Za-z_][A-Za-z0-9_]*)?\$/);
			if (tag) {
				const close = sql.indexOf(tag[0], i + tag[0].length);
				i = close === -1 ? n : close + tag[0].length;
			} else i++;
		} else if (ch === ";") {
			push(i);
			i++;
			start = i;
		} else i++;
	}
	push(n);
	return statements;
}

function stripComments(text: string): string {
	return text.replace(/--[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "");
}

export function isAsyncJobStatement(statement: string): boolean {
	return /^\s*(?:CREATE\s+(?:UNIQUE\s+)?INDEX\s+ASYNC|ALTER\s+TABLE\s+ASYNC)\b/i.test(
		stripComments(statement),
	);
}

export function isDdlStatement(statement: string): boolean {
	return /^\s*(?:CREATE|ALTER|DROP)\b/i.test(stripComments(statement));
}

// Only these may run while the migration's earlier async jobs are still
// building. Everything else waits for them first: ADD CONSTRAINT … FOREIGN KEY
// needs the referenced unique index to exist as a constraint, VALIDATE
// CONSTRAINT needs the constraint, and DROP/ALTER of a table with an index in
// flight is undefined territory.
export function mayOverlapAsyncJobs(statement: string): boolean {
	return /^\s*CREATE\s+(?:SCHEMA|TABLE|(?:UNIQUE\s+)?INDEX\s+ASYNC)\b/i.test(
		stripComments(statement),
	);
}

// A statement whose "already exists" on a retry means the failed attempt was
// committed after all (CREATE …, ALTER TABLE … ADD …).
export function isCreateOrAddStatement(statement: string): boolean {
	return /^\s*(?:CREATE\b|ALTER\s+TABLE\s+(?!ASYNC\b)[\s\S]*?\bADD\b)/i.test(
		stripComments(statement),
	);
}

// The index name of a CREATE [UNIQUE] INDEX ASYNC statement as sys.jobs spells
// it in object_name (unquoted; bare identifiers fold to lower case).
export function asyncIndexName(statement: string): string | null {
	const match = stripComments(statement).match(
		/^\s*CREATE\s+(?:UNIQUE\s+)?INDEX\s+ASYNC\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:"((?:[^"]|"")+)"|([A-Za-z_][A-Za-z0-9_$]*))/i,
	);
	if (!match) return null;
	const quoted = match[1];
	if (quoted !== undefined) return quoted.replace(/""/g, '"');
	return (match[2] as string).toLowerCase();
}

// ---------------------------------------------------------------------------
// Migration files and Prisma bookkeeping
// ---------------------------------------------------------------------------

export interface LocalMigration {
	readonly name: string;
	readonly sql: string;
	readonly checksum: string;
}

export interface AppliedRow {
	readonly id: string;
	readonly checksum: string;
	readonly migration_name: string;
	readonly logs: string | null;
	readonly finished_at: Date | string | null;
	readonly rolled_back_at: Date | string | null;
	readonly applied_steps_count: number;
}

// prisma-engines: schema-connector/src/checksum.rs (sha256 of the file bytes, lowercase hex).
export function migrationChecksum(sql: string): string {
	return createHash("sha256").update(sql).digest("hex");
}

// Same directory rules as Prisma's CLI: subdirectories only, sorted by name,
// each read for its migration.sql. Other files (migration_lock.toml,
// schema.snapshot.prisma) are ignored.
export function listLocalMigrations(dir: string): LocalMigration[] {
	if (!existsSync(dir))
		throw new MigrationError(`migrations directory ${dir} does not exist`);
	const names = readdirSync(dir, { withFileTypes: true })
		.filter((entry) => entry.isDirectory())
		.map((entry) => entry.name)
		.sort((a, b) => a.localeCompare(b));
	return names.map((name) => {
		const path = join(dir, name, "migration.sql");
		if (!existsSync(path))
			throw new MigrationError(`${name} has no migration.sql`);
		const sql = readFileSync(path, "utf8");
		return { name, sql, checksum: migrationChecksum(sql) };
	});
}

// prisma-engines: sql-schema-connector/src/flavour/postgres.rs create_migrations_table.
export const PRISMA_MIGRATIONS_TABLE_SQL = `CREATE TABLE IF NOT EXISTS _prisma_migrations (
    id                      VARCHAR(36) PRIMARY KEY NOT NULL,
    checksum                VARCHAR(64) NOT NULL,
    finished_at             TIMESTAMPTZ,
    migration_name          VARCHAR(255) NOT NULL,
    logs                    TEXT,
    rolled_back_at          TIMESTAMPTZ,
    started_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    applied_steps_count     INTEGER NOT NULL DEFAULT 0
)`;
export const PRISMA_MIGRATIONS_COLUMNS = [
	"id",
	"checksum",
	"finished_at",
	"migration_name",
	"logs",
	"rolled_back_at",
	"started_at",
	"applied_steps_count",
] as const;
export const LIST_APPLIED_SQL =
	"SELECT id, checksum, finished_at, migration_name, logs, rolled_back_at, started_at, applied_steps_count FROM _prisma_migrations ORDER BY started_at ASC";
export const RECORD_STARTED_SQL =
	"INSERT INTO _prisma_migrations (id, checksum, started_at, migration_name) VALUES ($1, $2, now(), $3)";
// Every statement is committed; the async jobs may still be building.
export const RECORD_APPLIED_SQL =
	"UPDATE _prisma_migrations SET applied_steps_count = 1 WHERE id = $1";
// The async jobs completed too: the schema this migration describes is usable.
export const RECORD_FINISHED_SQL =
	"UPDATE _prisma_migrations SET finished_at = now(), applied_steps_count = 1 WHERE id = $1";
export const RECORD_FAILED_SQL =
	"UPDATE _prisma_migrations SET logs = $2 WHERE id = $1";

export interface StartedRecord {
	readonly id: string;
	readonly checksum: string;
	readonly migration_name: string;
}

export function startedRecord(migration: LocalMigration): StartedRecord {
	const id = randomUUID();
	if (!UUID_PATTERN.test(id))
		throw new MigrationError("uuid v4 generation failed");
	return { id, checksum: migration.checksum, migration_name: migration.name };
}

// Every statement of the migration is committed (applied_steps_count = 1) and
// no failure was recorded, but the run ended (SIGKILL, wait budget, signal)
// before its async jobs were confirmed. Nothing is wrong with the schema: the
// next run drains sys.jobs, checks the catalog and sets finished_at.
export function awaitingJobs(row: AppliedRow): boolean {
	return (
		row.finished_at === null &&
		row.rolled_back_at === null &&
		row.applied_steps_count >= 1 &&
		row.logs === null
	);
}

// Mirrors prisma migrate deploy's diagnosis: a row without finished_at (and
// not rolled back) is a failed migration that blocks everything - unless it is
// merely awaiting its async jobs; applied rows must exist locally with the
// same checksum; the rest is pending, in local order.
export function pendingMigrations(
	local: readonly LocalMigration[],
	applied: readonly AppliedRow[],
): LocalMigration[] {
	const byName = new Map(local.map((m) => [m.name, m]));
	const done = new Set<string>();
	for (const row of applied) {
		if (row.rolled_back_at !== null) continue;
		if (row.finished_at === null && !awaitingJobs(row)) {
			throw new MigrationError(
				`migration ${row.migration_name} failed earlier (started ${row.id}, no finished_at). Read its logs column, repair the schema by hand, then run \`prisma migrate resolve --rolled-back ${row.migration_name}\` (or --applied) before retrying`,
			);
		}
		const file = byName.get(row.migration_name);
		if (!file) {
			throw new MigrationError(
				`migration ${row.migration_name} is applied to the database but missing locally`,
			);
		}
		if (file.checksum !== row.checksum) {
			throw new MigrationError(
				`migration ${row.migration_name} was modified after it was applied (checksum ${row.checksum} in the database, ${file.checksum} locally)`,
			);
		}
		done.add(row.migration_name);
	}
	return local.filter((m) => !done.has(m.name));
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

const { log, logError } = makeLog(LOG_PREFIX);

// Client-side expiry (timestamptz parameter): a 30-minute lease tolerates the
// clock skew between this task and the cluster, and needs no interval
// arithmetic on the server.
function leaseExpiry(): Date {
	return new Date(Date.now() + LEASE_MINUTES * 60_000);
}

interface PgError extends Error {
	code?: string;
}

export interface LeaseHolder {
	touch(): Promise<void>;
}

class Lease implements LeaseHolder {
	private renewedAt = 0;
	readonly holder: string;
	private readonly session: DsqlSession;

	constructor(session: DsqlSession, holder: string) {
		this.session = session;
		this.holder = holder;
	}

	static holderName(): string {
		return `${hostname()}:${process.pid}:${randomUUID().slice(0, 8)}`;
	}

	async acquire(): Promise<boolean> {
		// One DDL per transaction, DML in its own transaction: three autocommit statements.
		await this.session.run(
			`CREATE TABLE IF NOT EXISTS ${LEASE_TABLE} (id INTEGER PRIMARY KEY NOT NULL, holder TEXT, expires_at TIMESTAMPTZ)`,
		);
		await this.session.run(
			`INSERT INTO ${LEASE_TABLE} (id) VALUES (1) ON CONFLICT (id) DO NOTHING`,
		);
		// OCC: two racing updaters both see the row free; exactly one commit wins,
		// the other gets 40001 and re-reads a row that is now held.
		const result = await this.session.run(
			`UPDATE ${LEASE_TABLE} SET holder = $1, expires_at = $2 WHERE id = 1 AND (holder IS NULL OR expires_at < now())`,
			[this.holder, leaseExpiry()],
		);
		if (result.rowCount !== 1) {
			const current = await this.session.run<{
				holder: string | null;
				expires_at: Date | null;
			}>(`SELECT holder, expires_at FROM ${LEASE_TABLE} WHERE id = 1`);
			const row = current.rows[0];
			logError(
				`migration lease is held by ${row?.holder ?? "unknown"} until ${row?.expires_at ? new Date(row.expires_at).toISOString() : "unknown"}`,
			);
			return false;
		}
		this.renewedAt = Date.now();
		log(
			`acquired migration lease as ${this.holder} for ${LEASE_MINUTES} minutes`,
		);
		return true;
	}

	async touch(): Promise<void> {
		if (Date.now() - this.renewedAt < LEASE_RENEW_MS) return;
		const result = await this.session.run(
			`UPDATE ${LEASE_TABLE} SET expires_at = $2 WHERE id = 1 AND holder = $1`,
			[this.holder, leaseExpiry()],
		);
		if (result.rowCount !== 1) {
			throw new MigrationError(
				"migration lease was lost (expired and taken by another run)",
			);
		}
		this.renewedAt = Date.now();
	}

	async release(): Promise<void> {
		await this.session.run(
			`UPDATE ${LEASE_TABLE} SET holder = NULL, expires_at = NULL WHERE id = 1 AND holder = $1`,
			[this.holder],
		);
		log("released migration lease");
	}
}

interface JobRow {
	job_id: string;
	status: string;
	details: string | null;
	job_type: string | null;
	object_name: string | null;
}

// An accepted retry of CREATE INDEX ASYNC returns no job_id; sys.jobs still
// lists the build under the index's qualified name.
async function recoverJobId(
	session: Executor,
	statement: string,
	label: string,
): Promise<string | null> {
	const index = asyncIndexName(statement);
	if (index !== null) {
		const objectName = `public.${index}`;
		try {
			const found = await session.run<{ job_id: string }>(
				"SELECT job_id FROM sys.jobs WHERE object_name = $1 ORDER BY start_time DESC LIMIT 1",
				[objectName],
				`sys.jobs lookup ${objectName}`,
			);
			const jobId = found.rows[0]?.job_id;
			if (typeof jobId === "string" && jobId !== "") {
				log(`recovered job ${jobId} for ${objectName} from sys.jobs`);
				return jobId;
			}
		} catch (error) {
			log(
				`warning: sys.jobs lookup for ${objectName} failed: ${(error as Error).message}`,
			);
		}
	}
	log(
		`warning: no job_id for ${label}; the cluster-wide drain before the schema check covers it`,
	);
	return null;
}

export async function applyMigration(
	session: Executor,
	lease: LeaseHolder,
	migration: LocalMigration,
	jobWaitTimeoutMs: number,
): Promise<void> {
	const statements = splitStatements(migration.sql);
	const record = startedRecord(migration);
	log(
		`applying ${migration.name}: ${statements.length} statements (record ${record.id})`,
	);
	await session.run(RECORD_STARTED_SQL, [
		record.id,
		record.checksum,
		record.migration_name,
	]);
	const recordFailure = async (logs: string): Promise<void> => {
		await session.run(RECORD_FAILED_SQL, [record.id, logs]).catch((inner) => {
			logError(
				`could not record the failure in _prisma_migrations: ${(inner as Error).message}`,
			);
		});
	};

	const jobIds: string[] = [];
	for (let index = 0; index < statements.length; index++) {
		const statement = statements[index] as string;
		const label = `${migration.name} #${index + 1}/${statements.length}: ${stripComments(statement).trim().replace(/\s+/g, " ").slice(0, 100)}`;
		try {
			if (interruption.signal) {
				throw Object.assign(
					new Error(`interrupted by ${interruption.signal}`),
					{ code: "INTERRUPTED" },
				);
			}
			if (jobIds.length > 0 && !mayOverlapAsyncJobs(statement)) {
				log(`  ${label} waits for ${jobIds.length} pending job(s) first`);
				await waitForJobs(session, lease, jobIds.splice(0), jobWaitTimeoutMs);
			}
			const result = await session.run<{ job_id?: string }>(
				statement,
				[],
				label,
				{
					acceptOnRetry: (error) =>
						isCreateOrAddStatement(statement) && isAlreadyExistsError(error),
				},
			);
			if (isAsyncJobStatement(statement)) {
				const returned = result.rows[0]?.job_id;
				const jobId =
					typeof returned === "string" && returned !== ""
						? returned
						: await recoverJobId(session, statement, label);
				if (jobId !== null) jobIds.push(jobId);
			}
		} catch (error) {
			const e = error as PgError;
			await recordFailure(
				`statement ${index + 1}/${statements.length} failed (${e.code ?? "no SQLSTATE"}): ${e.message}\n${statement}`,
			);
			throw new MigrationError(
				`${label} failed: ${e.message}. Earlier statements stay committed (DSQL DDL is not transactional); repair by hand, then \`prisma migrate resolve --rolled-back ${migration.name}\` and re-run`,
			);
		}
		if ((index + 1) % 50 === 0) {
			log(`  ${index + 1}/${statements.length} statements applied`);
			await lease.touch();
		}
	}

	await session.run(RECORD_APPLIED_SQL, [record.id]);
	log(
		`${migration.name}: all ${statements.length} statements committed; confirming ${jobIds.length} async job(s)`,
	);
	try {
		if (jobIds.length > 0)
			await waitForJobs(session, lease, jobIds, jobWaitTimeoutMs);
	} catch (error) {
		// A failed job needs a human and blocks the next run; a wait that was
		// merely cut short leaves the row resumable (see awaitingJobs).
		if (!(error instanceof JobsPendingError))
			await recordFailure((error as Error).message);
		throw error;
	}
	await session.run(RECORD_FINISHED_SQL, [record.id]);
	log(`recorded ${migration.name} as applied`);
}

type WaitStrategy = "call" | "select" | "poll";

// Resolves with the outcome of `work`, or with the stop reason as soon as one
// appears while `work` is still blocked.
async function raceWithWatchdog<T>(
	work: Promise<T>,
	stopReason: () => string | null,
): Promise<T | string> {
	let timer: ReturnType<typeof setInterval> | undefined;
	const watchdog = new Promise<string>((resolve) => {
		timer = setInterval(() => {
			const reason = stopReason();
			if (reason !== null) resolve(reason);
		}, WATCHDOG_MS);
	});
	try {
		return await Promise.race([work, watchdog]);
	} finally {
		clearInterval(timer);
	}
}

export async function waitForJobs(
	session: Executor,
	lease: LeaseHolder,
	jobIds: readonly string[],
	timeoutMs: number,
): Promise<void> {
	const pending = new Set(jobIds);
	const failed: JobRow[] = [];
	const deadline = Date.now() + timeoutMs;
	let strategy: WaitStrategy = "call";
	let lastReport = 0;
	const stopReason = (): string | null => {
		if (interruption.signal) return `interrupted by ${interruption.signal}`;
		if (Date.now() >= deadline)
			return `gave up after ${Math.round(timeoutMs / 1000)} s (${JOB_WAIT_TIMEOUT_ENV})`;
		return null;
	};
	const pendingError = (reason: string): JobsPendingError =>
		new JobsPendingError(
			`${reason} with ${pending.size} async job(s) still pending: ${[...pending].join(", ")}. They keep running on the cluster; the next run waits for them before checking the schema`,
		);
	log(`waiting for ${pending.size} async index/validation job(s)`);

	while (pending.size > 0) {
		const reason = stopReason();
		if (reason !== null) throw pendingError(reason);
		await lease.touch();
		const inProgress = await session.run<JobRow>(
			"SELECT job_id, status, details, job_type, object_name FROM sys.jobs WHERE status IN ('submitted', 'processing')",
		);
		const all = await session.run<JobRow>(
			"SELECT job_id, status, details, job_type, object_name FROM sys.jobs",
		);
		const known = new Map(all.rows.map((row) => [row.job_id, row]));
		for (const id of [...pending]) {
			const row = known.get(id);
			if (!row) {
				// Completed rows are purged after 30 minutes; the index/constraint checks below cover this case.
				pending.delete(id);
			} else if (row.status === "completed") {
				pending.delete(id);
			} else if (row.status === "failed") {
				failed.push(row);
				pending.delete(id);
			}
		}
		if (Date.now() - lastReport > 30_000) {
			log(
				`  ${pending.size} of ours pending, ${inProgress.rowCount ?? 0} jobs in progress on the cluster`,
			);
			lastReport = Date.now();
		}
		if (pending.size === 0) break;

		const next = pending.values().next().value as string;
		if (strategy === "poll") {
			await sleep(JOB_POLL_MS);
			continue;
		}
		// sys.wait_for_job holds the connection until the job ends, so the
		// signal and the deadline are watched from the side; closing the
		// session is what unblocks the call. A single attempt: the loop itself
		// re-checks and re-issues after a transient error.
		const outcome = await raceWithWatchdog(
			session
				.run(
					strategy === "call"
						? "CALL sys.wait_for_job($1)"
						: "SELECT sys.wait_for_job($1)",
					[next],
					`wait_for_job ${next}`,
					{ maxAttempts: 1 },
				)
				.then(
					() => null,
					(error: unknown) =>
						error instanceof Error ? error : new Error(String(error)),
				),
			stopReason,
		);
		if (typeof outcome === "string") {
			await session.close();
			throw pendingError(outcome);
		}
		if (outcome === null || isTransientError(outcome)) continue;
		if (strategy === "call") {
			strategy = "select";
		} else {
			log(
				`sys.wait_for_job unavailable (${(outcome as PgError).code ?? "error"}: ${outcome.message}); polling sys.jobs`,
			);
			strategy = "poll";
		}
	}

	if (failed.length > 0) {
		for (const row of failed) {
			logError(
				`job ${row.job_id} (${row.job_type ?? "?"} ${row.object_name ?? "?"}) failed: ${row.details ?? "no details"}`,
			);
		}
		throw new MigrationError(
			`${failed.length} async DDL job(s) failed: ${failed.map((row) => `${row.job_id} ${row.object_name ?? "?"}: ${row.details ?? "no details"}`).join("; ")}. For a unique index: drop it, remove the duplicates, recreate it; for a constraint: fix the violating rows and re-run ALTER TABLE ASYNC … VALIDATE CONSTRAINT`,
		);
	}
	log("all async jobs completed");
}

// Jobs left behind by a run that ended mid-wait (or whose ids were never
// captured) would otherwise show up as INVALID indexes in the schema check.
// One cheap SELECT when the cluster is idle.
export async function drainClusterJobs(
	session: Executor,
	lease: LeaseHolder,
	timeoutMs: number,
): Promise<void> {
	const inProgress = await session.run<{ job_id: string }>(
		"SELECT job_id FROM sys.jobs WHERE status IN ('submitted', 'processing')",
	);
	if ((inProgress.rowCount ?? 0) === 0) return;
	log(
		`${inProgress.rowCount} job(s) from an earlier run are still in progress on the cluster`,
	);
	await waitForJobs(
		session,
		lease,
		inProgress.rows.map((row) => row.job_id),
		timeoutMs,
	);
}

async function assertSchemaValid(session: Executor): Promise<void> {
	const invalidIndexes = await session.run<{ relname: string }>(
		"SELECT c.relname FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid WHERE NOT i.indisvalid",
	);
	if ((invalidIndexes.rowCount ?? 0) > 0) {
		throw new MigrationError(
			`${invalidIndexes.rowCount} index(es) are INVALID: ${invalidIndexes.rows.map((r) => r.relname).join(", ")}. Drop and recreate them (see sys.jobs details)`,
		);
	}
	log("pg_index: every index is valid");
	try {
		const unvalidated = await session.run<{ conname: string }>(
			"SELECT conname FROM pg_constraint WHERE contype = 'f' AND NOT convalidated",
		);
		if ((unvalidated.rowCount ?? 0) > 0) {
			throw new MigrationError(
				`${unvalidated.rowCount} foreign key(s) are still NOT VALID: ${unvalidated.rows.map((r) => r.conname).join(", ")}`,
			);
		}
		log("pg_constraint: every foreign key is validated");
	} catch (error) {
		if (error instanceof MigrationError) throw error;
		log(
			`warning: pg_constraint check unavailable on this cluster: ${(error as Error).message}`,
		);
	}
}

// Design §3: the runtime Lambdas connect as a dedicated role bound to their IAM
// role with dsql:DbConnect; admin (dsql:DbConnectAdmin) stays with this job.
export function grantStatements(config: MigrationConfig): string[] {
	const role = config.runtimeDbRole;
	return [
		`GRANT USAGE ON SCHEMA public TO ${role}`,
		`GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO ${role}`,
		`GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO ${role}`,
		`ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${role}`,
		`ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO ${role}`,
	];
}

async function grantRuntimeRole(
	session: Executor,
	config: MigrationConfig,
	runtimeRoleArn: string,
): Promise<void> {
	const role = config.runtimeDbRole;
	const exists = await session.run(
		"SELECT 1 FROM pg_roles WHERE rolname = $1",
		[role],
	);
	if ((exists.rowCount ?? 0) === 0) {
		await session.run(`CREATE ROLE ${role} WITH LOGIN`);
		log(`created database role ${role}`);
	}
	const mapped = await session.run(
		"SELECT 1 FROM sys.iam_pg_role_mappings WHERE pg_role_name = $1 AND arn = $2",
		[role, runtimeRoleArn],
	);
	if ((mapped.rowCount ?? 0) === 0) {
		await session.run(`AWS IAM GRANT ${role} TO '${runtimeRoleArn}'`);
		log(`granted ${runtimeRoleArn} the database role ${role}`);
	}
	for (const statement of grantStatements(config)) await session.run(statement);
	log(
		`privileges on schema public granted to ${role} (existing and future tables)`,
	);
}

async function prismaMigrateStatus(
	config: MigrationConfig,
	session: DsqlSession,
): Promise<number> {
	const token = await session.token();
	const databaseUrl = composeDatabaseUrl(config, token);
	const command = [
		"bunx",
		"--bun",
		"prisma",
		"migrate",
		"status",
		"--config",
		"prisma.dsql.config.ts",
	];
	log(
		`verifying with: ${command.join(" ")} (DATABASE_URL=${redactDatabaseUrl(databaseUrl)})`,
	);
	let exitCode = 1;
	for (let attempt = 1; attempt <= 3; attempt++) {
		const proc = Bun.spawn(command, {
			cwd: import.meta.dir,
			env: {
				...process.env,
				DATABASE_URL: databaseUrl,
				PRISMA_SCHEMA_DISABLE_ADVISORY_LOCK: "1",
				PRISMA_HIDE_UPDATE_MESSAGE: "1",
				CHECKPOINT_DISABLE: "1",
			},
			stdin: "ignore",
			stdout: "pipe",
			stderr: "pipe",
		});
		const [stdout, stderr] = await Promise.all([
			new Response(proc.stdout).text(),
			new Response(proc.stderr).text(),
		]);
		exitCode = await proc.exited;
		process.stdout.write(redactSecret(stdout, token));
		process.stderr.write(redactSecret(stderr, token));
		if (exitCode === 0 || !/OC00[01]|40001/.test(stdout + stderr)) break;
		log(
			`prisma migrate status hit an OCC error (attempt ${attempt}/3); retrying`,
		);
		await sleep(3_000);
	}
	return exitCode;
}

async function main(): Promise<number> {
	let config: MigrationConfig;
	try {
		config = parseConfig(process.env);
	} catch (error) {
		if (error instanceof ConfigError) {
			logError(`refusing to run: ${error.message}`);
			return 2;
		}
		throw error;
	}
	const grantPlan =
		config.runtimeRoleArn === null
			? `no runtime role grant (${RUNTIME_ROLE_ARN_ENV} unset)`
			: `runtime role ${config.runtimeDbRole} ← ${config.runtimeRoleArn}`;
	log(
		`target ${ADMIN_USER}@${config.endpoint} (${config.region}); migrations from ${config.migrationsDir}; ${grantPlan}; wait budget ${config.jobWaitTimeoutMs / 1000} s per drain`,
	);

	const local = listLocalMigrations(config.migrationsDir);
	log(
		`${local.length} local migration(s): ${local.map((m) => m.name).join(", ") || "none"}`,
	);

	const signer = new DsqlSigner({
		hostname: config.endpoint,
		region: config.region,
		expiresIn: TOKEN_EXPIRES_IN_SECONDS,
	});
	const session = new DsqlSession(config, signer, APPLICATION_NAME, {
		log,
		logError,
	});
	const lease = new Lease(session, Lease.holderName());
	let held = false;
	const onSignal = (signal: NodeJS.Signals) => {
		interruption.signal = signal;
		logError(
			`received ${signal}; finishing the current statement, then recording the migration as failed and releasing the lease`,
		);
	};
	process.once("SIGTERM", onSignal);
	process.once("SIGINT", onSignal);

	try {
		await session.connect();
		held = await lease.acquire();
		if (!held) return 3;

		await session.run(PRISMA_MIGRATIONS_TABLE_SQL);
		const applied = await session.run<AppliedRow>(LIST_APPLIED_SQL);
		const pending = pendingMigrations(local, applied.rows);
		const unconfirmed = applied.rows.filter(awaitingJobs);
		log(
			`${applied.rowCount ?? 0} row(s) in _prisma_migrations; ${pending.length} pending; ${unconfirmed.length} applied but not yet confirmed`,
		);
		for (const row of unconfirmed) {
			log(
				`${row.migration_name} (record ${row.id}) was applied by a run that ended before its async jobs were confirmed`,
			);
		}

		await drainClusterJobs(session, lease, config.jobWaitTimeoutMs);
		for (const migration of pending) {
			if (interruption.signal)
				throw new MigrationError(
					`interrupted by ${interruption.signal} before ${migration.name}`,
				);
			await applyMigration(session, lease, migration, config.jobWaitTimeoutMs);
		}
		await drainClusterJobs(session, lease, config.jobWaitTimeoutMs);
		await assertSchemaValid(session);
		for (const row of unconfirmed) {
			await session.run(RECORD_FINISHED_SQL, [row.id]);
			log(`recorded ${row.migration_name} as applied`);
		}
		if (config.runtimeRoleArn === null) {
			log(
				`warning: ${RUNTIME_ROLE_ARN_ENV} is not set; skipping the runtime role grant - only admin can connect until it is set and the job runs again`,
			);
		} else {
			await grantRuntimeRole(session, config, config.runtimeRoleArn);
		}

		const status = await prismaMigrateStatus(config, session);
		if (status !== 0) {
			logError(
				`prisma migrate status exited with ${status}; the recorded history does not match the files`,
			);
			return 1;
		}
		log(
			pending.length === 0
				? "already up to date"
				: `applied ${pending.length} migration(s)`,
		);
		return 0;
	} catch (error) {
		const e = error as Error;
		logError(
			e instanceof MigrationError
				? e.message
				: `unexpected failure: ${e.message}`,
		);
		return 1;
	} finally {
		process.off("SIGTERM", onSignal);
		process.off("SIGINT", onSignal);
		if (held)
			await lease
				.release()
				.catch((e) =>
					logError(`lease release failed: ${(e as Error).message}`),
				);
		await session.close();
	}
}

if (import.meta.main) {
	main().then(
		(code) => process.exit(code),
		(error) => {
			logError(
				`unexpected failure: ${(error as Error)?.message ?? String(error)}`,
			);
			process.exit(1);
		},
	);
}
