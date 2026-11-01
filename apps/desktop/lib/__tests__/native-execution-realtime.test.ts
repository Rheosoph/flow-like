import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type {
	RealtimePollResponse,
	RealtimeRunSnapshot,
} from "@flow-like/flow-like-ui/lib/realtime/types";

const { invoke, channels } = vi.hoisted(() => ({
	invoke: vi.fn(),
	channels: [] as Array<{ onmessage: (message: unknown) => void }>,
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke,
	Channel: class {
		onmessage = (_message: unknown) => {};
		constructor() {
			channels.push(this);
		}
	},
}));
import { followNativeExecution } from "../native-execution-realtime";

beforeEach(() => {
	vi.useFakeTimers();
	invoke.mockResolvedValue(undefined);
});
afterEach(() => {
	vi.useRealTimers();
	invoke.mockReset();
	channels.length = 0;
});

const event = (sequence: number) => ({
	sequence,
	event_id: `original-${sequence}`,
	event_type: "chat_out",
	payload: { text: `${sequence}` },
	created_at: "2026-11-01T00:00:00Z",
});
const snapshot = (
	overrides: Partial<RealtimeRunSnapshot> = {},
): RealtimeRunSnapshot => ({
	run_id: "run",
	revision: "one",
	status: "running",
	progress: 0,
	events: [],
	after_sequence: -1,
	has_more: false,
	...overrides,
});
const deliver = (
	run: RealtimeRunSnapshot,
	id = "delivery",
	errors: RealtimePollResponse["errors"] = [],
) => {
	channels[0].onmessage({
		type: "update",
		delivery_id: id,
		response: { protocol_version: 1, runs: [run], errors },
	});
};
function start(onEvent = vi.fn()) {
	const controller = new AbortController();
	const outcome = followNativeExecution({
		profileId: "profile",
		subject: "user",
		issuer: "issuer",
		run: { run_id: "run", poll_token: "capability" },
		signal: controller.signal,
		onEvent,
	}).then(
		() => ({ error: undefined }),
		(error: Error) => ({ error }),
	);
	return { controller, outcome, onEvent };
}

test("drains terminal replay pages and acknowledges before unsubscribing", async () => {
	const run = start();
	let finished = false;
	void run.outcome.then(() => {
		finished = true;
	});
	expect(invoke.mock.calls[0][1].request.runs).toEqual([
		{ run_id: "run", poll_token: "capability" },
	]);
	deliver(
		snapshot({
			status: "completed",
			events: [event(0)],
			after_sequence: 0,
			has_more: true,
		}),
	);
	await vi.advanceTimersByTimeAsync(0);
	expect(finished).toBe(false);
	deliver(
		snapshot({ status: "completed", events: [event(1)], after_sequence: 1 }),
		"second",
	);
	await vi.runAllTimersAsync();
	expect((await run.outcome).error).toBeUndefined();
	expect(run.onEvent.mock.calls.map(([event]) => event.event_id)).toEqual([
		"original-0",
		"original-1",
		"run:terminal",
	]);
	const commands = invoke.mock.calls.map(([command]) => command);
	expect(commands.lastIndexOf("realtime_acknowledge")).toBeLessThan(
		commands.indexOf("realtime_unsubscribe"),
	);
});

test("retries a failed consumer without replaying its already accepted prefix", async () => {
	let fail = true;
	const received: string[] = [];
	const run = start(
		vi.fn((value) => {
			if (value.event_id === "original-1" && fail) {
				fail = false;
				throw new Error("render failed");
			}
			received.push(value.event_id);
		}),
	);
	const batch = snapshot({ events: [event(0), event(1)], after_sequence: 1 });
	deliver(batch);
	await vi.advanceTimersByTimeAsync(0);
	expect(
		invoke.mock.calls.some(([name]) => name === "realtime_acknowledge"),
	).toBe(false);
	deliver(batch);
	await vi.advanceTimersByTimeAsync(0);
	expect(received).toEqual(["original-0", "original-1"]);
	expect(
		invoke.mock.calls.filter(([name]) => name === "realtime_acknowledge"),
	).toHaveLength(1);
	run.controller.abort();
	expect((await run.outcome).error?.name).toBe("AbortError");
});

test("surfaces authoritative failure even when no completed event was stored", async () => {
	const run = start();
	deliver(snapshot({ status: "FAILED", error: "Executor stopped" }));
	await vi.runAllTimersAsync();
	expect((await run.outcome).error?.message).toBe("Executor stopped");
	expect(run.onEvent.mock.calls[0][0].payload.status).toBe("FAILED");
});

test("ignores other run failures and closes on logout without accepting late events", async () => {
	const run = start();
	deliver(snapshot(), "delivery", [{ run_id: "other", code: "unavailable" }]);
	await vi.advanceTimersByTimeAsync(0);
	channels[0].onmessage({ type: "state", state: "signed_out" });
	expect((await run.outcome).error?.message).toContain("signed out");
	deliver(snapshot({ events: [event(0)] }), "late");
	await vi.runAllTimersAsync();
	expect(run.onEvent).not.toHaveBeenCalled();
	expect(
		invoke.mock.calls.some(([name]) => name === "realtime_unsubscribe"),
	).toBe(true);
});
