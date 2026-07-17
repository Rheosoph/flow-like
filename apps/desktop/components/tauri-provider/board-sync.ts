import type { IBoard } from "@flow-like/flow-like-ui";
import { type MergeBoardResult, mergeBoardWithLocal } from "./board-merge";

interface PendingRequest {
	resolve: (result: MergeBoardResult) => void;
	reject: (error: Error) => void;
}

let worker: Worker | null | undefined;
let requestSeq = 0;
const pending = new Map<number, PendingRequest>();

function failAllPending(reason: string) {
	for (const request of pending.values()) {
		request.reject(new Error(reason));
	}
	pending.clear();
}

function getWorker(): Worker | null {
	if (worker !== undefined) return worker;
	if (typeof Worker === "undefined") {
		worker = null;
		return worker;
	}

	try {
		worker = new Worker(new URL("./board-sync-worker.ts", import.meta.url), {
			type: "module",
		});
		worker.onmessage = (
			event: MessageEvent<{
				id: number;
				ok: boolean;
				result?: MergeBoardResult;
				error?: string;
			}>,
		) => {
			const { id, ok, result, error } = event.data;
			const request = pending.get(id);
			if (!request) return;
			pending.delete(id);
			if (ok && result) {
				request.resolve(result);
			} else {
				request.reject(new Error(error ?? "Board sync worker failed"));
			}
		};
		worker.onerror = (event) => {
			console.warn("[board-sync] Worker crashed, falling back inline:", event);
			failAllPending("Board sync worker crashed");
			worker?.terminate();
			worker = null;
		};
	} catch (error) {
		console.warn("[board-sync] Failed to start worker:", error);
		worker = null;
	}

	return worker;
}

/**
 * Merges a remote board into the local one off the main thread. The merge walks
 * every node with structuredClone + deep equality, which blocks the UI for large
 * boards when run inline — inline execution remains only as a fallback.
 */
export async function mergeBoardOffThread(
	remoteBoard: IBoard,
	localBoard?: IBoard,
): Promise<MergeBoardResult> {
	const activeWorker = getWorker();
	if (!activeWorker) {
		return mergeBoardWithLocal(remoteBoard, localBoard);
	}

	try {
		return await new Promise<MergeBoardResult>((resolve, reject) => {
			const id = ++requestSeq;
			pending.set(id, { resolve, reject });
			activeWorker.postMessage({ id, remote: remoteBoard, local: localBoard });
		});
	} catch (error) {
		console.warn("[board-sync] Worker merge failed, falling back:", error);
		return mergeBoardWithLocal(remoteBoard, localBoard);
	}
}
