import type { IAppState } from "../../state/backend-state/app-state";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IApp, IBoard, IMetadata } from "../schema";
import type { IBoardSummary } from "../schema/flow/board-summary";
import type { StoredBoard, SuggestionStore } from "./store";
import type { BoardFacts } from "./types";

/** Apps synced per run, most recently changed first; the rest wait for the next run. */
export const MAX_APPS_PER_RUN = 60;
/** Full-board fetches per run; the rest wait for the next run. */
export const MAX_FETCHES_PER_RUN = 400;
const FETCH_CONCURRENCY = 2;

export interface SyncBackend {
	appState: Pick<IAppState, "getApps">;
	boardState: Pick<
		IBoardState,
		| "getBoardSummaries"
		| "getBoardSummariesSnapshot"
		| "getBoard"
		| "getBoardSnapshot"
	>;
}

export interface SyncSummary {
	/** Apps whose sync completed in this run. */
	apps: number;
	/** Boards downloaded and reduced to facts. */
	fetched: number;
	/** Boards whose stored facts were still current. */
	reused: number;
	/** Stored boards dropped because their board or app is gone. */
	removed: number;
	failed: number;
	/** Apps left for a later run by the per-run bounds. */
	skipped: number;
	/** Every app due today was synced: nothing is left for a later run today. */
	complete: boolean;
}

export interface SyncCorpusOptions {
	backend: SyncBackend;
	sub: string;
	/** Local calendar day, `YYYY-MM-DD` (see `localDay`). */
	today: string;
	store: SuggestionStore;
	extract: (board: IBoard) => Promise<BoardFacts>;
	signal?: AbortSignal;
	onProgress?: (summary: Readonly<SyncSummary>) => void;
}

export function localDay(date = new Date()): string {
	const month = String(date.getMonth() + 1).padStart(2, "0");
	const day = String(date.getDate()).padStart(2, "0");
	return `${date.getFullYear()}-${month}-${day}`;
}

type TimeLike =
	| { secs_since_epoch: number; nanos_since_epoch?: number }
	| string
	| number
	| null
	| undefined;

export function timeToMillis(time: TimeLike): number {
	if (time === null || time === undefined) return 0;
	if (typeof time === "number") return Number.isFinite(time) ? time : 0;
	if (typeof time === "string") {
		const parsed = Date.parse(time);
		return Number.isNaN(parsed) ? 0 : parsed;
	}
	if (typeof time.secs_since_epoch !== "number") return 0;
	return (
		time.secs_since_epoch * 1000 +
		Math.floor((time.nanos_since_epoch ?? 0) / 1_000_000)
	);
}

const appRecency = ([app, meta]: [IApp, IMetadata | undefined]) =>
	Math.max(timeToMillis(app.updated_at), timeToMillis(meta?.updated_at));

function yieldToIdle(): Promise<void> {
	return new Promise((resolve) => {
		if (typeof requestIdleCallback === "function") {
			requestIdleCallback(() => resolve(), { timeout: 2_000 });
		} else {
			setTimeout(resolve, 0);
		}
	});
}

/** A board is fetched when it is new or its `updatedAt` moved; without `updatedAt` only when new. */
function needsFetch(
	summary: IBoardSummary,
	stored: StoredBoard | undefined,
): boolean {
	if (!stored) return true;
	const updatedAt = timeToMillis(summary.updatedAt);
	return updatedAt > 0 && updatedAt !== stored.updatedAt;
}

/**
 * Brings the facts cache up to date with the user's apps. Each app is synced at most once per
 * calendar day; within an app only boards that are new or changed since their facts were stored are
 * downloaded (one full board each, reduced to facts and discarded). Everything else is answered by
 * the DB-cached summary listing. Errors are isolated per app and per board.
 */
