import { describe, expect, test } from "bun:test";
import {
	DRAG_FIELD,
	LAST_RUN_FIELD,
	PING_FIELD,
	PING_TTL_MS,
	SUMMON_FIELD,
} from "./presence-signals";
import {
	PRESENCE_ROSTER_GRACE_MS,
	type PeerPresenceEvent,
	type PeerRun,
	type PeerSummon,
	createDragPublisher,
	createPresenceSignalsObserver,
	deriveDrags,
	peerDragsEqual,
} from "./presence-signals-store";

const NODE_A = "nodedrag00000000a";
const NODE_B = "nodedrag00000000b";
const RUN_ID = "run0000000000001";

class FakeAwareness {
	clientID = 1;
	states = new Map<number, Record<string, unknown>>();
	published: [string, unknown][] = [];
	private listeners = new Set<() => void>();
	getStates() {
		return this.states;
	}
	getLocalState() {
		return this.states.get(this.clientID) ?? null;
	}
	on(_event: "change", cb: () => void) {
		this.listeners.add(cb);
	}
	off(_event: "change", cb: () => void) {
		this.listeners.delete(cb);
	}
	emitChange() {
		for (const cb of this.listeners) cb();
	}
	setLocalStateField(field: string, value: unknown) {
		this.published.push([field, value]);
	}
}

function fakeRaf() {
	const frames: (() => void)[] = [];
	return {
		raf: (cb: () => void) => {
			frames.push(cb);
			return frames.length;
		},
		caf: () => {},
		flushFrame: () => {
			const pending = frames.splice(0);
			for (const cb of pending) cb();
		},
	};
}

function fakeScheduler() {
	let nextId = 1;
	const tasks = new Map<number, { cb: () => void; ms: number }>();
	return {
		schedule: (cb: () => void, ms: number) => {
			const id = nextId++;
			tasks.set(id, { cb, ms });
			return id;
		},
		cancel: (handle: unknown) => {
			tasks.delete(handle as number);
		},
		// Shorter timers fire first, as they would on a real clock; a callback
		// may cancel a still-pending sibling, which then never runs.
		flush: () => {
			const pending = [...tasks.entries()].sort(
				(a, b) => a[1].ms - b[1].ms || a[0] - b[0],
			);
			for (const [id, task] of pending) {
				if (!tasks.has(id)) continue;
				tasks.delete(id);
				task.cb();
			}
		},
		size: () => tasks.size,
	};
}

function harness(options?: {
	selfSub?: string;
	onSummon?: (summon: PeerSummon) => void;
	onPeerRun?: (run: PeerRun) => void;
	onPeerPresenceEvent?: (event: PeerPresenceEvent) => void;
}) {
	const awareness = new FakeAwareness();
	const frame = fakeRaf();
	const scheduler = fakeScheduler();
	let clock = 10_000;
	const start = () =>
		createPresenceSignalsObserver(awareness, {
			selfSub: options?.selfSub,
			raf: frame.raf,
			caf: frame.caf,
			now: () => clock,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
			onSummon: options?.onSummon,
			onPeerRun: options?.onPeerRun,
			onPeerPresenceEvent: options?.onPeerPresenceEvent,
		});
	return {
		awareness,
		scheduler,
		start,
		tick: () => {
			awareness.emitChange();
			frame.flushFrame();
		},
		flushFrame: frame.flushFrame,
		advance: (ms: number) => {
			clock += ms;
		},
	};
}

const drag = (nodes: { id: string; x: number; y: number }[], ts = 1) => ({
	[DRAG_FIELD]: { nodes, ts },
});

