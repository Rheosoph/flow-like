// Pieces the Aurora DSQL jobs in this directory share: the environment contract
// flow_like_aws_data::dsql enforces (packages/aws-data/src/dsql.rs), redaction,
// the OCC/reconnect retry loop DSQL's commit-time conflicts and 60-minute
// connection limit require, and the admin session built on an IAM token.
//
// No password exists anywhere on a DSQL cluster: the token minted by
// @aws-sdk/dsql-signer becomes the PostgreSQL password of connections that live
// only inside the process that minted it.

import type pg from "pg";
import pgDriver from "pg";

export const ADMIN_USER = "admin";
export const DATABASE = "postgres";
export const PORT = 5432;
export const TOKEN_EXPIRES_IN_SECONDS = 900;
// DSQL closes connections after 60 minutes; reconnect well before that.
export const CONNECTION_MAX_AGE_MS = 50 * 60_000;
export const MAX_ATTEMPTS = 8;

export const ENDPOINT_ENV = "DSQL_CLUSTER_ENDPOINT";
export const REGION_ENV = "DSQL_REGION";

// Verbatim flow_like_aws_data::dsql::FORBIDDEN_SETTINGS: static database
// credentials and libpq's own connection sources are refused, even when empty,
// because any of them could redirect where the token goes or replace it with a
// password.
export const FORBIDDEN_SETTINGS = [
	"DATABASE_URL",
	"PGPASSWORD",
	"PGPASSFILE",
	"PGSERVICE",
	"PGSERVICEFILE",
	"PGHOST",
	"PGHOSTADDR",
	"PGPORT",
	"PGUSER",
	"PGDATABASE",
	"PGSSLMODE",
	"PGSSLROOTCERT",
	"PGSSLCERT",
	"PGSSLKEY",
	"PGOPTIONS",
] as const;

export const ENDPOINT_PATTERN =
	/^([a-z0-9]{1,63})\.dsql\.([a-z]{2}(?:-[a-z]+)+-\d)\.on\.aws$/;

export type Environment = Record<string, string | undefined>;

export class ConfigError extends Error {
	override readonly name = "ConfigError";
}

export class MigrationError extends Error {
	override readonly name: string = "MigrationError";
}

export interface DsqlTarget {
	readonly endpoint: string;
	readonly region: string;
}

export function invalid(name: string, reason: string): never {
	throw new ConfigError(`invalid ${name}: ${reason}`);
}

export interface EnvReader {
	optional(name: string): string | undefined;
	required(name: string): string;
}

export function envReader(env: Environment): EnvReader {
	const optional = (name: string): string | undefined => {
		const value = env[name];
		if (value === undefined) return undefined;
		if (value === "" || value.trim() !== value) {
			invalid(name, "must be non-empty and have no surrounding whitespace");
		}
		return value;
	};
	return {
		optional,
		required(name: string): string {
			const value = optional(name);
			if (value === undefined) {
				throw new ConfigError(`missing required environment variable ${name}`);
			}
			return value;
		},
	};
}

export function assertNoForbiddenSettings(env: Environment): void {
	for (const name of FORBIDDEN_SETTINGS) {
		if (env[name] !== undefined) {
			throw new ConfigError(
				`${name} is forbidden: Aurora DSQL uses IAM tokens, never a static password or libpq connection source`,
			);
		}
	}
}

// The endpoint carries the region, so a DSQL_REGION that disagrees with it is a
// misconfiguration rather than an override.
export function parseDsqlTarget(env: Environment): DsqlTarget {
	assertNoForbiddenSettings(env);
	const read = envReader(env);
	const endpoint = read.required(ENDPOINT_ENV);
	const match = endpoint.match(ENDPOINT_PATTERN);
	if (!match) {
		invalid(
			ENDPOINT_ENV,
			"must be the bare cluster endpoint <id>.dsql.<region>.on.aws (no scheme, port, or path)",
		);
	}
	const derivedRegion = match[2] as string;
	const region = read.optional(REGION_ENV) ?? derivedRegion;
	if (region !== derivedRegion) {
		invalid(REGION_ENV, `must match the endpoint's region ${derivedRegion}`);
	}
	return { endpoint, region };
}

// Prisma's schema engine (quaint) parses PostgreSQL URLs itself: `sslmode`
// knows only disable|prefer|require and silently falls back to `prefer` for
// anything else, and certificate verification is the separate `sslaccept`
// switch. `sslmode=require&sslaccept=strict` is quaint's spelling of
// verify-full. Only a Prisma child process ever receives this URL; the jobs'
// own connections are built from `clientConfig`.
export function composeDatabaseUrl(
	target: DsqlTarget,
	token: string,
	applicationName: string,
): string {
	const params = new URLSearchParams({
		sslmode: "require",
		sslaccept: "strict",
		connect_timeout: "15",
		application_name: applicationName,
	});
	return `postgresql://${ADMIN_USER}:${encodeURIComponent(token)}@${target.endpoint}:${PORT}/${DATABASE}?${params}`;
}

