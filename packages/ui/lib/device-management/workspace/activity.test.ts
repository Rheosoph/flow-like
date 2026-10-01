import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	pendingArtifactTransfers,
	rememberArtifactTransfer,
} from "../artifacts";
import type { DeviceAccountScope } from "../storage";
import { accountStorageKey } from "../storage";
import type { ManagementResponse } from "../types";
import {
	ACTIVITY_STORAGE_PREFIX,
	type ActivityStart,
	createActivityTracker,
} from "./activity";
import type {
	ActivityItem,
	HubPort,
	KeyPort,
	KeySessionSnapshot,
	LivePort,
	LiveState,
	WorkspaceDeps,
} from "./types";

const DAY = 86_400_000;
const T0 = Date.UTC(2026, 9, 1, 10);
const SHA = "a".repeat(64);

const account = (name = "ana"): DeviceAccountScope => ({
	issuer: "https://id.example",
	account: name,
	apiOrigin: "https://hub.example",
	profileId: "default",
});

let values: Map<string, string>;
let windowListeners: ((event: { key: string | null }) => void)[];
let previousStorage: PropertyDescriptor | undefined;
let previousWindow: PropertyDescriptor | undefined;

beforeEach(() => {
	values = new Map();
	windowListeners = [];
	previousStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			get length() {
				return values.size;
			},
			key: (index: number) => [...values.keys()][index] ?? null,
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => values.set(key, value),
			removeItem: (key: string) => values.delete(key),
		},
	});
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			addEventListener: (_: string, listener: never) =>
				windowListeners.push(listener),
			removeEventListener: (_: string, listener: never) => {
				windowListeners = windowListeners.filter((entry) => entry !== listener);
			},
		},
	});
});

afterEach(() => {
	for (const [name, previous] of [
		["localStorage", previousStorage],
		["window", previousWindow],
	] as const)
		if (previous) Object.defineProperty(globalThis, name, previous);
		else Reflect.deleteProperty(globalThis, name);
});

type Reply = (
	deviceId: string,
	command: Record<string, unknown>,
) => ManagementResponse | Promise<ManagementResponse>;

function harness(options: { scope?: DeviceAccountScope; reply?: Reply } = {}) {
	const clock = { now: T0 };
	const unlocked = new Set<string>(["edge-1"]);
	const known = new Set<string>(["edge-1", "edge-2"]);
	const live = new Map<string, LiveState>();
	const hub = new Map<string, unknown>();
	const sent: { deviceId: string; command: Record<string, unknown> }[] = [];
	let acquired = 0;
	const keys = {
		snapshot: (deviceId: string) =>
			({
				deviceId,
				state: unlocked.has(deviceId)
					? "unlocked"
					: known.has(deviceId)
						? "locked"
						: "none",
			}) as KeySessionSnapshot,
	} as unknown as KeyPort;
	const livePort = {
		acquire: () => {
			acquired++;
			return () => {
				acquired--;
			};
		},
		call: (deviceId: string) => async (command: Record<string, unknown>) => {
			sent.push({ deviceId, command });
			if (!options.reply) throw new Error("no device");
			return options.reply(deviceId, command);
		},
		state: (deviceId: string) => live.get(deviceId) ?? { kind: "idle" },
	} as unknown as LivePort;
	const hubPort: HubPort = {
		fetch: async <T>(path: string) => {
			if (!hub.has(path))
				throw Object.assign(new Error("404"), { status: 404 });
			return hub.get(path) as T;
		},
	};
	const create = () =>
		createActivityTracker(
			{
				scope: options.scope ?? account(),
				now: () => clock.now,
			} as unknown as WorkspaceDeps,
			{ live: livePort, keys, hub: hubPort },
		);
	return {
		clock,
		unlocked,
		known,
		live,
		hub,
		sent,
		create,
		acquired: () => acquired,
	};
}

const completed = (
	result: Record<string, unknown>,
	operation_id = "lookup",
	state = "completed",
): ManagementResponse => ({ operation_id, state, result });

