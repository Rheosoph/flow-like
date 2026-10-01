import { describe, expect, test } from "bun:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { ApiResponseError } from "../../api-error";
import { getApiOrigin } from "../../api-url";
import { HubError } from "./endpoints";
import {
	DEVICE_QUERY_GC_MS,
	HUB_CADENCE,
	MAX_BACKOFF_MS,
	type PollState,
	consecutiveFailures,
	deviceKeys,
	deviceQueryDefaults,
	hubDeviceSupport,
	pollInterval,
	queries,
	releaseConfigOf,
	retryHubQuery,
} from "./queries";

const profile = { id: "profile", hub: "hub.example.com" } as IProfile;
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
		revoked_devices: 0,
		pending_enrollments: 1,
		enrollments_last_24h: 1,
		account_backups: 1,
	},
};
const identity = {
	auth_key: { kty: "OKP", crv: "Ed25519", x: "a" },
	telemetry_key: { kty: "OKP", crv: "Ed25519", x: "t" },
	management_key: Array.from({ length: 32 }, () => 1),
};
const row = {
	device_id: "d1",
	owner_id: "me",
	name: "lab",
	status: "active",
	registered_at: 1,
	last_seen_at: 2,
	auth_epoch: 1,
	identity,
};

function context(respond: (path: string) => unknown) {
	const paths: string[] = [];
	const observed: [string, number, number][] = [];
	const api = {
		get: async (_profile: IProfile, path: string) => {
			paths.push(path);
			const value = respond(path);
			if (value instanceof Error) throw value;
			return value;
		},
		fetch: async (_profile: IProfile, path: string) => {
			paths.push(path);
			return respond(path);
		},
	} as unknown as IApiState;
	const ctx = {
		api,
		profile,
		scopeKey: "scope",
		now: () => 1_727_770_002_500,
		clock: {
			observe: (source: string, hubTimeS: number, localMs: number) =>
				observed.push([source, hubTimeS, localMs]),
		},
	};
	return { ctx, paths, observed };
}

const policyView = (version: number) => ({
	policy_jws: null,
	version,
	digest: null,
	applied_version: version,
	applied_digest: null,
});

const poll = (state: Partial<PollState>): PollState => ({
	data: undefined,
	error: null,
	dataUpdateCount: 0,
	errorUpdateCount: 0,
	dataUpdatedAt: 0,
	errorUpdatedAt: 0,
	...state,
});

describe("device query keys", () => {
	test("every key sits under the account scope and per-device keys under their kind", () => {
		const keys = [
			deviceKeys.hub("s"),
			deviceKeys.list("s"),
			deviceKeys.usage("s"),
			deviceKeys.enrollments("s"),
			deviceKeys.myAccess("s", "d"),
			deviceKeys.accountBackups("s"),
			deviceKeys.certInventoryAll("s"),
			deviceKeys.certNotices("s", "d"),
			deviceKeys.archiveUsage("s"),
			deviceKeys.resourceSummary("s"),
			deviceKeys.appPlacements("s", "app"),
		];
		for (const key of keys)
			expect(key.slice(0, 2)).toEqual([...deviceKeys.root("s")]);
		expect(new Set(keys.map((key) => JSON.stringify(key))).size).toBe(
			keys.length,
		);
		expect(deviceKeys.certInventory("s", "d").slice(0, 3)).toEqual([
			...deviceKeys.certInventoryAll("s"),
		]);
		expect(deviceKeys.enrollments("s", "recent")).not.toEqual(
			deviceKeys.enrollments("s"),
		);
		expect(deviceKeys.certNotices("s", "d", "c")).not.toEqual(
			deviceKeys.certNotices("s", "d"),
		);
	});
});

