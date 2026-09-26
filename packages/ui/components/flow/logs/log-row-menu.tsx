import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	CopyIcon,
	CrosshairIcon,
	EyeOffIcon,
	FilterIcon,
	LocateFixedIcon,
	TextSelectIcon,
} from "lucide-react";
import { memo } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import {
	ContextMenuItem,
	ContextMenuLabel,
	ContextMenuSeparator,
} from "../../ui/context-menu";
import type { IDetailActions } from "./log-detail-pane";
import { LEVEL_LABELS, levelIndex } from "./log-format";

export const LogRowMenu = memo(function LogRowMenu({
	log,
	selectedText,
	nodeName,
	similar,
	fingerprinted,
	actions,
	onCopySelection,
}: Readonly<{
	log: ILog;
	selectedText: string;
	nodeName?: string;
	similar?: number;
	fingerprinted: boolean;
	actions: IDetailActions;
	onCopySelection: (text: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const nodeId = log.node_id ?? undefined;
	const fingerprint = fingerprinted
		? (log.fingerprint ?? undefined)
		: undefined;
	const level = LEVEL_LABELS[levelIndex(log.log_level)].toLowerCase();

	return (
		<>
			<ContextMenuLabel className="truncate text-xs text-muted-foreground">
				{nodeName ? `${nodeName} · ${level}` : level}
				{similar && similar > 1 ? ` ×${similar.toLocaleString()}` : ""}
			</ContextMenuLabel>
			{selectedText ? (
				<ContextMenuItem onSelect={() => onCopySelection(selectedText)}>
					<TextSelectIcon />
					{t("logViewCopySelectedText", "Copy selected text")}
				</ContextMenuItem>
			) : null}
			{nodeId ? (
				<>
					<ContextMenuItem onSelect={() => actions.scopeNode(nodeId)}>
						<CrosshairIcon />
						{t("logViewScopeToNode", "Scope to {{name}}", {
							name: nodeName ?? "",
						})}
					</ContextMenuItem>
					<ContextMenuItem onSelect={() => actions.excludeNode(nodeId)}>
						<EyeOffIcon />
						{t("logViewExcludeNamed", "Exclude {{name}}", {
							name: nodeName ?? "",
						})}
					</ContextMenuItem>
				</>
			) : null}
			{fingerprint ? (
				<>
					<ContextMenuItem onSelect={() => actions.hideSimilar(fingerprint)}>
						<EyeOffIcon />
						{similar
							? t(
									"logViewHideSimilarCount",
									"Hide similar ({{count, number}})",
									{
										count: similar,
									},
								)
							: t("logViewHideSimilar", "Hide similar")}
					</ContextMenuItem>
					<ContextMenuItem onSelect={() => actions.onlySimilar(fingerprint)}>
						<FilterIcon />
						{t("logViewOnlySimilar", "Only similar")}
					</ContextMenuItem>
				</>
			) : null}
			<ContextMenuSeparator />
			<ContextMenuItem onSelect={() => actions.copyText(log)}>
				<CopyIcon />
				{t("logViewCopy", "Copy")}
			</ContextMenuItem>
			<ContextMenuItem onSelect={() => actions.copyJson(log)}>
				<BracesIcon />
				{t("logViewCopyAsJson", "Copy as JSON")}
			</ContextMenuItem>
			{nodeId ? (
				<>
					<ContextMenuSeparator />
					<ContextMenuItem onSelect={() => actions.showOnBoard(nodeId)}>
						<LocateFixedIcon />
						{t("logViewShowOnBoard", "Show on board")}
					</ContextMenuItem>
				</>
			) : null}
		</>
	);
});
