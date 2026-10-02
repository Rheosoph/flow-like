import { describe, expect, test } from "bun:test";
import type { InspectionPlus } from "../model/types";
import type { ManagementCall } from "../telemetry";
import type { BrowserController, ManagementResponse } from "../types";
import { LiveCallError } from "./errors";
import {
	type GroupMetricsSample,
	RECORD_BUFFER,
	type RecordStream,
	createLiveStreams,
} from "./streams";
import type {
	CallLane,
	DeviceCallOptions,
	KeyPort,
	LiveInspection,
	LiveState,
	StreamState,
	WorkspaceDeps,
} from "./types";

const DEVICE = "device-1";

type Handler = (
	command: Record<string, unknown>,
) => ManagementResponse | Promise<ManagementResponse>;

function completed(result: Record<string, unknown>): ManagementResponse {
	return { operation_id: "op", state: "completed", result };
}

function record(sequence: number, data: Record<string, unknown> = {}) {
	return {
		sequence,
		timestamp: 1_700_000_000 + sequence,
		kind: "log",
		data: { stream: "stdout", message: `line ${sequence}`, ...data },
	};
}

/** One page of a log that holds lines 1 to `newest`, 100 lines after the cursor. */
function logPage(command: Record<string, unknown>, newest: number) {
	const after = Number(command.after);
	const records = Array.from({ length: 100 }, (_, index) =>
		record(after + index + 1),
	).filter((row) => row.sequence <= newest);
	return completed({ records, next: records.at(-1)?.sequence ?? after });
}

class FakeLive {
	nowMs = 1_700_000_000_000;
	states = new Map<string, LiveState>();
	inspections = new Map<string, LiveInspection>();
	listeners = new Set<() => void>();
	demand = 0;
	sent: { command: Record<string, unknown>; lane?: CallLane }[] = [];
	exclusives = 0;
	handler: Handler = (command) => completed({ type: command.type });
	private timers: { at: number; run: () => void; id: number }[] = [];
	private nextId = 0;

	state(deviceId: string): LiveState {
		return this.states.get(deviceId) ?? { kind: "idle" };
	}
	acquire() {
		this.demand++;
		return () => {
			this.demand--;
		};
	}
	call(_deviceId: string, options: DeviceCallOptions = {}): ManagementCall {
		return async (command) => {
			this.sent.push({ command, lane: options.lane });
			await Promise.resolve();
			return this.handler(command);
		};
	}
	exclusive<T>(deviceId: string, run: (call: ManagementCall) => Promise<T>) {
		this.exclusives++;
		return run(this.call(deviceId, { lane: "poll" }));
	}
	close() {}
	subscribe(listener: () => void) {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}
	inspection(deviceId: string) {
		return this.inspections.get(deviceId);
	}
	devices() {
		return [...this.states.keys()];
	}
	set(state: LiveState, inspection?: InspectionPlus | null) {
		this.states.set(DEVICE, state);
		if (inspection === null) this.inspections.delete(DEVICE);
		else if (inspection)
			this.inspections.set(DEVICE, {
				value: inspection,
				readAt: Math.floor(this.nowMs / 1000),
			});
		for (const listener of this.listeners) listener();
	}
	live(connectedAt = Math.floor(this.nowMs / 1000)): LiveState {
		return {
			kind: "live",
			transport: "websocket",
			expiresAt: connectedAt + 300,
			bootId: "boot",
			connectedAt,
		};
	}
	schedule = (run: () => void, ms: number) => {
		const id = ++this.nextId;
		this.timers.push({ at: this.nowMs + ms, run, id });
		return () => {
			this.timers = this.timers.filter((timer) => timer.id !== id);
		};
	};
	async advance(ms: number) {
		const end = this.nowMs + ms;
		for (;;) {
			await flush();
			this.timers.sort((a, b) => a.at - b.at);
			const next = this.timers[0];
			if (!next || next.at > end) break;
			this.timers.shift();
			this.nowMs = Math.max(this.nowMs, next.at);
			next.run();
		}
		this.nowMs = end;
		await flush();
	}
	types() {
		return this.sent.map((row) => row.command.type);
	}
}

async function flush() {
	for (let round = 0; round < 6; round++)
		await new Promise<void>((resolve) => setImmediate(resolve));
}

