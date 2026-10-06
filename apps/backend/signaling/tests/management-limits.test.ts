import { describe, expect, test } from "bun:test";
import { replicaIdentifier } from "../device-frames";
import {
	CONTROLLER_BUDGET,
	ConnectionSlots,
	DEVICE_AGGREGATE_BUDGET,
	DEVICE_PARTICIPANT_BUDGET,
	DiscardCounter,
	type FrameBudgetLimits,
	FrameBudgets,
	MAX_CONTROLLERS_PER_DEVICE,
	MAX_CONTROLLERS_PER_DEVICE_ACCOUNT,
	ManagementOutbox,
	type OutboxFrame,
	SenderShares,
	connectionSlotsFor,
	managementFrameBudgets,
	soleAccount,
} from "../limits";

const small: FrameBudgetLimits = {
	frames: 2,
	framesPerSecond: 1,
	bytes: 1024,
	bytesPerSecond: 1024,
};

function manualClock() {
	let now = 0;
	const sleeps: number[] = [];
	return {
		now: () => now,
		sleeps,
		sleep: async (ms: number) => {
			sleeps.push(ms);
			now += ms;
		},
	};
}

async function settle() {
	for (let i = 0; i < 200; i++) await Promise.resolve();
}

describe("subject frame budgets", () => {
	test("charges every budget only when all admit and refills over time", () => {
		const budgets = new FrameBudgets();
		const keys = [
			["aggregate", small],
			["participant", small],
		] as const;
		expect(budgets.reserve(keys, 100, 0)).toBe(0);
		expect(budgets.reserve(keys, 100, 0)).toBe(0);
		expect(budgets.reserve(keys, 100, 0)).toBe(1000);
		expect(budgets.reserve(keys, 100, 1000)).toBe(0);
		expect(budgets.reserve([["participant", small]], 4096, 2000)).toBe(0);
		expect(budgets.reserve([["participant", small]], 1, 2000)).toBe(3001);
	});

	test("keeps state across sockets of one subject and forgets idle subjects", () => {
		const budgets = new FrameBudgets();
		for (let i = 0; i < CONTROLLER_BUDGET.frames; i++)
			expect(
				budgets.reserve(
					[["controller:device:1:owner", CONTROLLER_BUDGET]],
					1,
					0,
				),
			).toBe(0);
		expect(
			budgets.reserve([["controller:device:1:owner", CONTROLLER_BUDGET]], 1, 0),
		).toBeGreaterThan(0);
		budgets.sweep(0);
		expect(budgets.size).toBe(1);
		budgets.sweep(60_000);
		expect(budgets.size).toBe(0);
	});
});

describe("account-keyed device budgets", () => {
	const device = {
		role: "device",
		deviceId: "device",
		deviceAuthEpoch: 1,
		subject: "device",
	} as const;

	test("one account's sockets share a single slice of the device's budget", () => {
		const [aggregate, first] = managementFrameBudgets(
			device,
			"tab-1",
			"grantee",
		);
		const [, second] = managementFrameBudgets(device, "tab-2", "grantee");
		expect(aggregate?.[1]).toBe(DEVICE_AGGREGATE_BUDGET);
		expect(first).toEqual(second);
		expect(first?.[1]).toBe(DEVICE_PARTICIPANT_BUDGET);
		const budgets = new FrameBudgets();
		for (let i = 0; i < DEVICE_PARTICIPANT_BUDGET.frames; i++)
			expect(
				budgets.reserve(
					managementFrameBudgets(device, `tab-${i % 8}`, "grantee"),
					1,
					0,
				),
			).toBe(0);
		expect(
			budgets.reserve(managementFrameBudgets(device, "tab-9", "grantee"), 1, 0),
		).toBeGreaterThan(0);
		expect(
			budgets.reserve(managementFrameBudgets(device, "tab-9", "owner"), 1, 0),
		).toBe(0);
	});

	test("a participant whose account this replica cannot see keeps its own slice", () => {
		const [, remote] = managementFrameBudgets(device, "remote-tab", null);
		const [, known] = managementFrameBudgets(device, "remote-tab", "grantee");
		expect(remote?.[0]).not.toBe(known?.[0]);
		expect(
			managementFrameBudgets(
				{ ...device, role: "controller", subject: "grantee" },
				"device",
				null,
			),
		).toEqual([["controller:device:1:grantee", CONTROLLER_BUDGET]]);
	});

	test("resolves a participant to an account only when its sockets agree", () => {
		expect(soleAccount(["grantee", "grantee"])).toBe("grantee");
		expect(soleAccount(["grantee", "owner"])).toBeNull();
		expect(soleAccount(["grantee", undefined])).toBeNull();
		expect(soleAccount([])).toBeNull();
	});
});

