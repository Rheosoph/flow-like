/**
 * Applies the Flow-Like Prisma schema to Cloud SQL for PostgreSQL as the
 * migration identity.
 *
 * There is no database password anywhere in the GCP installation (D4): the
 * migration service account is a `CLOUD_IAM_SERVICE_ACCOUNT` Postgres role and
 * authenticates with a Google access token. This process mints that token from
 * the instance metadata server, composes a connection URL in memory, hands it
 * to exactly one child process (`prisma db push`) as `DATABASE_URL`, and exits
 * with that child's exit code. The URL is never printed and never written to
 * disk. Every environment variable that would let a static password, a
 * connection string, a key file or a proxy back in is refused before anything
 * else happens, so a mis-set job surfaces as a one-line configuration error
 * rather than as a silently different identity.
 *
 * The validation mirrors `flow_like_gcp_data::postgres` (the crate the API and
 * the queue workers use) name for name, so the same job env is accepted or
 * rejected by both — Terraform renders one `database_env` for all of them.
 */
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { isIPv4 } from "node:net";
import { constants as osConstants, tmpdir } from "node:os";
import { join } from "node:path";
import { GoogleAuth } from "google-auth-library";

// The narrowest scope Cloud SQL accepts for IAM database login. The token
// becomes a PostgreSQL password and is copied into driver state, and Prisma
// additionally passes it to its schema engine on the command line; a
// cloud-platform token in that position would be a project-wide credential.
export const SQL_LOGIN_SCOPE =
	"https://www.googleapis.com/auth/sqlservice.login";
export const POSTGRES_PORT = 5432;
const APPLICATION_NAME = "flow-like-gcp-migration";
const CONNECT_TIMEOUT_SECS = 30;

// Google-internal DNS names for a Cloud SQL instance. `.sql.goog` is what the
// platform issues (PSC and private-IP DNS names); `.cloudsql.goog` is the
// shape the runtime crate accepts today. Either stays inside the VPC, and a
// name here — unlike an address — is what makes server identity verifiable.
const CLOUD_SQL_DNS_SUFFIXES = [".sql.goog", ".cloudsql.goog"] as const;
// Cloud SQL names the IAM database user after the service-account email with
// this suffix removed. Leaving it on yields a bare "password authentication
// failed" that is indistinguishable from a bad token, so it is checked by name.
const SERVICE_ACCOUNT_SUFFIX = ".gserviceaccount.com";
const IAM_USER_SUFFIX = ".iam";
const MAX_SERVER_CA_BYTES = 64 * 1024;
const PEM_CERTIFICATE_HEADER = "-----BEGIN CERTIFICATE-----";
const PEM_CERTIFICATE_FOOTER = "-----END CERTIFICATE-----";

export const REQUIRED_SETTINGS = {
	authMode: "GCP_POSTGRES_AUTH_MODE",
	host: "GCP_POSTGRES_HOST",
	database: "GCP_POSTGRES_DATABASE",
	user: "GCP_POSTGRES_USER",
	serverCa: "GCP_POSTGRES_SERVER_CA",
} as const;

// Everything libpq would otherwise read from the environment, plus every way
// to reintroduce a static password or to interpose the Cloud SQL Auth Proxy.
// The proxy is not merely redundant: it terminates TLS on localhost, which
// turns any server verification into a check against a certificate the proxy
// chose. Same list as `flow_like_gcp_data::postgres::FORBIDDEN_SETTINGS`.
export const FORBIDDEN_POSTGRES_SETTINGS: readonly string[] = [
	"DATABASE_URL",
	"GCP_POSTGRES_PASSWORD",
	"GCP_POSTGRES_CONNECTION_STRING",
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
	"INSTANCE_CONNECTION_NAME",
	"CLOUD_SQL_CONNECTION_NAME",
	"CLOUD_SQL_PROXY_PATH",
	"CSQL_PROXY_ADDRESS",
	"CSQL_PROXY_PORT",
	"CSQL_PROXY_UNIX_SOCKET",
	"CSQL_PROXY_TOKEN",
	"CSQL_PROXY_CREDENTIALS_FILE",
	"CSQL_PROXY_JSON_CREDENTIALS",
	"CSQL_PROXY_AUTO_IAM_AUTHN",
	"CLOUDSDK_API_ENDPOINT_OVERRIDES_SQLADMIN",
];

