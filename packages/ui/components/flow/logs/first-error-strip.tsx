import { useTranslation } from "@flow-like/locales";
import {
	ArrowDownIcon,
	ArrowUpIcon,
	CircleDashedIcon,
	CircleXIcon,
	CornerDownRightIcon,
	LocateFixedIcon,
} from "lucide-react";
import { memo } from "react";
import type { ISummaryLog } from "../../../lib/schema/flow/log-query";
import type { INode } from "../../../lib/schema/flow/node";
import { cn } from "../../../lib/utils";
import { firstLine, formatRelative } from "./log-format";
import { LogNodeChip } from "./log-node-chip";

export interface IMissedLabel {
	name: string;
	target: string;
}

export type IJumpNotice = "hidden" | "no-more-errors" | "failed";

const ACTION =
	"inline-flex h-5.5 shrink-0 items-center gap-1 rounded-md border bg-card px-2 text-xs font-medium text-foreground transition-colors hover:bg-secondary focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50";
const STEP =
	"inline-flex h-5.5 shrink-0 items-center gap-1 rounded-md px-1.5 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50";
const KBD =
	"inline-flex h-4 items-center rounded border bg-card px-1 font-mono text-[10px] text-muted-foreground";

export const FirstErrorStrip = memo(function FirstErrorStrip({
	error,
	base,
	node,
	nodeName,
	missed,
	notice,
	busy,
	onJump,
	onShowOnBoard,
	onScope,
	onPrev,
	onNext,
	onClearFilters,
}: Readonly<{
	error: ISummaryLog;
	base: number;
	node?: INode;
	nodeName?: string;
	missed: readonly IMissedLabel[];
	notice?: IJumpNotice;
	busy: boolean;
	onJump: () => void;
	onShowOnBoard: () => void;
	onScope: () => void;
	onPrev: () => void;
	onNext: () => void;
	onClearFilters: () => void;
}>) {
	const { t } = useTranslation("flow");
	const { line } = firstLine(error.message ?? "");

	return (
		<section
			aria-label={t("logViewFirstError", "First error")}
			className="shrink-0 border-b bg-destructive/5 py-1 pr-2 pl-3"
		>
			<div className="flex h-5.5 min-w-0 items-center gap-2">
				<CircleXIcon
					aria-hidden
					className="size-3.5 shrink-0 text-destructive"
				/>
				<span className="shrink-0 text-xs font-bold text-destructive">
					{t("logViewFirstError", "First error")}
				</span>
				<span className="shrink-0 font-mono text-[11.5px] text-muted-foreground tabular-nums">
					{formatRelative(error.start, base)}
				</span>
				{error.node_id && nodeName ? (
					<LogNodeChip
						node={node}
						label={nodeName}
						title={t("logViewScopeToNode", "Scope to {{name}}", {
							name: nodeName,
						})}
						onClick={onScope}
						className="max-w-40"
					/>
				) : null}
				<span className="min-w-0 flex-1 select-text truncate font-mono text-xs text-foreground">
					{line}
				</span>
				<button
					type="button"
					className={ACTION}
					onClick={onJump}
					disabled={busy}
				>
					<CornerDownRightIcon aria-hidden className="size-3.5" />
					{t("logViewJump", "Jump")}
				</button>
				{error.node_id ? (
					<button type="button" className={ACTION} onClick={onShowOnBoard}>
						<LocateFixedIcon aria-hidden className="size-3.5" />
						<span className="hidden lg:inline">
							{t("logViewShowOnBoard", "Show on board")}
						</span>
					</button>
				) : null}
				<span aria-hidden className="h-4 w-px shrink-0 bg-border" />
				<button
					type="button"
					className={STEP}
					onClick={onPrev}
					disabled={busy}
					aria-label={t("logViewPreviousError", "Previous error (Shift+E)")}
					title={t("logViewPreviousError", "Previous error (Shift+E)")}
				>
					<ArrowUpIcon aria-hidden className="size-3.5" />
					<kbd className={KBD}>⇧E</kbd>
				</button>
				<button
					type="button"
					className={STEP}
					onClick={onNext}
					disabled={busy}
					aria-label={t("logViewNextError", "Next error (E)")}
					title={t("logViewNextError", "Next error (E)")}
				>
					<ArrowDownIcon aria-hidden className="size-3.5" />
					<kbd className={KBD}>E</kbd>
				</button>
			</div>
			{missed.length > 0 ? (
				<div className="flex h-4 min-w-0 items-center gap-1.5 pl-5 text-[11.5px] text-muted-foreground">
					<CircleDashedIcon aria-hidden className="size-3 shrink-0" />
					<span className="truncate">
						<span className="font-semibold text-foreground/80">
							{missed.map((m) => m.name).join(", ")}
						</span>{" "}
						{t("logViewDidNotRun", "didn't run in this run")}
						<span className="mx-1.5 text-muted-foreground/60">·</span>
						{t("logViewFeeds", "it feeds")}{" "}
						<span className="font-mono text-foreground/80">
							{missed[0].target}
						</span>
					</span>
				</div>
			) : null}
			{notice ? (
				<output
					className={cn(
						"flex h-4 items-center gap-1.5 pl-5 text-[11.5px]",
						notice === "failed" ? "text-destructive" : "text-muted-foreground",
					)}
				>
					{notice === "hidden" ? (
						<>
							{t(
								"logViewErrorHidden",
								"That error is hidden by the current filters.",
							)}
							<button
								type="button"
								onClick={onClearFilters}
								className="rounded px-1 text-foreground underline-offset-2 hover:underline"
							>
								{t("logViewClearFilters", "Clear filters")}
							</button>
						</>
					) : notice === "no-more-errors" ? (
						t("logViewNoMoreErrors", "No more errors in that direction.")
					) : (
						t("logViewJumpFailed", "Couldn't find that error.")
					)}
				</output>
			) : null}
		</section>
	);
});