function item(overrides: Partial<ActivityStart> = {}): ActivityStart {
	return {
		kind: "command",
		target: { deviceId: "edge-1", serviceId: "invoice" },
		state: "active",
		label: { code: "command" },
		startedBy: "you",
		actions: [],
		...overrides,
	};
}

const stored = (scope = account()) =>
	JSON.parse(
		values.get(`${ACTIVITY_STORAGE_PREFIX}${accountStorageKey(scope)}`) ??
			"null",
	);

describe("persistence", () => {
	test("items persist per account scope and survive a reload", () => {
		const h = harness();
		const tray = h.create();
		const id = tray.start(item());
		expect(stored().items.map((entry: ActivityItem) => entry.id)).toEqual([id]);
		expect(
			h
				.create()
				.list()
				.map((entry) => entry.id),
		).toEqual([id]);
		expect(
			harness({ scope: account("ben") })
				.create()
				.list(),
		).toEqual([]);
	});

	test("only contract fields are stored, so nothing secret reaches storage", () => {
		const tray = harness().create();
		tray.start({
			...item(),
			password: "hunter2",
			target: { deviceId: "edge-1", token: "jwt" },
		} as unknown as ActivityStart);
		const raw = JSON.stringify(stored());
		expect(raw).not.toContain("hunter2");
		expect(raw).not.toContain("jwt");
	});

	test("an invalid item is refused with the failing field", () => {
		const tray = harness().create();
		expect(() =>
			tray.start({ ...item(), kind: "teleport" } as unknown as ActivityStart),
		).toThrow("kind");
	});

	test("unreadable storage keeps the tray for this page", () => {
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			get() {
				throw new Error("blocked");
			},
		});
		const tray = harness().create();
		const id = tray.start(item());
		tray.finish(id, "done");
		expect(tray.list()[0]?.state).toBe("done");
	});

	test("storage that stops accepting writes keeps later items for this page", () => {
		const tray = harness().create();
		const saved = tray.start(item());
		const storage = globalThis.localStorage;
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			value: {
				getItem: (key: string) => storage.getItem(key),
				setItem: () => {
					throw new DOMException("Storage is full.", "QuotaExceededError");
				},
			},
		});
		const unsaved = tray.start(item());
		tray.finish(unsaved, "done");
		tray.update(saved, { state: "waiting" });
		expect(
			tray
				.list()
				.map((entry) => [entry.id, entry.state])
				.sort(),
		).toEqual(
			[
				[saved, "waiting"],
				[unsaved, "done"],
			].sort(),
		);
		expect(stored().items.map((entry: ActivityItem) => entry.id)).toEqual([
			saved,
		]);
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			value: storage,
		});
		tray.dismiss(unsaved);
		expect(stored().items.map((entry: ActivityItem) => entry.state)).toEqual([
			"waiting",
		]);
	});

	test("list() keeps its identity until something changes", () => {
		const tray = harness().create();
		tray.start(item());
		const first = tray.list();
		expect(tray.list()).toBe(first);
		tray.start(item());
		expect(tray.list()).not.toBe(first);
	});

	test("the tray orders in progress, then no reply, then finished", () => {
		const h = harness();
		const tray = h.create();
		const done = tray.start(item());
		tray.finish(done, "done");
		h.clock.now += 1000;
		const unknown = tray.start(item());
		tray.finish(unknown, "unknown", { code: "no_reply" });
		h.clock.now += 1000;
		const active = tray.start(item());
		expect(tray.list().map((entry) => entry.id)).toEqual([
			active,
			unknown,
			done,
		]);
	});
});

