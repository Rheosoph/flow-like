import { describe, expect, it } from "vitest";
import { EXECUTOR_WARM_INTERVAL_MS, executorWarmCount } from "./executor-warm";

describe("executor warm-up on page bootstrap", () => {
	it("asks for warm executors once per interval", () => {
		const start = 1_000_000_000;
		expect(executorWarmCount(start)).toBe(3);
		expect(executorWarmCount(start + 1)).toBeUndefined();
		expect(
			executorWarmCount(start + EXECUTOR_WARM_INTERVAL_MS - 1),
		).toBeUndefined();
		expect(executorWarmCount(start + EXECUTOR_WARM_INTERVAL_MS)).toBe(3);
	});
});
