import { beforeEach, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import type { FleetRead } from "../fleet";
import { type RetainedObservation, SnapshotIntegrityError } from "../inventory";
import type { LocalDeviceVault } from "../storage";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	ManagementGrant,
} from "../types";
import { LiveCallError } from "./errors";
import {
	FLEET_BACKOFF_MS,
	FLEET_POLL_MS,
	type FleetReaderIo,
	type FleetReaderRuntime,
	createFleetSnapshotReader,
} from "./fleet-reader";
import type {
	ClockModel,
	KeyPort,
	KeySessionSnapshot,
	KeyState,
	WorkspaceDeps,
} from "./types";

const DAY_S = 86_400;
const START_MS = 1_790_000_000_000;

interface Deferred<T> {
	promise: Promise<T>;
	resolve(value: T): void;
	reject(error: unknown): void;
}
function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((ok, fail) => {
		resolve = ok;
		reject = fail;
	});
	return { promise, resolve, reject };
}
async function flush() {
	for (let i = 0; i < 30; i++) await Promise.resolve();
}

let nowMs = START_MS;
const nowS = () => Math.floor(nowMs / 1000);

function fakeKeys() {
	const sessions = new Map<
		string,
		{ state: KeyState; controller?: BrowserController; grantId: string }
	>();
	const listeners = new Set<() => void>();
	const receipts = new Map<string, DeviceReceipt>();
	const notify = () => {
		for (const listener of [...listeners]) listener();
	};
	const open = (id: string) => sessions.get(id)?.controller;
	const port: KeyPort = {
		snapshot: (deviceId): KeySessionSnapshot => ({
			deviceId,
			state: sessions.get(deviceId)?.state ?? "none",
			role: "owner",
			grantId: sessions.get(deviceId)?.grantId ?? "owner",
			canSign: false,
			keepUnlocked: false,
			restoredNeedsFreshEndpoint: false,
		}),
		controller: (id) => open(id),
		vault: (id) =>
			open(id)
				? ({
						deviceId: id,
						grantId: sessions.get(id)?.grantId ?? "owner",
					} as LocalDeviceVault)
				: undefined,
		receipt: (id) => (open(id) ? receipts.get(id) : undefined),
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		touch: () => undefined,
	};
	return {
		port,
		unlock(id: string, grantId = "owner") {
			const controller = {
				publicBundle: () => ({
					device_id: id,
					controller_key: { kty: "OKP", crv: "Ed25519", x: `${id}-key` },
				}),
			} as unknown as BrowserController;
			receipts.set(id, { device_id: id } as DeviceReceipt);
			sessions.set(id, { state: "unlocked", controller, grantId });
			notify();
			return controller;
		},
		set(id: string, state: KeyState) {
			sessions.set(id, { state, grantId: "owner" });
			notify();
		},
		receipt: (id: string) => receipts.get(id),
	};
}

function observation(
	deviceId: string,
	revision: string,
	observedAtS: number,
	bootId = "boot",
): RetainedObservation {
	return {
		scope: { kind: "device" },
		device_id: deviceId,
		boot_id: bootId,
		observed_at: observedAtS * 1000,
		placements: [
			{
				id: `placement-${deviceId}`,
				project_id: "project",
				deployment_id: "deployment",
				revision,
				desired_state: "running",
				observed_state: "running",
				config_revision: 1,
				intent_revision: 1,
				applied_revision: 1,
			},
		],
	};
}