describe("sender shares", () => {
	test("only senders filling a device's buffer count as top contributors", () => {
		const shares = new SenderShares(1000);
		shares.add("account:flooder", 400_000, 0);
		shares.add("account:victim", 8_000, 0);
		expect(shares.isTopContributor("account:flooder", 0)).toBeTrue();
		expect(shares.isTopContributor("account:victim", 0)).toBeFalse();
		expect(shares.isTopContributor("account:silent", 0)).toBeFalse();
		shares.add("account:second-flooder", 300_000, 0);
		expect(shares.isTopContributor("account:second-flooder", 0)).toBeTrue();
	});

	test("an earlier flood stops counting once the buffer it filled has drained", () => {
		const shares = new SenderShares(1000);
		shares.add("account:earlier", 400_000, 0);
		shares.add("account:current", 50_000, 5_000);
		expect(shares.isTopContributor("account:current", 5_000)).toBeTrue();
		expect(shares.isTopContributor("account:earlier", 5_000)).toBeFalse();
	});

	test("forgets senders whose share has decayed away", () => {
		const shares = new SenderShares(1000);
		for (let i = 0; i < 64; i++) shares.add(`participant:${i}`, 1, 0);
		shares.add("account:new", 1, 60_000);
		expect(shares.size).toBe(1);
	});
});

describe("management outbox", () => {
	test("delays over-budget frames in order instead of refusing them", async () => {
		const clock = manualClock();
		const budgets = new FrameBudgets();
		const delivered: number[] = [];
		const outbox = new ManagementOutbox(
			(frame, now) =>
				budgets.reserve([[frame.target, small]], frame.bytes, now),
			{ pendingBytes: 4096, pendingBytesPerTarget: 4096 },
			clock.sleep,
			clock.now,
		);
		for (let index = 0; index < 5; index++)
			expect(
				outbox.enqueue({
					target: "controller",
					bytes: 10,
					deliver: async () => {
						delivered.push(index);
					},
				}),
			).toBeTrue();
		await settle();
		expect(delivered).toEqual([0, 1, 2, 3, 4]);
		expect(clock.sleeps.length).toBeGreaterThan(0);
		expect(outbox.pendingBytes).toBe(0);
	});

	test("isolates targets and bounds pending bytes per target and in total", async () => {
		const blocked: Promise<void>[] = [];
		const release: (() => void)[] = [];
		const frame = (target: string, bytes: number): OutboxFrame => ({
			target,
			bytes,
			deliver: () => {
				const pending = new Promise<void>((resolve) => release.push(resolve));
				blocked.push(pending);
				return pending;
			},
		});
		const outbox = new ManagementOutbox(() => 0, {
			pendingBytes: 300,
			pendingBytesPerTarget: 200,
		});
		expect(outbox.enqueue(frame("flooding", 150))).toBeTrue();
		expect(outbox.enqueue(frame("flooding", 60))).toBeFalse();
		expect(outbox.enqueue(frame("quiet", 100))).toBeTrue();
		expect(outbox.enqueue(frame("quiet", 100))).toBeFalse();
		await settle();
		expect(release).toHaveLength(2);
		for (const resolve of release.splice(0)) resolve();
		await settle();
		expect(outbox.pendingBytes).toBe(0);
		expect(outbox.enqueue(frame("flooding", 150))).toBeTrue();
		outbox.close();
		expect(outbox.pendingBytes).toBe(150);
		for (const resolve of release.splice(0)) resolve();
		await settle();
		expect(outbox.pendingBytes).toBe(0);
		expect(outbox.enqueue(frame("flooding", 1))).toBeFalse();
	});

	test("a device reply burst larger than the old per-socket limit drains", async () => {
		const clock = manualClock();
		const budgets = new FrameBudgets();
		let delivered = 0;
		const outbox = new ManagementOutbox(
			(frame, now) =>
				budgets.reserve(
					[[`device:to:${frame.target}`, DEVICE_PARTICIPANT_BUDGET]],
					frame.bytes,
					now,
				),
			{ pendingBytes: 1024 * 1024, pendingBytesPerTarget: 1024 * 1024 },
			clock.sleep,
			clock.now,
		);
		for (let index = 0; index < 400; index++)
			expect(
				outbox.enqueue({
					target: "controller",
					bytes: 2048,
					deliver: async () => {
						delivered++;
					},
				}),
			).toBeTrue();
		for (let i = 0; i < 50 && delivered < 400; i++) await settle();
		expect(delivered).toBe(400);
	});
});

