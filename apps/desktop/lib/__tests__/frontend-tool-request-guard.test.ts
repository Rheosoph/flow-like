import { FrontendToolRequestGuard } from "@flow-like/flow-like-ui/components/flowpilot/frontend-tool-request-guard";
import { describe, expect, test, vi } from "vitest";

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((settle) => {
		resolve = settle;
	});
	return { promise, resolve };
}

describe("embedded FlowPilot frontend tool request guard", () => {
	test("blocks a database mutation and success after its approval request is cancelled", async () => {
		const guard = new FrontendToolRequestGuard();
		const approval = deferred<{ approved: boolean }>();
		const invalidated = vi.fn();
		const lease = guard.begin({
			requestId: "database-approval",
			onInvalidated: invalidated,
		});
		if (!lease) throw new Error("expected the first generation");
		let mutationCount = 0;
		let successDelivered = false;
		const execution = (async () => {
			const answer = await approval.promise;
			lease.assertActive("approval response");
			if (!answer.approved) return;
			mutationCount += 1;
			lease.assertActive("success delivery");
			successDelivered = true;
		})();

		expect(guard.cancel("database-approval")).toBe(true);
		approval.resolve({ approved: true });
		await expect(execution).rejects.toMatchObject({ name: "AbortError" });
		expect(lease.signal.aborted).toBe(true);
		expect(invalidated).toHaveBeenCalledWith("cancelled");
		expect(mutationCount).toBe(0);
		expect(successDelivered).toBe(false);
	});

	test("keeps a cancelled generation fenced until its original execution settles", () => {
		const guard = new FrontendToolRequestGuard();
		const first = guard.begin({ requestId: "same-request" });
		if (!first) throw new Error("expected the first generation");
		guard.cancel("same-request");

		expect(guard.begin({ requestId: "same-request" })).toBeUndefined();
		first.settle();
		const second = guard.begin({ requestId: "same-request" });
		expect(second).toBeDefined();
		expect(second?.generation).not.toBe(first.generation);
		second?.settle();
	});

	test("invalidates an already-expired request before approval handling", () => {
		const invalidated = vi.fn();
		const guard = new FrontendToolRequestGuard();
		const lease = guard.begin({
			requestId: "expired-request",
			deadlineAtMs: Date.now() - 1,
			onInvalidated: invalidated,
		});
		if (!lease) throw new Error("expected a fenced expired generation");

		expect(lease.isActive()).toBe(false);
		expect(lease.invalidationReason()).toBe("deadline");
		expect(invalidated).toHaveBeenCalledWith("deadline");
		expect(() => lease.assertActive("approval handling")).toThrow(
			/late side effects were blocked/,
		);
		lease.settle();
	});
});