describe("retention", () => {
	test("at most 100 items: the oldest finished go first", () => {
		const h = harness();
		const tray = h.create();
		const running = tray.start(item());
		const finished: string[] = [];
		for (let index = 0; index < 104; index++) {
			h.clock.now += 1000;
			const id = tray.start(item());
			tray.finish(id, "done");
			finished.push(id);
		}
		const ids = tray.list().map((entry) => entry.id);
		expect(ids).toHaveLength(100);
		expect(ids).toContain(running);
		expect(ids).not.toContain(finished[0]);
		expect(ids).toContain(finished[103]);
	});

	test("finished results expire after 7 days", () => {
		const h = harness();
		const tray = h.create();
		tray.finish(tray.start(item()), "failed");
		h.clock.now += 7 * DAY + 1;
		expect(h.create().list()).toEqual([]);
	});

	test("operation handles lapse after 24 h: no reply and no more checking", () => {
		const h = harness();
		const tray = h.create();
		tray.start(
			item({
				state: "waiting",
				resume: {
					type: "operation",
					operationId: "op-1",
					command: "stop",
					issuedAt: T0 / 1000,
				},
			}),
		);
		h.clock.now += DAY + 1000;
		const [lapsed] = h.create().list();
		expect(lapsed?.state).toBe("unknown");
		expect(lapsed?.resume).toBeUndefined();
		expect(lapsed?.actions).toEqual(["dismiss"]);
	});

	test("upload hints lapse when the device drops the transfer", () => {
		const h = harness();
		const tray = h.create();
		tray.start(
			item({
				kind: "upload",
				state: "paused",
				resume: {
					type: "transfer",
					transferId: "t1",
					projectId: "app",
					manifestSha256: SHA,
					expiresAt: T0 / 1000 + 3600,
				},
			}),
		);
		h.clock.now += 3601_000;
		const [lapsed] = h.create().list();
		expect(lapsed?.state).toBe("failed");
		expect(lapsed?.resume).toBeUndefined();
	});
});

describe("uploads across reloads", () => {
	test("a reload restores an unfinished upload as a resumable item", () => {
		const h = harness();
		const tray = h.create();
		const upload = item({
			kind: "upload",
			state: "active",
			label: { code: "upload" },
			href: { screen: "device", deviceId: "edge-1" },
			progress: { done: 12, total: 38, unit: "files" },
			resume: {
				type: "transfer",
				transferId: "t1",
				projectId: "app",
				manifestSha256: SHA,
				expiresAt: T0 / 1000 + 3600,
			},
		});
		tray.start(upload);
		expect(tray.list()[0]?.state).toBe("active");
		const [restored] = h.create().list();
		expect(restored?.state).toBe("paused");
		expect(restored?.actions).toEqual(["open", "resume", "discard"]);
		expect(restored?.detail).toEqual({
			code: "resumable_until",
			params: { until: T0 + 3600_000 },
		});
		expect(restored?.progress).toEqual(upload.progress);
	});

	test("legacy per-device upload hints move into the tray once, only for devices with keys here", async () => {
		const transfer = {
			transfer_id: crypto.randomUUID(),
			project_id: "app",
			manifest_sha256: SHA,
			confirmed: true,
		};
		rememberArtifactTransfer("edge-1", transfer);
		rememberArtifactTransfer("edge-9", {
			...transfer,
			transfer_id: crypto.randomUUID(),
		});
		const h = harness();
		const tray = h.create();
		await tray.resume("edge-2");
		expect(tray.list()).toEqual([]);
		await tray.resume();
		const [imported] = tray.list();
		expect(imported).toMatchObject({
			kind: "upload",
			state: "paused",
			target: { deviceId: "edge-1", projectId: "app" },
			resume: {
				type: "transfer",
				transferId: transfer.transfer_id,
				confirmed: true,
			},
			actions: ["resume", "discard"],
		});
		expect(pendingArtifactTransfers("edge-1")).toEqual([]);
		expect(pendingArtifactTransfers("edge-9")).toHaveLength(1);
		await tray.resume();
		expect(tray.list()).toHaveLength(1);
	});
});

