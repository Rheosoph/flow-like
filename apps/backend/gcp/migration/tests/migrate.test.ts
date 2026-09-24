import { describe, expect, spyOn, test } from "bun:test";
import type { Client } from "pg";
import {
	API_ROLE_SETTING,
	AUDIT_ROLE_SETTING,
	type AuditRoleDependencies,
	ConfigError,
	type Environment,
	FORBIDDEN_CREDENTIAL_SETTINGS,
	FORBIDDEN_POSTGRES_SETTINGS,
	type Lookup,
	applyAuditRoles,
	databaseUrl,
	normalizePem,
	prePushDatabaseUrl,
	readConfig,
	tlsPosture,
} from "../migrate";

const SERVER_CA =
	"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";

function settings(overrides: Record<string, string | undefined> = {}): Lookup {
	const values: Record<string, string | undefined> = {
		GCP_POSTGRES_AUTH_MODE: "iam",
		GCP_POSTGRES_HOST: "10.24.0.3",
		GCP_POSTGRES_DATABASE: "flow_like",
		GCP_POSTGRES_USER: "flowlike-dev-migration@flow-like-dev.iam",
		GCP_POSTGRES_SERVER_CA: SERVER_CA,
		...overrides,
	};
	return (name) => values[name];
}

describe("readConfig", () => {
	test("accepts only the IAM configuration", () => {
		const config = readConfig(settings());
		expect(config.database).toBe("flow_like");
		expect(config.user).toBe("flowlike-dev-migration@flow-like-dev.iam");
		expect(config.hostIsAddress).toBe(true);
		expect(config.serverCaPem.startsWith("-----BEGIN CERTIFICATE-----")).toBe(
			true,
		);
	});

	test("rejects passwords and connection strings even when empty", () => {
		for (const forbidden of [
			"DATABASE_URL",
			"PGPASSWORD",
			"GCP_POSTGRES_PASSWORD",
		]) {
			expect(() => readConfig(settings({ [forbidden]: "" }))).toThrow(
				new RegExp(`^${forbidden} is forbidden`),
			);
		}
	});

	test("rejects the Cloud SQL proxy and alternate credential sources", () => {
		for (const name of [
			"CSQL_PROXY_ADDRESS",
			"INSTANCE_CONNECTION_NAME",
			"GOOGLE_APPLICATION_CREDENTIALS",
			"GCE_METADATA_HOST",
			"HTTPS_PROXY",
		]) {
			expect(() => readConfig(settings({ [name]: "anything" }))).toThrow(
				ConfigError,
			);
		}
		expect(FORBIDDEN_POSTGRES_SETTINGS).toContain("PGSSLROOTCERT");
		expect(FORBIDDEN_CREDENTIAL_SETTINGS).toContain(
			"GOOGLE_APPLICATION_CREDENTIALS_JSON",
		);
	});

	test("rejects hosts that would leave the VPC", () => {
		for (const host of [
			"34.76.10.4",
			"database.example.com",
			"10.24.0.3:5432",
			"HTTPS://10.24.0.3",
			"-instance.cloudsql.goog",
			"Instance.sql.goog",
		]) {
			expect(() => readConfig(settings({ GCP_POSTGRES_HOST: host }))).toThrow(
				ConfigError,
			);
		}
		for (const host of [
			"flow-like-dev.europe-west1.flow-like-dev.cloudsql.goog",
			"abcdef123.p-1234.europe-west1.sql.goog",
		]) {
			const config = readConfig(settings({ GCP_POSTGRES_HOST: host }));
			expect(config.hostIsAddress).toBe(false);
		}
	});

	test("rejects a user that still carries the service-account suffix", () => {
		expect(() =>
			readConfig(
				settings({
					GCP_POSTGRES_USER:
						"flowlike-dev-migration@flow-like-dev.iam.gserviceaccount.com",
				}),
			),
		).toThrow(/gserviceaccount\.com suffix stripped/);
	});

	test("rejects user shapes that are not IAM principals", () => {
		for (const user of [
			"flowlike",
			"flowlike-dev-migration@flow-like-dev",
			"Flowlike@flow-like-dev.iam",
			"a-very-long-service-account-name-that-postgres-would-truncate@flow-like-dev.iam",
			"flowlike@evil@flow-like-dev.iam",
		]) {
			expect(() => readConfig(settings({ GCP_POSTGRES_USER: user }))).toThrow(
				ConfigError,
			);
		}
	});

	test("rejects static auth mode and padded values", () => {
		expect(() =>
			readConfig(settings({ GCP_POSTGRES_AUTH_MODE: "password" })),
		).toThrow(/must be exactly 'iam'/);
		expect(() =>
			readConfig(settings({ GCP_POSTGRES_DATABASE: " flow_like" })),
		).toThrow(/surrounding whitespace/);
		expect(() =>
			readConfig(settings({ GCP_POSTGRES_HOST: undefined })),
		).toThrow(/missing required environment variable GCP_POSTGRES_HOST/);
	});

	test("server CA accepts escaped newlines and rejects key material", () => {
		const escaped = readConfig(
			settings({
				GCP_POSTGRES_SERVER_CA:
					"-----BEGIN CERTIFICATE-----\\nMIIB\\n-----END CERTIFICATE-----",
			}),
		);
		expect(escaped.serverCaPem).toContain("\n");
		expect(escaped.serverCaPem).not.toContain("\\n");

		expect(() =>
			readConfig(
				settings({
					GCP_POSTGRES_SERVER_CA: `${SERVER_CA}\n-----BEGIN PRIVATE KEY-----\nx\n-----END CERTIFICATE-----`,
				}),
			),
		).toThrow(/private key/);
		expect(() =>
			readConfig(settings({ GCP_POSTGRES_SERVER_CA: "not a certificate" })),
		).toThrow(ConfigError);
		expect(() =>
			readConfig(settings({ GCP_POSTGRES_SERVER_CA: undefined })),
		).toThrow(/missing required environment variable GCP_POSTGRES_SERVER_CA/);
	});
});

