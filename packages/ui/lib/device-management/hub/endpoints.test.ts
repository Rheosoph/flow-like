import { describe, expect, test } from "bun:test";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { ApiResponseError } from "../../api-error";
import type { GateId, HubErrorCode } from "../model/types";
import {
	HubError,
	type HubRouteScope,
	getArchiveUsage,
	getBillingEligibility,
	getBillingUsage,
	getDeviceUsage,
	getFleetCertificateInventory,
	getMyAccess,
	getResourceSummary,
	isMissingOnHub,
	listAccountBackups,
	listAppDevicePlacements,
	listCertificateNoticeMutes,
	listCertificateNotices,
	listEnrollments,
	muteCertificateNotices,
	parseHubStandalone,
	readCertificateInventory,
	readHubStandalone,
	sendTestCertificateNotice,
	toHubError,
	unmuteCertificateNotices,
} from "./endpoints";

const profile = { id: "profile", hub: "hub.example.com" } as IProfile;
const CERT = "00000000-0000-4000-8000-000000000001";
type Call = [string, string, unknown?];

function fakeApi(respond: (method: string, path: string) => unknown) {
	const calls: Call[] = [];
	const handle =
		(method: string) =>
		async (_profile: IProfile, path: string, body?: unknown) => {
			calls.push(body === undefined ? [method, path] : [method, path, body]);
			const value = respond(method, path);
			if (value instanceof Error) throw value;
			return value;
		};
	const api = {
		get: handle("GET"),
		put: handle("PUT"),
		post: handle("POST"),
		del: handle("DELETE"),
	} as unknown as IApiState;
	return { api, calls };
}

const refusal = (status: number, code?: string, message = "refused") =>
	new ApiResponseError({ status, code, message });

async function hubError(promise: Promise<unknown>): Promise<HubError> {
	try {
		await promise;
	} catch (error) {
		if (error instanceof HubError) return error;
		throw error;
	}
	throw new Error("expected a HubError");
}

const usage = {
	server_time: 1_727_770_000,
	limits: {
		max_devices: 100,
		max_pending_enrollments: 10,
		enrollment_ttl_seconds: 86_400,
		max_enrollments_per_day: 20,
		max_account_backups: 50,
	},
	usage: {
		active_devices: 3,
		revoked_devices: 1,
		pending_enrollments: 1,
		enrollments_last_24h: 2,
		account_backups: 2,
	},
};
const myAccess = {
	device_id: "a/b",
	role: "grantee",
	owner_id: "owner",
	policy_version: 4,
	policy_expires_at: 2_000,
	applied_version: 3,
	applied: false,
	grants: [
		{
			grant_id: "grant",
			scope: { kind: "project", project_id: "app" },
			capabilities: ["status", "teleport", "logs"],
			expires_at: 1_900,
			controller_key_thumbprint: "thumb",
			group_id: null,
			policy_jws: "never-cached",
		},
	],
};

const routes =
	(answers: Record<string, unknown>) => (method: string, path: string) =>
		answers[`${method} ${path}`];

const scopedAnswers: Record<string, unknown> = {
	"GET devices/a%2Fb/management/my-access": myAccess,
	"GET devices/a%2Fb/resource-grants/g%2F1/billing/eligibility": {
		payer_id: "me",
		plan: null,
		eligible: true,
		models: [],
	},
	"GET devices/a%2Fb/billing-grants/bill%3Fx/usage": {
		billing_grant_id: "bill?x",
		totals: { used_micros: 1, reserved_micros: 0, operations: 1 },
		instances: [],
	},
	"GET apps/app%231/device-placements": { server_time: 1, placements: [] },
	[`GET devices/a%2Fb/certificate-notices?certificate=${CERT}&limit=50`]: [],
	"GET devices/a%2Fb/certificate-notices?limit=50": [],
	"PUT devices/a%2Fb/certificate-notices/mute": {
		certificate_id: CERT,
		until: null,
	},
	"POST devices/a%2Fb/certificate-notices/test": {
		sent: ["push"],
		skipped: [],
	},
};

