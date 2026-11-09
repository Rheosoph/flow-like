import type { EmotionWorkerRequest, EmotionWorkerResponse } from "./protocol";

export type EmotionDispatchMessage =
	| EmotionWorkerRequest
	| { type: "disconnect" };

export interface EmotionDispatchPort {
	postMessage(message: EmotionWorkerResponse): void;
	start?(): void;
	close?(): void;
	onmessage: ((event: MessageEvent<EmotionDispatchMessage>) => void) | null;
}

/** One inference queue for every connected tab. Replies stay on the requesting port. */
export function createEmotionDispatcher(
	handle: (
		input: EmotionWorkerRequest,
		reply: (message: EmotionWorkerResponse) => void,
	) => Promise<void>,
	onIdle?: () => void,
) {
	const ports = new Set<EmotionDispatchPort>();
	let queue = Promise.resolve();
	let outstanding = 0;
	let idleTimer: ReturnType<typeof setTimeout> | undefined;
	let inactiveTimer: ReturnType<typeof setTimeout> | undefined;
	let closing = false;

	function close() {
		if (closing) return;
		closing = true;
		clearTimeout(idleTimer);
		clearTimeout(inactiveTimer);
		broadcast({ type: "idle" });
		for (const port of [...ports]) detach(port);
		onIdle?.();
	}

	function scheduleInactivity() {
		clearTimeout(inactiveTimer);
		if (!onIdle || closing || outstanding) return;
		// A closed tab may never send disconnect. Bound retention after the last
		// completed request even when its stale port remains in the set.
		inactiveTimer = setTimeout(close, 62_000);
	}

	function scheduleIdle() {
		if (!onIdle || closing || ports.size || idleTimer !== undefined) return;
		// Give a composer moving between views time to reconnect before closing.
		idleTimer = setTimeout(() => {
			idleTimer = undefined;
			if (!ports.size) close();
		}, 1_000);
	}

	function broadcast(message: EmotionWorkerResponse) {
		for (const port of [...ports]) deliver(port, message);
	}

	function deliver(port: EmotionDispatchPort, message: EmotionWorkerResponse) {
		if (!ports.has(port)) return;
		try {
			port.postMessage(message);
		} catch {
			detach(port);
		}
	}

	function detach(port: EmotionDispatchPort) {
		ports.delete(port);
		port.onmessage = null;
		port.close?.();
		scheduleIdle();
	}

	function attach(port: EmotionDispatchPort) {
		if (closing) {
			port.postMessage({ type: "idle" });
			port.close?.();
			return;
		}
		clearTimeout(idleTimer);
		idleTimer = undefined;
		ports.add(port);
		scheduleInactivity();
		port.onmessage = (event) => {
			const input = event.data;
			if ("type" in input) {
				detach(port);
				return;
			}
			outstanding++;
			clearTimeout(inactiveTimer);
			queue = queue.then(async () => {
				let deadline: ReturnType<typeof setTimeout> | undefined;
				try {
					if (!ports.has(port)) return;
					if (onIdle) {
						// Match the client's watchdogs so a vanished tab cannot leave a
						// stalled download or inference keeping the worker alive forever.
						deadline = setTimeout(
							() => {
								deliver(port, { type: "error", id: input.id });
								close();
							},
							input.warmup ? 10 * 60_000 : 30_000,
						);
					}
					await handle(input, (message) => deliver(port, message));
				} catch (error) {
					console.error("Emotion model failed", error);
					deliver(port, { type: "error", id: input.id });
				} finally {
					clearTimeout(deadline);
					outstanding--;
					scheduleIdle();
					scheduleInactivity();
				}
			});
		};
		port.start?.();
	}

	return { attach, broadcast };
}
