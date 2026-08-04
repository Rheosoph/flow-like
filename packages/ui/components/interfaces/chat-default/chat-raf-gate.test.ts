import { describe, expect, test } from "bun:test";

/**
 * Regression guard for the RAF gate in <Chat>'s `pushCurrentMessageUpdate`.
 *
 * The real component throttles live-bubble updates behind a single pending
 * animation frame: it only schedules one while `rafId === null`. The unmount
 * cleanup used to cancel the frame WITHOUT clearing the id — and refs survive
 * React's mount → cleanup → remount cycle, so the gate latched shut and every
 * later push was buffered and never flushed. The global chat pushes on mount,
 * landing squarely inside that window, so only it lost its live bubbles.
 *
 * This models the gate in isolation so the invariant is pinned without mounting
 * the full chat surface.
 */
function makeGate(clearIdOnCleanup: boolean) {
	let rafId: number | null = null;
	let pending: string[] | null = null;
	const flushed: string[][] = [];
	const frames: Array<() => void> = [];

	return {
		flushed,
		push(messages: string[]) {
			pending = messages;
			if (rafId === null) {
				rafId = frames.push(() => {
					rafId = null;
					if (pending) flushed.push(pending);
				});
			}
		},
		/** Run whatever frame is queued, as the browser would. */
		runFrames() {
			const queued = frames.splice(0, frames.length);
			for (const frame of queued) frame();
		},
		/** The unmount cleanup. */
		cleanup() {
			if (rafId !== null) {
				frames.length = 0; // cancelAnimationFrame
				if (clearIdOnCleanup) rafId = null;
			}
		},
	};
}

describe("Chat live-bubble RAF gate", () => {
	test("keeps flushing after a mount → cleanup → remount cycle", () => {
		const gate = makeGate(true);

		// Push during mount, then React tears the effect down and re-runs it.
		gate.push(["chunk-1"]);
		gate.cleanup();

		// Streaming continues after the remount.
		gate.push(["chunk-2"]);
		gate.runFrames();
		gate.push(["chunk-3"]);
		gate.runFrames();

		expect(gate.flushed).toEqual([["chunk-2"], ["chunk-3"]]);
	});

	test("leaving the id set after cleanup latches the gate shut", () => {
		// Documents the exact failure mode the fix removes.
		const broken = makeGate(false);

		broken.push(["chunk-1"]);
		broken.cleanup();

		broken.push(["chunk-2"]);
		broken.runFrames();
		broken.push(["chunk-3"]);
		broken.runFrames();

		expect(broken.flushed).toEqual([]);
	});
});