describe("connection slots", () => {
	test("admits only when every slot has room and releases each one", () => {
		const slots = new ConnectionSlots();
		const token = [
			["token:a", 1],
			["account:owner", 2],
		] as const;
		expect(slots.acquire(token)).toBeTrue();
		expect(slots.acquire(token)).toBeFalse();
		expect(slots.count("account:owner")).toBe(1);
		expect(
			slots.acquire([
				["token:b", 1],
				["account:owner", 2],
			]),
		).toBeTrue();
		expect(
			slots.acquire([
				["token:c", 1],
				["account:owner", 2],
			]),
		).toBeFalse();
		expect(slots.count("token:c")).toBe(0);
		slots.release(["token:a", "account:owner"]);
		expect(slots.count("account:owner")).toBe(1);
		expect(slots.acquire(token)).toBeTrue();
	});

	test("grantees filling their per-account share cannot lock the owner out of a device", () => {
		const slots = new ConnectionSlots();
		let token = 0;
		const controller = (subject: string) =>
			connectionSlotsFor(`device-signaling:device:controller:${subject}`, 16, {
				role: "controller",
				deviceId: "device",
				subject,
				tokenId: `token-${token++}`,
			});
		for (let grantee = 0; grantee < 24; grantee++) {
			for (let i = 0; i < MAX_CONTROLLERS_PER_DEVICE_ACCOUNT; i++)
				expect(slots.acquire(controller(`grantee-${grantee}`))).toBeTrue();
			expect(slots.acquire(controller(`grantee-${grantee}`))).toBeFalse();
		}
		for (let i = 0; i < MAX_CONTROLLERS_PER_DEVICE_ACCOUNT; i++)
			expect(slots.acquire(controller("owner"))).toBeTrue();
		expect(slots.count("device-signaling-controllers:device")).toBe(
			MAX_CONTROLLERS_PER_DEVICE,
		);
	});

	test("an operator's lower per-subject limit still applies to controllers", () => {
		const admission = {
			role: "controller",
			deviceId: "device",
			subject: "owner",
			tokenId: "token",
		} as const;
		expect(connectionSlotsFor("subject", 2, admission)[0]).toEqual([
			"subject",
			2,
		]);
		expect(
			connectionSlotsFor("subject", 16, { ...admission, role: "device" }),
		).toEqual([["subject", 16]]);
		expect(connectionSlotsFor(null, 16, null)).toEqual([]);
	});
});

describe("relay diagnostics", () => {
	test("summarizes discarded frames by reason without payloads", () => {
		const counter = new DiscardCounter();
		expect(counter.drain()).toBeNull();
		counter.add("invalid-fanout-envelope");
		counter.add("invalid-fanout-envelope");
		counter.add("device-congested");
		expect(counter.drain()).toBe(
			"invalid-fanout-envelope=2, device-congested=1",
		);
		expect(counter.drain()).toBeNull();
	});

	test("requires a replica id that peers accept in fan-out envelopes", () => {
		expect(replicaIdentifier(undefined, () => "generated-id")).toBe(
			"generated-id",
		);
		expect(replicaIdentifier(" replica-1.zone_a ", () => "unused")).toBe(
			"replica-1.zone_a",
		);
		for (const invalid of ["pod:1", "a/b", "x".repeat(129), ".."])
			expect(() => replicaIdentifier(invalid, () => "unused")).toThrow(
				"NODE_ID",
			);
	});
});
