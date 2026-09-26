import { describe, expect, spyOn, test } from "bun:test";
import type { Client } from "pg";
import {
	API_ROLE_SETTING,
	AUDIT_ROLE_SETTING,
	type AuditRoleDependencies,
	ConfigError,
	type Environment,
	applyAuditRoles,
	composeDatabaseUrl,
	composePrePushDatabaseUrl,
	parseConfig,
} from "./migrate";

const CLIENT_ID = "11111111-2222-4333-8444-555555555555";

function validSettings(): Environment {
	return {
		AZURE_POSTGRES_AUTH_MODE: "managed_identity",
		AZURE_POSTGRES_HOST: "flow-like-dev.flow-like.postgres.database.azure.com",
		AZURE_POSTGRES_DATABASE: "flow_like",
		AZURE_POSTGRES_USER: "flowlike-dev-migration-identity",
		AZURE_CLIENT_ID: CLIENT_ID,
		IDENTITY_ENDPOINT: "http://localhost:42356/msi/token",
		IDENTITY_HEADER: "11111111-2222-4333-8444-555555555555",
	};
}

describe("parseConfig", () => {
	test("accepts only managed-identity configuration", () => {
		const config = parseConfig(validSettings());
		expect(config.clientId).toBe(CLIENT_ID);
		expect(config.database).toBe("flow_like");
		expect(config.user).toBe("flowlike-dev-migration-identity");
	});

	test("rejects passwords and connection strings even when empty", () => {
		for (const forbidden of [
			"DATABASE_URL",
			"PGPASSWORD",
			"AZURE_POSTGRES_PASSWORD",
		]) {
			const env = { ...validSettings(), [forbidden]: "" };
			expect(() => parseConfig(env)).toThrow(
				new RegExp(`^${forbidden} is forbidden`),
			);
		}
	});

	test("rejects non-azure or ambiguous hosts", () => {
		for (const host of [
			"database.example.com",
			"HTTPS://server.postgres.database.azure.com",
			"server.postgres.database.azure.com:5432",
			"-server.postgres.database.azure.com",
		]) {
			expect(() =>
				parseConfig({ ...validSettings(), AZURE_POSTGRES_HOST: host }),
			).toThrow(ConfigError);
		}
	});

	test("rejects static auth mode and missing client id", () => {
		expect(() =>
			parseConfig({ ...validSettings(), AZURE_POSTGRES_AUTH_MODE: "password" }),
		).toThrow(ConfigError);

		const { AZURE_CLIENT_ID: _omitted, ...withoutClientId } = validSettings();
		expect(() => parseConfig(withoutClientId)).toThrow(/AZURE_CLIENT_ID/);
	});

	test("rejects non-local identity endpoint and alternate credential sources", () => {
		expect(() =>
			parseConfig({
				...validSettings(),
				IDENTITY_ENDPOINT: "https://identity.example.com/token",
			}),
		).toThrow(ConfigError);

		expect(() =>
			parseConfig({
				...validSettings(),
				IMDS_ENDPOINT: "http://localhost:1234",
			}),
		).toThrow(/^IMDS_ENDPOINT is forbidden/);

		expect(() =>
			parseConfig({
				...validSettings(),
				HTTPS_PROXY: "https://proxy.example.com",
			}),
		).toThrow(/^HTTPS_PROXY is forbidden/);
	});
});

describe("composeDatabaseUrl", () => {
	test("uses the token as password with quaint's verify-full spelling", () => {
		const config = parseConfig(validSettings());
		const url = new URL(composeDatabaseUrl(config, "eyJ.header/payload+sig="));

		expect(url.protocol).toBe("postgresql:");
		expect(url.username).toBe("flowlike-dev-migration-identity");
		expect(decodeURIComponent(url.password)).toBe("eyJ.header/payload+sig=");
		expect(url.host).toBe(
			"flow-like-dev.flow-like.postgres.database.azure.com:5432",
		);
		expect(url.pathname).toBe("/flow_like");
		expect(url.searchParams.get("sslmode")).toBe("require");
		expect(url.searchParams.get("sslaccept")).toBe("strict");
		expect(url.searchParams.get("connect_timeout")).toBe("15");
	});
});

