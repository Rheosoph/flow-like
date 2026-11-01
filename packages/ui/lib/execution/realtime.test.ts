import { expect, test, vi } from "vitest";
import type {
	RealtimeClient,
	RealtimeListener,
	RealtimeRunSnapshot,
} from "../realtime/types";
import { followRealtimeExecution, readExecutionReceipt } from "./realtime";
import { ExecutionDetachedError } from "./types";

function follow(createClient?: () => Promise<RealtimeClient>) {
	let listener: RealtimeListener | undefined;
	const close = vi.fn();
	const unsubscribe = vi.fn();
	const client: RealtimeClient = {
		setSession: vi.fn(),
		setSubscriptions: vi.fn(),
		setActivity: vi.fn(),
		close,
		subscribe: (callback) => {
			listener = callback;
			return unsubscribe;
		},
	};
	const controller = new AbortController();
	const result = followRealtimeExecution({
		createClient: createClient ?? (async () => client),
		session: {
			scope: "account",
			url: "https://hub.test/api/v1/realtime/poll",
			headers: {},
		},
		receipt: { run_id: "run", poll_token: "secret" },
		signal: controller.signal,
		onUnauthorized: async () => undefined,
		onRun: () => {},
	});
	const subscribed = () => vi.waitFor(() => expect(listener).toBeDefined());
	return {
		result,
		controller,
		close,
		unsubscribe,
		subscribed,
		emit: async (update: Parameters<RealtimeListener>[0]) => {
			await subscribed();
			await listener?.(update);
		},
	};
}

const snapshot = (
	runs: RealtimeRunSnapshot[],
	errors: { run_id: string; code: string }[] = [],
) => ({
	type: "snapshot" as const,
	snapshot: { protocol_version: 1 as const, runs, errors },
});

const terminal: RealtimeRunSnapshot = {
	run_id: "run",
	revision: "r2",
	status: "COMPLETED",
	progress: 100,
	events: [],
	after_sequence: -1,
	has_more: false,
};

test.each(["abort", "closed", "not_found"])(
	"replay releases its port after %s",
	async (failure) => {
		const { result, controller, emit, close, unsubscribe, subscribed } =
			follow();
		const rejected = expect(result).rejects.toThrow();
		await subscribed();
		if (failure === "abort")
			controller.abort(new Error("Account disconnected"));
		else if (failure === "closed")
			await emit({ type: "state", state: "closed" });
		else await emit(snapshot([], [{ run_id: "run", code: "not_found" }]));
		await rejected;
		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(close).toHaveBeenCalledOnce();
	},
);

test("retrying states and temporarily unavailable replays keep following the run", async () => {
	const { result, emit, close } = follow();
	for (const state of ["unsupported", "unauthorized", "reconnecting"] as const)
		await emit({ type: "state", state });
	await emit(snapshot([], [{ run_id: "run", code: "unavailable" }]));
	expect(close).not.toHaveBeenCalled();
	await emit(snapshot([terminal]));
	await expect(result).resolves.toMatchObject({ status: "COMPLETED" });
	expect(close).toHaveBeenCalledOnce();
});

test("a closed replay detaches from the run instead of failing it", async () => {
	const { result, emit } = follow();
	await emit({ type: "state", state: "closed" });
	const error = await result.catch((cause) => cause);
	expect(error).toBeInstanceOf(ExecutionDetachedError);
	expect(error).toMatchObject({ runId: "run" });
});

test("a replay that cannot start detaches from the queued run", async () => {
	const { result } = follow(async () => {
		throw new Error("worker unavailable");
	});
	const error = await result.catch((cause) => cause);
	expect(error).toBeInstanceOf(ExecutionDetachedError);
	expect(error.message).toContain("worker unavailable");
});

test("a capability cannot enter replay through an invalid or oversized receipt", async () => {
	for (const receipt of [
		{ run_id: "a", poll_token: "" },
		{ run_id: "a\nb" },
		{ run_id: "a", poll_token: "x".repeat(8193) },
	])
		await expect(readExecutionReceipt(Response.json(receipt))).rejects.toThrow(
			"invalid execution receipt",
		);
	await expect(
		readExecutionReceipt(new Response("x".repeat(65537))),
	).rejects.toThrow("too large");
});
