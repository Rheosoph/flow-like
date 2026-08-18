#!/usr/bin/env bun
// One-shot `prisma db push` against Azure Database for PostgreSQL Flexible
// Server as the migration user-assigned managed identity.
//
// The environment contract, the validation of every value and the forbidden
// list are the ones flow_like_azure_data::postgres enforces for the Azure API
// and queue workers (packages/azure-data/src/postgres.rs), so a job revision
// cannot be configured in a way its siblings would refuse. There is no
// password anywhere: one Entra token for the ossrdbms scope becomes the
// PostgreSQL password of a single connection URL that exists only in this
// process and in the environment of the Prisma child process. It is never
// logged and never written to disk.
//
// `--accept-data-loss` is intentionally not passed. See the Dockerfile header
// for what Prisma does instead and what the operator does then.

import { ManagedIdentityCredential } from "@azure/identity";

const POSTGRES_SCOPE = "https://ossrdbms-aad.database.windows.net/.default";
const AZURE_POSTGRES_SUFFIX = ".postgres.database.azure.com";
const APPLICATION_NAME = "flow-like-azure-migration";
const SCHEMA_DIR = "prisma/schema";
const LOG_PREFIX = "[azure-migration]";

const REQUIRED_SETTINGS = [
	"AZURE_POSTGRES_AUTH_MODE",
	"AZURE_POSTGRES_HOST",
	"AZURE_POSTGRES_DATABASE",
	"AZURE_POSTGRES_USER",
	"AZURE_CLIENT_ID",
	"IDENTITY_ENDPOINT",
	"IDENTITY_HEADER",
] as const;

// Verbatim flow_like_azure_data::postgres::FORBIDDEN_SETTINGS. Static database
// credentials, libpq's own connection sources, alternate identity endpoints and
// proxies are all refused - even when set to an empty string - because any of
// them could redirect where the token goes or replace it with a password.
const FORBIDDEN_SETTINGS = [
	"DATABASE_URL",
	"AZURE_POSTGRES_PASSWORD",
	"AZURE_POSTGRES_CONNECTION_STRING",
	"AZURE_POSTGRESQL_CONNECTIONSTRING",
	"POSTGRES_PASSWORD",
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
	"PGAPPNAME",
	"MSI_ENDPOINT",
	"MSI_SECRET",
	"IMDS_ENDPOINT",
	"IDENTITY_SERVER_THUMBPRINT",
	"AZURE_AUTHORITY_HOST",
	"HTTP_PROXY",
	"HTTPS_PROXY",
	"ALL_PROXY",
	"http_proxy",
	"https_proxy",
	"all_proxy",
] as const;

const UUID_PATTERN =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export type Environment = Record<string, string | undefined>;

export interface MigrationConfig {
	readonly host: string;
	readonly database: string;
	readonly user: string;
	readonly clientId: string;
}

export class ConfigError extends Error {
	override readonly name = "ConfigError";
}

function invalid(name: string, reason: string): never {
	throw new ConfigError(`invalid ${name}: ${reason}`);
}

function isLoopbackHost(host: string): boolean {
	if (host.toLowerCase() === "localhost") return true;
	const bare = host.replace(/^\[|\]$/g, "");
	if (bare === "::1") return true;
	const v4 = bare.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
	return (
		v4 !== null && v4[1] === "127" && v4.slice(1).every((o) => Number(o) <= 255)
	);
}

function validateAzurePostgresHost(host: string): void {
	if (!host.endsWith(AZURE_POSTGRES_SUFFIX)) {
		invalid("AZURE_POSTGRES_HOST", `must end in ${AZURE_POSTGRES_SUFFIX}`);
	}
	const privateLabels = host.slice(0, -AZURE_POSTGRES_SUFFIX.length);
	const valid =
		host.length <= 253 &&
		privateLabels.length > 0 &&
		privateLabels
			.split(".")
			.every(
				(label) =>
					label.length >= 1 &&
					label.length <= 63 &&
					/^[a-z0-9-]+$/.test(label) &&
					!label.startsWith("-") &&
					!label.endsWith("-"),
			);
	if (!valid) {
		invalid(
			"AZURE_POSTGRES_HOST",
			"must be a lowercase Azure PostgreSQL DNS name without a scheme, port, or path",
		);
	}
}

function validatePostgresName(name: string, value: string): void {
	if (Buffer.byteLength(value, "utf8") > 63 || /\p{Cc}/u.test(value)) {
		invalid(
			name,
			"must be at most 63 UTF-8 bytes and contain no control characters",
		);
	}
}

function validateIdentityEndpoint(value: string): void {
	let endpoint: URL;
	try {
		endpoint = new URL(value);
	} catch {
		invalid(
			"IDENTITY_ENDPOINT",
			"must be the local URL injected by Azure Container Apps",
		);
	}
	if (
		endpoint.protocol !== "http:" ||
		!isLoopbackHost(endpoint.hostname) ||
		endpoint.username !== "" ||
		endpoint.password !== "" ||
		endpoint.search !== "" ||
		endpoint.hash !== ""
	) {
		invalid(
			"IDENTITY_ENDPOINT",
			"must be an HTTP loopback URL without credentials, query, or fragment",
		);
	}
}

function validateIdentityHeader(value: string): void {
	if (
		value.length < 16 ||
		value.length > 512 ||
		!/^[\x21-\x7e]+$/.test(value)
	) {
		invalid(
			"IDENTITY_HEADER",
			"must be the non-empty, platform-injected SSRF header",
		);
	}
}

