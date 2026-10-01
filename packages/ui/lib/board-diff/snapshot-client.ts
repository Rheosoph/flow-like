import {
	type SnapshotEngine,
	type SnapshotRequest,
	type SnapshotResponse,
	createSnapshotEngine,
} from "./snapshot-engine";

interface Pending {
	resolve: (response: SnapshotResponse) => void;
	reject: (error: Error) => void;
}

let worker: Worker | null | undefined;
let fallback: SnapshotEngine | undefined;
let sequence = 0;
const pending = new Map<number, Pending>();

function inThread(): SnapshotEngine {
	fallback ??= createSnapshotEngine();
	return fallback;
}

function failAll(reason: string) {
	for (const request of pending.values()) request.reject(new Error(reason));
	pending.clear();
}

function snapshotWorker(): Worker | null {
	if (worker !== undefined) return worker;
	if (typeof Worker === "undefined") {
		worker = null;
		return worker;
	}
	try {
		worker = new Worker(new URL("./snapshot.worker.ts", import.meta.url), {
			type: "module",
		});
		worker.onmessage = (
			event: MessageEvent<{
				id: number;
				ok: boolean;
				response?: SnapshotResponse;
				error?: string;
			}>,
		) => {
			const { id, ok, response, error } = event.data;
			const request = pending.get(id);
			if (!request) return;
			pending.delete(id);
			if (ok && response) request.resolve(response);
			else request.reject(new Error(error ?? "Board snapshot worker failed"));
		};
		worker.onerror = (event) => {
			console.warn(
				"[board-last-seen] Snapshot worker crashed; continuing on the main thread.",
				event,
			);
			failAll("Board snapshot worker crashed");
			worker?.terminate();
			worker = null;
		};
	} catch (error) {
		console.warn(
			"[board-last-seen] Could not start the snapshot worker; continuing on the main thread.",
			error,
		);
		worker = null;
	}
	return worker;
}

/**
 * Sends a request to the snapshot worker. Without workers (SSR, tests, a crashed worker)
 * the same engine runs in this thread, so callers never need a second code path.
 */
export function snapshotRequest(
	request: SnapshotRequest,
): Promise<SnapshotResponse> {
	const active = snapshotWorker();
	if (!active) return inThread().handle(request);
	return new Promise<SnapshotResponse>((resolve, reject) => {
		const id = ++sequence;
		pending.set(id, { resolve, reject });
		try {
			active.postMessage({ id, request });
		} catch (error) {
			pending.delete(id);
			reject(error instanceof Error ? error : new Error(String(error)));
		}
	});
}
