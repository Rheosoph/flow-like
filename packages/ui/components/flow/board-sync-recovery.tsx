"use client";

import { useTranslation } from "@flow-like/locales";
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
import { BoardStatusItem } from "./shell/board-status-bar";

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
	const { t } = useTranslation("flow");
	const status = useBoardSyncStatus(appId, boardId);
	if (!status?.supported || status.pendingBatches === 0) return null;

	return (
		<BoardStatusItem
			icon={<CloudOffIcon />}
			tone="danger"
			className="cursor-pointer font-medium"
			onClick={onOpenRecovery}
			title={
				status.ownershipMismatch ??
				t(
					"theseEditsHaveNotReachedTheServerYet",
					"These edits have not reached the server yet.",
				)
			}
		>
			{t("countEditSNotSynced", {
				defaultValue_one: "{{count}} edit not synced",
				defaultValue_other: "{{count}} edits not synced",
				count: status.pendingBatches,
			})}
		</BoardStatusItem>
	);
}

function QueueEntryRow({ entry }: Readonly<{ entry: IBoardSyncQueueEntry }>) {
	const { t } = useTranslation("flow");
	const reason =
		entry.blockedReason ??
		entry.ownershipMismatch ??
		entry.lastFailureMessage ??
		t("notYetAcceptedByTheServer", "Not yet accepted by the server.");
	return (
		<li className="min-w-0 rounded-md border border-border/60 bg-muted/30 px-3 py-2">
			<div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
				<span className="text-xs font-medium">
					{t("countCommands", {
						defaultValue_one: "{{count}} command",
						defaultValue_other: "{{count}} commands",
						count: entry.commandCount,
					})}
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
					{t(
						"partiallyDeliveredTheServerAlreadyHoldsPartOfThisBatch",
						"Partially delivered — the server already holds part of this batch.",
					)}
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
	const { t } = useTranslation("flow");
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
				toast.info(
					t(
						"noPreviouslyDiscardedEditsAreStoredForThisBoard",
						"No previously discarded edits are stored for this board.",
					),
				);
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
			toast.error(
				t(
					"couldNotExportTheDiscardedEdits",
					"Could not export the discarded edits.",
				),
				{
					description: error instanceof Error ? error.message : String(error),
				},
			);
		} finally {
			setBusy(null);
		}
	}, [appId, backend.boardState, boardId, t]);

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
					? t(
							"boardReplacedWithTheServerCopyDiscardedCountUnsentEditBatches",
							"Board replaced with the server copy — {{count}} unsent edit batch discarded.",
							{ count: result.discardedBatches },
						)
					: t(
							"boardIsBackInSyncWithTheServer",
							"Board is back in sync with the server.",
						),
			);
			onOpenChange(false);
		} catch (error) {
			toast.error(
				t(
					"couldNotFetchTheBoardFromTheServer",
					"Could not fetch the board from the server.",
				),
				{
					description: error instanceof Error ? error.message : String(error),
				},
			);
		} finally {
			setBusy(null);
		}
	}, [appId, backend.boardState, boardId, onOpenChange, pendingBatches, t]);

	const destructive = pendingBatches > 0;

	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			{/* The content is a grid: every child needs min-w-0, or one long server error or a
			    row of nowrap buttons widens the single track and pushes all of them past the border. */}
			<AlertDialogContent className="grid-cols-[minmax(0,1fr)] sm:max-w-xl">
				<AlertDialogHeader className="min-w-0">
					<AlertDialogTitle>
						{t(
							"fetchThisBoardFromTheServer",
							"Fetch this Board from the server",
						)}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{destructive
							? t(
									"theServerCopyWillReplaceWhatIsOnThisDeviceEditsThatNeverReachedTheServerAreDiscardedTheyWillNotAppearAnywhereAfterwards",
									"The server copy will replace what is on this device. Edits that never reached the server are discarded — they will not appear anywhere afterwards.",
								)
							: t(
									"nothingIsQueuedForThisBoardFetchingReplacesTheLocalCopyWithTheServers",
									"Nothing is queued for this board. Fetching replaces the local copy with the server's.",
								)}
					</AlertDialogDescription>
				</AlertDialogHeader>

				{status?.ownershipMismatch ? (
					<p className="min-w-0 rounded-md border border-border/60 bg-muted/40 px-3 py-2 text-xs leading-snug text-muted-foreground wrap-anywhere">
						{t(
							"ownershipMismatchSigningBackIntoTheOriginalAccountAndHubWouldLetTheseEditsSync",
							"{{message}} Signing back into the original account and Hub would let these edits sync instead of being discarded.",
							{ message: status.ownershipMismatch },
						)}
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
								{t(
									"iUnderstandCountUnsentEditBatchesWillBeRemovedFromThisBoard",
									{
										defaultValue_one:
											"I understand that {{count}} unsent edit batch made on this device will be removed from this board.",
										defaultValue_other:
											"I understand that {{count}} unsent edit batches made on this device will be removed from this board.",
										count: pendingBatches,
									},
								)}
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
							{t("retrySync", "Retry sync")}
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
							{t("export", "Export")}
						</Button>
					</div>
					<div className="flex min-w-0 flex-wrap gap-2">
						<Button
							variant="outline"
							size="sm"
							disabled={busy !== null}
							onClick={() => onOpenChange(false)}
						>
							{t("cancel", "Cancel")}
						</Button>
						<Button
							variant={destructive ? "destructive" : "default"}
							size="sm"
							disabled={busy !== null || (destructive && !acknowledged)}
							onClick={() => void handleReset()}
						>
							{destructive
								? t("discardEditsFetch", "Discard edits & fetch")
								: t("fetchFromServer", "Fetch from server")}
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