describe("drag ghosts", () => {
	test("derives every other session's sanitized drag, sorted by clientId", () => {
		const states = new Map<number, Record<string, unknown>>([
			[
				7,
				{
					sub: "peer-b",
					layerPath: "layer000000000001",
					...drag([{ id: NODE_B, x: 5, y: 6 }]),
				},
			],
			[1, { sub: "me", ...drag([{ id: NODE_A, x: 0, y: 0 }]) }],
			[
				3,
				{
					sub: "peer-a",
					...drag([
						{ id: NODE_A, x: 1, y: 2 },
						{ id: "x", x: 1, y: 1 },
					]),
				},
			],
			[4, { sub: "peer-c", [DRAG_FIELD]: { nodes: "nope" } }],
		]);
		expect(deriveDrags(states, 1)).toEqual([
			{
				clientId: 3,
				sub: "peer-a",
				layerPath: "root",
				nodes: [{ id: NODE_A, x: 1, y: 2 }],
			},
			{
				clientId: 7,
				sub: "peer-b",
				layerPath: "layer000000000001",
				nodes: [{ id: NODE_B, x: 5, y: 6 }],
			},
		]);
	});

	test("the drag store notifies on moved positions and short-circuits otherwise", () => {
		const h = harness();
		h.awareness.states.set(2, {
			sub: "peer-a",
			...drag([{ id: NODE_A, x: 1, y: 2 }]),
		});
		const observer = h.start();
		let emissions = 0;
		observer.dragStore.subscribe(() => emissions++);
		expect(observer.dragStore.getSnapshot()[0]?.nodes[0]).toEqual({
			id: NODE_A,
			x: 1,
			y: 2,
		});

		// Heartbeat: same positions, new ts.
		h.awareness.states.set(2, {
			sub: "peer-a",
			...drag([{ id: NODE_A, x: 1, y: 2 }], 2),
		});
		h.tick();
		expect(emissions).toBe(0);

		// Five bursts inside one frame collapse to one emission with the last value.
		for (let i = 1; i <= 5; i++) {
			h.awareness.states.set(2, {
				sub: "peer-a",
				...drag([{ id: NODE_A, x: 10 + i, y: 2 }]),
			});
			h.awareness.emitChange();
		}
		expect(emissions).toBe(0);
		h.flushFrame();
		expect(emissions).toBe(1);
		expect(observer.dragStore.getSnapshot()[0]?.nodes[0]?.x).toBe(15);

		// Drop: the field is withdrawn.
		h.awareness.states.set(2, { sub: "peer-a" });
		h.tick();
		expect(observer.dragStore.getSnapshot()).toEqual([]);
		expect(peerDragsEqual(observer.dragStore.getSnapshot(), [])).toBe(true);
		observer.dispose();
	});

	test("the publisher throttles to the trailing edge and clears immediately", () => {
		const awareness = new FakeAwareness();
		const scheduler = fakeScheduler();
		let clock = 1_000;
		const publisher = createDragPublisher({
			awareness,
			now: () => clock,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		publisher.publish([{ id: NODE_A, x: 1, y: 1 }]);
		expect(awareness.published.length).toBe(1);
		expect(awareness.published[0]).toEqual([
			DRAG_FIELD,
			{ nodes: [{ id: NODE_A, x: 1, y: 1 }], ts: 1_000 },
		]);

		clock += 10;
		publisher.publish([{ id: NODE_A, x: 2, y: 2 }]);
		publisher.publish([{ id: NODE_A, x: 3, y: 3 }]);
		expect(awareness.published.length).toBe(1);
		clock += 40;
		scheduler.flush();
		expect(awareness.published.length).toBe(2);
		expect(
			(awareness.published[1][1] as { nodes: { x: number }[] }).nodes[0]?.x,
		).toBe(3);
		// The publish re-armed the watchdog; with no further drag it withdraws
		// the field (a drag XYFlow aborted must not stick on the wire).
		scheduler.flush();
		expect(awareness.published.length).toBe(3);
		expect(awareness.published[2]).toEqual([DRAG_FIELD, undefined]);

		clock += 100;
		publisher.publish([{ id: NODE_A, x: 4, y: 4 }]);
		// Immediate publish plus its armed watchdog.
		expect(awareness.published.length).toBe(4);
		expect(scheduler.size()).toBe(1);
		publisher.clear();
		expect(scheduler.size()).toBe(0);
		expect(awareness.published[4]).toEqual([DRAG_FIELD, undefined]);
		// Nothing on the wire: a second clear is silent.
		publisher.clear();
		expect(awareness.published.length).toBe(5);
	});
});

const ping = (seq: number, extra: Record<string, unknown> = {}) => ({
	[PING_FIELD]: {
		x: 100,
		y: 200,
		layerPath: "root",
		seq,
		ts: 999_999_999,
		...extra,
	},
});

describe("pings", () => {
	test("a ping already on the wire is history; a seq change enters at local first sight", () => {
		const h = harness();
		h.awareness.states.set(2, { sub: "peer-a", ...ping(4) });
		const observer = h.start();
		expect(observer.pingStore.getSnapshot()).toEqual([]);

		h.advance(100);
		h.awareness.states.set(2, { sub: "peer-a", ...ping(5, { emoji: "👀" }) });
		h.tick();
		expect(observer.pingStore.getSnapshot()).toEqual([
			{
				key: "2:5",
				clientId: 2,
				sub: "peer-a",
				layerPath: "root",
				x: 100,
				y: 200,
				emoji: "👀",
				seenAt: 10_100,
			},
		]);

		// Same seq re-broadcast (or re-added after an awareness timeout): not a new ping.
		h.awareness.states.set(2, { sub: "peer-a", ...ping(5, { ts: 1 }) });
		h.tick();
		expect(observer.pingStore.getSnapshot().length).toBe(1);
		observer.dispose();
	});

	test("the local user's own ping shows and pings expire on the store's own timer", () => {
		const h = harness({ selfSub: "me" });
		const observer = h.start();
		h.awareness.states.set(1, { sub: "me", ...ping(1) });
		h.tick();
		expect(observer.pingStore.getSnapshot().map((p) => p.key)).toEqual(["1:1"]);
		expect(h.scheduler.size()).toBe(1);

		h.advance(PING_TTL_MS - 1);
		// A session we already know pings: that is a new ping.
		h.awareness.states.set(3, { sub: "peer-a" });
		h.tick();
		h.awareness.states.set(3, { sub: "peer-a", ...ping(9) });
		h.tick();
		expect(observer.pingStore.getSnapshot().map((p) => p.key)).toEqual([
			"1:1",
			"3:9",
		]);

		h.advance(2);
		h.scheduler.flush();
		h.flushFrame();
		expect(observer.pingStore.getSnapshot().map((p) => p.key)).toEqual(["3:9"]);
		observer.dispose();
	});
});

describe("summon and last run (first-sight)", () => {
	test("summons fire once per new seq from another session, never for values present at subscription", () => {
		const summons: PeerSummon[] = [];
		const h = harness({ onSummon: (s) => summons.push(s) });
		const summon = (seq: number) => ({
			[SUMMON_FIELD]: { x: 1, y: 2, zoom: 1.5, layerPath: "root", seq, ts: 1 },
		});
		h.awareness.states.set(2, { sub: "peer-a", ...summon(1) });
		h.awareness.states.set(1, { sub: "me", ...summon(1) });
		const observer = h.start();
		expect(summons).toEqual([]);

		h.awareness.states.set(2, { sub: "peer-a", ...summon(2) });
		h.awareness.states.set(1, { sub: "me", ...summon(2) });
		h.tick();
		h.tick();
		expect(summons).toEqual([
			{ clientId: 2, sub: "peer-a", x: 1, y: 2, zoom: 1.5, layerPath: "root" },
		]);

		// A session that drops out and comes back with the same seq stays quiet.
		h.awareness.states.delete(2);
		h.tick();
		h.awareness.states.set(2, { sub: "peer-a", ...summon(2) });
		h.tick();
		expect(summons.length).toBe(1);
		observer.dispose();
	});

	test("a session that shows up AFTER subscription carries history: its summon and ping never fire", () => {
		const summons: PeerSummon[] = [];
		const h = harness({ onSummon: (s) => summons.push(s) });
		const observer = h.start();
		// Late joiner (or a WebRTC peer whose state arrives after ours was seeded)
		// with a summon and a ping already on the wire.
		h.awareness.states.set(3, {
			sub: "late",
			[SUMMON_FIELD]: { x: 1, y: 2, zoom: 1, layerPath: "root", seq: 7, ts: 1 },
			[PING_FIELD]: { x: 5, y: 5, layerPath: "root", seq: 3, ts: 1 },
		});
		h.tick();
		expect(summons).toEqual([]);
		expect(observer.pingStore.getSnapshot()).toEqual([]);
		// Its NEXT summon/ping is a real event.
		h.awareness.states.set(3, {
			sub: "late",
			[SUMMON_FIELD]: { x: 1, y: 2, zoom: 1, layerPath: "root", seq: 8, ts: 2 },
			[PING_FIELD]: { x: 5, y: 5, layerPath: "root", seq: 4, ts: 2 },
		});
		h.tick();
		expect(summons.length).toBe(1);
		expect(observer.pingStore.getSnapshot().map((p) => p.key)).toEqual(["3:4"]);
		observer.dispose();
	});

	test("run outcomes fire once per runId+ts", () => {
		const runs: PeerRun[] = [];
		const h = harness({ onPeerRun: (r) => runs.push(r) });
		h.awareness.states.set(2, { sub: "peer-a" });
		const observer = h.start();
		const run = (ts: number, status = "ok") => ({
			[LAST_RUN_FIELD]: { runId: RUN_ID, status, executed: 12, ts },
		});
		h.awareness.states.set(2, { sub: "peer-a", ...run(1) });
		h.tick();
		h.tick();
		expect(runs).toEqual([
			{ clientId: 2, sub: "peer-a", runId: RUN_ID, status: "ok", executed: 12 },
		]);
		h.awareness.states.set(2, { sub: "peer-a", ...run(2, "error") });
		h.tick();
		expect(runs.length).toBe(2);
		expect(runs[1]?.status).toBe("error");
		observer.dispose();
	});
});

describe("join / leave", () => {
	test("the initial set is silent; joins and leaves settle after the debounce", () => {
		const events: PeerPresenceEvent[] = [];
		const h = harness({
			selfSub: "me",
			onPeerPresenceEvent: (e) => events.push(e),
		});
		h.awareness.states.set(1, { sub: "me" });
		h.awareness.states.set(2, { sub: "peer-a" });
		const observer = h.start();
		h.scheduler.flush();
		expect(events).toEqual([]);

		// Peers still connecting right after we joined are the roster, not joins.
		h.awareness.states.set(7, { sub: "peer-c" });
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);

		h.advance(PRESENCE_ROSTER_GRACE_MS);
		h.awareness.states.set(3, { sub: "peer-b" });
		h.tick();
		expect(events).toEqual([]);
		h.scheduler.flush();
		expect(events).toEqual([{ sub: "peer-b", kind: "joined" }]);

		h.awareness.states.delete(2);
		h.tick();
		h.scheduler.flush();
		expect(events[1]).toEqual({ sub: "peer-a", kind: "left" });
		observer.dispose();
	});

	test("a reconnect flap and a second window of the same user fire nothing", () => {
		const events: PeerPresenceEvent[] = [];
		const h = harness({
			selfSub: "me",
			onPeerPresenceEvent: (e) => events.push(e),
		});
		h.awareness.states.set(2, { sub: "peer-a" });
		const observer = h.start();
		h.advance(PRESENCE_ROSTER_GRACE_MS);

		// Flap: gone and back before the debounce elapses.
		h.awareness.states.delete(2);
		h.tick();
		h.awareness.states.set(2, { sub: "peer-a" });
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);

		// Second window of an already-present user is not a join.
		h.awareness.states.set(5, { sub: "peer-a" });
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);

		// Closing one of two windows is not a leave; closing the last one is.
		h.awareness.states.delete(2);
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);
		h.awareness.states.delete(5);
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([{ sub: "peer-a", kind: "left" }]);
		observer.dispose();
	});

	test("the local user's own sessions never announce, and a brief join blip is dropped", () => {
		const events: PeerPresenceEvent[] = [];
		const h = harness({
			selfSub: "me",
			onPeerPresenceEvent: (e) => events.push(e),
		});
		const observer = h.start();
		h.advance(PRESENCE_ROSTER_GRACE_MS);
		h.awareness.states.set(1, { sub: "me" });
		h.awareness.states.set(9, { sub: "me" });
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);

		h.awareness.states.set(4, { sub: "peer-z" });
		h.tick();
		h.awareness.states.delete(4);
		h.tick();
		h.scheduler.flush();
		expect(events).toEqual([]);
		observer.dispose();
	});
});

describe("lifecycle", () => {
	test("dispose unsubscribes and cancels every pending timer", () => {
		const events: PeerPresenceEvent[] = [];
		const h = harness({ onPeerPresenceEvent: (e) => events.push(e) });
		h.awareness.states.set(2, { sub: "peer-a" });
		const observer = h.start();
		h.advance(PRESENCE_ROSTER_GRACE_MS);
		// One ping expiry timer plus one pending "joined" for peer-b.
		h.awareness.states.set(2, { sub: "peer-a", ...ping(1) });
		h.tick();
		h.awareness.states.set(3, { sub: "peer-b" });
		h.tick();
		expect(h.scheduler.size()).toBe(2);
		observer.dispose();
		expect(h.scheduler.size()).toBe(0);
		h.scheduler.flush();
		expect(events).toEqual([]);

		let emissions = 0;
		observer.pingStore.subscribe(() => emissions++);
		h.awareness.states.set(2, { sub: "peer-a", ...ping(2) });
		h.tick();
		expect(emissions).toBe(0);
	});
});
