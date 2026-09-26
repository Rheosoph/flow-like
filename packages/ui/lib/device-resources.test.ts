import { describe, expect, test } from "bun:test";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";
import {
	type BillingGrant,
	type ResourceGrant,
	approveDeviceBilling,
	createDeviceResourceGrant,
	createResourceRequest,
	eurosToMicros,
	formatEuroMicros,
	importOfflinePlacement,
	importResourcePlacement,
	loadDeviceResources,
	parseExpiry,
	publicResourceBinding,
	revokeDeviceGrant,
} from "./device-resources";

const grant: ResourceGrant = {
	grant_id: "grant",
	device_id: "device",
	placement_id: "placement",
	deployment_id: "deployment",
	project_id: "project",
	app_id: null,
	delegating_user_id: "owner",
	authz_version: 1,
	model_ids: ["bit"],
	max_instances: 2,
	expires_at: 2000,
	status: "active",
};
const billing: BillingGrant = {
	billing_grant_id: "billing",
	grant_id: "grant",
	payer_id: "owner",
	authz_version: 1,
	limit_micros: 10_000_001,
	used_micros: 1,
	reserved_micros: 1_000_000,
	expires_at: 1900,
	status: "active",
};

describe("device resource consent", () => {
	test("online storage is opt-in, permits storage-only leases and does not imply a payer", () => {
		const placement = importResourcePlacement(
			JSON.stringify({
				id: "placement",
				project_id: "project",
				deployment_id: "deployment",
				source: "online",
				variables: { secret: "discard" },
			}),
		);
		expect(placement).toEqual({
			placement_id: "placement",
			deployment_id: "deployment",
			project_id: "project",
			app_id: "project",
		});
		const expiry = new Date(1500 * 1000).toISOString().slice(0, 16);
		expect(() =>
			createResourceRequest(placement, "", "1", expiry, 1000),
		).toThrow();
		const request = createResourceRequest(
			placement,
			"",
			"1",
			expiry,
			1000,
			"read_only",
		);
		expect(request.online_access).toBe("read_only");
		expect(request.model_ids).toEqual([]);
		const storage = {
			...grant,
			app_id: "project",
			online_access: "read_only" as const,
			model_ids: [],
		};
		const binding = JSON.parse(
			publicResourceBinding(storage, undefined, "device", "owner", 1000),
		);
		expect(binding.resource_grant).toEqual({
			grant_id: "grant",
			authz_version: 1,
		});
		expect(binding.resource_grant).not.toHaveProperty("billing_grant_id");
		expect(() =>
			publicResourceBinding(grant, undefined, "device", "owner", 1000),
		).toThrow();
		expect(() =>
			createResourceRequest(
				{ ...placement, app_id: null },
				"",
				"1",
				expiry,
				1000,
				"read_write",
			),
		).toThrow();
	});
	test("converts exact decimal allowances and rejects rounding, signs, exponents, unsafe limits", () => {
		for (const [input, amount] of [
			["0.000001", 1],
			["10.000001", 10_000_001],
			["1000000", 1_000_000_000_000],
			["12.34", 12_340_000],
		] as const)
			expect(eurosToMicros(input)).toBe(amount);
		for (const value of [
			"0",
			"-1",
			"+1",
			"1e2",
			"1,20",
			"0.0000001",
			"1000000.000001",
			"Infinity",
			"NaN",
			"1.",
			" ",
			"9".repeat(100),
		])
			expect(() => eurosToMicros(value)).toThrow();
		expect(formatEuroMicros(10_000_001)).toBe("€10.000001");
		expect(formatEuroMicros(1)).toBe("€0.000001");
		expect(formatEuroMicros(1_000_000_000_000)).toBe("€1000000.00");
	});
	test("imports only offline identity and never retains variables, paths, or embedded credentials", () => {
		const identity = importOfflinePlacement(
			JSON.stringify({
				id: "placement",
				deployment_id: "deployment",
				project_id: "project",
				source: "offline",
				variables: { secret: "DO-NOT-EXPORT" },
				project_path: "/private/path",
				resource_grant: { grant_id: "untrusted" },
			}),
		);
		expect(identity).toEqual({
			placement_id: "placement",
			deployment_id: "deployment",
			project_id: "project",
		});
		for (const source of ["online", null])
			expect(() =>
				importOfflinePlacement(
					JSON.stringify({
						id: "p",
						deployment_id: "d",
						project_id: "a",
						source,
					}),
				),
			).toThrow();
		for (const id of [
			"../project",
			"..",
			"*",
			"name with space",
			"a".repeat(129),
		])
			expect(() =>
				importOfflinePlacement(
					JSON.stringify({
						id,
						deployment_id: "d",
						project_id: "a",
						source: "offline",
					}),
				),
			).toThrow();
		expect(() => importOfflinePlacement("x".repeat(1024 * 1024 + 1))).toThrow(
			"1 MiB",
		);
	});
	test("creates exact offline model scope and validates caps and expiry", () => {
		const placement = {
			placement_id: "p",
			deployment_id: "d",
			project_id: "project",
		};
		const request = createResourceRequest(
			placement,
			"bit.one, bit-two\nbit_three",
			"2",
			"1970-01-01T00:30:00Z",
			1000,
		);
		expect(request).toEqual({
			...placement,
			app_id: null,
			model_ids: ["bit.one", "bit-two", "bit_three"],
			max_instances: 2,
			expires_at: 1800,
		});
		for (const model of [
			"",
			"*",
			"bit bit",
			"a/bit",
			Array.from({ length: 65 }, (_, i) => `bit${i}`).join(","),
		])
			expect(() =>
				createResourceRequest(
					placement,
					model,
					"2",
					"1970-01-01T00:30:00Z",
					1000,
				),
			).toThrow();
		for (const cap of ["0", "101", "1.5", "1e2", "-1"])
			expect(() =>
				createResourceRequest(
					placement,
					"bit",
					cap,
					"1970-01-01T00:30:00Z",
					1000,
				),
			).toThrow();
		expect(() => parseExpiry("bad", 1000, 2000)).toThrow();
		expect(() => parseExpiry("1970-01-01T00:30:00Z", 1800, 2000)).toThrow();
		expect(() => parseExpiry("1970-01-01T00:30:00Z", 1000, 1799)).toThrow();
	});
	test("public binding requires matching current consent and contains only runtime references", () => {
		expect(
			JSON.parse(
				publicResourceBinding(grant, billing, "device", "owner", 1000),
			),
		).toEqual({
			resource_grant: {
				grant_id: "grant",
				authz_version: 1,
				billing_grant_id: "billing",
				billing_authz_version: 1,
			},
		});
		for (const changed of [
			{ ...billing, status: "revoked" as const },
			{ ...billing, grant_id: "other" },
			{ ...billing, expires_at: 1000 },
		])
			expect(() =>
				publicResourceBinding(grant, changed, "device", "owner", 1000),
			).toThrow();
		expect(() =>
			publicResourceBinding(
				{ ...grant, status: "revoked" },
				billing,
				"device",
				"owner",
				1000,
			),
		).toThrow();
		expect(() =>
			publicResourceBinding(grant, billing, "other", "owner", 1000),
		).toThrow();
	});
	test("host sponsorship preserves delegation while binding the exact billing grant", () => {
		const delegated = { ...grant, delegating_user_id: "project-owner" };
		expect(() =>
			publicResourceBinding(
				delegated,
				billing,
				"device",
				"project-owner",
				1000,
			),
		).not.toThrow();
		expect(() =>
			publicResourceBinding(
				delegated,
				billing,
				"device",
				"owner",
				1000,
				"owner",
			),
		).not.toThrow();
		expect(() =>
			publicResourceBinding(
				delegated,
				billing,
				"device",
				"unrelated",
				1000,
				"owner",
			),
		).toThrow();
	});
	test("uses separate authenticated endpoints and removes signed receipts from cached inventory", async () => {
		const calls: unknown[][] = [];
		const profile = { id: "profile" } as IProfile;
		const api = {
			get: async (_profile: unknown, path: string) =>
				path.endsWith("/resource-grants")
					? [grant]
					: path.endsWith("/billing-grants")
						? [billing]
						: [
								{
									instance_id: "instance",
									device_id: "device",
									grant_id: "grant",
									billing_grant_id: "billing",
									registered_at: 1000,
									lease_expires_at: 1600,
									registration_jws: "not-for-cache",
									workload_key: { x: "public" },
								},
							],
			post: async (...args: unknown[]) => {
				calls.push(["POST", ...args]);
				return String(args[1]).endsWith("/billing") ? billing : grant;
			},
			del: async (...args: unknown[]) => {
				calls.push(["DELETE", ...args]);
			},
		} as unknown as IApiState;
		const inventory = await loadDeviceResources(api, profile, "device");
		expect(inventory.instances[0]).not.toHaveProperty("registration_jws");
		expect(inventory.instances[0].purpose).toBe("workload");
		const request = {
			placement_id: "placement",
			deployment_id: "deployment",
			project_id: "project",
			app_id: null,
			model_ids: ["bit"],
			max_instances: 2,
			expires_at: 2000,
		};
		await createDeviceResourceGrant(api, profile, "device", request);
		expect(calls).toHaveLength(1);
		await approveDeviceBilling(api, profile, "device", "a/b", {
			limit_micros: 10_000_001,
			expires_at: 1900,
		});
		await revokeDeviceGrant(api, profile, "a/b", "billing", "grant?x");
		expect(calls[1]).toEqual([
			"POST",
			profile,
			"devices/device/resource-grants/a%2Fb/billing",
			{ limit_micros: 10_000_001, expires_at: 1900 },
		]);
		expect(calls[2]).toEqual([
			"DELETE",
			profile,
			"devices/a%2Fb/billing-grants/grant%3Fx",
		]);
	});
	test("preserves metadata validation identity and rejects unknown purposes", async () => {
		const profile = { id: "profile" } as IProfile;
		let purpose = "rollout_validation";
		const api = {
			get: async (_profile: unknown, path: string) =>
				path.endsWith("/instances")
					? [
							{
								instance_id: "validation",
								purpose,
								device_id: "device",
								grant_id: "grant",
								billing_grant_id: null,
								registered_at: 1000,
								lease_expires_at: 1600,
								registration_jws: "not-for-cache",
							},
						]
					: [],
		} as unknown as IApiState;
		const inventory = await loadDeviceResources(api, profile, "device");
		expect(inventory.instances[0]).toMatchObject({
			instance_id: "validation",
			purpose: "rollout_validation",
			billing_grant_id: null,
		});
		expect(inventory.instances[0]).not.toHaveProperty("registration_jws");
		purpose = "unknown";
		await expect(loadDeviceResources(api, profile, "device")).rejects.toThrow();
	});
});
