import { useTranslation } from "@flow-like/locales";
import { CheckIcon, ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { type MouseEvent, memo, useCallback, useMemo } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogGroup } from "../../../lib/schema/flow/log-query";
import type { INode } from "../../../lib/schema/flow/node";
import { cn } from "../../../lib/utils";
import { Skeleton } from "../../ui/skeleton";
import { templateParts } from "./fold-model";
import { LevelIcon } from "./level-style";
import {
	ERROR_LEVEL,
	firstLine,
	formatAbsolute,
	formatDuration,
	formatRelative,
	levelIndex,
	logDuration,
	logStart,
} from "./log-format";
import { LogNodeChip } from "./log-node-chip";

export const ROW_HEIGHT = 24;

export interface IRowActions {
	toggleSelect(index: number, log: ILog, shift: boolean): void;
	focusRow(index: number, log: ILog): void;
	scopeNode(nodeId: string): void;
	setFolded(fingerprint: string, folded: boolean): void;
}

const ROW_BASE =
	"absolute left-0 top-0 flex w-full items-center gap-2 pr-2.5 pl-1.5 font-mono text-xs";

export const LogRowSkeleton = memo(function LogRowSkeleton({
	top,
	failed,
}: Readonly<{ top: number; failed: boolean }>) {
	return (
		<div
			aria-hidden
			className={ROW_BASE}
			style={{ height: ROW_HEIGHT, transform: `translateY(${top}px)` }}
		>
			<span className="size-4 shrink-0" />
			{failed ? (
				<span className="text-muted-foreground/50">—</span>
			) : (
				<>
					<Skeleton className="h-2.5 w-21 shrink-0 rounded-sm bg-muted" />
					<Skeleton className="h-2.5 w-24 shrink-0 rounded-sm bg-muted" />
					<Skeleton className="h-2.5 max-w-[45%] flex-1 rounded-sm bg-muted" />
				</>
			)}
		</div>
	);
});

const FoldTemplate = memo(function FoldTemplate({
	group,
}: Readonly<{ group: ILogGroup }>) {
	const parts = useMemo(
		() => templateParts(group.template.split("\n", 1)[0] ?? "", group.slots),
		[group],
	);
	return (
		<span className="flex min-w-0 items-center overflow-hidden whitespace-pre">
			{parts.map((part, i) =>
				part.kind === "text" ? (
					<span
						// biome-ignore lint/suspicious/noArrayIndexKey: parts never reorder
						key={i}
						className={cn("min-w-0 truncate", i > 0 && "shrink-0")}
					>
						{part.text}
					</span>
				) : (
					<span
						// biome-ignore lint/suspicious/noArrayIndexKey: parts never reorder
						key={i}
						className="mx-0.5 inline-flex h-4 shrink-0 items-center rounded border border-dashed border-border bg-secondary px-1 text-[11px] text-foreground/80"
					>
						{part.label}
					</span>
				),
			)}
		</span>
	);
});