const accountAnswers: Record<string, unknown> = {
	"GET devices/usage": { ...usage, extra: 1 },
	"GET devices/enrollments?state=recent": [
		{
			enrollment_id: "e1",
			device_id: "d1",
			name: "lab",
			state: "pending",
			created_at: 1,
			expires_at: 2,
			controller_key_thumbprint: "t",
			jwt_id: "never-kept",
		},
	],
	"GET devices/controller-vaults": {
		vaults: [
			{ key_id: "d1", revision: 2, updated_at: 5, public_key_thumbprint: "p" },
		],
		used: 1,
		max: 50,
	},
	"GET devices/certificate-inventory": [
		{
			device_id: "d1",
			revision: 1,
			updated_at: null,
			certificates: [
				{
					certificate_id: CERT,
					revision: 1,
					fingerprint_sha256: "a".repeat(64),
					not_after: 10,
				},
			],
		},
	],
	"GET devices/archive-usage": {
		tier: "PRO",
		max_bytes: 10,
		retention_seconds: 20,
		used_bytes: 5,
		devices: [],
	},
	"GET devices/resource-summary": {
		server_time: 9,
		devices: [
			{
				device_id: "d1",
				approvals: [
					{
						grant_id: "g",
						placement_id: "p",
						app_id: null,
						status: "active",
						expires_at: 10,
						effective_expires_at: 8,
						effective_limit: "access_rules",
						online_access: null,
						online_write_blocked: "quota_exceeded",
						payer_is_me: true,
						approver_is_me: false,
					},
				],
				billing: [],
			},
		],
	},
};

