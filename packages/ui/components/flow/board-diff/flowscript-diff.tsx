"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronsUpDownIcon,
	FileCode2Icon,
	LoaderCircleIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { collapseContext, computeDiff } from "../../ui/diff-viewer/compute";
import {
	type HighlightToken,
	buildLinePieces,
	useHighlightedLines,
} from "../../ui/diff-viewer/highlight";
import type { DiffCell, DiffItem, DiffRow } from "../../ui/diff-viewer/types";
import { projectFlowScript } from "./flowscript-lines";

/** FlowScript reads like TypeScript closely enough for its grammar to colour it well. */
const HIGHLIGHT_LANGUAGE = "typescript";
const CONTEXT_LINES = 3;

export interface FlowScriptSource {
	text?: string;
	loading?: boolean;
	error?: string;
}

type Side = "left" | "right";

interface LineProps {
	cell: DiffCell;
	side: Side;
	tone: "add" | "del" | null;
	tokens?: HighlightToken[];
	owner?: string;
	highlighted: boolean;
	numbers: "both" | "one";
	pairedNumber?: number | null;
	onSelect: (owner?: string) => void;
}

const DiffLine = memo(function DiffLine({
	cell,
	side,
	tone,
	tokens,
	owner,
	highlighted,
	numbers,
	pairedNumber,
	onSelect,
}: LineProps) {
	if (cell.text === null) {
		return (
			<div
				className={cn(
					"grid min-h-[1.65em] bg-[repeating-linear-gradient(135deg,transparent_0_6px,color-mix(in_oklch,var(--muted)_70%,transparent)_6px_7px)]",
					numbers === "both"
						? "grid-cols-[2.6rem_2.6rem_1.25rem_minmax(0,1fr)]"
						: "grid-cols-[2.6rem_1.25rem_minmax(0,1fr)]",
				)}
			/>
		);
	}
	const pieces = buildLinePieces(
		cell.text,
		tokens,
		tone ? cell.segments : undefined,
	);
	const leftNo = side === "left" ? cell.lineNo : pairedNumber;
	const rightNo = side === "right" ? cell.lineNo : pairedNumber;
	return (
		<button
			type="button"
			tabIndex={-1}
			data-owner={owner}
			onClick={() => onSelect(owner)}
			className={cn(
				"grid min-h-[1.65em] w-full cursor-pointer text-left font-mono",
				numbers === "both"
					? "grid-cols-[2.6rem_2.6rem_1.25rem_minmax(0,1fr)]"
					: "grid-cols-[2.6rem_1.25rem_minmax(0,1fr)]",
				tone === "add" && "bg-emerald-500/10",
				tone === "del" && "bg-destructive/10",
				!tone && "hover:bg-muted/50",
				highlighted && "shadow-[inset_3px_0_0_var(--primary)]",
				highlighted && !tone && "bg-primary/5",
			)}
		>
			{numbers === "both" ? (
				<>
					<span className="select-none pr-2 text-right text-[11px] text-muted-foreground/60">
						{leftNo ?? ""}
					</span>
					<span className="select-none pr-2 text-right text-[11px] text-muted-foreground/60">
						{rightNo ?? ""}
					</span>
				</>
			) : (
				<span className="select-none pr-2 text-right text-[11px] text-muted-foreground/60">
					{cell.lineNo ?? ""}
				</span>
			)}
			<span
				className={cn(
					"select-none text-center",
					tone === "add" &&
						"font-semibold text-emerald-600 dark:text-emerald-400",
					tone === "del" && "font-semibold text-destructive",
				)}
			>
				{tone === "add" ? "+" : tone === "del" ? "−" : ""}
			</span>
			<code className="min-w-0 whitespace-pre-wrap wrap-break-word pr-4 font-mono">
				{pieces.map((piece, index) => (
					<span
						// biome-ignore lint/suspicious/noArrayIndexKey: pieces are positional within one immutable line
						key={index}
						style={piece.color ? { color: piece.color } : undefined}
						className={cn(
							piece.changed &&
								(tone === "add" ? "bg-emerald-500/30" : "bg-destructive/25"),
						)}
					>
						{piece.text}
					</span>
				))}
			</code>
		</button>
	);
});

