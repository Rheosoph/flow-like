"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Code,
	Columns2,
	Eye,
	FileDiff,
	FoldVertical,
	Pilcrow,
	Rows3,
	UnfoldVertical,
	WrapText,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	type CSSProperties,
	type ReactNode,
	useEffect,
	useMemo,
	useState,
} from "react";
import { cn } from "../../../lib";
import { Button } from "../button";
import { TextEditor } from "../text-editor";
import {
	collapseContext,
	computeBlockDiff,
	computeDiff,
	computeInlineSegments,
	trimTrailingLines,
} from "./compute";
import {
	type HighlightToken,
	buildLinePieces,
	useHighlightedLines,
} from "./highlight";
import type {
	DiffCell,
	DiffContentKind,
	DiffItem,
	DiffRow,
	DiffViewMode,
	MarkdownMode,
	RenderedBlock,
} from "./types";

export interface DiffViewerProps {
	original: string;
	modified: string;
	mode?: DiffViewMode;
	kind?: DiffContentKind;
	language?: string;
	markdownMode?: MarkdownMode;
	showLineNumbers?: boolean;
	wordWrap?: boolean;
	wordLevel?: boolean;
	collapseUnchanged?: boolean;
	contextLines?: number;
	showStats?: boolean;
	showToolbar?: boolean;
	originalLabel?: string;
	modifiedLabel?: string;
	ignoreWhitespace?: boolean;
	ignoreCase?: boolean;
	trimTrailingWhitespace?: boolean;
	swapSides?: boolean;
	className?: string;
	style?: CSSProperties;
}

type Tone = "add" | "del" | null;

const NUM_CLASS =
	"px-2 py-px text-right tabular-nums text-[11px] leading-[1.7] text-muted-foreground/60 select-none";
const CODE_CLASS = "px-3 py-px min-w-0";
const WORD_ADD = "bg-green-500/30 dark:bg-green-400/25 rounded-[2px]";
const WORD_DEL = "bg-red-500/30 dark:bg-red-400/25 rounded-[2px]";

function looksLikeUrl(value: string): boolean {
	const trimmed = value.trim();
	if (!trimmed || /\s/.test(trimmed)) return false;
	// Require an explicit protocol or absolute path so bare filenames like
	// "README.md" are diffed as text instead of loaded into an iframe.
	return /^(https?:|data:|blob:|asset:|file:|storage:|\/)/i.test(trimmed);
}