export function parseConfig(env: Environment): MigrationConfig {
	for (const name of FORBIDDEN_SETTINGS) {
		if (env[name] !== undefined) {
			throw new ConfigError(
				`${name} is forbidden: Azure PostgreSQL must use managed identity without a static password`,
			);
		}
	}

	const required = (name: (typeof REQUIRED_SETTINGS)[number]): string => {
		const value = env[name];
		if (value === undefined) {
			throw new ConfigError(`missing required environment variable ${name}`);
		}
		if (value === "" || value.trim() !== value) {
			invalid(name, "must be non-empty and have no surrounding whitespace");
		}
		return value;
	};

	if (required("AZURE_POSTGRES_AUTH_MODE") !== "managed_identity") {
		invalid("AZURE_POSTGRES_AUTH_MODE", "must be exactly 'managed_identity'");
	}

	const host = required("AZURE_POSTGRES_HOST");
	validateAzurePostgresHost(host);

	const database = required("AZURE_POSTGRES_DATABASE");
	validatePostgresName("AZURE_POSTGRES_DATABASE", database);

	const user = required("AZURE_POSTGRES_USER");
	validatePostgresName("AZURE_POSTGRES_USER", user);

	validateIdentityEndpoint(required("IDENTITY_ENDPOINT"));
	validateIdentityHeader(required("IDENTITY_HEADER"));

	const clientId = required("AZURE_CLIENT_ID");
	if (!UUID_PATTERN.test(clientId)) {
		invalid(
			"AZURE_CLIENT_ID",
			"must be a UUID client ID for a user-assigned managed identity",
		);
	}

	return { host, database, user, clientId };
}

// Prisma's schema engine (quaint) parses PostgreSQL URLs itself, not with
// libpq: `sslmode` knows only disable|prefer|require and silently falls back to
// `prefer` for anything else, and certificate verification is a separate
// `sslaccept` switch whose default is accept_invalid_certs. So the libpq
// spelling `sslmode=verify-full` would connect over TLS with NO chain or
// hostname check. `sslmode=require&sslaccept=strict` is the quaint spelling of
// verify-full: TLS mandatory, chain verified against the system CA store the
// image installs, hostname verified against AZURE_POSTGRES_HOST.
export function composeDatabaseUrl(
	config: MigrationConfig,
	token: string,
): string {
	const params = new URLSearchParams({
		sslmode: "require",
		sslaccept: "strict",
		connect_timeout: "15",
		application_name: APPLICATION_NAME,
	});
	return (
		`postgresql://${encodeURIComponent(config.user)}:${encodeURIComponent(token)}` +
		`@${config.host}:5432/${encodeURIComponent(config.database)}?${params}`
	);
}

function log(message: string): void {
	console.log(`${LOG_PREFIX} ${message}`);
}

function logError(message: string): void {
	console.error(`${LOG_PREFIX} ${message}`);
}

async function acquireToken(config: MigrationConfig): Promise<string> {
	// Managed identity only, bound to this job's identity: no DefaultAzureCredential
	// chain, so an injected service-principal secret or developer login can never
	// become the fallback the migration runs as.
	const credential = new ManagedIdentityCredential({
		clientId: config.clientId,
	});
	const accessToken = await credential.getToken(POSTGRES_SCOPE);
	if (!accessToken?.token) {
		throw new Error(
			"managed identity returned no token for the PostgreSQL scope",
		);
	}
	const remainingMinutes = Math.floor(
		(accessToken.expiresOnTimestamp - Date.now()) / 60_000,
	);
	log(
		`acquired managed-identity token for ${POSTGRES_SCOPE} (expires in ~${remainingMinutes} min)`,
	);
	return accessToken.token;
}

async function runPrismaDbPush(databaseUrl: string): Promise<number> {
	const proc = Bun.spawn(
		["bunx", "--bun", "prisma", "db", "push", `--schema=${SCHEMA_DIR}`],
		{
			cwd: import.meta.dir,
			env: { ...process.env, DATABASE_URL: databaseUrl },
			// No stdin: with data-loss warnings Prisma must hit its non-interactive
			// path and refuse ("Use the --accept-data-loss flag ..."), never a prompt.
			stdin: "ignore",
			stdout: "inherit",
			stderr: "inherit",
		},
	);

	// Container Apps stops a timed-out or cancelled execution with SIGTERM
	// (STOPSIGNAL); hand it on so Prisma releases its connection instead of
	// being orphaned mid-statement.
	const forward = (signal: NodeJS.Signals) => () => proc.kill(signal);
	const onTerm = forward("SIGTERM");
	const onInt = forward("SIGINT");
	process.once("SIGTERM", onTerm);
	process.once("SIGINT", onInt);
	try {
		const exitCode = await proc.exited;
		if (proc.signalCode) {
			logError(`prisma db push was terminated by ${proc.signalCode}`);
			return 1;
		}
		return exitCode;
	} finally {
		process.off("SIGTERM", onTerm);
		process.off("SIGINT", onInt);
	}
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

	log(
		`target ${config.user}@${config.host}:5432/${config.database} via managed identity ${config.clientId}`,
	);

	let token: string;
	try {
		token = await acquireToken(config);
	} catch (error) {
		logError(
			`managed-identity token acquisition failed: ${(error as Error).message}`,
		);
		return 1;
	}

	log(
		`running: bunx --bun prisma db push --schema=${SCHEMA_DIR} (without --accept-data-loss)`,
	);
	const exitCode = await runPrismaDbPush(composeDatabaseUrl(config, token));
	if (exitCode === 0) {
		log("schema push complete");
	} else {
		logError(`prisma db push exited with code ${exitCode}.`);
		logError(
			"If Prisma listed data-loss warnings above, that refusal is intentional: this job never passes --accept-data-loss.",
		);
		logError(
			"Review the change and, if it really is intended, apply the destructive step by hand from the management host per the runbook, then re-run this job.",
		);
	}
	return exitCode;
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
