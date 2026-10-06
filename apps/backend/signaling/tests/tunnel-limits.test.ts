import { describe, expect, test } from "bun:test";
import {
	type DeviceTransportAdmission,
	validateDeviceRenewal,
} from "../device-auth";
import { relayDeviceFrame } from "../device-frames";
import {
	FrameBudgets,
	ManagementOutbox,
	PendingByteBudget,
	TUNNEL_PARTICIPANT_BUDGET,
	managementFrameBudgets,
	tunnelFrameBudgets,
} from "../limits";

const admission: DeviceTransportAdmission = {
	deviceId: "device",
	deviceAuthEpoch: 1,
	participantId: "studio",
	role: "controller",
	subject: "owner",
	expiresAtMs: 1000,
	tokenId: "old",
};

describe("tunnel isolation", () => {
	test("bulk allowances cannot consume management capacity or multiply across sockets", () => {
		const budgets = new FrameBudgets();
		const tunnel = tunnelFrameBudgets(admission, "device", null);
		for (let i = 0; i < TUNNEL_PARTICIPANT_BUDGET.frames; i++)
			expect(budgets.reserve(tunnel, 1, 0)).toBe(0);
		expect(budgets.reserve(tunnel, 1, 0)).toBeGreaterThan(0);
		expect(
			budgets.reserve(managementFrameBudgets(admission, "device", null), 1, 0),
		).toBe(0);
		expect(
			tunnelFrameBudgets({ ...admission, subject: "owner" }, "device", null),
		).toEqual(tunnel);
		const device = { ...admission, role: "device" as const };
		expect(tunnelFrameBudgets(device, "tab-a", "owner")).toEqual(
			tunnelFrameBudgets(device, "tab-b", "owner"),
		);
	});

	test("replica byte bound is shared and released once on close during delivery", async () => {
		const shared = new PendingByteBudget(10);
		let complete!: () => void;
		const blocked = new Promise<void>((resolve) => {
			complete = resolve;
		});
		const limits = { pendingBytes: 10, pendingBytesPerTarget: 10 };
		const a = new ManagementOutbox(
			() => 0,
			limits,
			undefined,
			undefined,
			shared,
		);
		const b = new ManagementOutbox(
			() => 0,
			limits,
			undefined,
			undefined,
			shared,
		);
		expect(
			a.enqueue({ target: "device", bytes: 8, deliver: () => blocked }),
		).toBe(true);
		expect(
			b.enqueue({ target: "device", bytes: 8, deliver: () => blocked }),
		).toBe(false);
		a.close();
		a.close();
		expect(shared.pendingBytes).toBe(8);
		expect(
			b.enqueue({ target: "device", bytes: 8, deliver: () => blocked }),
		).toBe(false);
		complete();
		await blocked;
		await Promise.resolve();
		expect(shared.pendingBytes).toBe(0);
		expect(
			b.enqueue({ target: "device", bytes: 8, deliver: () => blocked }),
		).toBe(true);
		await Promise.resolve();
		b.close();
		expect(shared.pendingBytes).toBe(0);
	});

	test("closing a throttled queue releases waiting payloads before its timer wakes", async () => {
		const shared = new PendingByteBudget(10);
		let wake!: () => void;
		const sleeping = new Promise<void>((resolve) => {
			wake = resolve;
		});
		const outbox = new ManagementOutbox(
			() => 1000,
			{ pendingBytes: 10, pendingBytesPerTarget: 10 },
			() => sleeping,
			undefined,
			shared,
		);
		let delivered = false;
		expect(
			outbox.enqueue({
				target: "device",
				bytes: 8,
				deliver: async () => {
					delivered = true;
				},
			}),
		).toBe(true);
		outbox.close();
		expect(shared.pendingBytes).toBe(0);
		expect(outbox.pendingBytes).toBe(0);
		wake();
		await sleeping;
		expect(delivered).toBe(false);
		expect(shared.pendingBytes).toBe(0);
	});

	test("opaque binary envelopes retain the existing device routing boundary", () => {
		const payload = Buffer.from([70, 76, 84, 69, 3, 1, 115, 255]).toString(
			"base64url",
		);
		const frame = {
			type: "frame",
			to: "device",
			channel: "tunnel",
			payload,
		} as const;
		expect(relayDeviceFrame(admission, frame).frame).toEqual({
			...frame,
			from: "studio",
			from_role: "controller",
		});
		expect(() =>
			relayDeviceFrame(admission, { ...frame, to: "other-device" }),
		).toThrow();
	});
});

describe("in-place signaling renewal", () => {
	const renewed = { ...admission, expiresAtMs: 2000, tokenId: "fresh" };
	test("requires a fresh, longer lease for exactly the same identity", () => {
		expect(() => validateDeviceRenewal(admission, renewed, 500)).not.toThrow();
		for (const change of [
			{ deviceId: "other" },
			{ deviceAuthEpoch: 2 },
			{ subject: "other" },
			{ participantId: "other" },
			{ role: "device" as const },
			{ expiresAtMs: 1000 },
			{ tokenId: "old" },
		])
			expect(() =>
				validateDeviceRenewal(admission, { ...renewed, ...change }, 500),
			).toThrow();
		expect(() => validateDeviceRenewal(admission, renewed, 1000)).toThrow();
	});
});
