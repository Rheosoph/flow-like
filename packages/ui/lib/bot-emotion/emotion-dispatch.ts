import type { EmotionWorkerRequest, EmotionWorkerResponse } from "./protocol";

export interface EmotionDispatchPort {
	postMessage(message: EmotionWorkerResponse): void;
	start?(): void;
	onmessage: ((event: MessageEvent<EmotionWorkerRequest>) => void) | null;
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
		try {
			port.postMessage(message);
		} catch {
			ports.delete(port);
		}
	}

	function attach(port: EmotionDispatchPort) {
		ports.add(port);
		port.start?.();
		port.onmessage = (event) => {
			const input = event.data;
			queue = queue.then(async () => {
				try {
					await handle(input, (message) => deliver(port, message));
				} catch (error) {
					console.error("Emotion model failed", error);
					deliver(port, { type: "error", id: input.id });
				}
			});
		};
	}

	return { attach, broadcast };
}