describe("hub endpoint clients", () => {
	test("encode every path segment and build one request per call", async () => {
		const { api, calls } = fakeApi(routes(scopedAnswers));
		const id = "a/b";
		await getMyAccess(api, profile, id);
		await getBillingEligibility(api, profile, id, "g/1");
		await getBillingUsage(api, profile, id, "bill?x");
		await listAppDevicePlacements(api, profile, "app#1");
		await listCertificateNotices(api, profile, id, CERT);
		await listCertificateNotices(api, profile, id);
		await muteCertificateNotices(api, profile, id, CERT, null);
		await unmuteCertificateNotices(api, profile, id, null);
		await unmuteCertificateNotices(api, profile, id, CERT);
		await sendTestCertificateNotice(api, profile, id, "both");
		expect(calls).toEqual([
			["GET", "devices/a%2Fb/management/my-access"],
			["GET", "devices/a%2Fb/resource-grants/g%2F1/billing/eligibility"],
			["GET", "devices/a%2Fb/billing-grants/bill%3Fx/usage"],
			["GET", "apps/app%231/device-placements"],
			["GET", `devices/a%2Fb/certificate-notices?certificate=${CERT}&limit=50`],
			["GET", "devices/a%2Fb/certificate-notices?limit=50"],
			[
				"PUT",
				"devices/a%2Fb/certificate-notices/mute",
				{ certificate_id: CERT, until: null },
			],
			["DELETE", "devices/a%2Fb/certificate-notices/mute?certificate=*"],
			["DELETE", `devices/a%2Fb/certificate-notices/mute?certificate=${CERT}`],
			["POST", "devices/a%2Fb/certificate-notices/test", { channel: "both" }],
		]);
	});

	test("account routes read through to typed results", async () => {
		const { api } = fakeApi(routes(accountAnswers));
		expect(await getDeviceUsage(api, profile)).toEqual({
			kind: "ok",
			data: usage,
		});
		const enrollments = await listEnrollments(api, profile, "recent");
		expect(enrollments.kind === "ok" && enrollments.data[0]).not.toHaveProperty(
			"jwt_id",
		);
		expect(await listAccountBackups(api, profile)).toMatchObject({
			kind: "ok",
			data: { used: 1, max: 50 },
		});
		expect(await getFleetCertificateInventory(api, profile)).toMatchObject({
			kind: "ok",
			data: [{ device_id: "d1" }],
		});
		expect(await getArchiveUsage(api, profile)).toMatchObject({
			kind: "ok",
			data: { tier: "PRO" },
		});
		const summary = await getResourceSummary(api, profile);
		expect(
			summary.kind === "ok" &&
				summary.data.devices[0].approvals[0].online_write_blocked,
		).toBeNull();
	});

	test("a device's certificate inventory is typed, and its failures are coded", async () => {
		const row = {
			certificate_id: CERT,
			revision: 2,
			fingerprint_sha256: "a".repeat(64),
			not_after: 200,
		};
		const { api, calls } = fakeApi(() => ({
			revision: 3,
			certificates: [row],
			ignored: true,
		}));
		expect(await readCertificateInventory(api, profile, "a/b")).toEqual({
			revision: 3,
			updated_at: null,
			certificates: [row],
		});
		expect(calls).toEqual([["GET", "devices/a%2Fb/certificate-inventory"]]);
		expect(
			await hubError(
				readCertificateInventory(
					fakeApi(() => refusal(403)).api,
					profile,
					"edge",
				),
			),
		).toMatchObject({ code: "forbidden", gate: "G4" });
		expect(
			await hubError(
				readCertificateInventory(
					fakeApi(() => ({ revision: -1 })).api,
					profile,
					"edge",
				),
			),
		).toMatchObject({ code: "invalid_response" });
	});

	test("an omitted optional reads like null, and an approval without effective bounds keeps its own", async () => {
		const { api } = fakeApi(
			routes({
				"GET devices/resource-summary": {
					server_time: 9,
					devices: [
						{
							device_id: "d1",
							approvals: [
								{
									grant_id: "g",
									placement_id: "p",
									status: "active",
									expires_at: 10,
									payer_is_me: true,
									approver_is_me: false,
								},
							],
							billing: [],
						},
					],
				},
				"GET apps/app/device-placements": {
					server_time: 1,
					placements: [
						{
							device_id: "d1",
							placement_id: "p",
							deployment_id: "dep",
							relationship: "owner",
							grant: {
								grant_id: "g",
								status: "active",
								expires_at: 10,
								model_ids: [],
								max_instances: 1,
							},
							instances: { active: 0 },
						},
					],
				},
			}),
		);
		const summary = await getResourceSummary(api, profile);
		expect(
			summary.kind === "ok" && summary.data.devices[0].approvals[0],
		).toEqual({
			grant_id: "g",
			placement_id: "p",
			app_id: null,
			status: "active",
			expires_at: 10,
			effective_expires_at: 10,
			effective_limit: "approval",
			online_access: null,
			online_write_blocked: null,
			payer_is_me: true,
			approver_is_me: false,
		});
		const placements = await listAppDevicePlacements(api, profile, "app");
		expect(placements.kind === "ok" && placements.data.placements[0]).toEqual({
			device_id: "d1",
			placement_id: "p",
			deployment_id: "dep",
			relationship: "owner",
			grant: {
				grant_id: "g",
				status: "active",
				expires_at: 10,
				effective_expires_at: 10,
				effective_limit: "approval",
				online_access: null,
				model_ids: [],
				max_instances: 1,
				approved_by_user_id: null,
				created_at: null,
			},
			billing: null,
			instances: { active: 0, newest_lease_expires_at: null },
		});
	});

	test("my-access without a rules expiry reads it as null", async () => {
		const { api } = fakeApi(
			routes({
				"GET devices/d1/management/my-access": {
					device_id: "d1",
					role: "owner",
					owner_id: "me",
					policy_version: 1,
					applied_version: 1,
					applied: true,
					grants: [],
				},
			}),
		);
		const access = await getMyAccess(api, profile, "d1");
		expect(access.kind === "ok" && access.data.policy_expires_at).toBeNull();
	});

	test("the caller's mutes read back after a reload; a hub without the route is missing_on_hub", async () => {
		const { api, calls } = fakeApi((method, path) =>
			method === "GET" && path === "devices/a%2Fb/certificate-notices/mute"
				? [
						{ certificate_id: CERT, until: 1_790_000_000 },
						{ certificate_id: null },
					]
				: refusal(405),
		);
		expect(await listCertificateNoticeMutes(api, profile, "a/b")).toEqual({
			kind: "ok",
			data: [
				{ certificate_id: CERT, until: 1_790_000_000 },
				{ certificate_id: null, until: null },
			],
		});
		expect(calls).toEqual([["GET", "devices/a%2Fb/certificate-notices/mute"]]);
		expect(await listCertificateNoticeMutes(api, profile, "other")).toEqual({
			kind: "missing_on_hub",
		});
	});

	test("a 429 keeps how long the hub asked to wait when the API error carries it", () => {
		const limited = Object.assign(refusal(429, "TOO_MANY_REQUESTS"), {
			retryAfter: 540,
		});
		expect(toHubError(limited, "device")).toMatchObject({
			code: "rate_limited",
			retryAfterS: 540,
		});
		expect(toHubError(refusal(429), "device").retryAfterS).toBeUndefined();
		expect(
			toHubError(Object.assign(refusal(429), { retryAfter: "soon" }), "device")
				.retryAfterS,
		).toBeUndefined();
	});

	test("my-access drops unknown capabilities and refuses another device's answer", async () => {
		const { api } = fakeApi(() => myAccess);
		const result = await getMyAccess(api, profile, "a/b");
		expect(result.kind === "ok" && result.data.grants[0].capabilities).toEqual([
			"status",
			"logs",
		]);
		expect(result.kind === "ok" && result.data.grants[0]).not.toHaveProperty(
			"policy_jws",
		);
		const error = await hubError(getMyAccess(api, profile, "other"));
		expect(error.code).toBe("invalid_response");
		expect(error.message).toContain("GET devices/{id}/management/my-access");
		expect(error.message).toContain("a/b");
	});
});