function fakeKeys() {
	let controller: BrowserController | undefined = {} as BrowserController;
	const listeners = new Set<() => void>();
	const port: KeyPort = {
		controller: () => controller,
		vault: () => undefined,
		receipt: () => undefined,
		snapshot: () => {
			throw new Error("unused");
		},
		subscribe: (listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		touch: () => {},
	};
	return {
		port,
		set(next: BrowserController | undefined) {
			controller = next;
			for (const listener of listeners) listener();
		},
	};
}

function setup(
	options: {
		readGroup?: (
			deviceId: string,
			scope: string,
			call: ManagementCall,
		) => Promise<GroupMetricsSample | null>;
		keys?: KeyPort;
	} = {},
) {
	const live = new FakeLive();
	const streams = createLiveStreams(
		{ now: () => live.nowMs } as unknown as WorkspaceDeps,
		live,
		{ schedule: live.schedule, ...options },
	);
	return { live, streams };
}

function collect<T>(states: StreamState<T>[]) {
	return (state: StreamState<T>) => states.push(state);
}

/** Sequence numbers of the lines the newest state shows. */
function linesOf(states: StreamState<RecordStream>[]) {
	return (states.at(-1)?.data?.records ?? []).map((row) => row.sequence);
}

describe("polling", () => {
	test("metrics poll every 5 s on the poll lane while subscribed and live", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		const states: StreamState<Record<string, unknown>>[] = [];
		const stop = streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: "p1" },
			collect(states),
		);
		expect(live.demand).toBe(1);
		await live.advance(0);
		expect(live.sent).toEqual([
			{ command: { type: "metrics", placement_id: "p1" }, lane: "poll" },
		]);
		expect(states.at(-1)?.data).toEqual({ type: "metrics" });
		expect(states.at(-1)?.freshness).toMatchObject({
			src: "live",
			age: "live",
		});
		await live.advance(5_000);
		expect(live.sent).toHaveLength(2);
		stop();
		expect(live.demand).toBe(0);
		await live.advance(20_000);
		expect(live.sent).toHaveLength(2);
	});

	test("subscribers of one stream share its polls", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		streams.subscribe(DEVICE, { kind: "metrics", placementId: null }, () => {});
		streams.subscribe(DEVICE, { kind: "metrics", placementId: null }, () => {});
		await live.advance(5_000);
		expect(live.sent).toHaveLength(2);
		expect(live.demand).toBe(2);
	});

	test("errors and renewals keep the data; a new session reads at once", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "project_metrics", projectId: "proj" },
			collect(states),
		);
		await live.advance(0);
		live.handler = () => {
			throw new Error("Management connection timed out.");
		};
		await live.advance(5_000);
		expect(states.at(-1)?.data).toEqual({ type: "project_metrics" });
		expect(states.at(-1)?.freshness).toMatchObject({
			age: "error",
			error: { code: "timeout" },
		});
		live.set({ kind: "renewing", transport: "websocket", expiresAt: 0 });
		const sent = live.sent.length;
		await live.advance(10_000);
		expect(live.sent).toHaveLength(sent);
		expect(states.at(-1)?.data).toEqual({ type: "project_metrics" });
		live.handler = (command) => completed({ type: command.type, fresh: true });
		live.set(live.live(Math.floor(live.nowMs / 1000)));
		await live.advance(0);
		expect(live.sent).toHaveLength(sent + 1);
		expect(states.at(-1)?.data).toEqual({
			type: "project_metrics",
			fresh: true,
		});
	});

	test("an unsupported read stops polling and says so until the next session", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		live.handler = () => ({
			operation_id: "op",
			state: "rejected",
			result: { code: "unsupported", error: "old agent", retryable: false },
		});
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: null },
			collect(states),
		);
		await live.advance(30_000);
		expect(live.sent).toHaveLength(1);
		expect(states.at(-1)?.rejected?.code).toBe("unsupported");
		expect(states.at(-1)?.freshness).toMatchObject({
			age: "unsupported",
			reason: { code: "agent_update_needed" },
		});
		live.set(live.live(Math.floor(live.nowMs / 1000) + 1));
		await live.advance(0);
		expect(live.sent).toHaveLength(2);
	});
});