describe("composePrePushDatabaseUrl", () => {
	test("same identity as the Prisma URL, verify-full in node-postgres' libpq spelling", () => {
		const config = parseConfig(validSettings());
		const token = "eyJ.header/payload+sig=";
		const url = new URL(composePrePushDatabaseUrl(config, token));
		const prisma = new URL(composeDatabaseUrl(config, token));

		expect(url.protocol).toBe("postgresql:");
		expect(url.username).toBe(prisma.username);
		expect(url.password).toBe(prisma.password);
		expect(url.host).toBe(prisma.host);
		expect(url.pathname).toBe(prisma.pathname);
		expect(url.searchParams.get("uselibpqcompat")).toBe("true");
		expect(url.searchParams.get("sslmode")).toBe("verify-full");
		expect(url.searchParams.has("sslaccept")).toBe(false);
		expect(url.searchParams.has("sslcert")).toBe(false);
	});
});

describe("applyAuditRoles", () => {
	const DATABASE_URL = composePrePushDatabaseUrl(
		parseConfig(validSettings()),
		"token",
	);
	const API_ROLE = "flowlike-dev-api-identity";
	const AUDIT_ROLE = "flowlike-dev-audit-worker-identity";

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
		const result = await run(
			{ ...validSettings(), [API_ROLE_SETTING]: API_ROLE },
			deps,
		);
		expect(result.code).toBe(0);
		expect(result.logs).toEqual([
			"[azure-migration] audit role provisioning skipped: AUDIT_DATABASE_ROLE is not set",
		]);
		expect(result.errors).toEqual([]);
		expect(session.connected).toEqual([]);
		expect(session.provisioned).toEqual([]);
	});

	test("a worker role missing from pg_roles is skipped with one line and the connection is closed", async () => {
		const { session, deps } = fakeDependencies([API_ROLE]);
		const result = await run(
			{
				...validSettings(),
				[API_ROLE_SETTING]: API_ROLE,
				[AUDIT_ROLE_SETTING]: AUDIT_ROLE,
			},
			deps,
		);
		expect(result.code).toBe(0);
		expect(result.logs).toEqual([
			`[azure-migration] audit role provisioning skipped: role "${AUDIT_ROLE}" does not exist in pg_roles`,
		]);
		expect(result.errors).toEqual([]);
		expect(session.connected).toEqual([DATABASE_URL]);
		expect(session.provisioned).toEqual([]);
		expect(session.ended).toBe(true);
	});

	test("with both roles the helper runs in grant-only mode on the push connection", async () => {
		const { session, deps } = fakeDependencies([API_ROLE, AUDIT_ROLE]);
		const result = await run(
			{
				...validSettings(),
				[API_ROLE_SETTING]: API_ROLE,
				[AUDIT_ROLE_SETTING]: AUDIT_ROLE,
			},
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
			`[azure-migration] audit privilege boundary applied: api="${API_ROLE}" worker="${AUDIT_ROLE}"`,
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
			{
				...validSettings(),
				[API_ROLE_SETTING]: API_ROLE,
				[AUDIT_ROLE_SETTING]: AUDIT_ROLE,
			},
			deps,
		);
		expect(result.code).toBe(1);
		expect(result.errors).toEqual([
			"[azure-migration] audit role provisioning failed: Inherited audit write privileges defeat API and worker separation",
		]);
		expect(session.ended).toBe(true);
	});

	test("a connection failure is the job's failure and never prints the URL", async () => {
		const { deps } = fakeDependencies([API_ROLE, AUDIT_ROLE]);
		const failing: AuditRoleDependencies = {
			...deps,
			connect: async () => {
				throw new Error(
					'password authentication failed for user "flowlike-dev-migration-identity"',
				);
			},
		};
		const result = await run(
			{ ...validSettings(), [AUDIT_ROLE_SETTING]: AUDIT_ROLE },
			failing,
		);
		expect(result.code).toBe(1);
		expect(result.errors).toEqual([
			'[azure-migration] audit role provisioning failed: password authentication failed for user "flowlike-dev-migration-identity"',
		]);
		expect(
			[...result.logs, ...result.errors].some((line) => line.includes("token")),
		).toBe(false);
	});
});
