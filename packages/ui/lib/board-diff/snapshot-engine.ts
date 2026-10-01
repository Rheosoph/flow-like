import type { IBoard } from "../schema/flow/board";
import { diffBoards } from "./diff";
import {
	type PackedSnapshot,
	fingerprint,
	packSnapshot,
	unpackSnapshot,
} from "./snapshot-codec";
import type { IBoardDiff } from "./types";

/**
 * The CPU half of the last-seen snapshots: decompress, parse, diff, fingerprint and
 * compress. Runs in a worker; the main thread only serializes (in slices) and stores.
 */
export type SnapshotRequest =
	| { type: "open"; key: string; stored?: PackedSnapshot; current: string }
	| { type: "compare"; key: string; current: string; withBase?: boolean }
	| { type: "pack"; key: string; current: string }
	/** The packed snapshot never reached storage; the next pack must write again. */
	| { type: "unmark"; key: string }
	| { type: "forget"; key: string };

export type SnapshotResponse =
	| {
			type: "open";
			first: boolean;
			/** Only when there is news: the diff and the stored board's JSON. */
			diff?: IBoardDiff;
			base?: string;
	  }
	| { type: "compare"; diff?: IBoardDiff; base?: string }
	| { type: "pack"; packed?: PackedSnapshot }
	| { type: "unmark" }
	| { type: "forget" };

interface Opened {
	board: IBoard;
	json: string;
}

export interface SnapshotEngine {
	handle(request: SnapshotRequest): Promise<SnapshotResponse>;
}

const withNews = (diff: IBoardDiff) => (diff.logicCount > 0 ? diff : undefined);

export function createSnapshotEngine(): SnapshotEngine {
	/** The snapshot read when the board was opened, kept for the catch-up and pending checks. */
	const opened = new Map<string, Opened>();
	/** Fingerprint of what is stored per key, so an unchanged board is never rewritten. */
	const stored = new Map<string, string>();

	type Of<T extends SnapshotRequest["type"]> = Extract<
		SnapshotRequest,
		{ type: T }
	>;

	async function open(request: Of<"open">): Promise<SnapshotResponse> {
		if (!request.stored) {
			opened.delete(request.key);
			stored.delete(request.key);
			return { type: "open", first: true };
		}
		const json = await unpackSnapshot(request.stored);
		const board = JSON.parse(json) as IBoard;
		opened.set(request.key, { board, json });
		stored.set(request.key, fingerprint(json));
		const diff = withNews(
			diffBoards(board, JSON.parse(request.current) as IBoard),
		);
		return diff
			? { type: "open", first: false, diff, base: json }
			: { type: "open", first: false };
	}

	function compare(request: Of<"compare">): SnapshotResponse {
		const base = opened.get(request.key);
		if (!base) return { type: "compare" };
		const diff = withNews(
			diffBoards(base.board, JSON.parse(request.current) as IBoard),
		);
		return {
			type: "compare",
			diff,
			base: diff && request.withBase ? base.json : undefined,
		};
	}

	async function pack(request: Of<"pack">): Promise<SnapshotResponse> {
		const print = fingerprint(request.current);
		if (stored.get(request.key) === print) return { type: "pack" };
		const packed = await packSnapshot(request.current);
		stored.set(request.key, print);
		return { type: "pack", packed };
	}

	async function handle(request: SnapshotRequest): Promise<SnapshotResponse> {
		if (request.type === "open") return open(request);
		if (request.type === "compare") return compare(request);
		if (request.type === "pack") return pack(request);
		stored.delete(request.key);
		if (request.type === "unmark") return { type: "unmark" };
		opened.delete(request.key);
		return { type: "forget" };
	}

	return { handle };
}