export const LogRow = memo(function LogRow({
	index,
	top,
	log,
	selected,
	focused,
	node,
	nodeName,
	group,
	folded,
	relative,
	base,
	actions,
}: Readonly<{
	index: number;
	top: number;
	log: ILog;
	selected: boolean;
	focused: boolean;
	node?: INode;
	nodeName?: string;
	/** Set on a group's first occurrence while folding is possible. */
	group?: ILogGroup;
	folded: boolean;
	relative: boolean;
	base: number;
	actions: IRowActions;
}>) {
	const { t } = useTranslation("flow");
	const level = levelIndex(log.log_level);
	const start = logStart(log);
	const time = relative ? formatRelative(start, base) : formatAbsolute(start);
	const { line, extra } = useMemo(
		() => firstLine(log.message ?? ""),
		[log.message],
	);
	const duration = formatDuration(logDuration(log));
	const isError = level >= ERROR_LEVEL;
	const showFold = !!group && folded;

	const onRowClick = useCallback(
		() => actions.focusRow(index, log),
		[actions, index, log],
	);
	const onSelect = useCallback(
		(event: MouseEvent<HTMLButtonElement>) => {
			event.stopPropagation();
			actions.toggleSelect(index, log, event.shiftKey);
		},
		[actions, index, log],
	);
	const onScope = useCallback(
		(event: MouseEvent<HTMLButtonElement>) => {
			event.stopPropagation();
			if (log.node_id) actions.scopeNode(log.node_id);
		},
		[actions, log.node_id],
	);
	const onFoldToggle = useCallback(
		(event: MouseEvent<HTMLButtonElement>) => {
			event.stopPropagation();
			if (group) actions.setFolded(group.fingerprint, !folded);
		},
		[actions, group, folded],
	);

	const scopeLabel = t("logViewScopeToNode", "Scope to {{name}}", {
		name: nodeName ?? "",
	});

	return (
		// biome-ignore lint/a11y/useKeyWithClickEvents: the panel handles arrow keys for the focused row
		<div
			data-log-index={index}
			onClick={onRowClick}
			className={cn(
				ROW_BASE,
				"hover:bg-secondary/60",
				isError && "bg-destructive/5 hover:bg-destructive/10",
				selected && "bg-primary/10 hover:bg-primary/15",
				focused && "shadow-[inset_2px_0_0_var(--color-primary)]",
			)}
			style={{ height: ROW_HEIGHT, transform: `translateY(${top}px)` }}
		>
			<button
				type="button"
				aria-pressed={selected}
				aria-label={t("logViewSelectLog", "Select log")}
				onClick={onSelect}
				className={cn(
					"flex size-4 shrink-0 items-center justify-center rounded border transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
					selected
						? "border-primary bg-primary text-primary-foreground"
						: "border-border text-transparent hover:border-muted-foreground",
				)}
			>
				<CheckIcon className="size-3" />
			</button>
			<span className="w-23 shrink-0 select-text truncate tabular-nums text-muted-foreground">
				{time}
			</span>
			<LevelIcon level={level} />
			<span className="flex w-37.5 shrink-0 min-w-0 items-center">
				{log.node_id && nodeName ? (
					<LogNodeChip
						node={node}
						label={nodeName}
						title={scopeLabel}
						onClick={onScope}
					/>
				) : null}
			</span>
			<span className="flex w-4 shrink-0 items-center">
				{group ? (
					<button
						type="button"
						aria-expanded={!folded}
						aria-label={
							folded
								? t(
										"logViewUnfoldGroup",
										"Show all {{count, number}} repeats",
										{
											count: group.count,
										},
									)
								: t("logViewFoldGroup", "Fold these repeats")
						}
						onClick={onFoldToggle}
						className="flex size-4 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
					>
						{folded ? (
							<ChevronRightIcon className="size-3" />
						) : (
							<ChevronDownIcon className="size-3" />
						)}
					</button>
				) : null}
			</span>
			<span
				className={cn(
					"flex min-w-0 flex-1 select-text items-center gap-2",
					level === 0 ? "text-foreground/70" : "text-foreground",
				)}
			>
				{showFold && group ? (
					<FoldTemplate group={group} />
				) : (
					<span className="min-w-0 truncate">{line}</span>
				)}
				{extra > 0 && !showFold ? (
					<span className="shrink-0 rounded bg-secondary px-1 font-sans text-[10.5px] font-semibold text-muted-foreground">
						{t("logViewMoreLines", "+{{count, number}} lines", {
							count: extra,
						})}
					</span>
				) : null}
			</span>
			{showFold && group ? (
				<span
					className={cn(
						"inline-flex h-4.5 shrink-0 items-center rounded-full border px-1.5 text-[11px] font-semibold",
						isError
							? "border-destructive/30 bg-destructive/10 text-destructive"
							: "border-border bg-muted text-foreground/80",
					)}
				>
					×{group.count.toLocaleString()}
				</span>
			) : null}
			<span className="w-14.5 shrink-0 truncate text-right text-[11px] text-muted-foreground">
				{duration}
			</span>
		</div>
	);
});
