import { describe, expect, test } from "bun:test";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";
import {
	type DeviceRow,
	type DeviceStatus,
	getDevice,
	hasRecentDeviceContact,
	listDevices,
	normalizeDisplayName,
	parseDeviceRow,
	renameDevice,
	revokeDevice,
} from "./devices";

const device: DeviceStatus = {
	device_id: "device-1",
	owner_id: "owner-1",
	name: "Server",
	status: "active",
	registered_at: 100,
	last_seen_at: 200,
	auth_epoch: 1,
};
const key = (x: string) => ({
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x,
});
const identity = {
	auth_key: key("auth"),
	telemetry_key: key("telemetry"),
	management_key: Array.from({ length: 32 }, (_, index) => index),
};
const hubRow = {
	...device,
	identity,
	display_name: "Lab GPU (rack 2)",
	relationship: "shared",
	access_expires_at: 3_000,
	access_rules_expire_at: 4_000,
	revoked_at: null,
	cloud_approvals: { resource_grants: 1, billing_grants: 2, expires_at: 5_000 },
	auth_rejection: {
		code: "clock_skew",
		skew_seconds: -420,
		count: 3,
		first_at: 150,
		last_at: 190,
	},
};
const profile = { hub: "example.com" } as IProfile;

function recordingApi(response: (method: string, path: string) => unknown) {
	const calls: unknown[][] = [];
	const handle =
		(method: string) =>
		async (_profile: IProfile, path: string, body?: unknown) => {
			calls.push(body === undefined ? [method, path] : [method, path, body]);
			return response(method, path);
		};
	const api = {
		get: handle("GET"),
		patch: handle("PATCH"),
		del: handle("DELETE"),
	} as unknown as IApiState;
	return { api, calls };
}

describe("device inventory", () => {
	test("recent contact expires and revoked or unseen devices are never shown as recent", () => {
		expect(hasRecentDeviceContact(device, 200_000)).toBe(true);
		expect(hasRecentDeviceContact(device, 320_000)).toBe(true);
		expect(hasRecentDeviceContact(device, 320_001)).toBe(false);
		expect(hasRecentDeviceContact(device, 199_999)).toBe(false);
		expect(
			hasRecentDeviceContact({ ...device, status: "revoked" }, 200_000),
		).toBe(false);
		expect(
			hasRecentDeviceContact({ ...device, last_seen_at: null }, 200_000),
		).toBe(false);
	});

	test("uses the host authenticated API and encodes revocation IDs as one path segment", async () => {
		const { api, calls } = recordingApi((method) =>
			method === "GET" ? [device] : undefined,
		);
		expect(await listDevices(api, profile)).toEqual([device] as DeviceRow[]);
		expect(await revokeDevice(api, profile, "a/b?c#d")).toBeUndefined();
		expect(calls).toEqual([
			["GET", "devices"],
			["DELETE", "devices/a%2Fb%3Fc%23d"],
		]);
	});
});

describe("device rows", () => {
	test("an older hub row parses with every BG field absent", () => {
		const row = parseDeviceRow({ ...device, identity });
		expect(row).toEqual({ ...device, identity });
		for (const field of [
			"display_name",
			"relationship",
			"access_expires_at",
			"access_rules_expire_at",
			"revoked_at",
			"cloud_approvals",
			"auth_rejection",
		] as const)
			expect(row[field]).toBeUndefined();
	});

	test("keeps every BG field the hub sends and drops unknown fields", () => {
		expect(parseDeviceRow({ ...hubRow, future_field: true })).toEqual(
			hubRow as never,
		);
	});

	test("a malformed optional field reads as absent instead of failing the row", () => {
		const row = parseDeviceRow({
			...hubRow,
			relationship: "consent",
			cloud_approvals: { resource_grants: -1 },
			auth_rejection: { code: "unknown" },
			display_name: 7,
		});
		expect(row.relationship).toBeUndefined();
		expect(row.cloud_approvals).toBeUndefined();
		expect(row.auth_rejection).toBeUndefined();
		expect(row.display_name).toBeUndefined();
		expect(row.access_expires_at).toBe(3_000);
	});

	test("rejects rows whose registry facts or identity are malformed", () => {
		expect(() => parseDeviceRow({ ...hubRow, status: "paused" })).toThrow();
		expect(() => parseDeviceRow({ ...hubRow, device_id: "" })).toThrow();
		expect(() =>
			parseDeviceRow({
				...hubRow,
				identity: { ...identity, management_key: [1, 2, 3] },
			}),
		).toThrow();
	});

	test("lists parse every row and refuse a non-array body", async () => {
		const rows = recordingApi(() => [hubRow, { ...device, identity }]);
		expect(await listDevices(rows.api, profile)).toHaveLength(2);
		const broken = recordingApi(() => ({ devices: [] }));
		await expect(listDevices(broken.api, profile)).rejects.toThrow();
	});

	test("getDevice encodes the id and refuses a row for another device", async () => {
		const { api, calls } = recordingApi(() => hubRow);
		expect((await getDevice(api, profile, "device-1")).display_name).toBe(
			"Lab GPU (rack 2)",
		);
		await expect(getDevice(api, profile, "a/b")).rejects.toThrow(
			"The hub returned device device-1 for a request about a/b.",
		);
		expect(calls).toEqual([
			["GET", "devices/device-1"],
			["GET", "devices/a%2Fb"],
		]);
	});
});

describe("device display names", () => {
	test("normalizes, clears blanks and enforces the hub rules before sending", () => {
		expect(normalizeDisplayName("  Lab GPU  ")).toBe("Lab GPU");
		expect(normalizeDisplayName("Café")).toBe("Café");
		expect(normalizeDisplayName("   ")).toBeNull();
		expect(normalizeDisplayName(null)).toBeNull();
		expect(normalizeDisplayName("🙂".repeat(64))).toBe("🙂".repeat(64));
		expect(() => normalizeDisplayName("x".repeat(65))).toThrow(
			"at most 64 characters",
		);
		expect(() => normalizeDisplayName("lab\u0007gpu")).toThrow(
			"control characters",
		);
	});

	test("renameDevice patches one encoded path with the normalized name or null", async () => {
		const { api, calls } = recordingApi(() => hubRow);
		await renameDevice(api, profile, "device-1", "  Lab GPU (rack 2) ");
		await renameDevice(api, profile, "device-1", null);
		await expect(
			renameDevice(api, profile, "device-1", "x".repeat(65)),
		).rejects.toThrow();
		expect(calls).toEqual([
			["PATCH", "devices/device-1", { display_name: "Lab GPU (rack 2)" }],
			["PATCH", "devices/device-1", { display_name: null }],
		]);
	});
});
