import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	ClipboardCopyIcon,
	CopyIcon,
	FileTextIcon,
	Loader2Icon,
} from "lucide-react";
import { memo } from "react";

const BAR_BUTTON =
	"inline-flex h-5 shrink-0 items-center gap-1 rounded px-1.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50";

export const LogCopyBar = memo(function LogCopyBar({
	count,
	onCopyText,
	onCopyJson,
	onCopyMarkdown,
	onClear,
}: Readonly<{
	count: number;
	onCopyText: () => void;
	onCopyJson: () => void;
	onCopyMarkdown: () => void;
	onClear: () => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div
			role="toolbar"
			aria-label={t("logViewSelection", "Selection")}
			className="flex h-7 shrink-0 items-center gap-1 border-t bg-primary/5 px-2"
		>
			<span className="mr-1 text-xs font-semibold text-foreground">
				{t("logViewSelectedCount", "{{count, number}} selected", { count })}
			</span>
			<button type="button" className={BAR_BUTTON} onClick={onCopyText}>
				<CopyIcon aria-hidden className="size-3" />
				{t("logViewCopyText", "Copy text")}
			</button>
			<button type="button" className={BAR_BUTTON} onClick={onCopyJson}>
				<BracesIcon aria-hidden className="size-3" />
				{t("logViewCopyJson", "Copy JSON")}
			</button>
			<button type="button" className={BAR_BUTTON} onClick={onCopyMarkdown}>
				<FileTextIcon aria-hidden className="size-3" />
				{t("logViewCopyMarkdown", "Copy Markdown")}
			</button>
			<span className="flex-1" />
			<button type="button" className={BAR_BUTTON} onClick={onClear}>
				{t("logViewClearSelection", "Clear")}
				<kbd className="ml-1 rounded border px-1 font-mono text-[10px]">
					Esc
				</kbd>
			</button>
		</div>
	);
});

export const LogStatusBar = memo(function LogStatusBar({
	total,
	rows,
	matching,
	partial,
	folded,
	foldPaused,
	filtered,
	copying,
	loadFailed,
	onClearFilters,
	onCopyVisible,
	onRetry,
}: Readonly<{
	total?: number;
	rows: number;
	matching?: number;
	partial: boolean;
	folded: boolean;
	foldPaused: boolean;
	filtered: boolean;
	copying: boolean;
	loadFailed: boolean;
	onClearFilters: () => void;
	onCopyVisible: () => void;
	onRetry: () => void;
}>) {
	const { t } = useTranslation("flow");
	const hidden =
		total !== undefined && matching !== undefined
			? Math.max(0, total - matching)
			: 0;
	const parts: string[] = [];
	if (total !== undefined) {
		parts.push(
			partial
				? t("logViewTotalPartial", "{{count, number}}+ logs", { count: total })
				: t("logViewTotal", "{{count, number}} logs", { count: total }),
		);
	}
	parts.push(
		folded && matching !== undefined && rows < matching
			? t("logViewRowsFolded", "{{count, number}} rows after folding", {
					count: rows,
				})
			: t("logViewRows", "{{count, number}} rows", { count: rows }),
	);
	if (hidden > 0) {
		parts.push(
			t("logViewHiddenByFilters", "{{count, number}} hidden by filters", {
				count: hidden,
			}),
		);
	}

	return (
		<div className="flex h-6 shrink-0 items-center gap-1.5 border-t px-2 text-[11.5px] text-muted-foreground">
			<output className="truncate tabular-nums">{parts.join(" · ")}</output>
			{filtered ? (
				<button type="button" className={BAR_BUTTON} onClick={onClearFilters}>
					{t("logViewClearFilters", "Clear filters")}
				</button>
			) : null}
			{foldPaused ? (
				<span className="truncate text-muted-foreground/80">
					{t("logViewFoldPaused", "Folding paused while searching")}
				</span>
			) : null}
			{loadFailed ? (
				<>
					<span className="truncate text-destructive">
						{t("logViewLoadFailed", "Some logs failed to load")}
					</span>
					<button type="button" className={BAR_BUTTON} onClick={onRetry}>
						{t("logViewRetry", "Retry")}
					</button>
				</>
			) : null}
			<span className="flex-1" />
			<button
				type="button"
				className={BAR_BUTTON}
				onClick={onCopyVisible}
				disabled={copying || rows === 0}
			>
				{copying ? (
					<Loader2Icon aria-hidden className="size-3 animate-spin" />
				) : (
					<ClipboardCopyIcon aria-hidden className="size-3" />
				)}
				{t("logViewCopyVisible", "Copy visible")}
			</button>
		</div>
	);
});
