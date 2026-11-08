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
) {
	const ports = new Set<EmotionDispatchPort>();
	let queue = Promise.resolve();

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
	}

	function attach(port: EmotionDispatchPort) {
		ports.add(port);
		port.onmessage = (event) => {
			const input = event.data;
			if ("type" in input) {
				detach(port);
				return;
			}
			queue = queue.then(async () => {
				if (!ports.has(port)) return;
				try {
					await handle(input, (message) => deliver(port, message));
				} catch (error) {
					console.error("Emotion model failed", error);
					deliver(port, { type: "error", id: input.id });
				}
			});
		};
		port.start?.();
	}

	return { attach, broadcast };
}