describe("normalizePem", () => {
	test("keeps real newlines and trims", () => {
		expect(normalizePem(`  ${SERVER_CA}\n`)).toBe(SERVER_CA);
	});
});

describe("databaseUrl", () => {
	test("against an address: TLS mandatory, no chain check, no CA file", () => {
		const config = readConfig(settings());
		expect(tlsPosture(config)).toBe("require");
		const url = new URL(databaseUrl(config, "ya29.a/b+c=d", undefined));
		expect(url.protocol).toBe("postgresql:");
		expect(url.hostname).toBe("10.24.0.3");
		expect(url.port).toBe("5432");
		expect(url.pathname).toBe("/flow_like");
		expect(decodeURIComponent(url.username)).toBe(
			"flowlike-dev-migration@flow-like-dev.iam",
		);
		expect(decodeURIComponent(url.password)).toBe("ya29.a/b+c=d");
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("accept_invalid_certs");
		expect(url.searchParams.has("sslcert")).toBe(false);
		expect(url.searchParams.get("application_name")).toBe(
			"flow-like-gcp-migration",
		);
	});

	test("against a DNS name: pinned CA and strict verification", () => {
		const config = readConfig(
			settings({ GCP_POSTGRES_HOST: "abcdef123.p-1234.europe-west1.sql.goog" }),
		);
		expect(tlsPosture(config)).toBe("verify-full");
		expect(() => databaseUrl(config, "token", undefined)).toThrow(/server CA/);
		const url = new URL(databaseUrl(config, "token", "/tmp/x/server-ca.pem"));
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("strict");
		expect(url.searchParams.get("sslcert")).toBe("/tmp/x/server-ca.pem");
	});

	test("never uses the libpq-only parameters Prisma ignores", () => {
		const url = databaseUrl(readConfig(settings()), "token", undefined);
		expect(url).not.toContain("verify-ca");
		expect(url).not.toContain("sslrootcert");
	});
});

describe("prePushDatabaseUrl", () => {
	test("against an address: same identity, libpq semantics, no chain check", () => {
		const config = readConfig(settings());
		const url = new URL(prePushDatabaseUrl(config, "ya29.a/b+c=d", undefined));
		const prisma = new URL(databaseUrl(config, "ya29.a/b+c=d", undefined));
		expect(url.host).toBe(prisma.host);
		expect(url.pathname).toBe(prisma.pathname);
		expect(url.username).toBe(prisma.username);
		expect(url.password).toBe(prisma.password);
		expect(url.searchParams.get("uselibpqcompat")).toBe("true");
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.has("sslrootcert")).toBe(false);
		expect(url.searchParams.has("sslaccept")).toBe(false);
		expect(url.searchParams.get("application_name")).toBe(
			"flow-like-gcp-migration",
		);
	});

	test("against a DNS name: pinned CA as sslrootcert, verify-full", () => {
		const config = readConfig(
			settings({ GCP_POSTGRES_HOST: "abcdef123.p-1234.europe-west1.sql.goog" }),
		);
		expect(() => prePushDatabaseUrl(config, "token", undefined)).toThrow(
			/server CA/,
		);
		const url = new URL(
			prePushDatabaseUrl(config, "token", "/tmp/x/server-ca.pem"),
		);
		expect(url.searchParams.get("sslmode")).toBe("verify-full");
		expect(url.searchParams.get("sslrootcert")).toBe("/tmp/x/server-ca.pem");
		expect(url.searchParams.has("sslcert")).toBe(false);
	});
});

