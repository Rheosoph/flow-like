import { CopilotBackendConnectionCoordinator } from "@flow-like/flow-like-ui/hooks/copilot-backend-coordinator";
import { describe, expect, test, vi } from "vitest";

describe("copilot backend connection coordinator", () => {
	test("deduplicates concurrent starts across callers", async () => {
		const coordinator = new CopilotBackendConnectionCoordinator();
		let finish: (() => void) | undefined;
		const operation = vi.fn(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);

		const first = coordinator.start("codex", operation);
		const second = coordinator.start("codex", operation);
		await Promise.resolve();
		expect(operation).toHaveBeenCalledTimes(1);
		finish?.();
		await Promise.all([first, second]);
		expect(coordinator.snapshot("codex").isRunning).toBe(true);
	});

	test("keeps a timed-out caller's raw start as the shared in-flight work", async () => {
		const coordinator = new CopilotBackendConnectionCoordinator();
		const operation = vi.fn(() => new Promise<void>(() => undefined));
		const raw = coordinator.start("claude-code", operation);
		const duplicate = coordinator.start("claude-code", operation);

		expect(duplicate).toBe(raw);
		expect(operation).toHaveBeenCalledTimes(0);
		await Promise.resolve();
		expect(operation).toHaveBeenCalledTimes(1);
		expect(coordinator.snapshot("claude-code").isConnecting).toBe(true);
	});

	test("backs off repeated starts after a native failure", async () => {
		const coordinator = new CopilotBackendConnectionCoordinator(1_000, 8_000);
		await expect(
			coordinator.start(
				"github-copilot",
				async () => {
					throw new Error("native start failed");
				},
				10,
			),
		).rejects.toThrow("native start failed");

		await expect(
			coordinator.start("github-copilot", async () => undefined, Date.now()),
		).rejects.toThrow(/native start failed[\s\S]*cooling down/);
	});

	test("waits for an in-flight start before stopping to avoid a late orphan", async () => {
		const coordinator = new CopilotBackendConnectionCoordinator();
		let finishStart: (() => void) | undefined;
		const start = coordinator.start(
			"codex",
			() =>
				new Promise<void>((resolve) => {
					finishStart = resolve;
				}),
		);
		await Promise.resolve();
		const stopOperation = vi.fn(async () => undefined);
		const stop = coordinator.stop("codex", stopOperation);
		await Promise.resolve();
		expect(stopOperation).not.toHaveBeenCalled();

		finishStart?.();
		await start;
		await stop;
		expect(stopOperation).toHaveBeenCalledTimes(1);
		expect(coordinator.snapshot("codex").isRunning).toBe(false);
	});

	test("retains a runtime failure for every mounted backend picker", async () => {
		const coordinator = new CopilotBackendConnectionCoordinator();
		await coordinator.start("claude-code", async () => undefined);

		coordinator.reportFailure(
			"claude-code",
			new Error("OAuth session expired"),
		);

		expect(coordinator.snapshot("claude-code")).toMatchObject({
			isRunning: false,
			isConnecting: false,
			error: "OAuth session expired",
			retryAtMs: 0,
		});

		// A newly mounted hook observes the native backend marker, which can
		// remain alive after a per-request failure. That probe must not erase the
		// diagnosis before the user retries.
		coordinator.reconcile("claude-code", true);
		expect(coordinator.snapshot("claude-code")).toMatchObject({
			isRunning: false,
			error: "OAuth session expired",
		});

		await coordinator.start("claude-code", async () => undefined);
		expect(coordinator.snapshot("claude-code")).toMatchObject({
			isRunning: true,
			error: null,
		});
	});
});