function isJson(value: string): boolean {
	const trimmed = value.trim();
	if (!trimmed) return false;
	if (!/^[[{]/.test(trimmed)) return false;
	try {
		JSON.parse(trimmed);
		return true;
	} catch {
		return false;
	}
}

function prettyJson(value: string): string {
	try {
		return JSON.stringify(JSON.parse(value), null, 2);
	} catch {
		return value;
	}
}

function resolveKind(
	kind: DiffContentKind,
	original: string,
	modified: string,
	language: string | undefined,
): Exclude<DiffContentKind, "auto"> {
	if (kind !== "auto") return kind;
	if (looksLikeUrl(original) && looksLikeUrl(modified)) return "document";
	if (language && language.toLowerCase() !== "plaintext") return "code";
	if (isJson(original) && isJson(modified)) return "json";
	return "text";
}

export function DiffViewer({
	original,
	modified,
	mode: modeProp = "split",
	kind = "auto",
	language = "plaintext",
	markdownMode: markdownModeProp = "source",
	showLineNumbers = true,
	wordWrap: wordWrapProp = false,
	wordLevel = true,
	collapseUnchanged: collapseProp = false,
	contextLines = 3,
	showStats = true,
	showToolbar = true,
	originalLabel = "Original",
	modifiedLabel = "Modified",
	ignoreWhitespace = false,
	ignoreCase = false,
	trimTrailingWhitespace = false,
	swapSides = false,
	className,
	style,
}: DiffViewerProps) {
	const { t } = useTranslation("common");
	const { resolvedTheme } = useTheme();
	const themeMode = resolvedTheme === "dark" ? "dark" : "light";

	const [mode, setMode] = useState<DiffViewMode>(modeProp);
	const [wordWrap, setWordWrap] = useState<boolean>(wordWrapProp);
	const [collapse, setCollapse] = useState<boolean>(collapseProp);
	const [mdMode, setMdMode] = useState<MarkdownMode>(markdownModeProp);
	const [expandedGaps, setExpandedGaps] = useState<Set<string>>(new Set());

	useEffect(() => setMode(modeProp), [modeProp]);
	useEffect(() => setWordWrap(wordWrapProp), [wordWrapProp]);
	useEffect(() => setCollapse(collapseProp), [collapseProp]);
	useEffect(() => setMdMode(markdownModeProp), [markdownModeProp]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: reset expanded gaps when the diffed content changes
	useEffect(() => setExpandedGaps(new Set()), [original, modified]);

	const effectiveKind = resolveKind(kind, original, modified, language);

	const [left, right] = useMemo(() => {
		let a = swapSides ? modified : original;
		let b = swapSides ? original : modified;
		if (effectiveKind === "json") {
			a = prettyJson(a);
			b = prettyJson(b);
		}
		if (trimTrailingWhitespace) {
			a = trimTrailingLines(a);
			b = trimTrailingLines(b);
		}
		return [a, b];
	}, [original, modified, swapSides, effectiveKind, trimTrailingWhitespace]);

	const highlightLang =
		effectiveKind === "json"
			? "json"
			: effectiveKind === "markdown"
				? "markdown"
				: language;
	const highlightEnabled =
		mode !== "inline" &&
		(effectiveKind === "code" ||
			effectiveKind === "json" ||
			(effectiveKind === "markdown" && mdMode === "source"));

	const originalGrid = useHighlightedLines(
		left,
		highlightLang,
		themeMode,
		highlightEnabled,
	);
	const modifiedGrid = useHighlightedLines(
		right,
		highlightLang,
		themeMode,
		highlightEnabled,
	);

	const { rows, stats } = useMemo(
		() => computeDiff(left, right, { wordLevel, ignoreWhitespace, ignoreCase }),
		[left, right, wordLevel, ignoreWhitespace, ignoreCase],
	);

	const inlineSegments = useMemo(
		() =>
			mode === "inline"
				? computeInlineSegments(left, right, { ignoreCase })
				: [],
		[mode, left, right, ignoreCase],
	);

	const renderedBlocks: RenderedBlock[] = useMemo(
		() =>
			effectiveKind === "markdown" && mdMode === "rendered"
				? computeBlockDiff(left, right, { ignoreCase })
				: [],
		[effectiveKind, mdMode, left, right, ignoreCase],
	);

	const items: DiffItem[] = useMemo(
		() =>
			collapse
				? collapseContext(rows, contextLines)
				: rows.map((row, rowIndex) => ({ kind: "row", row, rowIndex })),
		[collapse, rows, contextLines],
	);

	const toggleGap = (gapId: string) =>
		setExpandedGaps((prev) => {
			const next = new Set(prev);
			if (next.has(gapId)) {
				next.delete(gapId);
			} else {
				next.add(gapId);
			}
			return next;
		});

	const renderLine = (
		cell: DiffCell,
		grid: HighlightToken[][] | null,
		tone: Tone,
	): ReactNode => {
		if (cell.text === null) return null;
		if (cell.text === "") return " ";
		const tokens =
			cell.lineNo != null && grid ? grid[cell.lineNo - 1] : undefined;
		const pieces = buildLinePieces(cell.text, tokens, cell.segments);
		let offset = 0;
		return pieces.map((piece) => {
			const key = offset;
			offset += piece.text.length;
			return (
				<span
					key={key}
					className={cn(
						piece.changed && (tone === "add" ? WORD_ADD : WORD_DEL),
					)}
					style={piece.color ? { color: piece.color } : undefined}
				>
					{piece.text}
				</span>
			);
		});
	};

	const codeWrap = wordWrap
		? "whitespace-pre-wrap break-words"
		: "whitespace-pre";
	const codeCol = wordWrap ? "minmax(0,1fr)" : "max-content";

	const renderSplitRow = (row: DiffRow, key: string): ReactNode => {
		const leftTone: Tone =
			row.type === "change" || row.type === "delete" ? "del" : null;
		const rightTone: Tone =
			row.type === "change" || row.type === "insert" ? "add" : null;
		const leftBg =
			row.left.text === null
				? "bg-muted/20"
				: leftTone === "del"
					? "bg-red-500/10"
					: "";
		const rightBg =
			row.right.text === null
				? "bg-muted/20"
				: rightTone === "add"
					? "bg-green-500/10"
					: "";
		const divider = "border-l border-border/60";
		return (
			<div key={key} className="contents">
				{showLineNumbers && (
					<div className={cn(NUM_CLASS, leftBg)}>{row.left.lineNo ?? ""}</div>
				)}
				<div className={cn(CODE_CLASS, codeWrap, leftBg)}>
					{renderLine(row.left, originalGrid, leftTone)}
				</div>
				{showLineNumbers && (
					<div className={cn(NUM_CLASS, rightBg, divider)}>
						{row.right.lineNo ?? ""}
					</div>
				)}
				<div
					className={cn(
						CODE_CLASS,
						codeWrap,
						rightBg,
						!showLineNumbers && divider,
					)}
				>
					{renderLine(row.right, modifiedGrid, rightTone)}
				</div>
			</div>
		);
	};

	const renderUnifiedLine = (
		key: string,
		oldNo: number | null,
		newNo: number | null,
		marker: "+" | "-" | " ",
		cell: DiffCell,
		grid: HighlightToken[][] | null,
		tone: Tone,
		bg: string,
	): ReactNode => {
		const markerColor =
			marker === "+"
				? "text-green-600 dark:text-green-400"
				: marker === "-"
					? "text-red-600 dark:text-red-400"
					: "text-transparent";
		return (
			<div key={key} className="contents">
				{showLineNumbers && (
					<div className={cn(NUM_CLASS, bg)}>{oldNo ?? ""}</div>
				)}
				{showLineNumbers && (
					<div className={cn(NUM_CLASS, bg)}>{newNo ?? ""}</div>
				)}
				<div
					className={cn(
						"select-none px-1 text-center text-[11px] leading-[1.7] font-bold",
						markerColor,
						bg,
					)}
				>
					{marker}
				</div>
				<div className={cn(CODE_CLASS, codeWrap, bg)}>
					{renderLine(cell, grid, tone)}
				</div>
			</div>
		);
	};

	const renderUnifiedRow = (row: DiffRow, key: string): ReactNode => {
		if (row.type === "context") {
			return renderUnifiedLine(
				key,
				row.left.lineNo,
				row.right.lineNo,
				" ",
				row.left,
				originalGrid,
				null,
				"",
			);
		}
		if (row.type === "change") {
			return (
				<div key={key} className="contents">
					{renderUnifiedLine(
						`${key}-d`,
						row.left.lineNo,
						null,
						"-",
						row.left,
						originalGrid,
						"del",
						"bg-red-500/10",
					)}
					{renderUnifiedLine(
						`${key}-a`,
						null,
						row.right.lineNo,
						"+",
						row.right,
						modifiedGrid,
						"add",
						"bg-green-500/10",
					)}
				</div>
			);
		}
		if (row.type === "delete") {
			return renderUnifiedLine(
				key,
				row.left.lineNo,
				null,
				"-",
				row.left,
				originalGrid,
				"del",
				"bg-red-500/10",
			);
		}
		return renderUnifiedLine(
			key,
			null,
			row.right.lineNo,
			"+",
			row.right,
			modifiedGrid,
			"add",
			"bg-green-500/10",
		);
	};

	const renderRow = mode === "unified" ? renderUnifiedRow : renderSplitRow;

	const renderGap = (item: Extract<DiffItem, { kind: "gap" }>): ReactNode => {
		const expanded = expandedGaps.has(item.gapId);
		if (expanded) {
			return item.hiddenRows.map((row, idx) =>
				renderRow(row, `${item.gapId}-${idx}`),
			);
		}
		return (
			<button
				key={item.gapId}
				type="button"
				onClick={() => toggleGap(item.gapId)}
				style={{ gridColumn: "1 / -1" }}
				className="flex items-center justify-center gap-2 border-y border-border/40 bg-muted/40 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
			>
				<UnfoldVertical className="h-3 w-3" />
				{t("lengthUnchanged", "{{length}} unchanged", {
					length: item.hiddenRows.length,
				})}{" "}
				{item.hiddenRows.length === 1 ? "line" : "lines"}
			</button>
		);
	};

	const gridTemplate =
		mode === "unified"
			? showLineNumbers
				? `minmax(2.5rem,max-content) minmax(2.5rem,max-content) minmax(1.25rem,max-content) ${codeCol}`
				: `minmax(1.25rem,max-content) ${codeCol}`
			: showLineNumbers
				? `minmax(2.75rem,max-content) ${codeCol} minmax(2.75rem,max-content) ${codeCol}`
				: `${codeCol} ${codeCol}`;

	const body = (() => {
		if (effectiveKind === "document") {
			return (
				<div className="grid h-full min-h-[24rem] grid-cols-2 gap-px bg-border">
					{[
						{ url: left, label: swapSides ? modifiedLabel : originalLabel },
						{ url: right, label: swapSides ? originalLabel : modifiedLabel },
					].map((side) => (
						<div key={side.label} className="flex flex-col bg-card">
							<div className="border-b bg-muted/40 px-3 py-1.5 text-xs font-medium text-muted-foreground">
								{side.label}
							</div>
							<iframe
								src={side.url}
								title={side.label}
								sandbox="allow-scripts allow-same-origin allow-popups"
								className="h-full min-h-[20rem] w-full border-0"
							/>
						</div>
					))}
				</div>
			);
		}

		if (effectiveKind === "markdown" && mdMode === "rendered") {
			const blockBg = (
				block: RenderedBlock,
				content: string | null,
				tone: "del" | "add",
			): string => {
				if (content === null) return "bg-muted/20";
				const changed = block.type === "change";
				if (tone === "del" && (changed || block.type === "delete"))
					return "bg-red-500/10";
				if (tone === "add" && (changed || block.type === "insert"))
					return "bg-green-500/10";
				return "";
			};
			const seenKeys = new Map<string, number>();
			return (
				<div className="grid grid-cols-2" style={{ minWidth: "100%" }}>
					{renderedBlocks.map((block) => {
						const base = `${block.type}:${block.left ?? ""}:${block.right ?? ""}`;
						const occurrence = seenKeys.get(base) ?? 0;
						seenKeys.set(base, occurrence + 1);
						return (
							<div key={`${base}#${occurrence}`} className="contents">
								<div
									className={cn(
										"min-w-0 border-b border-border/40 p-3",
										blockBg(block, block.left, "del"),
									)}
								>
									{block.left !== null && (
										<div className="prose prose-sm dark:prose-invert max-w-none">
											<TextEditor
												initialContent={block.left}
												isMarkdown
												editable={false}
											/>
										</div>
									)}
								</div>
								<div
									className={cn(
										"min-w-0 border-b border-l border-border/40 p-3",
										blockBg(block, block.right, "add"),
									)}
								>
									{block.right !== null && (
										<div className="prose prose-sm dark:prose-invert max-w-none">
											<TextEditor
												initialContent={block.right}
												isMarkdown
												editable={false}
											/>
										</div>
									)}
								</div>
							</div>
						);
					})}
				</div>
			);
		}

		if (mode === "inline") {
			return (
				<div className="overflow-auto px-4 py-3 font-mono text-[13px] leading-[1.7] whitespace-pre-wrap break-words">
					{(() => {
						let offset = 0;
						return inlineSegments.map((segment) => {
							const key = offset;
							offset += segment.text.length;
							if (segment.kind === "common")
								return <span key={key}>{segment.text}</span>;
							if (segment.kind === "added")
								return (
									<span
										key={key}
										className="rounded-[2px] bg-green-500/20 text-green-700 dark:text-green-300"
									>
										{segment.text}
									</span>
								);
							return (
								<span
									key={key}
									className="rounded-[2px] bg-red-500/20 text-red-700 line-through dark:text-red-300"
								>
									{segment.text}
								</span>
							);
						});
					})()}
				</div>
			);
		}

		return (
			<div
				className="grid font-mono text-[13px] leading-[1.7]"
				style={{ gridTemplateColumns: gridTemplate, minWidth: "100%" }}
			>
				{items.map((item) =>
					item.kind === "gap"
						? renderGap(item)
						: renderRow(item.row, `row-${item.rowIndex}`),
				)}
			</div>
		);
	})();

	const isMarkdown = effectiveKind === "markdown";
	const renderedMarkdown = isMarkdown && mdMode === "rendered";
	const showHeaderStats =
		showStats && effectiveKind !== "document" && !renderedMarkdown;

	return (
		<div
			className={cn(
				"flex w-full flex-col overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm",
				className,
			)}
			style={style}
		>
			{showToolbar && (
				<div className="flex flex-wrap items-center justify-between gap-2 border-b bg-muted/40 px-3 py-2">
					<div className="flex min-w-0 items-center gap-2 text-sm">
						<FileDiff className="h-4 w-4 shrink-0 text-muted-foreground" />
						<span className="truncate font-medium">{originalLabel}</span>
						<span className="text-muted-foreground">→</span>
						<span className="truncate font-medium">{modifiedLabel}</span>
					</div>
					<div className="flex items-center gap-1.5">
						{showHeaderStats && (
							<div className="mr-1 flex items-center gap-2 font-mono text-xs">
								<span className="text-green-600 dark:text-green-400">{`+${stats.additions}`}</span>
								<span className="text-red-600 dark:text-red-400">{`−${stats.deletions}`}</span>
							</div>
						)}
						{!renderedMarkdown && (
							<>
								<div className="flex items-center rounded-md border bg-background p-0.5">
									{(
										[
											{
												id: "split",
												icon: Columns2,
												label: t("split", "Split"),
											},
											{
												id: "unified",
												icon: Rows3,
												label: t("unified", "Unified"),
											},
											{
												id: "inline",
												icon: Pilcrow,
												label: t("inline", "Inline"),
											},
										] as const
									).map((option) => (
										<button
											key={option.id}
											type="button"
											title={option.label}
											aria-label={option.label}
											onClick={() => setMode(option.id)}
											className={cn(
												"flex h-6 w-7 items-center justify-center rounded-sm transition-colors",
												mode === option.id
													? "bg-primary text-primary-foreground"
													: "text-muted-foreground hover:text-foreground",
											)}
										>
											<option.icon className="h-3.5 w-3.5" />
										</button>
									))}
								</div>
								<Button
									variant="ghost"
									size="icon"
									className={cn("h-7 w-7", wordWrap && "text-primary")}
									title={t("toggleWordWrap", "Toggle word wrap")}
									onClick={() => setWordWrap((value) => !value)}
								>
									<WrapText className="h-3.5 w-3.5" />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									className={cn("h-7 w-7", collapse && "text-primary")}
									title={t("collapseUnchanged", "Collapse unchanged")}
									onClick={() => setCollapse((value) => !value)}
								>
									{collapse ? (
										<UnfoldVertical className="h-3.5 w-3.5" />
									) : (
										<FoldVertical className="h-3.5 w-3.5" />
									)}
								</Button>
							</>
						)}
						{isMarkdown && (
							<Button
								variant="ghost"
								size="icon"
								className={cn("h-7 w-7", renderedMarkdown && "text-primary")}
								title={
									renderedMarkdown
										? t("showMarkdownSource", "Show markdown source")
										: "Render markdown"
								}
								onClick={() =>
									setMdMode((value) =>
										value === "rendered" ? "source" : "rendered",
									)
								}
							>
								{renderedMarkdown ? (
									<Code className="h-3.5 w-3.5" />
								) : (
									<Eye className="h-3.5 w-3.5" />
								)}
							</Button>
						)}
					</div>
				</div>
			)}
			<div className="min-h-0 flex-1 overflow-auto bg-card">{body}</div>
		</div>
	);
}