export async function syncCorpus({
	backend,
	sub,
	today,
	store,
	extract,
	signal,
	onProgress,
}: SyncCorpusOptions): Promise<SyncSummary> {
	const summary: SyncSummary = {
		apps: 0,
		fetched: 0,
		reused: 0,
		removed: 0,
		failed: 0,
		skipped: 0,
		complete: false,
	};
	const report = () => onProgress?.({ ...summary });

	let listed: [IApp, IMetadata | undefined][];
	try {
		listed = await backend.appState.getApps();
		if (!Array.isArray(listed)) {
			throw new Error(`app listing returned ${typeof listed}, not a list`);
		}
	} catch (error) {
		console.warn("[node-suggestions] listing apps failed:", error);
		summary.failed++;
		return summary;
	}
	if (signal?.aborted) return summary;

	const syncedDays = await store.listApps(sub);
	const stored = await store.listBoards(sub);
	const listedIds = new Set(listed.map(([app]) => app.id));
	// An empty listing is more likely a backend that is not ready than a user without apps;
	// forgetting the whole corpus on it would re-download every board.
	if (listedIds.size > 0) {
		const goneApps = new Set(
			[...syncedDays.keys(), ...stored.map((board) => board.appId)].filter(
				(appId) => !listedIds.has(appId),
			),
		);
		if (goneApps.size > 0) {
			summary.removed += stored.filter((board) =>
				goneApps.has(board.appId),
			).length;
			await store.deleteApps(sub, [...goneApps]);
		}
	}

	const due = listed
		.filter(([app]) => syncedDays.get(app.id) !== today)
		.sort((left, right) => appRecency(right) - appRecency(left));
	const runnable = due.slice(0, MAX_APPS_PER_RUN);
	summary.skipped = due.length - runnable.length;
	let budget = MAX_FETCHES_PER_RUN;
	let appFailures = 0;

	for (const [index, [app]] of runnable.entries()) {
		if (signal?.aborted) break;
		if (budget <= 0) {
			summary.skipped += runnable.length - index;
			break;
		}
		try {
			const result = await syncApp(app.id, budget);
			budget -= result.attempted;
			if (result.done) {
				await store.markAppSynced(sub, app.id, today);
				summary.apps++;
			} else if (!signal?.aborted) {
				summary.skipped++;
			}
		} catch (error) {
			console.warn(`[node-suggestions] syncing app ${app.id} failed:`, error);
			summary.failed++;
			appFailures++;
		}
		report();
	}

	summary.complete =
		!signal?.aborted && summary.skipped === 0 && appFailures === 0;
	report();
	return summary;

	async function syncApp(
		appId: string,
		limit: number,
	): Promise<{ attempted: number; done: boolean }> {
		const { boardState } = backend;
		const summaries = boardState.getBoardSummariesSnapshot
			? await boardState.getBoardSummariesSnapshot(appId)
			: await boardState.getBoardSummaries(appId);
		const known = new Map(
			(await store.listBoards(sub, appId)).map((board) => [
				board.boardId,
				board,
			]),
		);
		const listedBoards = new Set(summaries.map((board) => board.id));
		const gone = [...known.values()].filter(
			(board) => !listedBoards.has(board.boardId),
		);
		if (gone.length > 0) {
			await store.deleteBoards(sub, gone);
			summary.removed += gone.length;
		}

		const changed = summaries.filter((board) =>
			needsFetch(board, known.get(board.id)),
		);
		summary.reused += summaries.length - changed.length;
		const queue = changed
			.sort(
				(left, right) =>
					timeToMillis(right.updatedAt) - timeToMillis(left.updatedAt),
			)
			.slice(0, limit);

		let next = 0;
		const worker = async () => {
			while (next < queue.length && !signal?.aborted) {
				const board = queue[next++];
				await fetchBoard(appId, board);
				report();
				await yieldToIdle();
			}
		};
		await Promise.all(
			Array.from({ length: Math.min(FETCH_CONCURRENCY, queue.length) }, worker),
		);
		return {
			attempted: next,
			done: !signal?.aborted && queue.length === changed.length,
		};
	}

	async function fetchBoard(
		appId: string,
		board: IBoardSummary,
	): Promise<void> {
		try {
			const { boardState } = backend;
			const full = boardState.getBoardSnapshot
				? await boardState.getBoardSnapshot(appId, board.id)
				: await boardState.getBoard(appId, board.id);
			const facts = await extract(full);
			await store.putBoard(
				sub,
				{
					appId,
					boardId: board.id,
					updatedAt: timeToMillis(board.updatedAt),
					transitions: facts.transitions.length,
				},
				JSON.stringify(facts),
			);
			summary.fetched++;
		} catch (error) {
			console.warn(
				`[node-suggestions] learning board ${board.id} of app ${appId} failed:`,
				error,
			);
			summary.failed++;
		}
	}
}
