/**
 * Client side of incremental board fetching.
 *
 * Holds the last full board and manifest per (app, board, version) and turns every fetch into
 * "send my manifest, apply the diff". Transport is injected so the web app (`apiPost`) and the
 * desktop app (`fetcher`) share one implementation.
 *
 * One rule governs every path that touches the held board: **a diff is applied only onto the
 * exact base it was computed against.** A response answers the request it was given; if the held
 * revision moved in between (a merged apply landed while a plain sync was in flight, or two
 * applies overlapped), the late response is discarded and the caller re-syncs. That is what makes
 * out-of-order arrivals harmless — they cost one extra round trip, never a rollback.
 */
import type { IBoard } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";
import {
	type CatalogByName,
	type IAppliedBoardSync,
	applyBoardSync,
	catalogByName,
} from "./apply";
import type {
	IBoardSyncManifest,
	IBoardSyncRequest,
	IBoardSyncResponse,
} from "./types";

export type BoardSyncTransport = (
	request: IBoardSyncRequest,
) => Promise<IBoardSyncResponse>;

interface HeldBoard {
	board: IBoard;
	manifest: IBoardSyncManifest;
}

/** Bound on "the held revision moved while my response was in flight" retries. */
const MAX_STALE_BASE_RETRIES = 3;

function boardKey(
	appId: string,
	boardId: string,
	version?: [number, number, number],
): string {
	return `${appId}${boardId}${version ? version.join(".") : ""}`;
}

function sameTokens(
	a: Record<string, string> | undefined,
	b: Record<string, string> | undefined,
): boolean {
	const left = a ?? {};
	const right = b ?? {};
	const keys = Object.keys(left);
	if (keys.length !== Object.keys(right).length) return false;
	return keys.every((key) => left[key] === right[key]);
}

/** Whether `request` was built from exactly `manifest` (hydrate/patch flags aside). */
export function requestMatchesManifest(
	request: IBoardSyncRequest,
	manifest: IBoardSyncManifest,
): boolean {
	return (
		request.meta === manifest.meta &&
		request.variables === manifest.variables &&
		request.comments === manifest.comments &&
		sameTokens(request.layers, manifest.layers) &&
		sameTokens(request.segments, manifest.segments)
	);
}

export class BoardSyncClient {
	private readonly held = new Map<string, HeldBoard>();
	private readonly inflight = new Map<string, Promise<IBoard>>();
	/**
	 * One follow-up per key for calls that arrive while a fetch is in flight. Sharing the in-flight
	 * promise would hand them a response to a request sent *before* they asked — after a merged
	 * apply that is exactly the stale answer that would hide the apply's own changes.
	 */
	private readonly queued = new Map<string, Promise<IBoard>>();
	private readonly catalogs = new Map<string, CatalogByName>();

	/**
	 * Make this app's catalog available for hydration. Hydration is opportunistic: without a
	 * catalog every node ships in full, which is always correct, just larger.
	 */
	setCatalog(appId: string, nodes: readonly INode[]): void {
		this.catalogs.set(appId, catalogByName(nodes));
	}

	/** The last board this client assembled, without touching the network. */
	peek(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): IBoard | undefined {
		return this.held.get(boardKey(appId, boardId, version))?.board;
	}

	/** Drop every held revision of a board. Without `appId`, matches the board id alone. */
	forget(appId: string | undefined, boardId: string): void {
		const marker = `${boardId}`;
		for (const key of [...this.held.keys()]) {
			const matches =
				appId === undefined
					? key.includes(marker)
					: key.startsWith(`${appId}${marker}`);
			if (matches) this.held.delete(key);
		}
	}

	/**
	 * The request this client would send for the board right now — what a merged apply carries
	 * so the server can answer with the diff against the revision the write produces. `undefined`
	 * when nothing is held yet (a first load must be a full sync anyway).
	 */
	syncRequest(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): IBoardSyncRequest | undefined {
		const held = this.held.get(boardKey(appId, boardId, version));
		if (!held) return undefined;
		return this.requestFor(held, appId);
	}

	/**
	 * Apply a diff obtained out of band (the sync tail of a merged apply). `base` is the request
	 * the server diffed against — the value `syncRequest` returned when the apply was sent.
	 *
	 * Applied only if the held revision is still exactly `base`; otherwise the diff is discarded
	 * and `undefined` is returned so the caller falls back to a plain sync. Also refused (and
	 * `undefined`) when the response needs a follow-up the caller cannot make here — nodes the
	 * catalog could not hydrate or patches onto an unknown base — since a plain sync heals both.
	 */
	ingest(
		appId: string,
		boardId: string,
		version: [number, number, number] | undefined,
		base: IBoardSyncRequest,
		response: IBoardSyncResponse,
	): IBoard | undefined {
		const key = boardKey(appId, boardId, version);
		const held = this.held.get(key);
		if (!held || !requestMatchesManifest(base, held.manifest)) return undefined;
		const applied = applyBoardSync(
			held.board,
			response,
			base.hydrate ? this.catalogs.get(appId) : undefined,
			base,
		);
		if (applied.unhydratable.size > 0 || applied.unpatchable.size > 0) {
			return undefined;
		}
		this.held.set(key, { board: applied.board, manifest: applied.manifest });
		return applied.board;
	}

