import { afterEach, describe, expect, test } from "bun:test";
import type { Query } from "@tanstack/react-query";
import {
	ADAPTIVE_POLL_MAX_MS,
	adaptiveInterval,
	backedOffMs,
	resetAdaptivePolling,
} from "./adaptive-polling";
import { useRealtimeStatus } from "./realtime/realtime-status";
import type { RealtimeStatus } from "./realtime/types";

const LIVE: RealtimeStatus = {
	runner: "in-tab",
	state: "connected",
	provider: { live: true },
};
const POLLING: RealtimeStatus = { runner: "in-tab", state: "connected" };

function deliver(status: RealtimeStatus | null) {
	useRealtimeStatus.setState({ status });
}

/** A query whose answers are `data` objects; the same object means an unchanged answer. */
function fakeQuery() {
	const state = { data: {} as unknown, dataUpdateCount: 0 };
	// biome-ignore lint/suspicious/noExplicitAny: the helper reads only `state`.
	const query = { state } as unknown as Query<any, any, any, any>;
	return {
		query,
		answer(changed: boolean) {
			state.dataUpdateCount++;
			if (changed) state.data = {};
		},
	};
}

afterEach(() => deliver(null));

describe("adaptive polling", () => {
	test("keeps its base interval while realtime carries nothing", () => {
		deliver(null);
		const { query, answer } = fakeQuery();
		const intervals = [adaptiveInterval(query, { baseMs: 15_000 })];
		for (let i = 0; i < 6; i++) {
			answer(false);
			intervals.push(adaptiveInterval(query, { baseMs: 15_000 }));
		}
		expect(new Set(intervals)).toEqual(new Set([15_000]));
	});

	test("doubles after quiet answers over a live connection, up to its cap", () => {
		deliver(LIVE);
		const { query, answer } = fakeQuery();
		const intervals = [adaptiveInterval(query, { baseMs: 30_000 })];
		for (let i = 0; i < 6; i++) {
			answer(false);
			intervals.push(adaptiveInterval(query, { baseMs: 30_000 }));
		}
		expect(intervals).toEqual([
			30_000, 30_000, 60_000, 120_000, 240_000, 300_000, 300_000,
		]);
	});

	test("a changed answer or a reset starts over at the base interval", () => {
		deliver(LIVE);
		const { query, answer } = fakeQuery();
		adaptiveInterval(query, { baseMs: 30_000 });
		for (let i = 0; i < 4; i++) {
			answer(false);
			adaptiveInterval(query, { baseMs: 30_000 });
		}
		answer(true);
		expect(adaptiveInterval(query, { baseMs: 30_000 })).toBe(30_000);
		for (let i = 0; i < 4; i++) {
			answer(false);
			adaptiveInterval(query, { baseMs: 30_000 });
		}
		expect(adaptiveInterval(query, { baseMs: 30_000 })).toBe(240_000);
		resetAdaptivePolling();
		expect(adaptiveInterval(query, { baseMs: 30_000 })).toBe(30_000);
	});

	test("a poll that needs a live push connection stays fast while realtime only polls", () => {
		const options = { baseMs: 10_000, liveBaseMs: 30_000, requireLive: true };
		deliver(POLLING);
		const { query, answer } = fakeQuery();
		for (let i = 0; i < 3; i++) answer(false);
		expect(adaptiveInterval(query, options)).toBe(10_000);
		deliver(LIVE);
		expect(adaptiveInterval(query, options)).toBe(30_000);
		for (let i = 0; i < 3; i++) {
			answer(false);
			adaptiveInterval(query, options);
		}
		expect(adaptiveInterval(query, options)).toBe(120_000);
	});

	test("other polls back off while realtime polls, and start over when delivery changes", () => {
		deliver(POLLING);
		const { query, answer } = fakeQuery();
		adaptiveInterval(query, { baseMs: 15_000 });
		for (let i = 0; i < 3; i++) {
			answer(false);
			adaptiveInterval(query, { baseMs: 15_000 });
		}
		expect(adaptiveInterval(query, { baseMs: 15_000 })).toBe(60_000);
		deliver(LIVE);
		expect(adaptiveInterval(query, { baseMs: 15_000 })).toBe(15_000);
	});

	test("backoff math", () => {
		expect(backedOffMs(1_000, 0)).toBe(1_000);
		expect(backedOffMs(1_000, 1)).toBe(1_000);
		expect(backedOffMs(1_000, 3)).toBe(4_000);
		expect(backedOffMs(60_000, 40)).toBe(ADAPTIVE_POLL_MAX_MS);
		expect(backedOffMs(1_000, 5, 3_000)).toBe(3_000);
	});
});