// Environment that would redirect, replace or intercept the credential this
// process mints. Rejected rather than ignored: silently ignoring a set
// `GCE_METADATA_HOST` leaves an operator believing an override took effect,
// and silently honouring it lets anything that can write the environment
// steal the workload identity. Same list as
// `flow_like_gcp_data::metadata::FORBIDDEN_CREDENTIAL_SETTINGS`.
export const FORBIDDEN_CREDENTIAL_SETTINGS: readonly string[] = [
	"GOOGLE_APPLICATION_CREDENTIALS",
	"GOOGLE_APPLICATION_CREDENTIALS_JSON",
	"GOOGLE_CREDENTIALS",
	"GOOGLE_OAUTH_ACCESS_TOKEN",
	"CLOUDSDK_AUTH_ACCESS_TOKEN",
	"GCE_METADATA_HOST",
	"GCE_METADATA_IP",
	"GCE_METADATA_ROOT",
	"METADATA_SERVER_DETECTION",
	"HTTP_PROXY",
	"HTTPS_PROXY",
	"ALL_PROXY",
	"http_proxy",
	"https_proxy",
	"all_proxy",
];

export class ConfigError extends Error {
	override readonly name = "ConfigError";
}

export type Lookup = (name: string) => string | undefined;

export interface MigrationConfig {
	readonly host: string;
	// True when `host` is an RFC1918 address rather than a Cloud SQL DNS name.
	// This decides the TLS posture, see `databaseUrl`.
	readonly hostIsAddress: boolean;
	readonly database: string;
	readonly user: string;
	readonly serverCaPem: string;
}

// Terraform and Secret Manager both round-trip PEM material through
// single-line values with escaped newlines; accepting that form spares an
// operator discovering that a valid certificate was rejected purely by how it
// travelled.
export function normalizePem(value: string): string {
	const normalized = value.includes("\n")
		? value
		: value.replaceAll("\\n", "\n");
	return normalized.trim();
}

function invalid(name: string, reason: string): ConfigError {
	return new ConfigError(`invalid ${name}: ${reason}`);
}

function isPrivateIpv4(address: string): boolean {
	const octets = address.split(".").map(Number);
	const [a, b] = octets;
	return (
		a === 10 ||
		(a === 172 && b !== undefined && b >= 16 && b <= 31) ||
		(a === 192 && b === 168)
	);
}

// Accept only an address that stays inside the VPC: a PSA private IP or the
// instance's Google-internal DNS name. A public hostname here would mean the
// connection had left the network.
function validateHost(host: string): boolean {
	if (isIPv4(host)) {
		if (!isPrivateIpv4(host)) {
			throw invalid(
				REQUIRED_SETTINGS.host,
				"must be a private address; Cloud SQL is reached over Private Service Access",
			);
		}
		return true;
	}

	const suffix = CLOUD_SQL_DNS_SUFFIXES.find((candidate) =>
		host.endsWith(candidate),
	);
	if (suffix === undefined) {
		throw invalid(
			REQUIRED_SETTINGS.host,
			`must be an RFC1918 address or end in ${CLOUD_SQL_DNS_SUFFIXES.join(" or ")}`,
		);
	}

	const privateLabels = host.slice(0, -suffix.length);
	const valid =
		host.length <= 253 &&
		privateLabels.length > 0 &&
		privateLabels
			.split(".")
			.every((label) => /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(label));
	if (!valid) {
		throw invalid(
			REQUIRED_SETTINGS.host,
			"must be a lowercase Cloud SQL DNS name without a scheme, port, or path",
		);
	}
	return false;
}

function validateDatabase(database: string): void {
	if (Buffer.byteLength(database) > 63 || /\p{Cc}/u.test(database)) {
		throw invalid(
			REQUIRED_SETTINGS.database,
			"must be at most 63 UTF-8 bytes and contain no control characters",
		);
	}
}

// The value is read from the environment rather than derived from the runtime
// service account: Terraform owns the `google_sql_user`, and deriving the name
// here would let the two drift apart silently. Validating the shape keeps the
// drift loud.
function validateUser(user: string): void {
	if (user.endsWith(SERVICE_ACCOUNT_SUFFIX)) {
		throw invalid(
			REQUIRED_SETTINGS.user,
			`must have the ${SERVICE_ACCOUNT_SUFFIX} suffix stripped; Cloud SQL names the IAM user after the truncated service-account email`,
		);
	}
	const at = user.indexOf("@");
	if (at < 0) {
		throw invalid(
			REQUIRED_SETTINGS.user,
			"must be a service-account email of the form <account>@<project>.iam",
		);
	}
	const account = user.slice(0, at);
	const project = user.slice(at + 1);
	// PostgreSQL truncates identifiers at 63 bytes, and so does Cloud SQL when it
	// creates the IAM user. A longer value would connect as a different
	// principal than the one the grants were written for.
	const valid =
		Buffer.byteLength(user) <= 63 &&
		!project.includes("@") &&
		project.endsWith(IAM_USER_SUFFIX) &&
		/^[a-z][a-z0-9-]{0,29}$/.test(account) &&
		/^[a-z0-9.:-]+$/.test(project);
	if (!valid) {
		throw invalid(
			REQUIRED_SETTINGS.user,
			"must be at most 63 bytes and shaped like <account>@<project>.iam",
		);
	}
}