	/**
	 * Fetch the board, incrementally when possible. Every call is answered by a request sent no
	 * earlier than the call itself: concurrent calls coalesce into at most one in-flight fetch plus
	 * one queued follow-up, and callers arriving mid-flight share the follow-up.
	 */
	sync(
		appId: string,
		boardId: string,
		version: [number, number, number] | undefined,
		transport: BoardSyncTransport,
	): Promise<IBoard> {
		const key = boardKey(appId, boardId, version);
		const pending = this.inflight.get(key);
		if (!pending) return this.start(key, appId, transport);
		const queued = this.queued.get(key);
		if (queued) return queued;
		const followup = pending
			.then(
				() => undefined,
				() => undefined,
			)
			.then(() => {
				if (this.queued.get(key) === followup) this.queued.delete(key);
				return this.start(key, appId, transport);
			});
		this.queued.set(key, followup);
		return followup;
	}

	private start(
		key: string,
		appId: string,
		transport: BoardSyncTransport,
	): Promise<IBoard> {
		const run = this.syncOnce(key, appId, transport).finally(() => {
			if (this.inflight.get(key) === run) this.inflight.delete(key);
		});
		this.inflight.set(key, run);
		return run;
	}

	private requestFor(held: HeldBoard, appId: string): IBoardSyncRequest {
		return {
			...held.manifest,
			hydrate: this.catalogs.has(appId),
			patch: true,
		};
	}

	private async syncOnce(
		key: string,
		appId: string,
		transport: BoardSyncTransport,
	): Promise<IBoard> {
		for (let attempt = 0; ; attempt++) {
			const held = this.held.get(key);
			const request: IBoardSyncRequest = held
				? this.requestFor(held, appId)
				: { hydrate: this.catalogs.has(appId), patch: true };
			const response = await transport(request);
			// The held revision moved while this response was in flight (an ingest landed). The
			// response answers a base we no longer hold; ask again from the new one.
			if (this.held.get(key) !== held) {
				if (attempt < MAX_STALE_BASE_RETRIES) continue;
				throw new Error(
					"Board sync could not converge: the held revision kept moving during the fetch.",
				);
			}
			const applied = await this.settle(
				key,
				appId,
				held,
				request,
				response,
				transport,
			);
			if (!applied) {
				if (attempt < MAX_STALE_BASE_RETRIES) continue;
				throw new Error(
					"Board sync could not converge: the held revision kept moving during the fetch.",
				);
			}
			this.held.set(key, {
				board: applied.board,
				manifest: applied.manifest,
			});
			return applied.board;
		}
	}

	/**
	 * Apply `response`, then re-request whatever it could not deliver in one go: segments the
	 * catalog could not hydrate (taken verbatim, no hydration) and patches onto a base this client
	 * turned out not to hold (taken whole). Both retries drop the segment's token so the server
	 * ships it complete. `undefined` when the held revision moved during the retry — the caller
	 * starts over from the new one.
	 */
	private async settle(
		key: string,
		appId: string,
		held: HeldBoard | undefined,
		request: IBoardSyncRequest,
		response: IBoardSyncResponse,
		transport: BoardSyncTransport,
	): Promise<IAppliedBoardSync | undefined> {
		const catalog = request.hydrate ? this.catalogs.get(appId) : undefined;
		const applied = applyBoardSync(held?.board, response, catalog, request);
		const incomplete = new Set([
			...applied.unhydratable,
			...applied.unpatchable,
		]);
		if (incomplete.size === 0) return applied;

		const segments = { ...applied.manifest.segments };
		for (const segmentId of incomplete) delete segments[segmentId];
		const retry: IBoardSyncRequest = {
			...applied.manifest,
			segments,
			hydrate: false,
			patch: true,
		};
		// Commit the partial revision before the round trip: an `ingest` landing meanwhile is then
		// refused (its base is the older manifest) and falls back to a queued sync, instead of
		// being silently overwritten by the retry's result below.
		const partial: HeldBoard = {
			board: applied.board,
			manifest: applied.manifest,
		};
		this.held.set(key, partial);
		const retried = await transport(retry);
		if (this.held.get(key) !== partial) return undefined;
		return applyBoardSync(applied.board, retried, undefined, retry);
	}
}
