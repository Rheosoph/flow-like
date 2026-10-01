"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactFlowInstance, Viewport } from "@xyflow/react";
import {
	ChevronLeftIcon,
	ChevronRightIcon,
	CodeIcon,
	Columns2Icon,
	FileCode2Icon,
	LayersIcon,
	Rows2Icon,
	WorkflowIcon,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	BoardChangeStatus,
	BoardEdgeStatus,
	IBoardChange,
	IBoardDiff,
} from "../../../lib/board-diff";
import type { IBoard } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { Switch } from "../../ui/switch";
import { BoardChangeList, useChangeGroups } from "./board-change-list";
import { BoardDiffCanvas, overlayBoard } from "./board-diff-canvas";
import { StatusGlyph, useStatusLabel } from "./diff-status";
import {
	FlowScriptDiff,
	type FlowScriptSource,
	flowScriptStats,
} from "./flowscript-diff";

type View = "board" | "flowscript" | "both";
type Mode = "overlay" | "side";
type Filter = "all" | Exclude<BoardChangeStatus, "moved">;

function Segmented<T extends string>({
	value,
	onChange,
	options,
	label,
}: Readonly<{
	value: T;
	onChange: (value: T) => void;
	options: { value: T; label: string; icon?: ReactNode }[];
	label: string;
}>) {
	return (
		<fieldset
			aria-label={label}
			className="inline-flex gap-0.5 rounded-lg bg-muted p-0.5"
		>
			{options.map((option) => (
				<button
					key={option.value}
					type="button"
					aria-pressed={value === option.value}
					onClick={() => onChange(option.value)}
					className={cn(
						"inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
						value === option.value &&
							"bg-background text-foreground shadow-sm ring-1 ring-border",
					)}
				>
					{option.icon}
					{option.label}
				</button>
			))}
		</fieldset>
	);
}

function ToggleSetting({
	id,
	checked,
	onChange,
	children,
}: Readonly<{
	id: string;
	checked: boolean;
	onChange: (v: boolean) => void;
	children: ReactNode;
}>) {
	return (
		<label
			htmlFor={id}
			className="inline-flex cursor-pointer select-none items-center gap-2 whitespace-nowrap text-xs text-muted-foreground"
		>
			<Switch
				id={id}
				checked={checked}
				onCheckedChange={onChange}
				className="scale-90"
			/>
			{children}
		</label>
	);
}

function canvasLayerOf(
	change: IBoardChange | undefined,
	head: IBoard,
	base: IBoard,
) {
	if (!change) return undefined;
	if (change.kind === "variable") {
		const first = change.focusIds[0];
		const node = first
			? (head.nodes?.[first] ?? base.nodes?.[first])
			: undefined;
		return node?.layer || undefined;
	}
	return change.layerId;
}

function sideStatuses(
	diff: IBoardDiff,
	side: "overlay" | "base" | "head",
	moves: boolean,
) {
	const out = new Map<string, BoardChangeStatus>();
	for (const [id, status] of diff.itemStatus) {
		if (status === "moved" && !moves) continue;
		if (side === "base" && status === "added") continue;
		if (side === "head" && status === "removed") continue;
		out.set(id, status);
	}
	return out;
}

function sideEdges(diff: IBoardDiff, side: "overlay" | "base" | "head") {
	if (side === "overlay") return diff.edgeStatus;
	const keep: BoardEdgeStatus = side === "base" ? "removed" : "added";
	const out = new Map<string, BoardEdgeStatus>();
	for (const [id, status] of diff.edgeStatus)
		if (status === keep) out.set(id, status);
	return out;
}

export interface BoardDiffWorkspaceProps {
	base: IBoard;
	head: IBoard;
	diff: IBoardDiff;
	baseLabel: string;
	headLabel: string;
	baseScript: FlowScriptSource;
	headScript: FlowScriptSource;
	renderMeta?: (change: IBoardChange) => ReactNode;
	initialKey?: string;
	initialView?: View;
}

