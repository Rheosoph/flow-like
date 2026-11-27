/** A page open sends its onLoad and its widgets' first runs at once. */
const EXECUTOR_WARM_COUNT = 3;
/** A tenant's idle executors were gone after about four minutes; within two they still serve. */
export const EXECUTOR_WARM_INTERVAL_MS = 2 * 60_000;
let lastExecutorWarm = Number.NEGATIVE_INFINITY;

/**
 * How many cloud executors the page bootstrap should start for the runs that follow it, or
 * `undefined` while the ones a recent page open started are likely still warm.
 */
export function executorWarmCount(now = Date.now()): number | undefined {
	if (now - lastExecutorWarm < EXECUTOR_WARM_INTERVAL_MS) return undefined;
	lastExecutorWarm = now;
	return EXECUTOR_WARM_COUNT;
}