describe("resume", () => {
	const journal =
		(state: string) => (_: string, command: Record<string, unknown>) =>
			completed({}, String(command.operation_id), state);

	test("an unconfirmed command is settled by the device journal", async () => {
		const h = harness({ reply: journal("completed") });
		const tray = h.create();
		const id = tray.start(
			item({
				state: "unknown",
				resume: {
					type: "operation",
					operationId: "op-1",
					command: "stop",
					issuedAt: T0 / 1000,
				},
			}),
		);
		expect(tray.list()[0]?.actions).toEqual(["check_again", "dismiss"]);
		await h.create().resume();
		const settled = h
			.create()
			.list()
			.find((entry) => entry.id === id);
		expect(settled?.state).toBe("done");
		expect(h.sent).toEqual([
			{
				deviceId: "edge-1",
				command: { type: "operation", operation_id: "op-1" },
			},
		]);
		expect(h.acquired()).toBe(0);
	});

	test("journal answers: accepted waits, unknown ids failed, rolled back failed", async () => {
		for (const [reply, expected] of [
			[
				journal("accepted"),
				{ state: "waiting", detail: { code: "waiting_for_apply" } },
			],
			[
				() => ({
					operation_id: "lookup",
					state: "rejected",
					result: {
						code: "invalid",
						error: "Unknown operation",
						retryable: false,
					},
				}),
				{ state: "failed", detail: { code: "failed" } },
			],
			[
				journal("rolled_back"),
				{ state: "failed", detail: { code: "rolled_back" } },
			],
		] as const) {
			values.clear();
			const h = harness({ reply });
			const id = h.create().start(
				item({
					state: "unknown",
					resume: {
						type: "operation",
						operationId: "op-1",
						command: "start",
						issuedAt: T0 / 1000,
					},
				}),
			);
			const tray = h.create();
			await tray.resume();
			expect(tray.list().find((entry) => entry.id === id)).toMatchObject(
				expected,
			);
		}
	});

	test("locked keys leave live handles untouched and send nothing", async () => {
		const h = harness({ reply: journal("completed") });
		h.unlocked.clear();
		const tray = h.create();
		tray.start(
			item({
				state: "unknown",
				resume: {
					type: "operation",
					operationId: "op-1",
					command: "stop",
					issuedAt: T0 / 1000,
				},
			}),
		);
		await h.create().resume();
		expect(h.sent).toEqual([]);
		expect(h.create().list()[0]?.state).toBe("unknown");
	});

	test("an item this page is driving is not re-read", async () => {
		const h = harness({ reply: journal("completed") });
		const tray = h.create();
		tray.start(
			item({
				resume: {
					type: "operation",
					operationId: "op-1",
					command: "stop",
					issuedAt: T0 / 1000,
				},
			}),
		);
		await tray.resume();
		expect(h.sent).toEqual([]);
	});

	test("rollouts follow the device's rollout status", async () => {
		const rollout = {
			rollout_id: "r1",
			placement_id: "p1",
			project_id: "app",
		};
		let state = "activating";
		const h = harness({
			reply: () =>
				completed({ ...rollout, state, deadline_at: T0 / 1000 + 90 }),
		});
		const id = h.create().start(
			item({
				kind: "safe_update",
				state: "waiting",
				label: { code: "safe_update" },
				resume: {
					type: "rollout",
					rolloutId: "r1",
					placementId: "p1",
					projectId: "app",
				},
			}),
		);
		const tray = h.create();
		await tray.resume();
		expect(tray.list()[0]).toMatchObject({
			id,
			state: "active",
			deadlineAt: T0 + 90_000,
		});
		state = "rolled_back";
		await h.create().resume();
		expect(h.create().list()[0]).toMatchObject({
			state: "failed",
			detail: { code: "rolled_back" },
		});
	});

	test("uploads re-read their transfer: receiving pauses, committed is done, gone has failed", async () => {
		const descriptor = {
			project_id: "app",
			manifest_sha256: SHA,
			manifest_size: 10,
			file_count: 1,
			total_bytes: 10,
		};
		let reply: ManagementResponse = completed({
			transfer_id: "00000000-0000-4000-8000-000000000001",
			descriptor,
			state: "receiving",
			expires_at: T0 / 1000 + 7200,
		});
		const h = harness({ reply: () => reply });
		const upload = item({
			kind: "upload",
			state: "paused",
			label: { code: "upload" },
			resume: {
				type: "transfer",
				transferId: "00000000-0000-4000-8000-000000000001",
				projectId: "app",
				manifestSha256: SHA,
				expiresAt: T0 / 1000 + 3600,
				confirmed: false,
			},
		});
		h.create().start(upload);
		await h.create().resume();
		expect(h.create().list()[0]).toMatchObject({
			state: "paused",
			resume: { expiresAt: T0 / 1000 + 7200, confirmed: true },
			actions: ["resume", "discard"],
		});
		reply = completed({ ...reply.result, state: "committed" });
		await h.create().resume();
		expect(h.create().list()[0]?.state).toBe("done");
		values.clear();
		h.create().start(upload);
		reply = {
			operation_id: "x",
			state: "rejected",
			result: { code: "failed", error: "Unknown transfer", retryable: true },
		};
		await h.create().resume();
		expect(h.create().list()[0]?.state).toBe("failed");
	});

	test("a reboot is done once the device reports a new boot, without asking it", async () => {
		const h = harness({ reply: journal("draining") });
		h.create().start(
			item({
				kind: "reboot",
				state: "waiting",
				label: { code: "reboot" },
				resume: {
					type: "host_operation",
					kind: "reboot",
					operationId: "op-r",
					bootIdBefore: "boot-1",
				},
			}),
		);
		await h.create().resume();
		expect(h.create().list()[0]?.detail).toEqual({
			code: "waiting_for_device",
		});
		h.live.set("edge-1", {
			kind: "live",
			transport: "websocket",
			expiresAt: 0,
			bootId: "boot-2",
			connectedAt: 0,
		});
		h.sent.length = 0;
		await h.create().resume();
		expect(h.create().list()[0]?.state).toBe("done");
		expect(h.sent).toEqual([]);
	});

	test("signing requests wait until the request is gone from the device", async () => {
		const requestId = "00000000-0000-4000-8000-00000000000a";
		let requests: unknown[] = [
			{
				request_id: requestId,
				certificate_id: requestId,
				label: "Gateway",
				expected_revision: 0,
				dns_names: ["edge.example.com"],
				ip_addresses: [],
				csr_pem: "public CSR",
				created_at: 1,
				expires_at: 2,
				purpose: "service",
			},
		];
		const h = harness({ reply: () => completed({ requests, next: null }) });
		h.create().start(
			item({
				kind: "signing_request",
				state: "waiting",
				label: { code: "signing_request" },
				resume: { type: "csr", requestId },
			}),
		);
		await h.create().resume();
		expect(h.create().list()[0]?.state).toBe("waiting");
		requests = [];
		await h.create().resume();
		expect(h.create().list()[0]?.state).toBe("done");
	});

	test("hub handles: access rules, account backup and setup", async () => {
		const h = harness();
		h.unlocked.clear();
		h.hub.set("devices/edge-1/management/policy", { applied_version: 3 });
		h.hub.set("devices/controller-vaults/edge-2", { revision: 1 });
		h.hub.set("devices/edge-3", { last_seen_at: T0 / 1000 });
		const tray = h.create();
		const rules = tray.start(
			item({
				kind: "access_rules",
				state: "waiting",
				label: { code: "access_rules" },
				resume: { type: "policy", version: 3 },
			}),
		);
		const backup = tray.start(
			item({
				kind: "account_backup",
				target: { deviceId: "edge-2" },
				state: "waiting",
				label: { code: "account_backup" },
				resume: { type: "account_backup", revision: 2 },
			}),
		);
		const setup = tray.start(
			item({
				kind: "setup",
				target: { deviceId: "edge-3" },
				state: "waiting",
				label: { code: "setup" },
				resume: { type: "setup", enrollmentId: "enr-1", deviceId: "edge-3" },
			}),
		);
		const reloaded = h.create();
		await reloaded.resume();
		const byId = new Map(reloaded.list().map((entry) => [entry.id, entry]));
		expect(byId.get(rules)?.state).toBe("done");
		expect(byId.get(backup)).toMatchObject({
			state: "waiting",
			detail: { code: "waiting_for_backup" },
		});
		expect(byId.get(setup)?.state).toBe("done");
	});

	test("resume(deviceId) re-reads only that device", async () => {
		const h = harness();
		h.hub.set("devices/edge-1/management/policy", { applied_version: 1 });
		h.hub.set("devices/edge-2/management/policy", { applied_version: 1 });
		const tray = h.create();
		for (const deviceId of ["edge-1", "edge-2"])
			tray.start(
				item({
					kind: "access_rules",
					target: { deviceId },
					state: "waiting",
					label: { code: "access_rules" },
					resume: { type: "policy", version: 1 },
				}),
			);
		const reloaded = h.create();
		await reloaded.resume("edge-2");
		expect(
			reloaded.list().map((entry) => [entry.target.deviceId, entry.state]),
		).toEqual(
			expect.arrayContaining([
				["edge-1", "waiting"],
				["edge-2", "done"],
			]),
		);
	});
});