describe("applyAuditRoles", () => {
	const DATABASE_URL = prePushDatabaseUrl(
		readConfig(settings()),
		"token",
		undefined,
	);
	const API_ROLE = "flowlike-dev-api@flow-like-dev.iam";
	const AUDIT_ROLE = "flowlike-dev-audit-worker@flow-like-dev.iam";

	interface Session {
		readonly connected: string[];
		readonly provisioned: NodeJS.ProcessEnv[];
		ended: boolean;
	}

	// A catalog that holds `roles`; `provision` stands in for the shared helper.
	function fakeDependencies(
		roles: readonly string[],
		provision: AuditRoleDependencies["provision"] = async () => undefined,
	): { session: Session; deps: AuditRoleDependencies } {
		const session: Session = { connected: [], provisioned: [], ended: false };
		const client = {
			async query(_text: string, values: readonly unknown[] = []) {
				return {
					rows: roles.includes(String(values[0])) ? [{ exists: 1 }] : [],
				};
			},
			async end() {
				session.ended = true;
			},
		} as unknown as Client;
		const deps: AuditRoleDependencies = {
			connect: async (url) => {
				session.connected.push(url);
				return client;
			},
			provision: async (target, env) => {
				session.provisioned.push(env ?? {});
				await provision(target, env);
			},
		};
		return { session, deps };
	}

	async function run(env: Environment, deps: AuditRoleDependencies) {
		const logs = spyOn(console, "log").mockImplementation(() => undefined);
		const errors = spyOn(console, "error").mockImplementation(() => undefined);
		try {
			const code = await applyAuditRoles(env, DATABASE_URL, deps);
			return {
				code,
				logs: logs.mock.calls.map((call) => String(call[0])),
				errors: errors.mock.calls.map((call) => String(call[0])),
			};
		} finally {
			logs.mockRestore();
			errors.mockRestore();
		}
	}

	test("without AUDIT_DATABASE_ROLE the step is skipped before any connection", async () => {
		const { session, deps } = fakeDependencies([API_ROLE, AUDIT_ROLE]);
		const result = await run({ [API_ROLE_SETTING]: API_ROLE }, deps);
		expect(result.code).toBe(0);
		expect(result.logs).toEqual([
			"[migration] audit role provisioning skipped: AUDIT_DATABASE_ROLE is not set",
		]);
		expect(result.errors).toEqual([]);
		expect(session.connected).toEqual([]);
		expect(session.provisioned).toEqual([]);
	});

	test("a worker role missing from pg_roles is skipped with one line and the connection is closed", async () => {
		const { session, deps } = fakeDependencies([API_ROLE]);
		const result = await run(
			{ [API_ROLE_SETTING]: API_ROLE, [AUDIT_ROLE_SETTING]: AUDIT_ROLE },
			deps,
		);
		expect(result.code).toBe(0);
		expect(result.logs).toEqual([
			`[migration] audit role provisioning skipped: role "${AUDIT_ROLE}" does not exist in pg_roles`,
		]);
		expect(result.errors).toEqual([]);
		expect(session.connected).toEqual([DATABASE_URL]);
		expect(session.provisioned).toEqual([]);
		expect(session.ended).toBe(true);
	});

	test("with both roles the helper runs in grant-only mode on the push connection", async () => {
		const { session, deps } = fakeDependencies([API_ROLE, AUDIT_ROLE]);
		const result = await run(
			{ [API_ROLE_SETTING]: API_ROLE, [AUDIT_ROLE_SETTING]: AUDIT_ROLE },
			deps,
		);
		expect(result.code).toBe(0);
		expect(session.connected).toEqual([DATABASE_URL]);
		expect(session.provisioned).toHaveLength(1);
		expect(session.provisioned[0]).toMatchObject({
			AUDIT_DB_GRANTS_ONLY: "true",
			API_DATABASE_ROLE: API_ROLE,
			AUDIT_DATABASE_ROLE: AUDIT_ROLE,
		});
		expect(session.provisioned[0]?.DATABASE_URL).toBeUndefined();
		expect(result.logs).toEqual([
			`[migration] audit privilege boundary applied: api="${API_ROLE}" worker="${AUDIT_ROLE}"`,
		]);
		expect(session.ended).toBe(true);
	});

	test("a provisioning failure is the job's failure", async () => {
		const { session, deps } = fakeDependencies(
			[API_ROLE, AUDIT_ROLE],
			async () => {
				throw new Error(
					"Inherited audit write privileges defeat API and worker separation",
				);
			},
		);
		const result = await run(
			{ [API_ROLE_SETTING]: API_ROLE, [AUDIT_ROLE_SETTING]: AUDIT_ROLE },
			deps,
		);
		expect(result.code).toBe(1);
		expect(result.errors).toEqual([
			"[migration] audit role provisioning failed: Inherited audit write privileges defeat API and worker separation",
		]);
		expect(session.ended).toBe(true);
	});

	test("a connection failure is the job's failure and never prints the URL", async () => {
		const { deps } = fakeDependencies([API_ROLE, AUDIT_ROLE]);
		const failing: AuditRoleDependencies = {
			...deps,
			connect: async () => {
				throw new Error(
					'password authentication failed for user "flowlike-dev-migration@flow-like-dev.iam"',
				);
			},
		};
		const result = await run({ [AUDIT_ROLE_SETTING]: AUDIT_ROLE }, failing);
		expect(result.code).toBe(1);
		expect(result.errors).toEqual([
			'[migration] audit role provisioning failed: password authentication failed for user "flowlike-dev-migration@flow-like-dev.iam"',
		]);
		expect(
			[...result.logs, ...result.errors].some((line) => line.includes("token")),
		).toBe(false);
	});
});
