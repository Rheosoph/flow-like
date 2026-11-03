"use client";

import { useTranslation } from "@flow-like/locales";
import { CopyIcon, ExternalLinkIcon, Loader2Icon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { parseBoardVersionString } from "../../../lib/event-history";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
import type { IEventTimelineRun } from "../../../state/backend-state/event-state";
import { LogsPanel } from "../../flow/logs/logs-panel";
import { RunErrorBanner } from "../../flow/run-error-message";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui";
import { PermissionNotice } from "../permission/permission-notice";

export interface EventHistoryFlowTarget {
	boardId: string;
	appId: string;
	nodeId?: string;
	version?: [number, number, number];
}

export function EventHistoryRunDialog({
	run,
	name,
	initialNodeId,
	onNavigateToFlow,
	onClose,
}: Readonly<{
	run: IEventTimelineRun;
	name: string;
	initialNodeId?: string;
	onNavigateToFlow?: (target: EventHistoryFlowTarget) => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const router = useRouter();
	const permissions = useAppPermissions(run.app_id);
	const canReadLogs =
		!run.is_remote || permissions.canStrict(RolePermissions.ReadLogs);
	const version = useMemo(
		() => parseBoardVersionString(run.version),
		[run.version],
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[run.app_id, run.board_id, version ?? undefined],
		Boolean(
			run.app_id &&
				run.board_id &&
				version &&
				permissions.can(RolePermissions.ReadBoards),
		),
	);
	const boardRef = useMemo(() => ({ current: board.data }), [board.data]);
	const [nodeIdFilter, setNodeIdFilter] = useState(initialNodeId);
	const focusNode = useCallback(
		(nodeId: string) => {
			if (onNavigateToFlow) {
				onNavigateToFlow({
					boardId: run.board_id,
					appId: run.app_id,
					nodeId,
					version: version ?? undefined,
				});
			} else {
				const params = new URLSearchParams({
					id: run.board_id,
					app: run.app_id,
					node: nodeId,
				});
				if (version) params.set("version", version.join("_"));
				router.push(`/flow?${params.toString()}`);
			}
			onClose();
		},
		[onClose, onNavigateToFlow, router, run.app_id, run.board_id, version],
	);
	const copyRunId = async () => {
		try {
			await navigator.clipboard.writeText(run.run_id);
			toast.success(t("runIdCopied", "Run ID copied"));
		} catch {
			toast.error(t("runIdCopyFailed", "Could not copy the run ID"));
		}
	};

	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="h-[85dvh] overflow-hidden p-0 sm:max-w-6xl">
				<DialogHeader className="px-4 pt-4 pr-10 text-left">
					<DialogTitle className="break-words">{name}</DialogTitle>
					<DialogDescription>
						{t("eventHistoryRunDetails", "Logs and node details for this run.")}
					</DialogDescription>
					<div className="flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
						<span>{t("runId", "Run ID")}</span>
						<code className="min-w-0 select-text break-all">{run.run_id}</code>
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="size-6 shrink-0"
							aria-label={t("copyRunId", "Copy run ID")}
							title={t("copyRunId", "Copy run ID")}
							onClick={() => void copyRunId()}
						>
							<CopyIcon className="size-3" />
						</Button>
						{run.board_id && (
							<Button
								type="button"
								variant="outline"
								size="sm"
								className="ml-auto"
								onClick={() => focusNode(nodeIdFilter ?? run.node_id)}
							>
								<ExternalLinkIcon className="size-3.5" />
								{t("openInEditor", "Open in editor")}
							</Button>
						)}
					</div>
				</DialogHeader>
				<div className="flex min-h-0 flex-1 flex-col overflow-hidden border-t">
					{!canReadLogs && run.error_message && (
						<RunErrorBanner message={run.error_message} />
					)}
					{run.is_remote && permissions.isLoading ? (
						<div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
							<Loader2Icon className="size-4 animate-spin" />
							{t("loadingRunPermissions", "Checking access to run logs…")}
						</div>
					) : !canReadLogs ? (
						<PermissionNotice
							className="m-4"
							title={t("runLogsUnavailable", "Run logs unavailable")}
							description={t(
								"eventHistoryRunLogsNeedPermission",
								"Reading logs of cloud runs needs permission to view this project's logs.",
							)}
							missing={[RolePermissions.ReadLogs]}
						/>
					) : (
						<>
							{(board.isError ||
								!version ||
								(board.isSuccess && !board.data)) && (
								<p className="shrink-0 border-b px-3 py-2 text-xs text-muted-foreground">
									{t(
										"eventHistoryRunBoardUnavailable",
										"The flow version for this run is unavailable. Logs may show node IDs instead of names.",
									)}
								</p>
							)}
							<LogsPanel
								key={`${run.app_id}/${run.board_id}/${run.run_id}`}
								meta={run}
								board={boardRef}
								onFocusNode={focusNode}
								nodeIdFilter={nodeIdFilter}
								onClearNodeIdFilter={() => setNodeIdFilter(undefined)}
								variant="panel"
							/>
						</>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}
