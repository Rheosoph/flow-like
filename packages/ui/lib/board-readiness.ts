/**
 * Tracks board refreshes that a later execution has to wait for.
 *
 * Opening a page and running its workflow need the board for different reasons and at
 * different times: rendering only needs the page payload, while execution needs the board this
 * device will actually run. Registering the refresh here lets the read start immediately and
 * the run still wait, instead of the render paying for the run's guarantee.
 */

const inFlight = new Map<string, Promise<void>>();

export function boardReadinessKey(
	appId: string,
	boardId: string,
	version?: readonly number[],
): string {
	return `${appId}:${boardId}:${version?.join(".") ?? "latest"}`;
}

/**
 * Starts a refresh, or joins the one already running for this board. A failed refresh resolves
 * rather than rejects: callers wait for it to be *over*, and the board they already hold stays
 * the one they run.
 */
export function trackBoardReadiness(
	key: string,
	start: () => Promise<unknown>,
): Promise<void> {
	const existing = inFlight.get(key);
	if (existing) return existing;

	const task = start()
		.then(() => undefined)
		.catch(() => undefined)
		.finally(() => {
			// Only retract this run's entry: a refresh started after it is the current one.
			if (inFlight.get(key) === task) inFlight.delete(key);
		});

	inFlight.set(key, task);
	return task;
}

/** Resolves immediately when no refresh is in flight for this board. */
export function whenBoardReady(key: string): Promise<void> {
	return inFlight.get(key) ?? Promise.resolve();
}

/** Test seam: drops all tracked refreshes. */
export function resetBoardReadiness(): void {
	inFlight.clear();
}
