import { useTranslation } from "@flow-like/locales";
import { LockIcon, ScrollIcon } from "lucide-react";
import type { RefObject } from "react";
import { useAppPermissions } from "../../hooks/use-app-permissions";
import { RolePermissions } from "../../lib/permission/role-permission";
import type { IBoard } from "../../lib/schema/flow/board";
import { useLogAggregation } from "../../state/log-aggregation-state";
import { LogsPanel } from "./logs/logs-panel";

function PanelNotice({
	icon: Icon,
	title,
	hint,
}: Readonly<{ icon: typeof ScrollIcon; title: string; hint: string }>) {
	return (
		<div className="flex h-full flex-col items-center justify-center gap-1 text-center">
			<Icon className="size-5 text-muted-foreground/60" />
			<p className="text-sm font-medium">{title}</p>
			<p className="max-w-sm text-xs text-muted-foreground">{hint}</p>
		</div>
	);
}

export function Traces({
	appId,
	boardId: _boardId,
	board,
	onFocusNode,
	nodeIdFilter,
	onClearNodeIdFilter,
	variant = "card",
}: Readonly<{
	appId: string;
	boardId: string;
	board: RefObject<IBoard | undefined>;
	onFocusNode: (nodeId: string) => void;
	nodeIdFilter?: string;
	onClearNodeIdFilter?: () => void;
	/** `panel` drops the card chrome — the shell's panel already frames it. */
	variant?: "card" | "panel";
}>) {
	const { t } = useTranslation("flow");
	const currentMetadata = useLogAggregation((state) => state.currentMetadata);
	const remote = currentMetadata?.is_remote === true;
	const permissions = useAppPermissions(remote ? appId : undefined);

	if (!currentMetadata) {
		return (
			<PanelNotice
				icon={ScrollIcon}
				title={t("noLogs", "No Logs")}
				hint={t(
					"noLogsFoundYetStartAnEventToSeeYourResultsHere",
					"No logs found yet, start an event to see your results here!",
				)}
			/>
		);
	}

	// The server serves cloud run logs only with ReadLogs; say so instead of failing every query.
	if (remote && !permissions.can(RolePermissions.ReadLogs)) {
		return (
			<PanelNotice
				icon={LockIcon}
				title={t("logViewNoLogPermission", "Your role can't read run logs")}
				hint={t(
					"logViewNoLogPermissionHint",
					"Logs of cloud runs need the Logs permission. Ask an app admin to add it to your role.",
				)}
			/>
		);
	}

	return (
		<LogsPanel
			key={`${currentMetadata.app_id}/${currentMetadata.board_id}`}
			meta={currentMetadata}
			board={board}
			onFocusNode={onFocusNode}
			nodeIdFilter={nodeIdFilter}
			onClearNodeIdFilter={onClearNodeIdFilter}
			variant={variant}
		/>
	);
}