// Cloud SQL signs its serving certificate with a per-instance CA that is in no
// public trust store, so the pinned instance CA is the only thing that can
// ever make server verification mean anything here.
function validateServerCa(pem: string): void {
	if (Buffer.byteLength(pem) > MAX_SERVER_CA_BYTES) {
		throw invalid(
			REQUIRED_SETTINGS.serverCa,
			`must be at most ${MAX_SERVER_CA_BYTES} bytes`,
		);
	}
	if (
		!pem.startsWith(PEM_CERTIFICATE_HEADER) ||
		!pem.endsWith(PEM_CERTIFICATE_FOOTER)
	) {
		throw invalid(
			REQUIRED_SETTINGS.serverCa,
			"must be the instance server CA as a PEM certificate",
		);
	}
	if (pem.includes("PRIVATE KEY")) {
		throw invalid(
			REQUIRED_SETTINGS.serverCa,
			"must contain only certificates; a private key was supplied",
		);
	}
}

export function readConfig(lookup: Lookup): MigrationConfig {
	for (const name of FORBIDDEN_POSTGRES_SETTINGS) {
		if (lookup(name) !== undefined) {
			throw new ConfigError(
				`${name} is forbidden: Cloud SQL must use IAM database authentication, where no password exists to configure`,
			);
		}
	}
	for (const name of FORBIDDEN_CREDENTIAL_SETTINGS) {
		if (lookup(name) !== undefined) {
			throw new ConfigError(
				`${name} is set; GCP workloads must authenticate through the instance metadata server only, with no key file, no ambient token and no proxy in the path`,
			);
		}
	}

	const required = (name: string): string => {
		const value = lookup(name);
		if (value === undefined) {
			throw new ConfigError(`missing required environment variable ${name}`);
		}
		if (value === "" || value.trim() !== value) {
			throw invalid(
				name,
				"must be non-empty and have no surrounding whitespace",
			);
		}
		return value;
	};

	if (required(REQUIRED_SETTINGS.authMode) !== "iam") {
		throw invalid(REQUIRED_SETTINGS.authMode, "must be exactly 'iam'");
	}
	const host = required(REQUIRED_SETTINGS.host);
	const hostIsAddress = validateHost(host);
	const database = required(REQUIRED_SETTINGS.database);
	validateDatabase(database);
	const user = required(REQUIRED_SETTINGS.user);
	validateUser(user);

	// The PEM is the one setting that legitimately contains newlines, so it
	// bypasses the whitespace-trimming helper.
	const rawServerCa = lookup(REQUIRED_SETTINGS.serverCa);
	if (rawServerCa === undefined) {
		throw new ConfigError(
			`missing required environment variable ${REQUIRED_SETTINGS.serverCa}`,
		);
	}
	const serverCaPem = normalizePem(rawServerCa);
	validateServerCa(serverCaPem);

	return { host, hostIsAddress, database, user, serverCaPem };
}

export type TlsPosture = "verify-full" | "require";

// What Prisma can actually enforce for this host.
//
// Prisma's schema engine (quaint) knows `sslmode=disable|prefer|require`,
// `sslcert=<CA path>` and `sslaccept=strict|accept_invalid_certs`; libpq's
// `verify-ca` / `verify-full` / `sslrootcert` are silently ignored — an
// unknown sslmode falls back to `prefer`, which is why they are never used
// here. `strict` verifies the chain and the hostname together, with no way to
// verify the chain alone. Cloud SQL's per-instance CA issues a serving
// certificate whose identity is the instance name, not its private address,
// so against an address the strict check cannot succeed and the only honest
// posture is `require`: TLS is mandatory (the instance also refuses cleartext),
// the token is login-scoped and short-lived, and the path never leaves the
// VPC. Against the instance's DNS name (shared-CA mode puts it in the SAN) the
// full check works, so it is switched on.
export function tlsPosture(
	config: Pick<MigrationConfig, "hostIsAddress">,
): TlsPosture {
	return config.hostIsAddress ? "require" : "verify-full";
}

