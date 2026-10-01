"use client";

import { useTranslation } from "@flow-like/locales";
import { MoveIcon } from "lucide-react";
import { useCallback, useMemo } from "react";
import type {
	BoardChangeStatus,
	IBoardChange,
	IBoardChangeDetail,
} from "../../../lib/board-diff";
import { cn } from "../../../lib/utils";

export const STATUS_TONE: Record<
	BoardChangeStatus,
	{ text: string; soft: string; border: string; solid: string; stroke: string }
> = {
	added: {
		text: "text-emerald-600 dark:text-emerald-400",
		soft: "bg-emerald-500/10",
		border: "border-emerald-500",
		solid: "bg-emerald-500 text-white dark:text-emerald-950",
		stroke: "var(--color-emerald-500)",
	},
	changed: {
		text: "text-sky-600 dark:text-sky-400",
		soft: "bg-sky-500/10",
		border: "border-sky-500",
		solid: "bg-sky-500 text-white dark:text-sky-950",
		stroke: "var(--color-sky-500)",
	},
	removed: {
		text: "text-destructive",
		soft: "bg-destructive/10",
		border: "border-destructive",
		solid: "bg-destructive text-white",
		stroke: "var(--destructive)",
	},
	moved: {
		text: "text-muted-foreground",
		soft: "bg-muted",
		border: "border-muted-foreground/60",
		solid: "bg-muted-foreground text-background",
		stroke: "var(--muted-foreground)",
	},
};

const GLYPH: Record<Exclude<BoardChangeStatus, "moved">, string> = {
	added: "+",
	changed: "~",
	removed: "−",
};

export function StatusGlyph({
	status,
	className,
}: Readonly<{ status: BoardChangeStatus; className?: string }>) {
	const label = useStatusLabel();
	return (
		<span
			role="img"
			aria-label={label(status)}
			className={cn(
				"inline-grid size-[18px] shrink-0 place-items-center rounded-[5px] font-mono text-xs font-semibold leading-none ring-1 ring-inset ring-current/30",
				STATUS_TONE[status].soft,
				STATUS_TONE[status].text,
				className,
			)}
		>
			{status === "moved" ? <MoveIcon className="size-3" /> : GLYPH[status]}
		</span>
	);
}

export function useStatusLabel() {
	const { t } = useTranslation("flow");
	return useCallback(
		(status: BoardChangeStatus) => {
			switch (status) {
				case "added":
					return t("boardDiffAdded", "Added");
				case "removed":
					return t("boardDiffRemoved", "Removed");
				case "changed":
					return t("boardDiffChanged", "Changed");
				case "moved":
					return t("boardDiffMoved", "Moved");
			}
		},
		[t],
	);
}

/** Translated name of what a detail row is about; pin names stay as the board has them. */
export function useDetailLabel() {
	const { t } = useTranslation("flow");
	const labels = useMemo<Record<string, string>>(
		() => ({
			"runs-after": t("boardDiffRunsAfter", "Runs after"),
			feeds: t("boardDiffFeeds", "Feeds"),
			input: t("boardDiffFieldInput", "Input"),
			output: t("boardDiffFieldOutput", "Output"),
			friendly_name: t("boardDiffFieldName", "Name"),
			name: t("boardDiffFieldName", "Name"),
			node_type: t("boardDiffFieldNodeType", "Node type"),
			comment: t("boardDiffFieldComment", "Comment"),
			layer: t("boardDiffFieldLayer", "Layer"),
			type: t("boardDiffFieldType", "Type"),
			data_type: t("boardDiffFieldType", "Type"),
			category: t("boardDiffFieldFolder", "Folder"),
			parent_id: t("boardDiffFieldParent", "Parent"),
			cache: t("boardDiffFieldCaching", "Result caching"),
			default_value: t("boardDiffFieldDefault", "Default value"),
			exposed: t("boardDiffFieldExposed", "Exposed"),
			editable: t("boardDiffFieldEditable", "Editable"),
			secret: t("boardDiffFieldSecret", "Secret"),
			runtime_configured: t("boardDiffFieldPerUser", "Set per user"),
			description: t("boardDiffFieldDescription", "Description"),
			content: t("boardDiffFieldText", "Text"),
			color: t("boardDiffFieldColor", "Color"),
			execution_mode: t("boardDiffFieldExecutionMode", "Execution mode"),
			log_level: t("boardDiffFieldLogLevel", "Log level"),
			nodes: t("boardDiffFieldNodes", "Nodes"),
		}),
		[t],
	);
	return useCallback(
		(detail: IBoardChangeDetail) => {
			if (detail.role === "runs-after") return labels["runs-after"];
			if (detail.role === "feeds") return labels.feeds;
			if (detail.role === "pin") return detail.label;
			const field = detail.field.split(":")[0];
			return labels[field] ?? detail.label;
		},
		[labels],
	);
}

/** One line under a change's title: what a reader needs to decide whether to open it. */
export function useChangeSummary() {
	const { t } = useTranslation("flow");
	const detailLabel = useDetailLabel();
	return useCallback(
		(change: IBoardChange) => {
			const runsAfter = change.details.find((d) => d.role === "runs-after");
			if (change.status === "moved")
				return t("boardDiffMovedOnly", "Moved on the canvas");
			const feeds = change.details.find((d) => d.role === "feeds");
			if (change.status === "added") {
				if (runsAfter?.after)
					return t("boardDiffRunsAfterSource", "Runs after {{source}}", {
						source: runsAfter.after,
					});
				if (feeds?.after)
					return t("boardDiffFeedsTarget", "Feeds {{target}}", {
						target: feeds.after,
					});
				return change.kind === "layer"
					? t("boardDiffNewLayer", "New {{type}}", {
							type: change.details.find((d) => d.field === "type")?.after ?? "",
						})
					: t("boardDiffNew", "New");
			}
			if (change.status === "removed") {
				if (runsAfter?.before)
					return t("boardDiffRanAfterSource", "Ran after {{source}}", {
						source: runsAfter.before,
					});
				if (feeds?.before)
					return t("boardDiffFedTarget", "Fed {{target}}", {
						target: feeds.before,
					});
				return t("boardDiffDeleted", "Deleted");
			}
			const parts: string[] = [];
			const edited = change.details.filter((d) => d.role !== "runs-after");
			if (edited.length) parts.push(edited.map(detailLabel).join(", "));
			if (runsAfter) {
				parts.push(
					runsAfter.after
						? t("boardDiffNowRunsAfter", "now runs after {{source}}", {
								source: runsAfter.after,
							})
						: t("boardDiffNoLongerRuns", "no longer runs after {{source}}", {
								source: runsAfter.before ?? "",
							}),
				);
			}
			if (change.move) parts.push(t("boardDiffAlsoMoved", "moved"));
			return parts.join(" · ");
		},
		[detailLabel, t],
	);
}