// The userinfo ends at the LAST `@` of the authority, not the first: a password
// may legally contain an unencoded `@`, and stopping at the first one would
// print the rest of it verbatim.
export function redactDatabaseUrl(url: string): string {
	const match = url.match(/^(postgres(?:ql)?:\/\/)([^/?#]*)([\s\S]*)$/i);
	if (!match) return url;
	const [, scheme = "", authority = "", rest = ""] = match;
	const at = authority.lastIndexOf("@");
	if (at === -1) return url;
	const userinfo = authority.slice(0, at);
	const colon = userinfo.indexOf(":");
	if (colon === -1) return url;
	return `${scheme}${userinfo.slice(0, colon)}:***${authority.slice(at)}${rest}`;
}

export function redactSecret(text: string, secret: string): string {
	return secret.length === 0 ? text : text.split(secret).join("***");
}

// Every secret the jobs hold is registered here once and scrubbed out of every
// line makeLog emits, so a driver or proxy error that echoes a connection
// string cannot put a password on stdout, stderr or into a log collector.
const logSecrets = new Set<string>();

export function addLogSecret(secret: string | null | undefined): void {
	if (typeof secret === "string" && secret.length > 0) logSecrets.add(secret);
}

export function scrubSecrets(text: string): string {
	let scrubbed = text;
	for (const secret of logSecrets) scrubbed = redactSecret(scrubbed, secret);
	return scrubbed;
}

// verify-full for the jobs' own connections: node-postgres verifies the chain
// against the system CA store and sets `servername` to the host for SNI and
// hostname checking whenever `ssl` is an options object.
export function clientConfig(
	target: DsqlTarget,
	token: string,
	applicationName: string,
): pg.ClientConfig {
	return {
		host: target.endpoint,
		port: PORT,
		user: ADMIN_USER,
		password: token,
		database: DATABASE,
		ssl: { rejectUnauthorized: true },
		application_name: applicationName,
		connectionTimeoutMillis: 15_000,
		keepAlive: true,
	};
}

export interface Logger {
	log(message: string): void;
	logError(message: string): void;
}

export function makeLog(prefix: string): Logger {
	return {
		log: (message: string) => console.log(`${prefix} ${scrubSecrets(message)}`),
		logError: (message: string) =>
			console.error(`${prefix} ${scrubSecrets(message)}`),
	};
}

// These two settings are the timestamp conversion, not formatting preferences.
//
// The source's date/time columns are `timestamp without time zone` holding UTC
// instants; the target's are `timestamptz(3)`. The sync carries every one of
// them as TEXT (see sourceExpression), which makes the conversion a pair of
// server-side steps that these settings decide:
//
//   render (source)  `"createdAt"::text` -> `2026-09-04 08:18:44.123`.
//                    DateStyle picks that layout; `German` would render
//                    `04.09.2026 08:18:44.123` instead. TimeZone does not move
//                    a plain `timestamp`, but it does decide the offset printed
//                    for any `timestamptz`/`timetz` column, which the same
//                    `::text` rule covers.
//   parse  (target)  that literal carries NO offset, so PostgreSQL resolves it
//                    against the session's TimeZone on the way into
//                    `timestamptz`. UTC means the instant is preserved; any
//                    other zone silently moves every timestamp in the database
//                    by that zone's offset — no error, no failed row.
//
// Because the failure mode is silent, issuing them is not enough: both sessions
// read the effective values back and refuse to proceed if they did not take.
export const SESSION_SETTINGS = [
	"SET TimeZone = 'UTC'",
	"SET DateStyle = 'ISO, YMD'",
] as const;

export const SESSION_SETTINGS_CHECK =
	"SELECT current_setting('TimeZone') AS time_zone, current_setting('DateStyle') AS date_style";

// `Etc/UTC` is the same zone under the name some servers echo back.
export const REQUIRED_TIME_ZONE = /^(etc\/)?utc$/i;
// DateStyle is reported as an output format and an input field order; both
// halves have to be the ones that were asked for.
export const REQUIRED_DATE_STYLE = /^ISO,\s*YMD$/i;

// Returns the reason the session is unusable, or null when it is safe.
export function sessionSettingsProblem(
	timeZone: string,
	dateStyle: string,
): string | null {
	if (!REQUIRED_TIME_ZONE.test(timeZone.trim())) {
		return `TimeZone is ${timeZone}, not UTC; every offset-less timestamp literal would be read as ${timeZone} and stored at the wrong instant`;
	}
	if (!REQUIRED_DATE_STYLE.test(dateStyle.trim())) {
		return `DateStyle is ${dateStyle}, not ISO, YMD; timestamps would be rendered and parsed in an ambiguous layout`;
	}
	return null;
}

interface SessionSettingsRow extends pg.QueryResultRow {
	time_zone: string;
	date_style: string;
}

// `where` names the side in the error, because the two sessions fail for
// different reasons: the source renders wrong, the target parses wrong.
export async function assertSessionSettings(
	client: pg.Client,
	where: string,
): Promise<void> {
	const result = await client.query<SessionSettingsRow>(SESSION_SETTINGS_CHECK);
	const row = result.rows[0];
	if (!row) {
		throw new MigrationError(
			`${where}: could not read back TimeZone/DateStyle; refusing to move timestamps blind`,
		);
	}
	const problem = sessionSettingsProblem(row.time_zone, row.date_style);
	if (problem) throw new MigrationError(`${where}: ${problem}`);
}

export function sleep(ms: number): Promise<void> {
	return new Promise((done) => setTimeout(done, ms));
}

// Set by the signal handlers in each job's main(); read between statements and,
// while a blocking wait holds the session, by a watchdog.
export const interruption: { signal: NodeJS.Signals | null } = { signal: null };

interface PgError extends Error {
	code?: string;
}

// DSQL reports OCC conflicts as SQLSTATE 40001 with OC000/OC001 in the message;
// OC001 also fires on the first statement of a session after a schema change.
// Connection loss (60-minute limit, idle close) is retried after a reconnect.
export function isTransientError(error: unknown): boolean {
	const e = error as PgError;
	if (!e) return false;
	if (e.code === "40001") return true;
	if (/\bOC00[01]\b/.test(e.message ?? "")) return true;
	return isConnectionError(error);
}

export function isConnectionError(error: unknown): boolean {
	const e = error as PgError;
	if (!e) return false;
	return (
		e.code === "57P01" ||
		e.code === "08006" ||
		e.code === "08003" ||
		e.code === "ECONNRESET" ||
		e.code === "EPIPE" ||
		/connection terminated|client has encountered a connection error|closed the connection/i.test(
			e.message ?? "",
		)
	);
}

// duplicate_table (also indexes), duplicate_object (constraints), duplicate_column.
export function isAlreadyExistsError(error: unknown): boolean {
	const e = error as PgError;
	if (!e) return false;
	return e.code === "42P07" || e.code === "42710" || e.code === "42701";
}

export interface RunOptions {
	readonly maxAttempts?: number;
	// Consulted on the second and later attempts only: an error the first
	// attempt could not have produced if it had not been committed after all.
	readonly acceptOnRetry?: (error: unknown) => boolean;
}

export interface RetryOptions extends RunOptions {
	readonly onTransient?: (error: unknown) => Promise<void>;
	readonly pause?: (ms: number) => Promise<void>;
	readonly log?: (message: string) => void;
}

export const ACCEPTED: unique symbol = Symbol("accepted");

// Runs `attempt` until it succeeds. OCC conflicts and connection loss are
// retried with jittered backoff. DSQL can commit a DDL statement and still
// report OC001 for it, so a retry may hit "already exists": `acceptOnRetry`
// turns that into ACCEPTED (with a warning) instead of a failure.
export async function withRetries<T>(
	label: string,
	attempt: () => Promise<T>,
	options: RetryOptions = {},
): Promise<T | typeof ACCEPTED> {
	const maxAttempts = options.maxAttempts ?? MAX_ATTEMPTS;
	const pause = options.pause ?? sleep;
	const note = options.log ?? (() => undefined);
	for (let n = 1; ; n++) {
		try {
			return await attempt();
		} catch (error) {
			const e = error as PgError;
			if (n > 1 && options.acceptOnRetry?.(error)) {
				note(
					`warning: accepted ${e.code ?? "error"} "${e.message}" on attempt ${n} as already applied by attempt ${n - 1} — ${label}`,
				);
				return ACCEPTED;
			}
			if (n >= maxAttempts || !isTransientError(error)) throw error;
			note(
				`retrying (${n}/${maxAttempts}) after ${e.code ?? "error"}: ${e.message} — ${label}`,
			);
			await options.onTransient?.(error);
			await pause(Math.min(200 * 2 ** n, 5_000) * (0.5 + Math.random()));
		}
	}
}

export function emptyResult<R extends pg.QueryResultRow>(): pg.QueryResult<R> {
	return { command: "", rowCount: 0, oid: 0, fields: [], rows: [] };
}

// What the jobs need from a connection; DsqlSession is the real one, the tests
// substitute a scripted one.
export interface Executor {
	run<R extends pg.QueryResultRow = pg.QueryResultRow>(
		sql: string,
		values?: unknown[],
		label?: string,
		options?: RunOptions,
	): Promise<pg.QueryResult<R>>;
	close(): Promise<void>;
}

export interface TokenSource {
	getDbConnectAdminAuthToken(): Promise<string>;
}

// An admin connection that mints a fresh IAM token whenever it (re)connects, so
// neither the 15-minute token lifetime nor the 60-minute connection limit ever
// reaches a caller.
export class DsqlSession implements Executor {
	private client: pg.Client | null = null;
	private openedAt = 0;
	private readonly target: DsqlTarget;
	private readonly signer: TokenSource;
	private readonly applicationName: string;
	private readonly logger: Logger;

	constructor(
		target: DsqlTarget,
		signer: TokenSource,
		applicationName: string,
		logger: Logger,
	) {
		this.target = target;
		this.signer = signer;
		this.applicationName = applicationName;
		this.logger = logger;
	}

	async token(): Promise<string> {
		return this.signer.getDbConnectAdminAuthToken();
	}

	async connect(): Promise<void> {
		await this.close();
		const token = await this.token();
		const client = new pgDriver.Client(
			clientConfig(this.target, token, this.applicationName),
		);
		client.on("error", (error) =>
			this.logger.logError(`connection error: ${error.message}`),
		);
		// A client that connected but could not be configured is ended here:
		// leaving it to the caller's `finally` would orphan a backend, and the
		// retry loop would orphan one more on every attempt.
		try {
			await client.connect();
			// This is the writing side, so a wrong TimeZone here does not fail a
			// row — it stores every offset-less timestamp literal at the wrong
			// instant. A refused SET used to be logged and carried on; it is now
			// fatal, and the effective values are read back in case a server-side
			// default or a connection-time option overrode them anyway.
			for (const statement of SESSION_SETTINGS) await client.query(statement);
			await assertSessionSettings(client, "target session");
		} catch (error) {
			await client.end().catch(() => undefined);
			throw error;
		}
		this.client = client;
		this.openedAt = Date.now();
		this.logger.log(
			`connected to ${ADMIN_USER}@${this.target.endpoint}:${PORT}/${DATABASE} (verify-full)`,
		);
	}

	async ensureFresh(): Promise<pg.Client> {
		if (!this.client || Date.now() - this.openedAt > CONNECTION_MAX_AGE_MS) {
			if (this.client) {
				this.logger.log(
					"connection is approaching DSQL's 60-minute limit; reconnecting",
				);
			}
			await this.connect();
		}
		return this.client as pg.Client;
	}

	async close(): Promise<void> {
		const client = this.client;
		this.client = null;
		if (client) await client.end().catch(() => undefined);
	}

	// One statement, autocommit, retried on OCC conflicts and connection loss.
	// An accepted retry (see withRetries) yields an empty result.
	async run<R extends pg.QueryResultRow = pg.QueryResultRow>(
		sql: string,
		values: unknown[] = [],
		label = sql.slice(0, 80),
		options: RunOptions = {},
	): Promise<pg.QueryResult<R>> {
		const outcome = await this.attempt(
			label,
			async (client) =>
				values.length === 0
					? await client.query<R>(sql)
					: await client.query<R>(sql, values),
			options,
		);
		return outcome === ACCEPTED ? emptyResult<R>() : outcome;
	}

	// A transaction is retried as a whole because DSQL surfaces the conflict at
	// COMMIT: by then every statement of the attempt has already been sent.
	async transaction<T>(
		label: string,
		work: (client: pg.Client) => Promise<T>,
		options: RunOptions = {},
	): Promise<T | typeof ACCEPTED> {
		return this.attempt(
			label,
			async (client) => {
				await client.query("BEGIN");
				try {
					const value = await work(client);
					await client.query("COMMIT");
					return value;
				} catch (error) {
					await client.query("ROLLBACK").catch(() => undefined);
					throw error;
				}
			},
			options,
		);
	}

	private async attempt<T>(
		label: string,
		work: (client: pg.Client) => Promise<T>,
		options: RunOptions,
	): Promise<T | typeof ACCEPTED> {
		return withRetries(
			label,
			async () => {
				const client = await this.ensureFresh();
				return work(client);
			},
			{
				...options,
				log: this.logger.log,
				onTransient: async (error) => {
					if (isConnectionError(error)) await this.close();
				},
			},
		);
	}
}
