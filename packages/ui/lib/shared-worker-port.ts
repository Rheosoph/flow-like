/** The dedicated-worker surface a model client talks to, whichever kind of worker is behind it. */
export interface WorkerPort<Request, Response> {
	postMessage(message: Request): void;
	terminate(): void;
	onmessage: ((event: MessageEvent<Response>) => void) | null;
	onerror: ((event: ErrorEvent) => void) | null;
}

/**
 * A SharedWorker's port behind the dedicated-worker surface, so every tab uses one worker.
 * Terminating it only closes this tab's port (after a `disconnect` message) and leaves the shared
 * worker running for the others. Requests sent before the worker reports `ready` are replayed to
 * `createFallback`'s worker when the shared module fails to load.
 */
export function portFromSharedWorker<
	Request,
	Response extends { type: string },
>(
	worker: SharedWorker,
	createFallback?: () => WorkerPort<Request, Response>,
): WorkerPort<Request, Response> {
	const port = worker.port;
	let fallback: WorkerPort<Request, Response> | undefined;
	let ready = false;
	let terminated = false;
	const pending: Request[] = [];

	function closeSharedPort() {
		worker.onerror = null;
		port.onmessage = null;
		try {
			port.postMessage({ type: "disconnect" });
		} catch {
			// A failed connection may already have detached its port.
		} finally {
			port.close();
		}
	}

	const adapter: WorkerPort<Request, Response> = {
		onmessage: null,
		onerror: null,
		postMessage(message) {
			if (terminated) return;
			if (fallback) {
				fallback.postMessage(message);
				return;
			}
			if (!ready) pending.push(message);
			port.postMessage(message);
		},
		terminate() {
			if (terminated) return;
			terminated = true;
			pending.length = 0;
			if (fallback) fallback.terminate();
			else closeSharedPort();
		},
	};
	port.onmessage = (event: MessageEvent<Response>) => {
		if (terminated || fallback) return;
		if (event.data.type === "ready") {
			ready = true;
			pending.length = 0;
		}
		adapter.onmessage?.(event);
	};
	worker.onerror = (event: ErrorEvent) => {
		if (terminated) return;
		if (!createFallback || ready || fallback) {
			adapter.onerror?.(event);
			return;
		}
		// Module loading failures are reported after the constructor returns.
		try {
			closeSharedPort();
			fallback = createFallback();
			fallback.onmessage = (message) => {
				if (!terminated) adapter.onmessage?.(message);
			};
			fallback.onerror = (error) => {
				if (!terminated) adapter.onerror?.(error);
			};
			for (const request of pending.splice(0)) fallback.postMessage(request);
		} catch {
			adapter.onerror?.(event);
		}
	};
	port.start();
	return adapter;
}
