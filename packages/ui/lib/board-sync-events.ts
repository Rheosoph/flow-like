/** Emitted whenever a board's offline sync queue changes: pushed, failed, or discarded. */
export const BOARD_SYNC_CHANGED_EVENT = "flow:board-sync-changed";

/** Asks the mounted recovery dialog to open. Toasts must not render a dialog themselves. */
export const BOARD_SYNC_RECOVERY_EVENT = "flow:open-board-sync-recovery";

export interface BoardSyncEventDetail {
	appId: string;
	boardId: string;
}

function dispatchBoardSyncEvent(
	eventName: string,
	detail: BoardSyncEventDetail,
): void {
	if (typeof window === "undefined") return;
	window.dispatchEvent(new CustomEvent(eventName, { detail }));
}

export function dispatchBoardSyncChanged(appId: string, boardId: string): void {
	dispatchBoardSyncEvent(BOARD_SYNC_CHANGED_EVENT, { appId, boardId });
}

export function dispatchBoardSyncRecoveryRequest(
	appId: string,
	boardId: string,
): void {
	dispatchBoardSyncEvent(BOARD_SYNC_RECOVERY_EVENT, { appId, boardId });
}

export function isBoardSyncEventFor(
	event: Event,
	appId: string,
	boardId: string,
): boolean {
	const detail = (event as CustomEvent<Partial<BoardSyncEventDetail>>).detail;
	return detail?.appId === appId && detail?.boardId === boardId;
}
