/**
 * Client side of incremental board fetching.
 *
 * Holds the last full board and manifest per (app, board, version) and turns every fetch into
 * "send my manifest, apply the diff". Transport is injected so the web app (`apiPost`) and the
 * desktop app (`fetcher`) share one implementation.
 */
import type { IBoard } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";
import { type CatalogByName, applyBoardSync, catalogByName } from "./apply";
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

function boardKey(
	appId: string,
	boardId: string,
	version?: [number, number, number],
): string {
	return `${appId}${boardId}${version ? version.join(".") : ""}`;
}

export class BoardSyncClient {
	private readonly held = new Map<string, HeldBoard>();
	private readonly inflight = new Map<string, Promise<IBoard>>();
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
		const marker = `${boardId}`;
		for (const key of [...this.held.keys()]) {
			const matches =
				appId === undefined
					? key.includes(marker)
					: key.startsWith(`${appId}${marker}`);
			if (matches) this.held.delete(key);
		}
	}

	/**
	 * Fetch the board, incrementally when possible. Concurrent calls for the same board share
	 * one round trip.
	 */
	sync(
		appId: string,
		boardId: string,
		version: [number, number, number] | undefined,
		transport: BoardSyncTransport,
	): Promise<IBoard> {
		const key = boardKey(appId, boardId, version);
		const pending = this.inflight.get(key);
		if (pending) return pending;
		const run = this.syncOnce(key, appId, transport).finally(() => {
			if (this.inflight.get(key) === run) this.inflight.delete(key);
		});
		this.inflight.set(key, run);
		return run;
	}

	private async syncOnce(
		key: string,
		appId: string,
		transport: BoardSyncTransport,
	): Promise<IBoard> {
		const held = this.held.get(key);
		const catalog = this.catalogs.get(appId);
		const request: IBoardSyncRequest = held
			? { ...held.manifest, hydrate: catalog !== undefined }
			: { hydrate: catalog !== undefined };

		let applied = applyBoardSync(
			held?.board,
			await transport(request),
			catalog,
		);

		if (applied.unhydratable.size > 0) {
			// The catalog we hold is behind the server's for these nodes. Drop those segments
			// from the manifest so the server resends them, and take them verbatim this time.
			const segments = { ...applied.manifest.segments };
			for (const segmentId of applied.unhydratable) delete segments[segmentId];
			const retry: IBoardSyncRequest = {
				...applied.manifest,
				segments,
				hydrate: false,
			};
			applied = applyBoardSync(
				applied.board,
				await transport(retry),
				undefined,
			);
		}

		this.held.set(key, { board: applied.board, manifest: applied.manifest });
		return applied.board;
	}
}