describe("poll schedule", () => {
	test("polls at the base cadence while reads succeed", () => {
		expect(pollInterval(30_000, poll({ dataUpdateCount: 3 }))).toBe(30_000);
	});

	test("backs off as min(base·2^failures, 5 min) and keeps polling", () => {
		const neverRead = (errors: number) =>
			poll({ error: new Error("x"), errorUpdateCount: errors });
		expect(pollInterval(30_000, neverRead(1))).toBe(60_000);
		expect(pollInterval(30_000, neverRead(2))).toBe(120_000);
		expect(pollInterval(30_000, neverRead(3))).toBe(240_000);
		expect(pollInterval(30_000, neverRead(4))).toBe(MAX_BACKOFF_MS);
		expect(pollInterval(30_000, neverRead(40))).toBe(MAX_BACKOFF_MS);

		const afterGood = (sinceGoodMs: number) =>
			poll({
				error: new Error("x"),
				dataUpdateCount: 5,
				errorUpdateCount: 9,
				dataUpdatedAt: 1_000_000,
				errorUpdatedAt: 1_000_000 + sinceGoodMs,
			});
		// Failures at base, then 2·base, 4·base later: 30 s, 90 s, 210 s after the last good read.
		expect(consecutiveFailures(30_000, afterGood(30_000))).toBe(1);
		expect(consecutiveFailures(30_000, afterGood(36_000))).toBe(1);
		expect(consecutiveFailures(30_000, afterGood(90_000))).toBe(2);
		expect(consecutiveFailures(30_000, afterGood(210_000))).toBe(3);
		expect(pollInterval(30_000, afterGood(30_000))).toBe(60_000);
		expect(pollInterval(30_000, afterGood(90_000))).toBe(120_000);
		expect(pollInterval(30_000, afterGood(3_600_000))).toBe(MAX_BACKOFF_MS);
	});

	test("never polls faster than the base, and an older hub is asked rarely", () => {
		const failing = poll({ error: new Error("x"), errorUpdateCount: 1 });
		expect(pollInterval(3_600_000, failing)).toBe(3_600_000);
		expect(pollInterval(300_000, failing)).toBe(300_000);
		const missing = poll({
			data: { kind: "missing_on_hub" },
			dataUpdateCount: 1,
		});
		expect(pollInterval(30_000, missing)).toBe(MAX_BACKOFF_MS);
		expect(pollInterval(3_600_000, missing)).toBe(3_600_000);
	});

	test("retries transient failures twice and never retries a refusal", () => {
		const network = new TypeError("Failed to fetch");
		expect(retryHubQuery(0, network)).toBe(true);
		expect(retryHubQuery(1, network)).toBe(true);
		expect(retryHubQuery(2, network)).toBe(false);
		expect(
			retryHubQuery(0, new ApiResponseError({ status: 503, message: "" })),
		).toBe(true);
		for (const status of [401, 403, 404, 429])
			expect(
				retryHubQuery(0, new ApiResponseError({ status, message: "no" })),
			).toBe(false);
		expect(retryHubQuery(0, new HubError("invalid_response", "bad"))).toBe(
			false,
		);
	});

	test("shared defaults keep data across route switches and are never persisted", () => {
		const live = deviceQueryDefaults(HUB_CADENCE.list);
		expect(live.gcTime).toBe(DEVICE_QUERY_GC_MS);
		expect(DEVICE_QUERY_GC_MS).toBe(5 * 60_000);
		expect(live).not.toHaveProperty("placeholderData");
		expect(live.meta).toEqual({ persist: false });
		expect(live.refetchIntervalInBackground).toBe(false);
		expect(live.staleTime).toBe(15_000);
		expect(deviceQueryDefaults(HUB_CADENCE.device).refetchInterval).toBe(false);
	});
});

describe("queries over a real QueryClient", () => {
	test("a failing poll keeps the last rows, codes the error and lengthens the interval", async () => {
		let broken = false;
		const { ctx } = context(() => (broken ? { not: "a list" } : [row]));
		const client = new QueryClient();
		const options = queries.list(ctx);
		expect(await client.fetchQuery(options)).toHaveLength(1);
		const query = client.getQueryCache().find({ queryKey: options.queryKey });
		if (!query) throw new Error("query missing");
		const interval = options.refetchInterval as unknown as (q: {
			state: PollState;
		}) => number;
		expect(interval(query)).toBe(30_000);

		broken = true;
		await query.fetch().catch(() => undefined);
		expect(query.state.data).toHaveLength(1);
		expect(query.state.error).toBeInstanceOf(HubError);
		expect((query.state.error as HubError).code).toBe("invalid_response");
		expect((query.state.error as HubError).message).toStartWith("GET devices:");
		expect(query.state.fetchFailureCount).toBe(1);
		expect(interval(query)).toBeGreaterThanOrEqual(60_000);
		client.clear();
	});

	test("another device's data never stands in while a device's query loads", async () => {
		const second: { release?: () => void } = {};
		const { ctx } = context((path) =>
			path.startsWith("devices/d2/")
				? new Promise((resolve) => {
						second.release = () => resolve(policyView(2));
					})
				: policyView(1),
		);
		const client = new QueryClient();
		const observer = new QueryObserver(client, queries.policy(ctx, "d1"));
		const unsubscribe = observer.subscribe(() => undefined);
		await client.fetchQuery(queries.policy(ctx, "d1"));
		expect(observer.getCurrentResult().data?.version).toBe(1);

		observer.setOptions(queries.policy(ctx, "d2"));
		const loading = observer.getCurrentResult();
		expect([loading.data, loading.isPlaceholderData, loading.status]).toEqual([
			undefined,
			false,
			"pending",
		]);
		await Promise.resolve();
		second.release?.();
		await client.fetchQuery(queries.policy(ctx, "d2"));
		expect(observer.getCurrentResult().data?.version).toBe(2);

		observer.setOptions(queries.policy(ctx, "d1"));
		expect(observer.getCurrentResult().data?.version).toBe(1);
		unsubscribe();
		client.clear();
	});

	test("401 is coded with its gate and not retried", async () => {
		const { ctx, paths } = context(
			() => new ApiResponseError({ status: 401, message: "expired" }),
		);
		const client = new QueryClient();
		const error = await client
			.fetchQuery(queries.list(ctx))
			.catch((caught: unknown) => caught);
		expect(error).toBeInstanceOf(HubError);
		expect((error as HubError).code).toBe("unauthorized");
		expect((error as HubError).gate).toBe("G3");
		expect(paths).toEqual(["devices"]);
		client.clear();
	});

	test("new routes resolve to missing_on_hub on an older hub", async () => {
		const { ctx } = context(
			() => new ApiResponseError({ status: 405, message: "" }),
		);
		const client = new QueryClient();
		expect(await client.fetchQuery(queries.enrollments(ctx))).toEqual({
			kind: "missing_on_hub",
		});
		client.clear();
	});

	test("usage, resource summary and app placements feed the clock with server_time", async () => {
		const { ctx, observed, paths } = context((path) => {
			if (path === "devices/usage") return usage;
			if (path === "devices/resource-summary")
				return { server_time: 1_727_770_001, devices: [] };
			if (path === "apps/app%2F1/device-placements")
				return { server_time: 1_727_770_002, placements: [] };
			return new ApiResponseError({ status: 404, message: "" });
		});
		const client = new QueryClient();
		await client.fetchQuery(queries.usage(ctx));
		await client.fetchQuery(queries.resourceSummary(ctx));
		await client.fetchQuery(queries.appPlacements(ctx, "app/1"));
		expect(await client.fetchQuery(queries.myAccess(ctx, "d"))).toEqual({
			kind: "missing_on_hub",
		});
		expect(observed).toEqual([
			["server_time", 1_727_770_000, 1_727_770_002_500],
			["server_time", 1_727_770_001, 1_727_770_002_500],
			["server_time", 1_727_770_002, 1_727_770_002_500],
		]);
		expect(paths).toEqual([
			"devices/usage",
			"devices/resource-summary",
			"apps/app%2F1/device-placements",
			"devices/d/management/my-access",
		]);
		client.clear();
	});

	test("the hub JSON query reads the profile's origin and refetches on focus", async () => {
		const urls: string[] = [];
		const { ctx } = context(() => undefined);
		const options = queries.hub({
			...ctx,
			fetch: (async (url: string) => {
				urls.push(url);
				return new Response(JSON.stringify({ standalone: { enabled: true } }));
			}) as unknown as typeof fetch,
		});
		expect(options.refetchOnWindowFocus).toBe(true);
		const client = new QueryClient();
		expect(await client.fetchQuery(options)).toEqual({ enabled: true });
		expect(urls).toEqual([`${getApiOrigin(profile)}/api/v1`]);
		client.clear();
	});

	test("the release query stays idle without release trust", () => {
		const { ctx } = context(() => undefined);
		const idle = queries.release(ctx, undefined);
		expect(typeof idle.queryFn).not.toBe("function");
		const armed = queries.release(ctx, {
			manifestUrl: "https://releases.example.com/manifest.jws",
			publicKeys: ["k"],
			minimumSequence: 0,
		});
		expect(typeof armed.queryFn).toBe("function");
	});
});