interface ReadSpec {
	sequence?: number;
	bootId?: string;
	revision?: string;
	observedAt?: number;
	expiresAt?: number;
	policyVersion?: number;
	myGrant?: ManagementGrant;
	streams?: "none";
}
function fleetRead(deviceId: string, spec: ReadSpec = {}): FleetRead {
	const observedAt = spec.observedAt ?? nowS();
	const bootId = spec.bootId ?? "boot";
	const id = JSON.stringify(["device", "status", "owner"]);
	const withStream = spec.streams !== "none";
	return {
		observations: withStream
			? [
					observation(
						deviceId,
						spec.revision ?? `revision-${deviceId}`,
						observedAt,
						bootId,
					),
				]
			: [],
		metrics: [],
		readerRevision: 1,
		readerExpiresAt: spec.expiresAt ?? nowS() + 365 * DAY_S,
		...(spec.policyVersion !== undefined && {
			policyVersion: spec.policyVersion,
		}),
		...(spec.myGrant && { myGrant: spec.myGrant }),
		streams: withStream
			? [
					{
						id,
						kind: "status",
						scope: { kind: "device" },
						grantId: "owner",
						sequence: spec.sequence ?? 1,
						bootId,
						observedAt,
					},
				]
			: [],
		authorized: [id],
	};
}

type ReadReply = FleetRead | Error | Promise<FleetRead>;

function fakeIo() {
	const timers = new Map<
		number,
		{ run: () => void; dueAt: number; ms: number }
	>();
	const visibility = new Set<() => void>();
	const calls: string[] = [];
	const receipts: (DeviceReceipt | undefined)[] = [];
	const replies = new Map<string, ReadReply[]>();
	const registrations = new Map<string, Error>();
	const saved = new Map<string, RetainedObservation[]>();
	let timerId = 0;
	let visible = true;
	const reply = (deviceId: string): Promise<FleetRead> => {
		const queued = replies.get(deviceId)?.shift();
		if (queued instanceof Error) return Promise.reject(queued);
		return Promise.resolve(queued ?? fleetRead(deviceId));
	};
	const io: FleetReaderIo = {
		readFleet: async (
			_api,
			_profile,
			_scope,
			_controller,
			vault,
			_crypto,
			_active,
			options,
		) => {
			calls.push(`read ${vault.deviceId}`);
			receipts.push(options?.receipt);
			return reply(vault.deviceId);
		},
		registerFleetReader: async (
			_api,
			_profile,
			_scope,
			_controller,
			vault,
			_receipt,
			_active,
			options,
		) => {
			calls.push(`register ${vault.deviceId}${options?.renew ? " renew" : ""}`);
			const failure = registrations.get(vault.deviceId);
			if (failure) throw failure;
		},
		removeFleetReader: async (_api, _profile, deviceId, key) => {
			calls.push(`remove ${deviceId} ${key}`);
		},
		readSavedInventory: async (_api, _profile, _scope, controller) => {
			const deviceId = controller.publicBundle().device_id;
			calls.push(`saved ${deviceId}`);
			return saved.get(deviceId) ?? [];
		},
		setTimer(run, ms) {
			const id = ++timerId;
			timers.set(id, { run, ms, dueAt: nowMs + ms });
			return id;
		},
		clearTimer: (handle) => timers.delete(handle as number),
		visible: () => visible,
		onVisibilityChange(listener) {
			visibility.add(listener);
			return () => visibility.delete(listener);
		},
	};
	return {
		io,
		calls,
		receipts,
		replies,
		registrations,
		saved,
		timers,
		queue(deviceId: string, ...values: ReadReply[]) {
			replies.set(deviceId, [...(replies.get(deviceId) ?? []), ...values]);
		},
		setVisible(value: boolean) {
			visible = value;
			for (const listener of [...visibility]) listener();
		},
		/** Delays of the armed timers. */
		armed: () => [...timers.values()].map((timer) => timer.ms),
		async advance(ms: number) {
			nowMs += ms;
			for (let round = 0; round < 50; round++) {
				const due = [...timers].filter(([, timer]) => timer.dueAt <= nowMs);
				if (!due.length) break;
				for (const [id, timer] of due) {
					timers.delete(id);
					timer.run();
				}
				await flush();
			}
			await flush();
		},
	};
}