export const FlowScriptDiff = memo(function FlowScriptDiff({
	base,
	head,
	layout,
	ignoreWhitespace,
	selectedIds,
	scrollRequest,
	onSelectId,
}: Readonly<{
	base: FlowScriptSource;
	head: FlowScriptSource;
	layout: "unified" | "split";
	ignoreWhitespace: boolean;
	selectedIds: string[];
	scrollRequest: number;
	onSelectId: (id: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const { resolvedTheme } = useTheme();
	const theme = resolvedTheme === "dark" ? "dark" : "light";
	const scrollRef = useRef<HTMLDivElement>(null);
	const [expanded, setExpanded] = useState<Set<string>>(new Set());

	const left = useMemo(() => projectFlowScript(base.text ?? ""), [base.text]);
	const right = useMemo(() => projectFlowScript(head.text ?? ""), [head.text]);
	const ready = base.text !== undefined && head.text !== undefined;

	const result = useMemo(
		() =>
			ready
				? computeDiff(left.text, right.text, {
						ignoreWhitespace,
						wordLevel: true,
					})
				: undefined,
		[ignoreWhitespace, left.text, ready, right.text],
	);
	const items = useMemo<DiffItem[]>(
		() => (result ? collapseContext(result.rows, CONTEXT_LINES) : []),
		[result],
	);
	const leftTokens = useHighlightedLines(
		left.text,
		HIGHLIGHT_LANGUAGE,
		theme,
		ready,
	);
	const rightTokens = useHighlightedLines(
		right.text,
		HIGHLIGHT_LANGUAGE,
		theme,
		ready,
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: a new comparison starts collapsed
	useEffect(
		() => setExpanded(new Set()),
		[base.text, head.text, ignoreWhitespace],
	);

	const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
	const ownerOf = (side: Side, lineNo: number | null) =>
		lineNo
			? side === "left"
				? left.owners[lineNo - 1]
				: right.owners[lineNo - 1]
			: undefined;
	const tokensOf = (side: Side, lineNo: number | null) =>
		lineNo
			? (side === "left" ? leftTokens : rightTokens)?.[lineNo - 1]
			: undefined;
	const onSelect = (owner?: string) => {
		if (owner) onSelectId(owner);
	};

	const selectedKey = selectedIds.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: scroll on explicit requests and selection changes only
	useEffect(() => {
		const root = scrollRef.current;
		if (!root || !selected.size) return;
		const target = [...root.querySelectorAll<HTMLElement>("[data-owner]")].find(
			(el) => selected.has(el.dataset.owner ?? ""),
		);
		if (!target) return;
		const top = target.offsetTop - root.clientHeight / 3;
		root.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
	}, [scrollRequest, selectedKey, items]);

	if (base.error || head.error) {
		return (
			<p className="p-6 text-sm text-destructive">{base.error ?? head.error}</p>
		);
	}
	if (!ready || !result) {
		return (
			<div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
				<LoaderCircleIcon className="size-4 animate-spin" />
				{t("boardDiffRenderingScript", "Rendering FlowScript…")}
			</div>
		);
	}
	if (result.stats.additions === 0 && result.stats.deletions === 0) {
		return (
			<p className="flex h-full items-center justify-center gap-2 p-6 text-sm text-muted-foreground">
				<FileCode2Icon className="size-4" />
				{t("boardDiffScriptSame", "The FlowScript is the same on both sides.")}
			</p>
		);
	}

	const renderUnified = (row: DiffRow, key: string) => {
		const leftOwner = ownerOf("left", row.left.lineNo);
		const rightOwner = ownerOf("right", row.right.lineNo);
		if (row.type === "context") {
			return (
				<DiffLine
					key={key}
					cell={row.right}
					side="right"
					tone={null}
					numbers="both"
					pairedNumber={row.left.lineNo}
					tokens={tokensOf("right", row.right.lineNo)}
					owner={rightOwner ?? leftOwner}
					highlighted={selected.has(rightOwner ?? leftOwner ?? "")}
					onSelect={onSelect}
				/>
			);
		}
		return (
			<div key={key}>
				{row.left.text !== null && (
					<DiffLine
						cell={row.left}
						side="left"
						tone="del"
						numbers="both"
						tokens={tokensOf("left", row.left.lineNo)}
						owner={leftOwner}
						highlighted={selected.has(leftOwner ?? "")}
						onSelect={onSelect}
					/>
				)}
				{row.right.text !== null && (
					<DiffLine
						cell={row.right}
						side="right"
						tone="add"
						numbers="both"
						tokens={tokensOf("right", row.right.lineNo)}
						owner={rightOwner}
						highlighted={selected.has(rightOwner ?? "")}
						onSelect={onSelect}
					/>
				)}
			</div>
		);
	};

	const renderSplit = (row: DiffRow, key: string) => {
		const leftOwner = ownerOf("left", row.left.lineNo);
		const rightOwner = ownerOf("right", row.right.lineNo);
		const changed = row.type !== "context";
		return (
			<div key={key} className="grid grid-cols-2 divide-x divide-border">
				<DiffLine
					cell={row.left}
					side="left"
					tone={changed && row.left.text !== null ? "del" : null}
					numbers="one"
					tokens={tokensOf("left", row.left.lineNo)}
					owner={leftOwner}
					highlighted={selected.has(leftOwner ?? "")}
					onSelect={onSelect}
				/>
				<DiffLine
					cell={row.right}
					side="right"
					tone={changed && row.right.text !== null ? "add" : null}
					numbers="one"
					tokens={tokensOf("right", row.right.lineNo)}
					owner={rightOwner}
					highlighted={selected.has(rightOwner ?? "")}
					onSelect={onSelect}
				/>
			</div>
		);
	};

	return (
		<div
			ref={scrollRef}
			className="relative h-full overflow-auto bg-card py-1.5 font-mono text-[12.5px] leading-[1.65]"
		>
			{items.map((item) => {
				if (item.kind === "gap") {
					if (expanded.has(item.gapId)) {
						return item.hiddenRows.map((row, i) => {
							const key = `r${item.firstRowIndex + i}`;
							return layout === "split"
								? renderSplit(row, key)
								: renderUnified(row, key);
						});
					}
					return (
						<button
							key={item.gapId}
							type="button"
							onClick={() =>
								setExpanded((prev) => new Set(prev).add(item.gapId))
							}
							className="my-0.5 flex h-7 w-full items-center gap-2 bg-sky-500/5 pl-5 text-left font-sans text-[11.5px] font-medium text-sky-700 hover:bg-sky-500/10 dark:text-sky-300"
						>
							<ChevronsUpDownIcon className="size-3.5" />
							{t("boardDiffShowUnchanged", {
								defaultValue_one: "Show {{count}} unchanged line",
								defaultValue_other: "Show {{count}} unchanged lines",
								count: item.hiddenRows.length,
							})}
						</button>
					);
				}
				const key = `r${item.rowIndex}`;
				return layout === "split"
					? renderSplit(item.row, key)
					: renderUnified(item.row, key);
			})}
		</div>
	);
});

export function flowScriptStats(
	base?: string,
	head?: string,
	ignoreWhitespace = true,
) {
	if (base === undefined || head === undefined) return undefined;
	const { stats } = computeDiff(
		projectFlowScript(base).text,
		projectFlowScript(head).text,
		{ ignoreWhitespace, wordLevel: false },
	);
	return stats;
}
