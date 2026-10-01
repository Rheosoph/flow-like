import type { IBoard } from "../schema/flow/board";

/** Maps whose entries are serialized one at a time so no slice holds the thread long. */
const SLICED_MAPS = new Set(["nodes", "layers", "comments", "variables"]);
const SLICE_BUDGET_MS = 4;

/** Hands the thread back to the browser so it can paint and handle input. */
export function yieldToMain(): Promise<void> {
	const scheduler = (
		globalThis as { scheduler?: { yield?: () => Promise<void> } }
	).scheduler;
	if (scheduler?.yield) return scheduler.yield();
	if (typeof MessageChannel === "undefined") {
		return new Promise((resolve) => setTimeout(resolve, 0));
	}
	// Unlike `setTimeout(0)`, a message is not clamped to 4 ms after a few nested calls.
	return new Promise((resolve) => {
		const channel = new MessageChannel();
		channel.port1.onmessage = () => {
			channel.port1.close();
			resolve();
		};
		channel.port2.postMessage(null);
	});
}

/**
 * `JSON.stringify(board)`, built entry by entry and yielding whenever a slice runs past
 * its budget. A board with thousands of nodes serializes in many short tasks instead of
 * one long one, so the canvas keeps painting while it happens.
 */
export async function serializeBoard(
	board: IBoard,
	budgetMs = SLICE_BUDGET_MS,
): Promise<string> {
	const parts: string[] = [];
	let sliceStart = performance.now();
	const pace = async () => {
		if (performance.now() - sliceStart < budgetMs) return;
		await yieldToMain();
		sliceStart = performance.now();
	};
	await writeObject(parts, board, pace, true);
	return parts.join("");
}

const isSlicedMap = (key: string, value: unknown) =>
	SLICED_MAPS.has(key) &&
	value !== null &&
	typeof value === "object" &&
	!Array.isArray(value);

async function writeObject(
	parts: string[],
	value: object,
	pace: () => Promise<void>,
	top: boolean,
) {
	parts.push("{");
	let first = true;
	for (const [key, entry] of Object.entries(value)) {
		const nested = top && isSlicedMap(key, entry);
		const json = nested ? "" : JSON.stringify(entry);
		if (json === undefined) continue;
		parts.push(first ? "" : ",", JSON.stringify(key), ":");
		first = false;
		if (nested) {
			await writeObject(parts, entry as object, pace, false);
			continue;
		}
		parts.push(json);
		await pace();
	}
	parts.push("}");
}