function setup(devices?: () => readonly string[]) {
	const keys = fakeKeys();
	const fake = fakeIo();
	const observed: Parameters<ClockModel["observe"]>[] = [];
	const clock: ClockModel = {
		now: () => nowMs,
		deviceSkewS: () => undefined,
		observe: (...args) => {
			observed.push(args);
		},
	};
	const deps: WorkspaceDeps = {
		api: {} as IApiState,
		profile: {} as IProfile,
		scope: {
			issuer: "issuer",
			account: "reader",
			apiOrigin: "https://hub.test",
			profileId: "profile",
		},
		queryClient: {} as QueryClient,
		crypto: async () => ({}) as DeviceCrypto,
		platform: "web",
		now: () => nowMs,
	};
	const reader: FleetReaderRuntime = createFleetSnapshotReader(
		deps,
		{
			keys: keys.port,
			hub: { fetch: async () => undefined as never },
			clock,
			devices,
		},
		fake.io,
	);
	const reads = (deviceId: string) =>
		fake.calls.filter((call) => call === `read ${deviceId}`).length;
	const revisionOf = (deviceId: string) =>
		reader.get(deviceId)?.status?.observations[0]?.placements[0]?.revision;
	return { keys, fake, reader, observed, reads, revisionOf };
}

beforeEach(() => {
	nowMs = START_MS;
});

test("a watched unlocked device registers once and polls with the key session's receipt", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
	]);
	expect(t.reads("one")).toBe(1);
	expect(t.fake.receipts).toEqual([t.keys.receipt("one")]);
	expect(t.revisionOf("one")).toBe("revision-one");
	expect(t.reader.get("one")?.freshness.status.age).toBe("current");
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reads("one")).toBe(2);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
	]);
	expect(
		t.fake.receipts.every((value) => value === t.keys.receipt("one")),
	).toBe(true);
});

test("devices without open keys are never registered or read", async () => {
	const t = setup();
	t.keys.set("held", "held_elsewhere");
	t.reader.watch("held");
	t.reader.watch("elsewhere");
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.fake.calls).toEqual([]);
	expect(t.fake.armed()).toEqual([]);
	expect(t.reader.get("held")?.freshness.status).toMatchObject({
		age: "locked",
		reason: { code: "unlock_required" },
	});
	expect(t.reader.get("elsewhere")?.freshness.status).toMatchObject({
		age: "notloaded",
		reason: { code: "no_keys_here" },
	});
});

test("locking discards late decrypted reads and the data read with those keys", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	const late = deferred<FleetRead>();
	t.fake.queue("one", late.promise);
	await t.fake.advance(FLEET_POLL_MS.visible);
	t.keys.set("one", "locked");
	expect(t.reader.get("one")?.status).toBeUndefined();
	late.resolve(fleetRead("one", { sequence: 2, revision: "late" }));
	await flush();
	expect(t.reader.get("one")?.status).toBeUndefined();
	expect(t.reader.get("one")?.freshness.status.age).toBe("locked");
	expect(t.fake.armed()).toEqual([]);
});

test("disposing discards in-flight reads and stops every timer", async () => {
	const t = setup();
	t.keys.unlock("one");
	const pending = deferred<FleetRead>();
	t.fake.queue("one", pending.promise);
	let emits = 0;
	t.reader.subscribe(() => emits++);
	t.reader.watch("one");
	await t.fake.advance(0);
	const before = emits;
	t.reader.dispose();
	pending.resolve(fleetRead("one"));
	await flush();
	expect(t.reader.get("one")?.status).toBeUndefined();
	expect(emits).toBe(before);
	expect(t.fake.armed()).toEqual([]);
});

test("independent devices keep only their own decrypted snapshots", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.keys.unlock("two");
	t.reader.watch("one");
	t.reader.watch("two");
	await t.fake.advance(0);
	expect(t.revisionOf("one")).toBe("revision-one");
	expect(t.revisionOf("two")).toBe("revision-two");
	t.keys.set("one", "locked");
	expect(t.reader.get("one")?.status).toBeUndefined();
	expect(t.revisionOf("two")).toBe("revision-two");
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reads("one")).toBe(1);
	expect(t.reads("two")).toBe(2);
});