// Compose the URL Prisma's schema engine will parse. Only ever passed to the
// child process environment; callers must not log the return value.
export function databaseUrl(
	config: MigrationConfig,
	token: string,
	serverCaPath: string | undefined,
): string {
	const params = new URLSearchParams();
	params.set("sslmode", "require");
	if (tlsPosture(config) === "verify-full") {
		if (serverCaPath === undefined) {
			throw new Error(
				"verify-full posture requires the server CA to be written to disk first",
			);
		}
		params.set("sslcert", serverCaPath);
		params.set("sslaccept", "strict");
	} else {
		params.set("sslaccept", "accept_invalid_certs");
	}
	params.set("application_name", APPLICATION_NAME);
	params.set("connect_timeout", String(CONNECT_TIMEOUT_SECS));

	const user = encodeURIComponent(config.user);
	const password = encodeURIComponent(token);
	const database = encodeURIComponent(config.database);
	return `postgresql://${user}:${password}@${config.host}:${POSTGRES_PORT}/${database}?${params.toString()}`;
}

// Application Default Credentials with every file-, token- and proxy-based
// source refused above resolve to exactly one thing on Cloud Run: the metadata
// server, asked for a token carrying only the SQL login scope.
async function mintLoginToken(): Promise<string> {
	const auth = new GoogleAuth({ scopes: [SQL_LOGIN_SCOPE] });
	const token = await auth.getAccessToken();
	if (typeof token !== "string" || token.length === 0) {
		throw new Error(
			"the metadata server returned no access token for the Cloud SQL login scope",
		);
	}
	return token;
}

// `--accept-data-loss` is deliberately absent, see the Dockerfile header.
// Prisma 7 removed `--skip-generate` (db push no longer generates a client)
// and errors on unknown options, so it is not passed either.
const PRISMA_ARGS = [
	"x",
	"--bun",
	"prisma",
	"db",
	"push",
	"--schema=prisma/schema",
];

async function runPrisma(url: string): Promise<number> {
	const child = Bun.spawn([process.execPath, ...PRISMA_ARGS], {
		cwd: import.meta.dir,
		env: { ...process.env, DATABASE_URL: url },
		stdin: "ignore",
		stdout: "inherit",
		stderr: "inherit",
	});
	// Cloud Run cancels a job with SIGTERM; a schema push interrupted mid-flight
	// must see the same signal rather than being orphaned behind an exiting
	// parent.
	const forward = (signal: NodeJS.Signals) => child.kill(signal);
	process.once("SIGTERM", forward);
	process.once("SIGINT", forward);
	try {
		const code = await child.exited;
		if (child.signalCode !== null) {
			const number =
				osConstants.signals[
					child.signalCode as keyof typeof osConstants.signals
				] ?? 0;
			console.error(
				`[migration] prisma db push was terminated by ${child.signalCode}`,
			);
			return 128 + number;
		}
		return code;
	} finally {
		process.off("SIGTERM", forward);
		process.off("SIGINT", forward);
	}
}

export async function main(): Promise<number> {
	const config = readConfig((name) => process.env[name]);
	const posture = tlsPosture(config);
	console.log(
		`[migration] target host=${config.host} database=${config.database} user=${config.user} tls=${posture}`,
	);
	if (posture === "require") {
		console.warn(
			"[migration] GCP_POSTGRES_HOST is an address: Prisma cannot verify a certificate chain without also verifying the hostname, and Cloud SQL's per-instance CA issues no address identity, so the server certificate is not verified on this run (TLS is still mandatory and the token is login-scoped). Set GCP_POSTGRES_HOST to the instance DNS name to enable verify-full.",
		);
	}

	const token = await mintLoginToken();
	console.log(
		"[migration] minted a Cloud SQL login token from the metadata server",
	);

	let scratch: string | undefined;
	try {
		let serverCaPath: string | undefined;
		if (posture === "verify-full") {
			// mkdtemp creates the directory 0700; the file is 0600 on top. It holds
			// a public CA certificate, so the mode is hygiene rather than secrecy —
			// what must never touch disk is the URL, and it does not.
			scratch = mkdtempSync(join(tmpdir(), "flow-like-migration-"));
			serverCaPath = join(scratch, "server-ca.pem");
			writeFileSync(serverCaPath, `${config.serverCaPem}\n`, { mode: 0o600 });
		}
		return await runPrisma(databaseUrl(config, token, serverCaPath));
	} finally {
		if (scratch !== undefined) {
			rmSync(scratch, { recursive: true, force: true });
		}
	}
}

if (import.meta.main) {
	main().then(
		(code) => process.exit(code),
		(error: unknown) => {
			const message = error instanceof Error ? error.message : String(error);
			console.error(`[migration] ${message}`);
			process.exit(1);
		},
	);
}
