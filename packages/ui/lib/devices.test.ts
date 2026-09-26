import { describe, expect, test } from "bun:test";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";
import {
	type DeviceStatus,
	hasRecentDeviceContact,
	listDevices,
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
		const calls: unknown[][] = [];
		const profile = { hub: "example.com" } as IProfile;
		const api = {
			get: async (...args: unknown[]) => {
				calls.push(["GET", ...args]);
				return [device];
			},
			del: async (...args: unknown[]) => {
				calls.push(["DELETE", ...args]);
			},
		} as unknown as IApiState;
		expect(await listDevices(api, profile)).toEqual([device]);
		expect(await revokeDevice(api, profile, "a/b?c#d")).toBeUndefined();
		expect(calls).toEqual([
			["GET", profile, "devices"],
			["DELETE", profile, "devices/a%2Fb%3Fc%23d"],
		]);
	});
});