describe("finishing", () => {
	test("finish announces once, dismiss removes finished items and hides running ones", () => {
		const tray = harness().create();
		const announced: string[] = [];
		tray.onFinishedElsewhere((entry) => announced.push(entry.id));
		const running = tray.start(item({ href: { screen: "hub" } }));
		tray.update(running, {
			progress: { done: 1, total: 2, unit: "instances" },
		});
		expect(announced).toEqual([]);
		tray.finish(running, "done");
		tray.finish(running, "done");
		expect(announced).toEqual([running]);
		expect(tray.list()[0]?.actions).toEqual(["open", "dismiss"]);
		tray.dismiss(running);
		expect(tray.list()).toEqual([]);
		const other = tray.start(item());
		tray.dismiss(other);
		expect(tray.list()).toEqual([]);
		tray.update(other, { state: "failed" });
		expect(announced).toEqual([running, other]);
	});

	test("expiry bookkeeping is not announced", () => {
		const h = harness();
		h.create().start(
			item({
				state: "waiting",
				resume: {
					type: "operation",
					operationId: "op-1",
					command: "stop",
					issuedAt: T0 / 1000,
				},
			}),
		);
		h.clock.now += DAY + 1000;
		const tray = h.create();
		const announced: string[] = [];
		tray.onFinishedElsewhere((entry) => announced.push(entry.id));
		tray.start(item());
		expect(announced).toEqual([]);
	});

	test("another tab finishing an item is announced and listed", () => {
		const h = harness();
		const here = h.create();
		const there = h.create();
		const id = there.start(item());
		const announced: string[] = [];
		here.onFinishedElsewhere((entry) => announced.push(entry.id));
		let changes = 0;
		const stop = here.subscribe(() => changes++);
		for (const listener of windowListeners)
			listener({
				key: `${ACTIVITY_STORAGE_PREFIX}${accountStorageKey(account())}`,
			});
		expect(here.list().map((entry) => entry.id)).toEqual([id]);
		there.finish(id, "done");
		for (const listener of windowListeners)
			listener({
				key: `${ACTIVITY_STORAGE_PREFIX}${accountStorageKey(account())}`,
			});
		expect(announced).toEqual([id]);
		expect(changes).toBe(2);
		stop();
		expect(windowListeners).toEqual([]);
	});

	test("one tab's change keeps the other tab's items", () => {
		const h = harness();
		const here = h.create();
		const there = h.create();
		const mine = here.start(item());
		const theirs = there.start(item());
		here.finish(mine, "done");
		expect(
			h
				.create()
				.list()
				.map((entry) => entry.id)
				.sort(),
		).toEqual([mine, theirs].sort());
	});
});

describe("multi-device runs", () => {
	test("a run keeps its targets in order and is restored after a reload", () => {
		const h = harness();
		const tray = h.create();
		const run = tray.startRun({
			title: { code: "deploy_run", params: { app: "CRM Sync" } },
			oneAtATime: true,
			stopOnFail: true,
			items: ["edge-1", "edge-2", "edge-3"].map((deviceId, index) =>
				item({
					kind: "safe_update",
					target: { deviceId, projectId: "app" },
					state: index ? "waiting" : "active",
					label: { code: "safe_update" },
				}),
			),
		});
		expect(run.itemIds).toHaveLength(3);
		const reloaded = h.create();
		expect(reloaded.run(run.id)).toEqual(run);
		expect(
			reloaded.runItems(run.id).map((entry) => entry.target.deviceId),
		).toEqual(["edge-1", "edge-2", "edge-3"]);
		for (const itemId of run.itemIds) {
			reloaded.finish(itemId, "done");
			reloaded.dismiss(itemId);
		}
		expect(reloaded.run(run.id)).toBeUndefined();
		expect(reloaded.runs()).toEqual([]);
	});
});