test("a transient failure keeps the last good status with an error and a retry time, and never locks", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	const good = t.reader.get("one")?.status;
	const readAt = good?.observedAt;
	t.fake.queue(
		"one",
		Object.assign(new Error("Service unavailable"), { status: 503 }),
	);
	await t.fake.advance(FLEET_POLL_MS.visible);
	const state = t.reader.get("one");
	expect(state?.status).toBe(good);
	expect(state?.error).toEqual({
		kind: "network",
		message: "Service unavailable",
		at: nowS(),
		retryAt: nowS() + FLEET_BACKOFF_MS[0] / 1000,
	});
	expect(state?.freshness.status).toMatchObject({
		age: "error",
		error: { code: "server_error" },
		dataFrom: readAt,
	});
	expect(t.keys.port.snapshot("one").state).toBe("unlocked");
	await t.fake.advance(FLEET_BACKOFF_MS[0]);
	expect(t.reads("one")).toBe(3);
	expect(t.reader.get("one")?.error).toBeUndefined();
	expect(t.reader.get("one")?.freshness.status.age).toBe("current");
});

test("failed reads back off 30, 60, 120 and then every 300 seconds", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.fake.queue(
		"one",
		...Array.from({ length: 6 }, () => new Error("Failed to fetch")),
	);
	t.reader.watch("one");
	await t.fake.advance(0);
	const delays: number[] = [];
	for (let attempt = 0; attempt < 5; attempt++) {
		const error = t.reader.get("one")?.error;
		const delay = ((error?.retryAt ?? 0) - (error?.at ?? 0)) * 1000;
		delays.push(delay);
		expect(t.reader.get("one")?.freshness.status.error?.code).toBe("network");
		const reads = t.reads("one");
		await t.fake.advance(delay - 1);
		expect(t.reads("one")).toBe(reads);
		await t.fake.advance(1);
		expect(t.reads("one")).toBe(reads + 1);
	}
	expect(delays).toEqual([30_000, 60_000, 120_000, 300_000, 300_000]);
});

test("a definitive refusal clears the decrypted status and reports No access", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	t.fake.queue("one", Object.assign(new Error("Forbidden"), { status: 403 }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	const state = t.reader.get("one");
	expect(state?.status).toBeUndefined();
	expect(state?.error?.kind).toBe("access");
	expect(state?.freshness.status).toMatchObject({
		age: "noaccess",
		reason: { code: "access_ended" },
	});
});

test("an integrity failure never shows the rejected data, keeps the last good status and never locks", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	const good = t.reader.get("one")?.status;
	t.fake.queue(
		"one",
		new SnapshotIntegrityError(
			"A fleet snapshot moved backwards or changed at the same sequence.",
		),
	);
	await t.fake.advance(FLEET_POLL_MS.visible);
	const state = t.reader.get("one");
	expect(state?.status).toBe(good);
	expect(state?.error?.kind).toBe("integrity");
	expect(state?.freshness.status).toMatchObject({
		age: "error",
		error: { code: "integrity" },
	});
	expect(t.keys.port.snapshot("one").state).toBe("unlocked");
	expect(t.fake.armed()).toEqual([FLEET_BACKOFF_MS[0]]);
});

test("a failed old read cannot touch a newly unlocked controller or delay its first read", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	const old = deferred<FleetRead>();
	t.fake.queue("one", old.promise);
	await t.fake.advance(FLEET_POLL_MS.visible);
	t.keys.set("one", "locked");
	t.keys.unlock("one");
	await t.fake.advance(0);
	expect(t.reads("one")).toBe(3);
	old.reject(new Error("old cancelled poll"));
	await flush();
	expect(t.revisionOf("one")).toBe("revision-one");
	expect(t.reader.get("one")?.error).toBeUndefined();
	expect(t.keys.port.snapshot("one").state).toBe("unlocked");
});