describe("older hubs and refusals", () => {
	test("404/405 on an account route is missing_on_hub; a coded device 404 is a refusal", async () => {
		const answers: Record<string, Error> = {
			"devices/usage": refusal(404, "NOT_FOUND", "Device not found"),
			"devices/enrollments?state=open": refusal(405),
			"devices/d/management/my-access": refusal(404),
			"apps/app/device-placements": refusal(404),
			"devices/d/certificate-notices?limit=50": refusal(404, "NOT_FOUND"),
		};
		const { api } = fakeApi((_method, path) => answers[path]);
		expect(await getDeviceUsage(api, profile)).toEqual({
			kind: "missing_on_hub",
		});
		expect(await listEnrollments(api, profile)).toEqual({
			kind: "missing_on_hub",
		});
		expect(await getMyAccess(api, profile, "d")).toEqual({
			kind: "missing_on_hub",
		});
		expect(await listAppDevicePlacements(api, profile, "app")).toEqual({
			kind: "missing_on_hub",
		});
		const notFound = await hubError(listCertificateNotices(api, profile, "d"));
		expect(notFound.code).toBe("not_found");
		expect(notFound.status).toBe(404);
	});

	test("refusals map to error codes and gates", () => {
		const cases: [unknown, HubRouteScope, HubErrorCode, GateId?][] = [
			[refusal(401), "account", "unauthorized", "G3"],
			[
				refusal(
					403,
					"FORBIDDEN",
					"Device registry access requires an unrestricted personal access token",
				),
				"account",
				"token_restricted",
				"G3",
			],
			[refusal(403, "FORBIDDEN"), "app", "forbidden", "G12"],
			[refusal(403, "FORBIDDEN"), "device", "forbidden", "G4"],
			[refusal(429), "device", "rate_limited"],
			[refusal(503), "account", "server_error"],
			[refusal(502, "UPSTREAM_UNAVAILABLE"), "account", "server_error"],
			[refusal(408), "account", "timeout"],
			[refusal(400), "device", "invalid_response"],
			[new TypeError("Failed to fetch"), "account", "network"],
			[new Error("Network unavailable: devices"), "account", "network"],
			[
				Object.assign(new Error("deadline"), { name: "RequestTimeoutError" }),
				"account",
				"timeout",
			],
			[new SyntaxError("Unexpected token <"), "account", "invalid_response"],
		];
		for (const [error, scope, code, gate] of cases) {
			const hub = toHubError(error, scope);
			expect([hub.code, hub.gate]).toEqual([code, gate]);
			expect(hub.cause).toBe(error);
		}
		const already = new HubError("network", "x");
		expect(toHubError(already)).toBe(already);
	});

	test("missing_on_hub never hides a refusal or a transport failure", () => {
		expect(isMissingOnHub(refusal(404, "NOT_FOUND"), "device")).toBe(false);
		expect(isMissingOnHub(refusal(404, "NOT_FOUND"), "account")).toBe(true);
		expect(isMissingOnHub(refusal(405), "app")).toBe(true);
		expect(isMissingOnHub(refusal(403), "account")).toBe(false);
		expect(isMissingOnHub(new TypeError("Failed to fetch"), "account")).toBe(
			false,
		);
	});

	test("a malformed body is invalid_response naming the operation", async () => {
		const { api } = fakeApi(() => ({ server_time: "soon" }));
		const error = await hubError(getDeviceUsage(api, profile));
		expect(error.code).toBe("invalid_response");
		expect(error.message).toStartWith("GET devices/usage: unexpected response");
		expect(error.message).toContain("server_time");
	});

	test("billing usage for another approval is refused", async () => {
		const { api } = fakeApi(() => ({
			billing_grant_id: "other",
			totals: { used_micros: 0, reserved_micros: 0, operations: 0 },
			instances: [],
		}));
		expect((await hubError(getBillingUsage(api, profile, "d", "b"))).code).toBe(
			"invalid_response",
		);
	});
});

