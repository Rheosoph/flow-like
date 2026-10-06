import { describe, expect, test } from "bun:test";
import {
	type PlacementConfiguration,
	createDeploymentPlan,
} from "./deployment";
import {
	configuredTunnelServices,
	parseTunnelHeaders,
	readServiceListeners,
	tunnelServicesSchema,
} from "./tunnel-services";

const database = {
	id: "database",
	host: "127.0.0.1",
	port: 5432,
	protocol: "tcp" as const,
};
const https = {
	id: "api",
	host: "::1",
	port: 8443,
	protocol: "https" as const,
	tls_server_name: "api.localhost",
	tls_sha256_fingerprint: "a".repeat(64),
};

describe("configured tunnel services", () => {
	test("connect-only discovery binds the placement and rejects permission failures or private metadata", async () => {
		const result = {
			placement_id: "placement",
			project_id: "project",
			config_revision: 1,
			services: [database],
		};
		let command: Record<string, unknown> | undefined;
		expect(
			await readServiceListeners(async (value) => {
				command = value;
				return { operation_id: "read", state: "completed", result };
			}, "placement"),
		).toEqual([database]);
		expect(command).toEqual({
			type: "service_listeners",
			placement_id: "placement",
		});
		await expect(
			readServiceListeners(
				async () => ({
					operation_id: "read",
					state: "rejected",
					result: {
						code: "unauthorized",
						error: "Service access denied",
						retryable: false,
					},
				}),
				"placement",
			),
		).rejects.toThrow("Service access denied");
		for (const invalid of [
			{ ...result, placement_id: "other" },
			{
				...result,
				services: [{ ...database, tls_sha256_fingerprint: "a".repeat(64) }],
			},
			{ ...result, services: [database, database] },
			{ ...result, services: [{ ...database, host: "192.168.1.1" }] },
		])
			await expect(
				readServiceListeners(
					async () => ({
						operation_id: "read",
						state: "completed",
						result: invalid,
					}),
					"placement",
				),
			).rejects.toThrow();
		const abort = new AbortController();
		abort.abort();
		await expect(
			readServiceListeners(
				async () => {
					throw new Error("Must not send");
				},
				"placement",
				abort.signal,
			),
		).rejects.toThrow();
	});
	test("accepts device loopback addresses and requires explicit HTTPS authentication", () => {
		for (const host of [
			"127.0.0.1",
			"127.255.255.254",
			"::1",
			"0:0:0:0:0:0:0:1",
			"::0001",
		])
			expect(
				tunnelServicesSchema.safeParse([{ ...database, host }]).success,
			).toBe(true);
		for (const host of [
			"localhost",
			"0.0.0.0",
			"::",
			"192.168.1.2",
			"127.0.0.01",
			"127.256.0.1",
			"::ffff:127.0.0.1",
		])
			expect(
				tunnelServicesSchema.safeParse([{ ...database, host }]).success,
			).toBe(false);
		expect(tunnelServicesSchema.parse([https])).toEqual([https]);
		for (const tls_server_name of [
			"localhost",
			"api.example.test.",
			"_service.example.test",
			"127.0.0.1",
			"::1",
		])
			expect(
				tunnelServicesSchema.safeParse([{ ...https, tls_server_name }]).success,
			).toBe(true);
		for (const tls_server_name of [
			"",
			"a..test",
			"-api.test",
			"api-.test",
			"api:443",
			"*.test",
			"host.123",
			`${"a".repeat(64)}.test`,
		])
			expect(
				tunnelServicesSchema.safeParse([{ ...https, tls_server_name }]).success,
			).toBe(false);
		for (const change of [
			{ tls_server_name: undefined },
			{ tls_sha256_fingerprint: null },
			{ tls_sha256_fingerprint: "A".repeat(64) },
			{ protocol: "http" },
			{ protocol: "tcp" },
		])
			expect(
				tunnelServicesSchema.safeParse([{ ...https, ...change }]).success,
			).toBe(false);
	});

	test("binds bounded unique service IDs and includes the implicit hosting listener", () => {
		for (const id of ["hosting", ".", "..", "database:5432", "../database"])
			expect(
				tunnelServicesSchema.safeParse([{ ...database, id }]).success,
			).toBe(false);
		expect(tunnelServicesSchema.safeParse([database, database]).success).toBe(
			false,
		);
		expect(
			tunnelServicesSchema.safeParse(
				Array.from({ length: 17 }, (_, i) => ({
					...database,
					id: `database-${i}`,
				})),
			).success,
		).toBe(false);
		expect(
			tunnelServicesSchema.safeParse([
				{ ...database, host_override: "192.168.1.1" },
			]).success,
		).toBe(false);
		expect(configuredTunnelServices({})).toEqual([]);
		expect(() => configuredTunnelServices({ tunnel_services: null })).toThrow();
		expect(
			configuredTunnelServices({
				hosting: { host: "0.0.0.0", port: 8080 },
				tls_certificate_id: "certificate",
				tunnel_services: [database],
			}),
		).toEqual([
			{ id: "hosting", host: "0.0.0.0", port: 8080, protocol: "https" },
			database,
		]);
	});

	test("project revision updates preserve configured listener identities and pins", () => {
		const input: Parameters<typeof createDeploymentPlan>[0] = {
			installed: {
				project_id: "project",
				project_path: "/private/projects/project/revision",
				revision: "a".repeat(64),
				source: "offline",
			},
			placement: "placement",
			deployment: "deployment",
			events: [
				{
					id: "api",
					name: "API",
					event_type: "http",
					event_version: [1, 0, 0],
					board_version: [1, 0, 0],
					hosted: true,
					eligible: true,
				},
			],
			variables: [],
			overrides: {},
			host: "127.0.0.1",
			port: 8080,
			replicas: 1,
			serviceToken: "",
			serviceAuthentication: "none",
		};
		const initial = createDeploymentPlan(input);
		const existing: PlacementConfiguration = {
			placement_id: "placement",
			project_id: "project",
			deployment_id: "deployment",
			config_revision: 1,
			config: {
				...initial.config,
				tunnel_services: [database, https],
			} as PlacementConfiguration["config"],
		};
		const update = createDeploymentPlan({
			...input,
			existing,
			installed: { ...input.installed, revision: "b".repeat(64) },
		});
		expect(update.config.revision).toBe("b".repeat(64));
		expect(update.config.tunnel_services).toEqual([database, https]);
		expect(existing.config.tunnel_services).toEqual([database, https]);
	});
});

describe("tunnel request header input", () => {
	test("keeps credential values and punctuation while normalizing names", () => {
		expect(
			parseTunnelHeaders(
				"Authorization: Bearer secret\r\nX-Time: 12:34:56\n\n",
			),
		).toEqual({ authorization: "Bearer secret", "x-time": "12:34:56" });
		const headers = parseTunnelHeaders("__proto__: value");
		expect(Object.getPrototypeOf(headers)).toBeNull();
		expect(Object.hasOwn(headers, "__proto__")).toBe(true);
	});
	test("rejects duplicate names, control characters and unbounded UTF-8 input", () => {
		for (const input of [
			"No colon",
			": value",
			"X: first\nx: second",
			"Bad Name: value",
			"X: a\rb",
			"X: a\0b",
			"X: a\tb",
			`X: ${"é".repeat(16 * 1024)}`,
		])
			expect(() => parseTunnelHeaders(input)).toThrow();
	});
});