test("polls every 30 s while visible and 120 s while hidden, and catches up when shown", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	await t.fake.advance(FLEET_POLL_MS.visible - 1);
	expect(t.reads("one")).toBe(1);
	t.fake.setVisible(false);
	await t.fake.advance(1);
	expect(t.reads("one")).toBe(2);
	await t.fake.advance(FLEET_POLL_MS.hidden - 1);
	expect(t.reads("one")).toBe(2);
	await t.fake.advance(1);
	expect(t.reads("one")).toBe(3);
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reads("one")).toBe(3);
	t.fake.setVisible(true);
	await t.fake.advance(0);
	expect(t.reads("one")).toBe(4);
});

test("unwatching stops polling and watching again resumes it", async () => {
	const t = setup();
	t.keys.unlock("one");
	const stop = t.reader.watch("one");
	await t.fake.advance(0);
	stop();
	stop();
	expect(t.fake.armed()).toEqual([]);
	await t.fake.advance(FLEET_POLL_MS.hidden);
	expect(t.reads("one")).toBe(1);
	expect(t.revisionOf("one")).toBe("revision-one");
	t.reader.watch("one");
	await t.fake.advance(0);
	expect(t.reads("one")).toBe(2);
});

test("a reader with 30 days or less left is renewed on the next poll", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.fake.queue("one", fleetRead("one", { expiresAt: nowS() + 40 * DAY_S }));
	t.reader.watch("one");
	await t.fake.advance(0);
	const expiresAt = nowS() + 29 * DAY_S;
	t.fake.queue("one", fleetRead("one", { expiresAt }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reader.get("one")?.reader?.expiresAt).toBe(expiresAt);
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
		"register one",
	]);
});

test("exposes the reader expiry, the policy version and the viewer's own grant", async () => {
	const t = setup();
	const grant = {
		grant_id: "grant",
		scope: { kind: "project", project_id: "project" },
		capabilities: ["status"],
		expires_at: nowS() + DAY_S,
	} as ManagementGrant;
	t.keys.unlock("one", "grant");
	t.fake.queue("one", fleetRead("one", { policyVersion: 3, myGrant: grant }));
	t.reader.watch("one");
	await t.fake.advance(0);
	expect(t.reader.get("one")?.reader).toEqual({
		revision: 1,
		expiresAt: nowS() + 365 * DAY_S,
	});
	expect(t.reader.get("one")?.policy).toEqual({ version: 3, myGrant: grant });
});

test("the device clock is sampled only when a status sequence advances", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.observed).toEqual([]);
	const observedAt = nowS() + 5;
	t.fake.queue("one", fleetRead("one", { sequence: 2, observedAt }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.observed).toEqual([["snapshot", observedAt, nowMs, "one"]]);
});

test("a new boot id records the previous boot", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	t.fake.queue("one", fleetRead("one", { sequence: 1, bootId: "rebooted" }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reader.get("one")?.status?.bootId).toBe("rebooted");
	expect(t.reader.get("one")?.previousBootId).toBe("boot");
});

test("an authorized stream keeps its last status until the device publishes the next one", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	const good = t.reader.get("one")?.status;
	t.fake.queue("one", fleetRead("one", { streams: "none" }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reader.get("one")?.status).toBe(good);
	t.fake.queue("one", fleetRead("one", { revision: "same-sequence" }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reader.get("one")?.status).toBe(good);
	t.fake.queue("one", fleetRead("one", { sequence: 2, revision: "next" }));
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.revisionOf("one")).toBe("next");
	t.fake.queue("one", {
		...fleetRead("one", { streams: "none" }),
		authorized: [],
	});
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(t.reader.get("one")?.status).toBeUndefined();
});

