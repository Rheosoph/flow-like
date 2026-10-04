import { describe, expect, test } from "bun:test";
import { trackPageSave, waitForPendingPageSaves } from "./pending-page-saves";

function deferred() {
	let resolve = () => {};
	let reject = (_error: unknown) => {};
	const promise = new Promise<void>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

describe("pending page saves", () => {
	test("registers saves synchronously and waits for every pending save", async () => {
		const backend = {};
		const first = deferred();
		const second = deferred();
		expect(trackPageSave(backend, "app", first.promise)).toBe(first.promise);
		trackPageSave(backend, "app", second.promise);
		let finished = false;
		const waiting = waitForPendingPageSaves(backend, "app").then(() => {
			finished = true;
		});

		first.resolve();
		await first.promise;
		expect(finished).toBe(false);
		second.resolve();
		await waiting;
		expect(finished).toBe(true);
		await expect(
			waitForPendingPageSaves(backend, "app"),
		).resolves.toBeUndefined();
	});

	test("also waits for a follow-up save queued while the first save completes", async () => {
		const backend = {};
		const first = deferred();
		const followUp = deferred();
		trackPageSave(backend, "app", first.promise);
		let finished = false;
		const waiting = waitForPendingPageSaves(backend, "app").then(() => {
			finished = true;
		});
		const queued = first.promise.then(() => {
			trackPageSave(backend, "app", followUp.promise);
		});

		first.resolve();
		await queued;
		await Promise.resolve();
		expect(finished).toBe(false);
		followUp.resolve();
		await waiting;
		expect(finished).toBe(true);
	});

	test("keeps apps and backend instances separate", async () => {
		const backend = {};
		const otherBackend = {};
		const own = deferred();
		const otherApp = deferred();
		const otherHost = deferred();
		trackPageSave(backend, "app", own.promise);
		trackPageSave(backend, "other-app", otherApp.promise);
		trackPageSave(otherBackend, "app", otherHost.promise);

		const waiting = waitForPendingPageSaves(backend, "app");
		own.resolve();
		await waiting;
		await expect(waitForPendingPageSaves({}, "app")).resolves.toBeUndefined();

		otherApp.resolve();
		otherHost.resolve();
		await Promise.all([
			waitForPendingPageSaves(backend, "other-app"),
			waitForPendingPageSaves(otherBackend, "app"),
		]);
	});

	test("propagates a pending failure after the other saves finish and clears it for a later retry", async () => {
		const backend = {};
		const failed = deferred();
		const successful = deferred();
		const error = new Error("Draft could not be saved");
		const tracked = trackPageSave(backend, "app", failed.promise);
		trackPageSave(backend, "app", successful.promise);
		const originalResult = tracked.then(
			() => undefined,
			(reason) => reason,
		);
		const waiting = waitForPendingPageSaves(backend, "app");
		const waitResult = waiting.then(
			() => undefined,
			(reason) => reason,
		);
		let finished = false;
		void waiting.then(
			() => {
				finished = true;
			},
			() => {
				finished = true;
			},
		);

		failed.reject(error);
		expect(await originalResult).toBe(error);
		expect(finished).toBe(false);
		successful.resolve();
		expect(await waitResult).toBe(error);
		await expect(
			waitForPendingPageSaves(backend, "app"),
		).resolves.toBeUndefined();
	});

	test("cleans up a rejected save when no view is waiting without an unhandled rejection", async () => {
		const backend = {};
		trackPageSave(backend, "app", Promise.reject(new Error("Save failed")));
		await Promise.resolve();

		await expect(
			waitForPendingPageSaves(backend, "app"),
		).resolves.toBeUndefined();
		const next = deferred();
		trackPageSave(backend, "app", next.promise);
		const waiting = waitForPendingPageSaves(backend, "app");
		next.resolve();
		await waiting;
	});

	test("observes a newly queued save that fails before an earlier save finishes", async () => {
		const backend = {};
		const first = deferred();
		const later = deferred();
		const error = new Error("Queued save failed");
		trackPageSave(backend, "app", first.promise);
		const result = waitForPendingPageSaves(backend, "app").then(
			() => undefined,
			(reason) => reason,
		);
		trackPageSave(backend, "app", later.promise);
		later.reject(error);
		await later.promise.catch(() => {});
		first.resolve();

		expect(await result).toBe(error);
		await expect(
			waitForPendingPageSaves(backend, "app"),
		).resolves.toBeUndefined();
	});
});