export function BoardDiffWorkspace({
	base,
	head,
	diff,
	baseLabel,
	headLabel,
	baseScript,
	headScript,
	renderMeta,
	initialKey,
	initialView = "both",
}: Readonly<BoardDiffWorkspaceProps>) {
	const { t } = useTranslation("flow");
	const statusLabel = useStatusLabel();
	const [view, setView] = useState<View>(initialView);
	const [mode, setMode] = useState<Mode>("overlay");
	const [filter, setFilter] = useState<Filter>("all");
	const [moves, setMoves] = useState(false);
	const [dim, setDim] = useState(true);
	const [codeLayout, setCodeLayout] = useState<"unified" | "split">("unified");
	const [ignoreWhitespace, setIgnoreWhitespace] = useState(true);
	const [selectedKey, setSelectedKey] = useState<string | undefined>(
		initialKey,
	);
	const [layerId, setLayerId] = useState<string | undefined>();
	const [focusRequest, setFocusRequest] = useState(0);
	const listRef = useRef<HTMLDivElement>(null);
	const instances = useRef<{
		base: ReactFlowInstance | null;
		head: ReactFlowInstance | null;
	}>({ base: null, head: null });

	const visible = useMemo(
		() =>
			diff.changes.filter(
				(c) =>
					(moves || c.status !== "moved") &&
					(filter === "all" || c.status === filter),
			),
		[diff.changes, filter, moves],
	);
	const groups = useChangeGroups(visible, head, base);
	const ordered = useMemo(() => groups.flatMap((g) => g.changes), [groups]);
	const selected = selectedKey ? diff.byKey.get(selectedKey) : undefined;

	const select = useCallback(
		(change: IBoardChange | undefined) => {
			setSelectedKey(change?.key);
			setLayerId(canvasLayerOf(change, head, base));
			setFocusRequest((n) => n + 1);
		},
		[base, head],
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: pick a starting change once per comparison
	useEffect(() => {
		const start = (initialKey && diff.byKey.get(initialKey)) || ordered[0];
		select(start);
	}, [diff]);

	useEffect(() => {
		if (selectedKey && !visible.some((c) => c.key === selectedKey))
			select(ordered[0]);
	}, [ordered, select, selectedKey, visible]);

	const position = ordered.findIndex((c) => c.key === selectedKey);
	const step = useCallback(
		(delta: number) => {
			const next =
				ordered[Math.min(ordered.length - 1, Math.max(0, position + delta))];
			if (next && next.key !== selectedKey) select(next);
		},
		[ordered, position, select, selectedKey],
	);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			const target = event.target as HTMLElement | null;
			if (target?.closest("input, textarea, select, [contenteditable=true]"))
				return;
			if (event.metaKey || event.ctrlKey || event.altKey) return;
			if (event.key === "j") step(1);
			if (event.key === "k") step(-1);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [step]);

	useEffect(() => {
		if (!selectedKey) return;
		const element = listRef.current?.querySelector<HTMLElement>(
			`[data-change-key="${CSS.escape(selectedKey)}"]`,
		);
		element?.scrollIntoView({ block: "nearest" });
	}, [selectedKey]);

	const changesById = useMemo(() => {
		const map = new Map<string, IBoardChange>();
		for (const change of diff.changes) {
			if (
				change.kind === "node" ||
				change.kind === "layer" ||
				change.kind === "comment"
			)
				map.set(change.id, change);
		}
		return map;
	}, [diff.changes]);

	const selectById = useCallback(
		(id: string) => {
			const change =
				changesById.get(id) ??
				diff.changes.find((c) => c.focusIds.includes(id));
			if (!change) return;
			if (change.status === "moved" && !moves) setMoves(true);
			if (filter !== "all" && change.status !== filter) setFilter("all");
			setSelectedKey(change.key);
			setFocusRequest((n) => n + 1);
		},
		[changesById, diff.changes, filter, moves],
	);

	const overlay = useMemo(
		() => overlayBoard(base, head, diff.edgeStatus),
		[base, diff.edgeStatus, head],
	);
	const statuses = useMemo(
		() => ({
			overlay: sideStatuses(diff, "overlay", moves),
			base: sideStatuses(diff, "base", moves),
			head: sideStatuses(diff, "head", moves),
		}),
		[diff, moves],
	);
	const edges = useMemo(
		() => ({
			overlay: sideEdges(diff, "overlay"),
			base: sideEdges(diff, "base"),
			head: sideEdges(diff, "head"),
		}),
		[diff],
	);
	const selectedIds = useMemo(
		() =>
			selected
				? selected.focusIds.length
					? selected.focusIds
					: [selected.id]
				: [],
		[selected],
	);
	const stats = useMemo(
		() => flowScriptStats(baseScript.text, headScript.text, ignoreWhitespace),
		[baseScript.text, headScript.text, ignoreWhitespace],
	);

	const syncFrom = useCallback(
		(source: "base" | "head") => (viewport: Viewport) => {
			const other = instances.current[source === "base" ? "head" : "base"];
			void other?.setViewport(viewport);
		},
		[],
	);

	const canvasProps = {
		inner: diff.layersWithChanges,
		changes: changesById,
		layerId,
		onLayerChange: setLayerId,
		selectedIds,
		focusRequest,
		onSelectId: selectById,
		dim,
	};

	const counts = diff.counts;
	const filters: { value: Filter; label: string; count: number }[] = [
		{
			value: "all",
			label: t("boardDiffFilterAll", "All"),
			count:
				counts.added +
				counts.changed +
				counts.removed +
				(moves ? counts.moved : 0),
		},
		{ value: "added", label: statusLabel("added"), count: counts.added },
		{ value: "changed", label: statusLabel("changed"), count: counts.changed },
		{ value: "removed", label: statusLabel("removed"), count: counts.removed },
	];

	const sideTag = (kind: "base" | "head", text: string) => (
		<span className="inline-flex min-w-0 items-center gap-1.5 truncate font-medium">
			<span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
				{kind === "base"
					? t("boardDiffBase", "Base")
					: t("boardDiffCompare", "Compare")}
			</span>
			<span className="truncate">{text}</span>
		</span>
	);

	const boardPane = (
		<div className="relative min-h-0">
			{mode === "overlay" ? (
				<BoardDiffCanvas
					key="overlay"
					board={overlay}
					statuses={statuses.overlay}
					edgeStatus={edges.overlay}
					detailSide="both"
					{...canvasProps}
				/>
			) : (
				<div className="grid h-full grid-cols-1 divide-y md:grid-cols-2 md:divide-x md:divide-y-0">
					<BoardDiffCanvas
						key="base"
						board={base}
						statuses={statuses.base}
						edgeStatus={edges.base}
						detailSide="before"
						tag={sideTag("base", baseLabel)}
						onInstance={(instance) => {
							instances.current.base = instance;
						}}
						onUserMove={syncFrom("base")}
						{...canvasProps}
					/>
					<BoardDiffCanvas
						key="head"
						board={head}
						statuses={statuses.head}
						edgeStatus={edges.head}
						detailSide="after"
						tag={sideTag("head", headLabel)}
						onInstance={(instance) => {
							instances.current.head = instance;
						}}
						onUserMove={syncFrom("head")}
						{...canvasProps}
					/>
				</div>
			)}
			{mode === "overlay" && (
				<div className="pointer-events-none absolute right-3 top-3 z-10 hidden gap-3 rounded-md border bg-popover/95 px-2.5 py-1 text-[11px] text-muted-foreground shadow-sm sm:flex">
					{(["added", "changed", "removed", "moved"] as const).map((status) => (
						<span key={status} className="inline-flex items-center gap-1.5">
							<StatusGlyph status={status} className="size-3.5 text-[10px]" />
							{statusLabel(status)}
						</span>
					))}
				</div>
			)}
		</div>
	);

	const codePane = (
		<div className="flex min-h-0 flex-col">
			<div className="flex flex-wrap items-center gap-3 border-b bg-background px-3 py-1.5 text-xs">
				<span className="inline-flex items-center gap-1.5 font-mono font-medium">
					<FileCode2Icon className="size-3.5 text-primary" />
					{head.name || base.name}
				</span>
				{stats && (
					<span className="font-mono">
						<span className="text-emerald-600 dark:text-emerald-400">
							+{stats.additions}
						</span>{" "}
						<span className="text-destructive">−{stats.deletions}</span>
					</span>
				)}
				<span className="flex-1" />
				<ToggleSetting
					id="board-diff-whitespace"
					checked={ignoreWhitespace}
					onChange={setIgnoreWhitespace}
				>
					{t("boardDiffIgnoreWhitespace", "Ignore whitespace")}
				</ToggleSetting>
				<Segmented
					label={t("boardDiffCodeLayout", "Diff layout")}
					value={codeLayout}
					onChange={setCodeLayout}
					options={[
						{ value: "unified", label: t("boardDiffUnified", "Unified") },
						{ value: "split", label: t("boardDiffSplit", "Split") },
					]}
				/>
			</div>
			<div className="min-h-0 flex-1">
				<FlowScriptDiff
					base={baseScript}
					head={headScript}
					layout={codeLayout}
					ignoreWhitespace={ignoreWhitespace}
					selectedIds={selectedIds}
					scrollRequest={focusRequest}
					onSelectId={selectById}
				/>
			</div>
		</div>
	);

	return (
		<div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] md:grid-cols-[minmax(260px,320px)_minmax(0,1fr)] md:grid-rows-1">
			<aside className="flex max-h-72 min-h-0 flex-col border-b md:max-h-none md:border-b-0 md:border-r">
				<div className="flex flex-col gap-2.5 border-b px-3 py-2.5">
					<fieldset
						aria-label={t("boardDiffFilter", "Filter changes")}
						className="flex flex-wrap gap-1"
					>
						{filters.map((option) => (
							<button
								key={option.value}
								type="button"
								disabled={option.value !== "all" && option.count === 0}
								aria-pressed={filter === option.value}
								onClick={() => setFilter(option.value)}
								className={cn(
									"inline-flex h-7 items-center gap-1.5 rounded-md border border-transparent px-2 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40",
									filter === option.value &&
										"border-border bg-card text-foreground",
								)}
							>
								{option.value !== "all" && (
									<StatusGlyph
										status={option.value}
										className="size-4 text-[10px]"
									/>
								)}
								{option.label}
								<span className="font-semibold tabular-nums">
									{option.count}
								</span>
							</button>
						))}
					</fieldset>
					<ToggleSetting
						id="board-diff-moves"
						checked={moves}
						onChange={setMoves}
					>
						{t("boardDiffShowMoves", "Show layout moves")}
						<span className="font-medium tabular-nums">{counts.moved}</span>
					</ToggleSetting>
				</div>
				<div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
					<BoardChangeList
						groups={groups}
						selectedKey={selectedKey}
						onSelect={select}
						renderMeta={renderMeta}
						emptyText={
							diff.changes.length
								? t("boardDiffNothingMatches", "Nothing matches this filter.")
								: t(
										"boardDiffIdentical",
										"These two states of the board are identical.",
									)
						}
					/>
				</div>
			</aside>
			<section className="flex min-h-0 min-w-0 flex-col">
				<div className="flex flex-wrap items-center gap-2.5 border-b px-3 py-2">
					<Segmented
						label={t("boardDiffView", "View")}
						value={view}
						onChange={setView}
						options={[
							{
								value: "board",
								label: t("boardDiffViewBoard", "Board"),
								icon: <WorkflowIcon className="size-3.5" />,
							},
							{
								value: "flowscript",
								label: "FlowScript",
								icon: <CodeIcon className="size-3.5" />,
							},
							{
								value: "both",
								label: t("boardDiffViewBoth", "Both"),
								icon: <Rows2Icon className="size-3.5" />,
							},
						]}
					/>
					{view !== "flowscript" && (
						<>
							<Segmented
								label={t("boardDiffMode", "Board comparison")}
								value={mode}
								onChange={setMode}
								options={[
									{
										value: "overlay",
										label: t("boardDiffOverlay", "Overlay"),
										icon: <LayersIcon className="size-3.5" />,
									},
									{
										value: "side",
										label: t("boardDiffSideBySide", "Side by side"),
										icon: <Columns2Icon className="size-3.5" />,
									},
								]}
							/>
							<ToggleSetting
								id="board-diff-dim"
								checked={dim}
								onChange={setDim}
							>
								{t("boardDiffDimUnchanged", "Dim unchanged")}
							</ToggleSetting>
						</>
					)}
					<span className="flex-1" />
					<div className="flex items-center gap-1 text-xs text-muted-foreground">
						<Button
							variant="ghost"
							size="icon"
							className="size-7"
							disabled={position <= 0}
							onClick={() => step(-1)}
							aria-label={t("boardDiffPrevious", "Previous change")}
							title={t("boardDiffPreviousKey", "Previous change (K)")}
						>
							<ChevronLeftIcon className="size-4" />
						</Button>
						<span className="min-w-14 text-center tabular-nums">
							{ordered.length
								? t("boardDiffPosition", "{{index}} of {{count}}", {
										index: position + 1,
										count: ordered.length,
									})
								: t("boardDiffPositionNone", "0 of 0")}
						</span>
						<Button
							variant="ghost"
							size="icon"
							className="size-7"
							disabled={position < 0 || position >= ordered.length - 1}
							onClick={() => step(1)}
							aria-label={t("boardDiffNext", "Next change")}
							title={t("boardDiffNextKey", "Next change (J)")}
						>
							<ChevronRightIcon className="size-4" />
						</Button>
					</div>
				</div>
				<div
					className={cn(
						"grid min-h-0 flex-1",
						view === "both"
							? "grid-rows-[minmax(0,1.15fr)_minmax(0,1fr)] divide-y"
							: "grid-rows-1",
					)}
				>
					{view !== "flowscript" && boardPane}
					{view !== "board" && codePane}
				</div>
			</section>
		</div>
	);
}