describe("hub device support (G2)", () => {
	const standalone = {
		enabled: true,
		telemetry_tiers: { PRO: { max_bytes: 1, retention_seconds: 2 } },
		release_trust: {
			manifest_url: "https://releases.example.com/m.jws",
			public_keys: ["k"],
			minimum_sequence: 3,
		},
	};

	test("tells checking, unreachable, off and on apart", () => {
		expect(hubDeviceSupport({})).toEqual({ state: "checking" });
		expect(
			hubDeviceSupport({ error: new TypeError("Failed to fetch") }),
		).toEqual({ state: "unreachable", error: { code: "network" } });
		expect(hubDeviceSupport({ data: { enabled: false } })).toEqual({
			state: "off",
		});
		expect(
			hubDeviceSupport(
				{ data: standalone },
				{ data: { kind: "ok", data: usage } },
			),
		).toEqual({
			state: "on",
			limits: usage.limits,
			usage: usage.usage,
			serverTime: usage.server_time,
			releaseTrust: standalone.release_trust,
			telemetryTiers: standalone.telemetry_tiers,
		});
	});

	test("a failing refresh keeps the known state and adds the error; an older hub shows no usage", () => {
		const support = hubDeviceSupport(
			{
				data: { enabled: true },
				error: new ApiResponseError({ status: 503, message: "down" }),
			},
			{ data: { kind: "missing_on_hub" } },
		);
		expect(support).toEqual({
			state: "on",
			error: { code: "server_error" },
		});
	});

	test("an older hub still states its limits, without usage (BG3 interim)", () => {
		expect(
			hubDeviceSupport(
				{
					data: {
						enabled: true,
						max_devices_per_user: 100,
						max_pending_enrollments_per_user: 10,
						enrollment_ttl_seconds: 86_400,
					},
				},
				{ data: { kind: "missing_on_hub" } },
			),
		).toEqual({
			state: "on",
			configuredLimits: {
				max_devices: 100,
				max_pending_enrollments: 10,
				enrollment_ttl_seconds: 86_400,
			},
		});
	});

	test("release trust maps to the verifier's config", () => {
		expect(releaseConfigOf(standalone)).toEqual({
			manifestUrl: "https://releases.example.com/m.jws",
			publicKeys: ["k"],
			minimumSequence: 3,
		});
		expect(releaseConfigOf({ enabled: true })).toBeUndefined();
		expect(releaseConfigOf(undefined)).toBeUndefined();
	});
});