test("saved inventory is read every 60 seconds and reported in seconds", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.fake.saved.set("one", [observation("one", "saved", nowS() - 600)]);
	t.reader.watch("one");
	await t.fake.advance(0);
	expect(t.reader.get("one")?.saved).toMatchObject({
		observedAt: nowS() - 600,
	});
	expect(t.reader.get("one")?.freshness.saved.age).not.toBe("notloaded");
	const savedReads = () =>
		t.fake.calls.filter((call) => call === "saved one").length;
	await t.fake.advance(FLEET_POLL_MS.visible);
	expect(savedReads()).toBe(1);
	await t.fake.advance(FLEET_POLL_MS.saved - FLEET_POLL_MS.visible);
	expect(savedReads()).toBe(2);
});

test("refresh reads now and rethrows the status failure", async () => {
	const t = setup();
	t.keys.unlock("one");
	await t.reader.refresh("one");
	expect(t.reads("one")).toBe(1);
	const failure = Object.assign(new Error("Forbidden"), { status: 403 });
	t.fake.queue("one", failure);
	await expect(t.reader.refresh("one")).rejects.toBe(failure);
	expect(t.fake.armed()).toEqual([]);
});

test("a failed registration is retried later and explains a failed first read", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.fake.registrations.set("one", new Error("Failed to fetch"));
	t.fake.queue("one", Object.assign(new Error("Not found"), { status: 404 }));
	t.reader.watch("one");
	await t.fake.advance(0);
	expect(t.reader.get("one")?.freshness.status.error?.code).toBe("network");
	t.fake.registrations.delete("one");
	await t.fake.advance(FLEET_BACKOFF_MS[0]);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
	]);
	await t.fake.advance(300_000);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
		"register one",
	]);
});

test("stop receiving deletes this computer's reader and renew restores it", async () => {
	const t = setup();
	t.keys.unlock("one");
	t.reader.watch("one");
	await t.fake.advance(0);
	await t.reader.stopReceiving("one");
	expect(t.fake.calls).toContain("remove one one-key");
	expect(t.reader.get("one")?.status).toBeUndefined();
	expect(t.reader.get("one")?.reader).toBeUndefined();
	expect(t.reader.get("one")?.freshness.status).toMatchObject({
		age: "noaccess",
		reason: { code: "not_a_reader" },
	});
	await t.fake.advance(FLEET_POLL_MS.hidden);
	await t.reader.refresh("one");
	expect(t.reads("one")).toBe(1);
	expect(t.fake.calls.filter((call) => call.startsWith("register"))).toEqual([
		"register one",
	]);
	await t.reader.renew("one");
	await t.fake.advance(0);
	expect(t.fake.calls).toContain("register one renew");
	expect(t.reads("one")).toBe(2);
	expect(t.revisionOf("one")).toBe("revision-one");
});

test("renew and stop receiving need the device's keys", async () => {
	const t = setup();
	t.keys.set("one", "locked");
	await expect(t.reader.renew("one")).rejects.toBeInstanceOf(LiveCallError);
	await expect(t.reader.stopReceiving("one")).rejects.toMatchObject({
		code: "keys_locked",
	});
	expect(t.fake.calls).toEqual([]);
});

test("coverage counts readable, locked and keyless devices, optionally for one app", async () => {
	const project = (project_id: string) =>
		({
			grant_id: "grant",
			scope: { kind: "project", project_id },
		}) as ManagementGrant;
	const t = setup(() => ["mine", "other", "locked", "keyless"]);
	t.keys.unlock("mine", "grant");
	t.keys.unlock("other", "grant");
	t.keys.set("locked", "locked");
	t.fake.queue(
		"mine",
		fleetRead("mine", { policyVersion: 1, myGrant: project("app") }),
	);
	t.fake.queue(
		"other",
		fleetRead("other", { policyVersion: 1, myGrant: project("elsewhere") }),
	);
	for (const id of ["mine", "other"]) t.reader.watch(id);
	await t.fake.advance(0);
	expect(t.reader.coverage()).toEqual({
		readable: 2,
		locked: 1,
		noKeys: 1,
		total: 4,
	});
	expect(t.reader.coverage("app")).toEqual({
		readable: 1,
		locked: 1,
		noKeys: 1,
		total: 3,
	});
});