describe("locking", () => {
	test("a lock clears every buffer; errors never do", async () => {
		const { live, streams } = setup();
		live.set(live.live(), {
			device_id: DEVICE,
			boot_id: "boot",
			placements: [],
			features: {},
		});
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: null },
			collect(states),
		);
		await live.advance(0);
		expect(states.at(-1)?.data).toBeDefined();
		live.set({ kind: "idle" });
		expect(states.at(-1)?.data).toBeDefined();
		live.set({ kind: "idle" }, null);
		expect(states.at(-1)?.data).toBeUndefined();
		expect(states.at(-1)?.freshness.age).toBe("locked");
	});

	test("a keys_locked refusal clears the device too", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: null },
			collect(states),
		);
		await live.advance(0);
		live.handler = () => {
			throw new LiveCallError("keys_locked", "locked");
		};
		await live.advance(5_000);
		expect(states.at(-1)?.data).toBeUndefined();
		expect(states.at(-1)?.freshness.age).toBe("locked");
	});

	test("locking the keys clears the buffers even when the session never read the services", async () => {
		const keys = fakeKeys();
		const { live, streams } = setup({ keys: keys.port });
		live.set(live.live());
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: null },
			collect(states),
		);
		await live.advance(0);
		expect(states.at(-1)?.data).toBeDefined();
		const emitted = states.length;
		keys.set({} as BrowserController);
		expect(states).toHaveLength(emitted);
		keys.set(undefined);
		expect(states.at(-1)?.data).toBeUndefined();
		expect(states.at(-1)?.freshness.age).toBe("locked");
		const cleared = states.length;
		keys.set(undefined);
		expect(states).toHaveLength(cleared);
	});

	test("a stream opened on a locked device reads Locked from the start", () => {
		const keys = fakeKeys();
		keys.set(undefined);
		const { streams } = setup({ keys: keys.port });
		const states: StreamState<unknown>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "metrics", placementId: null },
			collect(states),
		);
		expect(states.at(-1)?.freshness.age).toBe("locked");
	});
});

