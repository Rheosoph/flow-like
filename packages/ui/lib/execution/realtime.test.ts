import { expect, test, vi } from "vitest";
import type { RealtimeClient, RealtimeListener } from "../realtime/types";
import { followRealtimeExecution, readExecutionReceipt } from "./realtime";

test.each(["abort", "unsupported", "unavailable"])(
	"replay releases its port after %s",
	async (failure) => {
		let listener: RealtimeListener | undefined;
		const close = vi.fn();
		const unsubscribe = vi.fn();
		const client: RealtimeClient = {
			mode: "in-tab",
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
			createClient: async () => client,
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
		await Promise.resolve();
		const rejected = expect(result).rejects.toThrow();
		if (failure === "abort")
			controller.abort(new Error("Account disconnected"));
		else if (failure === "unsupported")
			await listener?.({ type: "state", state: "unsupported" });
		else
			await listener?.({
				type: "snapshot",
				snapshot: {
					protocol_version: 1,
					runs: [],
					errors: [{ run_id: "run", code: "unavailable" }],
				},
			});
		await rejected;
		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(close).toHaveBeenCalledOnce();
	},
);

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
