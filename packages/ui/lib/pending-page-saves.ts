interface PendingPageSaves {
	saves: Set<Promise<void>>;
	observers: Set<Set<Promise<void>>>;
}

const pendingByBackend = new WeakMap<object, Map<string, PendingPageSaves>>();

function removeIfSettled(pageState: object, appId: string) {
	const byApp = pendingByBackend.get(pageState);
	const pending = byApp?.get(appId);
	if (!byApp || !pending || pending.saves.size || pending.observers.size)
		return;
	byApp.delete(appId);
	if (byApp.size === 0) pendingByBackend.delete(pageState);
}

/** Registers editor saves before another view reads or updates the saved pages. */
export function trackPageSave(
	pageState: object,
	appId: string,
	save: Promise<void>,
): Promise<void> {
	let byApp = pendingByBackend.get(pageState);
	if (!byApp) {
		byApp = new Map();
		pendingByBackend.set(pageState, byApp);
	}
	let pending = byApp.get(appId);
	if (!pending) {
		pending = { saves: new Set(), observers: new Set() };
		byApp.set(appId, pending);
	}
	if (pending.saves.has(save)) return save;
	pending.saves.add(save);
	for (const observer of pending.observers) observer.add(save);
	const cleanup = () => {
		pending.saves.delete(save);
		removeIfSettled(pageState, appId);
	};
	// Both handlers settle successfully; a rejected save creates no unhandled cleanup promise.
	void save.then(cleanup, cleanup);
	return save;
}

/** Waits for in-flight editor saves, including saves registered while waiting. */
export async function waitForPendingPageSaves(
	pageState: object,
	appId: string,
): Promise<void> {
	const pending = pendingByBackend.get(pageState)?.get(appId);
	if (!pending) return;
	const observed = new Set(pending.saves);
	pending.observers.add(observed);
	let failure: PromiseRejectedResult | undefined;
	try {
		while (observed.size) {
			const batch = [...observed];
			observed.clear();
			const results = await Promise.allSettled(batch);
			failure ??= results.find(
				(result): result is PromiseRejectedResult =>
					result.status === "rejected",
			);
		}
		if (failure) throw failure.reason;
	} finally {
		pending.observers.delete(observed);
		removeIfSettled(pageState, appId);
	}
}