describe("logs and messages", () => {
	test("logs follow the cursor, catch up at once on full pages and keep gap markers", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let next = 0;
		live.handler = (command) => {
			const after = Number(command.after);
			const records = Array.from({ length: 3 }, (_, index) =>
				record(after + index + 1, {
					...(after + index + 1 === 2 ? { dropped_lines: 37 } : {}),
					...(after + index + 1 === 3 ? { truncated: true } : {}),
				}),
			).filter((row) => row.sequence <= 7);
			next = records.at(-1)?.sequence ?? after;
			return completed({
				records,
				next,
				...(after === 0 ? { evicted_through: 0 } : {}),
			});
		};
		const states: StreamState<RecordStream>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "logs", placementId: null, follow: true, limit: 3 },
			collect(states),
		);
		await live.advance(0);
		expect(live.sent.map((row) => row.command.after)).toEqual([0, 3, 6]);
		const latest = states.at(-1);
		expect(latest?.data?.records.map((row) => row.sequence)).toEqual([
			1, 2, 3, 4, 5, 6, 7,
		]);
		expect(latest?.behind).toBe(0);
		expect(latest?.gaps).toEqual([
			{
				kind: "evicted_through",
				through: 0,
				at: Math.floor(live.nowMs / 1000),
			},
			{ kind: "dropped_lines", through: 2, count: 37, at: 1_700_000_002 },
			{ kind: "truncated", through: 3, at: 1_700_000_003 },
		]);
		await live.advance(5_000);
		expect(live.sent.at(-1)?.command).toMatchObject({ type: "logs", after: 7 });
	});

	test("a paused viewer keeps its lines and counts how many arrived since", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let sequence = 0;
		live.handler = () => {
			sequence += 2;
			return completed({
				records: [record(sequence - 1), record(sequence)],
				next: sequence,
			});
		};
		const following: StreamState<RecordStream>[] = [];
		const paused: StreamState<RecordStream>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "logs", placementId: "p", follow: true },
			collect(following),
		);
		await live.advance(0);
		streams.subscribe(
			DEVICE,
			{ kind: "logs", placementId: "p", follow: false },
			collect(paused),
		);
		await live.advance(10_000);
		expect(following.at(-1)?.data?.records).toHaveLength(6);
		expect(paused.at(-1)?.data?.records).toHaveLength(2);
		expect(paused.at(-1)?.behind).toBe(4);
	});

	test("a viewer that pauses before the first lines arrive still gets them", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let sequence = 0;
		live.handler = () => {
			sequence += 2;
			return completed({
				records: [record(sequence - 1), record(sequence)],
				next: sequence,
			});
		};
		const paused: StreamState<RecordStream>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "logs", placementId: "p", follow: false },
			collect(paused),
		);
		expect(paused.at(-1)?.data).toBeUndefined();
		await live.advance(10_000);
		expect(paused.at(-1)?.data?.records.map((row) => row.sequence)).toEqual([
			1, 2,
		]);
		expect(paused.at(-1)?.behind).toBe(4);
	});

	test("the buffer keeps the newest records and loadOlder re-reads a trimmed page", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		live.handler = (command) => logPage(command, 650);
		const states: StreamState<RecordStream>[] = [];
		const spec = { kind: "logs", placementId: null, follow: true } as const;
		streams.subscribe(DEVICE, spec, collect(states));
		await live.advance(0);
		expect(linesOf(states)).toHaveLength(RECORD_BUFFER);
		expect(linesOf(states)[0]).toBe(151);
		await streams.loadOlder(DEVICE, spec);
		expect(live.sent.at(-1)?.command).toMatchObject({ after: 100 });
		expect(linesOf(states)[0]).toBe(101);
		expect(linesOf(states)).toHaveLength(550);
		await live.advance(5_000);
		expect(linesOf(states)).toHaveLength(550);
		await streams.loadOlder(DEVICE, spec);
		expect(live.sent.at(-1)?.command).toMatchObject({ after: 0 });
		expect(linesOf(states)[0]).toBe(1);
		expect(linesOf(states)).toHaveLength(650);
	});

	test("older lines reach a paused viewer's own lines", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let newest = 650;
		live.handler = (command) => logPage(command, newest);
		const following = {
			kind: "logs",
			placementId: null,
			follow: true,
		} as const;
		const paused = { ...following, follow: false } as const;
		streams.subscribe(DEVICE, following, () => {});
		await live.advance(0);
		const states: StreamState<RecordStream>[] = [];
		streams.subscribe(DEVICE, paused, collect(states));
		newest = 700;
		await live.advance(5_000);
		expect(states.at(-1)?.data?.records[0].sequence).toBe(151);
		expect(states.at(-1)?.behind).toBe(50);
		await streams.loadOlder(DEVICE, paused);
		const lines = states.at(-1)?.data?.records ?? [];
		expect(lines[0].sequence).toBe(101);
		expect(lines.at(-1)?.sequence).toBe(650);
		expect(lines).toHaveLength(550);
		expect(states.at(-1)?.behind).toBe(50);
	});

	test("lines that arrive while an older page is read are kept", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let newest = 450;
		let release!: () => void;
		const gate = new Promise<void>((resolve) => {
			release = resolve;
		});
		let gated = false;
		live.handler = async (command) => {
			if (gated && command.after === 100) await gate;
			return logPage(command, newest);
		};
		const states: StreamState<RecordStream>[] = [];
		const spec = { kind: "logs", placementId: null, follow: true } as const;
		streams.subscribe(DEVICE, spec, collect(states));
		await live.advance(0);
		await streams.loadOlder(DEVICE, spec);
		expect(linesOf(states)).toHaveLength(450);

		newest = 460;
		await live.advance(5_000);
		expect(linesOf(states)).toHaveLength(460);
		expect(linesOf(states)[0]).toBe(1);

		newest = 620;
		await live.advance(5_000);
		expect(linesOf(states)).toHaveLength(RECORD_BUFFER);
		expect(linesOf(states)[0]).toBe(121);
		gated = true;
		const loading = streams.loadOlder(DEVICE, spec);
		newest = 630;
		await live.advance(5_000);
		release();
		await loading;
		const sequences = linesOf(states);
		expect(sequences.at(-1)).toBe(630);
		expect(
			sequences.every((value, index) => value === sequences[0] + index),
		).toBe(true);
	});

	test("messages scope by project and report newly expired outbox entries once", async () => {
		const { live, streams } = setup();
		live.set(live.live());
		let dropped = 3;
		live.handler = (command) =>
			completed({
				records: [],
				next: Number(command.after),
				outbox_dropped: dropped,
				retention_limit: 2000,
			});
		const states: StreamState<RecordStream>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "messages", placementId: null, projectId: "proj" },
			collect(states),
		);
		await live.advance(0);
		expect(live.sent[0].command).toEqual({
			type: "messages",
			placement_id: null,
			project_id: "proj",
			after: 0,
			limit: 100,
		});
		await live.advance(5_000);
		dropped = 5;
		await live.advance(5_000);
		expect(states.at(-1)?.gaps.map((gap) => [gap.kind, gap.count])).toEqual([
			["outbox_dropped", 3],
			["outbox_dropped", 2],
		]);
		expect(states.at(-1)?.data?.meta).toEqual({
			outbox_dropped: 5,
			retention_limit: 2000,
		});
	});
});