describe("hub device support (GET /api/v1)", () => {
	const respond =
		(body: string, init: ResponseInit = {}) =>
		async () =>
			new Response(body, init);

	test("reads the standalone block, typed and lenient", async () => {
		const calls: [string, RequestInit | undefined][] = [];
		const fetchImpl = (async (url: string, init?: RequestInit) => {
			calls.push([url, init]);
			return new Response(
				JSON.stringify({
					name: "hub",
					standalone: {
						enabled: true,
						max_devices_per_user: 100,
						max_pending_enrollments_per_user: 10,
						enrollment_ttl_seconds: 86_400,
						telemetry_tiers: { PRO: { max_bytes: 1, retention_seconds: 2 } },
						release_trust: "broken",
					},
				}),
				{ headers: { "content-type": "application/json" } },
			);
		}) as unknown as typeof fetch;
		expect(
			await readHubStandalone("https://hub.example.com", fetchImpl),
		).toEqual({
			enabled: true,
			max_devices_per_user: 100,
			max_pending_enrollments_per_user: 10,
			enrollment_ttl_seconds: 86_400,
			telemetry_tiers: { PRO: { max_bytes: 1, retention_seconds: 2 } },
		});
		expect(calls[0][0]).toBe("https://hub.example.com/api/v1");
		expect(calls[0][1]?.cache).toBe("no-store");
	});

	test("a hub without device support reads as off", () => {
		expect(parseHubStandalone({ name: "hub" })).toEqual({ enabled: false });
		expect(parseHubStandalone({ standalone: null })).toEqual({
			enabled: false,
		});
		expect(parseHubStandalone({ standalone: { enabled: "yes" } })).toEqual({
			enabled: false,
		});
	});

	test("unreachable, failing and non-hub answers are coded errors", async () => {
		const origin = "https://hub.example.com";
		const offline = (async () => {
			throw new TypeError("Failed to fetch");
		}) as unknown as typeof fetch;
		expect((await hubError(readHubStandalone(origin, offline))).code).toBe(
			"network",
		);
		const failing = respond("{}", { status: 503 }) as unknown as typeof fetch;
		expect((await hubError(readHubStandalone(origin, failing))).code).toBe(
			"server_error",
		);
		const portal = respond("<html></html>", {
			headers: { "content-type": "text/html" },
		}) as unknown as typeof fetch;
		expect((await hubError(readHubStandalone(origin, portal))).code).toBe(
			"server_error",
		);
		const list = respond("[]") as unknown as typeof fetch;
		expect((await hubError(readHubStandalone(origin, list))).code).toBe(
			"invalid_response",
		);
	});
});
