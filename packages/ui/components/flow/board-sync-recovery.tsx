"use client";

import { CloudOffIcon, DownloadIcon, RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import {
	BOARD_SYNC_CHANGED_EVENT,
	BOARD_SYNC_RECOVERY_EVENT,
	isBoardSyncEventFor,
} from "../../lib/board-sync-events";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type {
	IBoardSyncQueueEntry,
	IBoardSyncStatus,
} from "../../state/backend-state/board-state";
import {
	AlertDialog,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";

const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

/** Keeps the hook order stable on backends that have no offline queue. */
const unsupportedBoardSyncStatus = async (
	_appId: string,
	_boardId: string,
): Promise<IBoardSyncStatus | undefined> => undefined;

export function useBoardSyncStatus(appId: string, boardId: string) {
	const backend = useBackend();
	const supported = Boolean(backend.boardState.getBoardSyncStatus);
	const query = useInvoke<IBoardSyncStatus | undefined, [string, string]>(
		backend.boardState.getBoardSyncStatus ?? unsupportedBoardSyncStatus,
		backend.boardState,
		[appId, boardId],
		supported && appId.length > 0 && boardId.length > 0,
	);

	const { refetch } = query;
	useEffect(() => {
		if (!supported) return;
		const handler = (event: Event) => {
			if (!isBoardSyncEventFor(event, appId, boardId)) return;
			void refetch();
		};
		window.addEventListener(BOARD_SYNC_CHANGED_EVENT, handler);
		window.addEventListener(REMOTE_BOARD_APPLIED_EVENT, handler);
		return () => {
			window.removeEventListener(BOARD_SYNC_CHANGED_EVENT, handler);
			window.removeEventListener(REMOTE_BOARD_APPLIED_EVENT, handler);
		};
	}, [appId, boardId, refetch, supported]);

	return query.data;
}

export function BoardSyncStatusPill({
	appId,
	boardId,
	onOpenRecovery,
}: Readonly<{
	appId: string;
	boardId: string;
	onOpenRecovery: () => void;
}>) {
	const status = useBoardSyncStatus(appId, boardId);
	if (!status?.supported || status.pendingBatches === 0) return null;

	return (
		<button
			type="button"
			onClick={onOpenRecovery}
			title={
				status.ownershipMismatch ??
				"These edits have not reached the server yet."
			}
			className="flex items-center gap-2 rounded-xl border border-[color-mix(in_oklch,var(--destructive)_35%,transparent)] bg-[color-mix(in_oklch,var(--background)_92%,transparent)] px-3 py-1.5 shadow-sm hover:bg-[color-mix(in_oklch,var(--background)_85%,transparent)] transition-colors cursor-pointer"
		>
			<CloudOffIcon className="h-3.5 w-3.5 text-destructive" />
			<span className="text-xs font-medium text-destructive">
				{status.pendingBatches} edit
				{status.pendingBatches === 1 ? "" : "s"} not synced
			</span>
		</button>
	);
}

function QueueEntryRow({ entry }: Readonly<{ entry: IBoardSyncQueueEntry }>) {
	const reason =
		entry.blockedReason ??
		entry.ownershipMismatch ??
		entry.lastFailureMessage ??
		"Not yet accepted by the server.";
	return (
		<li className="min-w-0 rounded-md border border-border/60 bg-muted/30 px-3 py-2">
			<div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
				<span className="text-xs font-medium">
					{entry.commandCount} command{entry.commandCount === 1 ? "" : "s"}
				</span>
				<span className="font-mono text-[11px] tabular-nums text-muted-foreground">
					{new Date(entry.createdAt).toLocaleString()}
				</span>
			</div>
			{/* Server errors carry unbreakable ids; without this they widen the grid track. */}
			<p className="mt-1 text-[11px] leading-snug text-muted-foreground wrap-anywhere">
				{reason}
			</p>
			{entry.partiallyDelivered ? (
				<p className="mt-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
					Partially delivered — the server already holds part of this batch.
				</p>
			) : null}
		</li>
	);
}

export function BoardSyncRecoveryDialog({
	appId,
	boardId,
	open,
	onOpenChange,
}: Readonly<{
	appId: string;
	boardId: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const backend = useBackend();
	const status = useBoardSyncStatus(appId, boardId);
	const [acknowledged, setAcknowledged] = useState(false);
	const [busy, setBusy] = useState<"retry" | "reset" | "export" | null>(null);

	useEffect(() => {
		if (!open) setAcknowledged(false);
	}, [open]);

	const pendingBatches = status?.pendingBatches ?? 0;
	const entries = useMemo(() => status?.entries ?? [], [status?.entries]);

	const handleRetry = useCallback(async () => {
		if (!backend.boardState.retryOfflineSync) return;
		setBusy("retry");
		try {
			const { remainingBatches } = await backend.boardState.retryOfflineSync(
				appId,
				boardId,
			);
			if (remainingBatches === 0) onOpenChange(false);
		} finally {
			setBusy(null);
		}
	}, [appId, backend.boardState, boardId, onOpenChange]);

	const handleExport = useCallback(async () => {
		if (!backend.boardState.exportBoardSyncArchive) return;
		setBusy("export");
		try {
			const archive = await backend.boardState.exportBoardSyncArchive(
				appId,
				boardId,
			);
			if (archive.length === 0) {
				toast.info("No previously discarded edits are stored for this board.");
				return;
			}
			const url = URL.createObjectURL(
				new Blob([JSON.stringify(archive, null, 2)], {
					type: "application/json",
				}),
			);
			const link = document.createElement("a");
			link.href = url;
			link.download = `board-${boardId}-discarded-edits.json`;
			link.click();
			URL.revokeObjectURL(url);
		} catch (error) {
			toast.error("Could not export the discarded edits.", {
				description: error instanceof Error ? error.message : String(error),
			});
		} finally {
			setBusy(null);
		}
	}, [appId, backend.boardState, boardId]);

	const handleReset = useCallback(async () => {
		if (!backend.boardState.resetBoardFromServer) return;
		setBusy("reset");
		try {
			const result = await backend.boardState.resetBoardFromServer(
				appId,
				boardId,
				{ discardQueuedEdits: pendingBatches > 0 },
			);
			toast.success(
				result.discardedBatches > 0
					? `Board replaced with the server copy — ${result.discardedBatches} unsent edit ${
							result.discardedBatches === 1 ? "batch" : "batches"
						} discarded.`
					: "Board is back in sync with the server.",
			);
			onOpenChange(false);
		} catch (error) {
			toast.error("Could not fetch the board from the server.", {
				description: error instanceof Error ? error.message : String(error),
			});
		} finally {
			setBusy(null);
		}
	}, [appId, backend.boardState, boardId, onOpenChange, pendingBatches]);

	const destructive = pendingBatches > 0;

	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			{/* The content is a grid: every child needs min-w-0, or one long server error or a
			    row of nowrap buttons widens the single track and pushes all of them past the border. */}
			<AlertDialogContent className="grid-cols-[minmax(0,1fr)] sm:max-w-xl">
				<AlertDialogHeader className="min-w-0">
					<AlertDialogTitle>Fetch this board from the server</AlertDialogTitle>
					<AlertDialogDescription>
						{destructive
							? "The server copy will replace what is on this device. Edits that never reached the server are discarded — they will not appear anywhere afterwards."
							: "Nothing is queued for this board. Fetching replaces the local copy with the server's."}
					</AlertDialogDescription>
				</AlertDialogHeader>

				{status?.ownershipMismatch ? (
					<p className="min-w-0 rounded-md border border-border/60 bg-muted/40 px-3 py-2 text-xs leading-snug text-muted-foreground wrap-anywhere">
						{status.ownershipMismatch}. Signing back into the original account
						and Hub would let these edits sync instead of being discarded.
					</p>
				) : null}

				{destructive ? (
					<div className="flex min-w-0 flex-col gap-2">
						<ul className="flex max-h-48 min-w-0 flex-col gap-1.5 overflow-y-auto">
							{entries.map((entry) => (
								<QueueEntryRow key={entry.commandId} entry={entry} />
							))}
						</ul>
						<div className="flex items-start gap-2 text-xs leading-snug">
							<Checkbox
								id="board-sync-discard-ack"
								checked={acknowledged}
								onCheckedChange={(value) => setAcknowledged(value === true)}
								className="mt-0.5"
							/>
							<label htmlFor="board-sync-discard-ack">
								I understand {pendingBatches} unsent edit{" "}
								{pendingBatches === 1 ? "batch" : "batches"} made on this device
								will be removed from this board.
							</label>
						</div>
					</div>
				) : null}

				<AlertDialogFooter className="min-w-0 flex-wrap sm:flex-wrap sm:justify-between">
					<div className="flex min-w-0 flex-wrap gap-2">
						<Button
							variant="ghost"
							size="sm"
							disabled={busy !== null || !backend.boardState.retryOfflineSync}
							onClick={() => void handleRetry()}
						>
							<RefreshCwIcon
								className={cn("size-3.5", busy === "retry" && "animate-spin")}
							/>
							Retry sync
						</Button>
						<Button
							variant="ghost"
							size="sm"
							disabled={
								busy !== null || !backend.boardState.exportBoardSyncArchive
							}
							onClick={() => void handleExport()}
						>
							<DownloadIcon className="size-3.5" />
							Export
						</Button>
					</div>
					<div className="flex min-w-0 flex-wrap gap-2">
						<Button
							variant="outline"
							size="sm"
							disabled={busy !== null}
							onClick={() => onOpenChange(false)}
						>
							Cancel
						</Button>
						<Button
							variant={destructive ? "destructive" : "default"}
							size="sm"
							disabled={busy !== null || (destructive && !acknowledged)}
							onClick={() => void handleReset()}
						>
							{destructive ? "Discard edits & fetch" : "Fetch from server"}
						</Button>
					</div>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

/** Opens the recovery dialog when a toast or another surface asks for it. */
export function useBoardSyncRecoveryRequests(
	appId: string,
	boardId: string,
	onRequest: () => void,
) {
	useEffect(() => {
		const handler = (event: Event) => {
			if (!isBoardSyncEventFor(event, appId, boardId)) return;
			onRequest();
		};
		window.addEventListener(BOARD_SYNC_RECOVERY_EVENT, handler);
		return () => window.removeEventListener(BOARD_SYNC_RECOVERY_EVENT, handler);
	}, [appId, boardId, onRequest]);
}