describe("connect auto-load and group metrics", () => {
	test("offline queues per placement and certificates load on connect without a viewer", async () => {
		const { live, streams } = setup();
		live.handler = (command) =>
			command.type === "offline_queue"
				? completed({
						placement_id: command.placement_id,
						queues: [],
						next: null,
					})
				: completed({ certificates: [], inventory_revision: 4, next: null });
		live.set(live.live(), {
			device_id: DEVICE,
			boot_id: "boot",
			features: {},
			certificate_management: 1,
			placements: [
				{ id: "a" } as InspectionPlus["placements"][number],
				{ id: "b" } as InspectionPlus["placements"][number],
			],
		});
		await live.advance(0);
		expect(live.types().sort()).toEqual([
			"certificates",
			"offline_queue",
			"offline_queue",
		]);
		expect(
			streams.peek(DEVICE, { kind: "offline_queues", placementId: "b" })?.data,
		).toEqual([]);
		expect(streams.peek(DEVICE, { kind: "certificates" })?.data).toEqual({
			certificates: [],
			inventory_revision: 4,
		});
		live.set(live.live());
		await live.advance(60_000);
		expect(live.sent).toHaveLength(3);
	});

	test("the connect auto-load follows the session's own read, whatever the two clocks say", async () => {
		const { live } = setup();
		live.handler = (command) =>
			completed({ placement_id: command.placement_id, queues: [], next: null });
		const read = (placement: string, readAt: number) => {
			live.inspections.set(DEVICE, {
				value: {
					device_id: DEVICE,
					boot_id: "boot",
					features: {},
					placements: [
						{ id: placement } as InspectionPlus["placements"][number],
					],
				},
				readAt,
			});
			live.set(live.state(DEVICE));
		};
		const hubS = Math.floor(live.nowMs / 1000) - 120;
		live.set(live.live());
		read("a", hubS);
		await live.advance(0);
		expect(live.sent.map((row) => row.command.placement_id)).toEqual(["a"]);

		live.set(live.live(Math.floor(live.nowMs / 1000) + 255));
		await live.advance(0);
		expect(live.sent).toHaveLength(1);
		read("b", hubS + 255);
		await live.advance(0);
		expect(live.sent.map((row) => row.command.placement_id)).toEqual([
			"a",
			"b",
		]);
	});

	test("group metrics run as an exclusive section and keep the last sample", async () => {
		const samples: (GroupMetricsSample | null)[] = [
			{
				sequence: 1,
				sample: { cpu: 3 },
				state: "application",
				receiptPending: false,
			},
			{ sequence: 2, state: "commit", receiptPending: false },
		];
		const { live, streams } = setup({
			readGroup: async () => samples.shift() ?? null,
		});
		live.set(live.live());
		const states: StreamState<GroupMetricsSample>[] = [];
		streams.subscribe(
			DEVICE,
			{ kind: "group_metrics", scope: "device" },
			collect(states),
		);
		await live.advance(3_000);
		expect(live.exclusives).toBe(2);
		expect(states.at(-1)?.data).toEqual({
			sequence: 2,
			state: "commit",
			receiptPending: false,
			sample: { cpu: 3 },
		});
	});
});
