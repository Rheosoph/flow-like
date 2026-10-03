"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type UseQueryOptions,
	type UseQueryResult,
	useQueries,
} from "@tanstack/react-query";
import { Check, ChevronDown, FileText, Loader2, Search } from "lucide-react";
import {
	type KeyboardEvent,
	memo,
	useCallback,
	useDeferredValue,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { useSearch, useSearchIndex } from "../../hooks/use-search-index";
import { formatEventTypeLabel } from "../../lib/event-type-label";
import { asArray, isRecord } from "../../lib/response-shape";
import type { IBoard, IExecutionMode } from "../../lib/schema/flow/board";
import type {
	IBoardEntryNode,
	IBoardSummary,
} from "../../lib/schema/flow/board-summary";
import type { BoardVersion } from "../../lib/schema/flow/board-version";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { PageListItem } from "../../state/backend-state/page-state";
import type { IEventMapping } from "../interfaces/interfaces";
import { Button } from "./button";
import {
	Command,
	CommandEmpty,
	CommandInput,
	CommandItem,
	CommandList,
} from "./command";
import { Input } from "./input";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

export interface EventStartSelection {
	boardId: string;
	nodeId?: string;
	pageId?: string;
	boardVersion?: BoardVersion;
}

export interface EventStartTarget extends EventStartSelection {
	name: string;
	nodeType?: string;
	boardExecutionMode?: IExecutionMode;
}

export interface EventStartPickerProps {
	appId: string;
	eventConfig: IEventMapping;
	selected: EventStartSelection;
	onSelect(target: EventStartTarget): void;
	disabled?: boolean;
	/** Diagnostics hook, called with the row key whenever a row renders. */
	onRowRender?: (key: string) => void;
}

interface StartRow extends EventStartTarget {
	key: string;
	flowName: string;
	kind: string;
	label: string;
	unavailable?: boolean;
}

interface StartFlow {
	id: string;
	name: string;
}

interface BoardGraph {
	executionMode?: IExecutionMode;
	entries: IBoardEntryNode[];
}

interface ViewGroup {
	key: string;
	name: string;
	total: number;
	shown: StartRow[];
	pending: boolean;
	loadable: boolean;
}

interface StartView {
	groups: ViewGroup[];
	hiddenGroups: number;
}

interface SelectedKeys {
	boardId: string;
	nodeId?: string;
	pageId?: string;
}

const GROUP_PAGE_SIZE = 50;
const TOTAL_ROW_CAP = 150;
const FLOW_OPTION_CAP = 100;
const ROW_FIELDS = { fields: ["label", "flowName", "kind"] } as const;
const FLOW_FIELDS = { fields: ["name"] } as const;
const collator = new Intl.Collator(undefined, {
	numeric: true,
	sensitivity: "base",
});

function toGraph(board: IBoard): BoardGraph {
	return {
		executionMode: board.execution_mode,
		entries: isRecord(board.nodes)
			? Object.values(board.nodes)
					.filter((node) => node.start)
					.map((node) => ({
						nodeId: node.id,
						nodeType: node.name,
						friendlyName: node.friendly_name,
					}))
			: [],
	};
}

function combineReads(results: UseQueryResult<BoardGraph, Error>[]) {
	return results.map(({ data, isError, isLoading, refetch }) => ({
		data,
		isError,
		isLoading,
		refetch,
	}));
}

function isSelected(row: StartRow, selected: SelectedKeys): boolean {
	return row.pageId
		? row.pageId === selected.pageId
		: !selected.pageId &&
				row.boardId === selected.boardId &&
				row.nodeId === selected.nodeId;
}

function toTarget(row: StartRow): EventStartTarget {
	return {
		boardId: row.boardId,
		nodeId: row.nodeId,
		pageId: row.pageId,
		boardVersion: row.boardVersion,
		name: row.name,
		nodeType: row.nodeType,
		boardExecutionMode: row.boardExecutionMode,
	};
}

function isLegacy(summary: IBoardSummary): boolean {
	return !Array.isArray(summary.entryNodes);
}

export function EventStartPicker({
	appId,
	eventConfig,
	selected,
	onSelect,
	disabled = false,
	onRowRender,
}: Readonly<EventStartPickerProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const id = useId();
	const listRef = useRef<HTMLDivElement>(null);
	const onSelectRef = useRef(onSelect);
	onSelectRef.current = onSelect;
	const [search, setSearch] = useState("");
	const [flow, setFlow] = useState("");
	const [expanded, setExpanded] = useState<readonly string[]>([]);
	const [limits, setLimits] = useState<Readonly<Record<string, number>>>({});
	const [activeKey, setActiveKey] = useState<string | null>(null);
	const deferredSearch = useDeferredValue(search);
	const summaries = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId, ["node_types"]],
		!!appId,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		!!appId,
	);
	const flows = useMemo(
		() => asArray<IBoardSummary>(summaries.data),
		[summaries.data],
	);
	const pageList = useMemo(
		() => asArray<PageListItem>(pages.data),
		[pages.data],
	);
	const {
		boardId: selectedBoard,
		nodeId: selectedNode,
		pageId: selectedPage,
		boardVersion: selectedVersion,
	} = selected;
	const versionKey = selectedVersion?.join(".");
	const pinnedVersion = useMemo(
		() =>
			versionKey
				? (versionKey.split(".").map(Number) as BoardVersion)
				: undefined,
		[versionKey],
	);

	const legacy = useMemo(
		() => new Set(flows.filter(isLegacy).map((item) => item.id)),
		[flows],
	);
	// Older summaries have no entry nodes: their graph loads on demand. A pinned flow always needs its exact graph.
	const loadIds = useMemo(() => {
		const ids = new Set(expanded.filter((item) => legacy.has(item)));
		if (flow && legacy.has(flow)) ids.add(flow);
		const known = flows.some((item) => item.id === selectedBoard);
		if (known && (versionKey || legacy.has(selectedBoard)))
			ids.add(selectedBoard);
		return [...ids];
	}, [flows, legacy, expanded, flow, selectedBoard, versionKey]);
	const queries: UseQueryOptions<IBoard, Error, BoardGraph>[] = loadIds.map(
		(boardId) => {
			const version = boardId === selectedBoard ? pinnedVersion : undefined;
			return {
				queryKey: [
					backend.boardState.getBoard.name || "backendFn",
					appId,
					boardId,
					...(version ? [version] : []),
				],
				queryFn: () => backend.boardState.getBoard(appId, boardId, version),
				select: toGraph,
				enabled: !!appId,
			};
		},
	);
	const reads = useQueries({ queries, combine: combineReads });
	const graphs = useMemo(() => {
		const map = new Map<string, BoardGraph>();
		loadIds.forEach((boardId, index) => {
			const data = reads[index]?.data;
			if (data) map.set(boardId, data);
		});
		return map;
	}, [loadIds, reads]);

	const customStart = t("eventStartPicker.customStart", "Custom start");
	const pageKind = t("page", "Page");
	const pagesGroup = t("pages", "Pages");
	const rows = useMemo(
		() =>
			buildRows({
				flows,
				pages: pageList,
				graphs,
				pinnedBoardId: selectedBoard,
				pinnedVersion,
				eventConfig,
				customStart,
				pageKind,
				pagesGroup,
			}),
		[
			flows,
			pageList,
			graphs,
			selectedBoard,
			pinnedVersion,
			eventConfig,
			customStart,
			pageKind,
			pagesGroup,
		],
	);
	const flowOptions = useMemo<StartFlow[]>(
		() =>
			flows
				.map(({ id: flowId, name }) => ({ id: flowId, name }))
				.sort(
					(a, b) =>
						collator.compare(a.name, b.name) || collator.compare(a.id, b.id),
				),
		[flows],
	);
	const index = useSearchIndex(rows, ROW_FIELDS);
	const ranked = useMemo(
		() => index.search(deferredSearch),
		[index, deferredSearch],
	);
	const needle = deferredSearch.trim().toLocaleLowerCase();
	const filtering = needle !== "" || flow !== "";
	const matched = useMemo(
		() => (flow ? ranked.filter((row) => row.boardId === flow) : ranked),
		[ranked, flow],
	);
	const pendingFlows = useMemo(() => {
		const loadable = new Set(loadIds);
		return flowOptions.filter(
			(item) =>
				legacy.has(item.id) &&
				!loadable.has(item.id) &&
				(!flow || item.id === flow),
		);
	}, [flowOptions, legacy, loadIds, flow]);
	const view = useMemo(
		() => buildView(matched, pendingFlows, limits, needle),
		[matched, pendingFlows, limits, needle],
	);
	const selectedKeys = useMemo<SelectedKeys>(
		() => ({
			boardId: selectedBoard,
			nodeId: selectedNode,
			pageId: selectedPage,
		}),
		[selectedBoard, selectedNode, selectedPage],
	);
	const chosen = useMemo(
		() => rows.find((row) => isSelected(row, selectedKeys)),
		[rows, selectedKeys],
	);
	const tabKey = useMemo(() => {
		const focusable = view.groups
			.flatMap((group) => group.shown)
			.filter((row) => !row.unavailable)
			.map((row) => row.key);
		if (activeKey && focusable.includes(activeKey)) return activeKey;
		if (chosen && focusable.includes(chosen.key)) return chosen.key;
		return focusable[0];
	}, [activeKey, chosen, view]);

	const problems: { key: string; message: string; retry(): void }[] = [];
	if (summaries.isError)
		problems.push({
			key: "flows",
			message: t("eventStartPicker.flowsError", "Could not load flows."),
			retry: () => void summaries.refetch(),
		});
	if (pages.isError)
		problems.push({
			key: "pages",
			message: t("eventStartPicker.pagesError", "Could not load pages."),
			retry: () => void pages.refetch(),
		});
	loadIds.forEach((boardId, position) => {
		const read = reads[position];
		if (!read?.isError) return;
		problems.push({
			key: boardId,
			message: t(
				"eventStartPicker.boardError",
				"Could not load start nodes in {{flow}}.",
				{ flow: flowOptions.find((item) => item.id === boardId)?.name ?? "" },
			),
			retry: () => void read.refetch(),
		});
	});
	const loading =
		summaries.isLoading ||
		pages.isLoading ||
		reads.some((read) => read.isLoading);

	const pick = useCallback(
		(row: StartRow) => onSelectRef.current(toTarget(row)),
		[],
	);
	const changeSearch = useCallback((value: string) => {
		setSearch(value);
		setLimits({});
	}, []);
	const changeFlow = useCallback((flowId: string) => {
		setFlow(flowId);
		setLimits({});
	}, []);
	const showMore = useCallback(
		(groupKey: string, shown: number) =>
			setLimits((current) => ({
				...current,
				[groupKey]: shown + GROUP_PAGE_SIZE,
			})),
		[],
	);
	const loadFlow = useCallback(
		(flowId: string) =>
			setExpanded((current) =>
				current.includes(flowId) ? current : [...current, flowId],
			),
		[],
	);
	const moveFocus = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
		if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
		const target = event.target as HTMLElement;
		if (!target.matches("[data-start-row]")) return;
		const available = Array.from(
			listRef.current?.querySelectorAll<HTMLButtonElement>(
				"[data-start-row]",
			) ?? [],
		).filter((element) => !element.disabled);
		const current = available.indexOf(target as HTMLButtonElement);
		if (current < 0) return;
		const next =
			event.key === "Home"
				? 0
				: event.key === "End"
					? available.length - 1
					: Math.min(
							Math.max(current + (event.key === "ArrowDown" ? 1 : -1), 0),
							available.length - 1,
						);
		event.preventDefault();
		available[next]?.focus();
	}, []);

	const count =
		loading || problems.length
			? t("eventStartPicker.loadedCount", "{{count}} loaded", {
					count: rows.length,
				})
			: filtering
				? t("eventStartPicker.filteredCount", "{{shown}} of {{count}}", {
						shown: matched.length,
						count: rows.length,
					})
				: t("eventStartPicker.totalCount", "{{rows}} in {{count}} flows", {
						count: flows.length,
						rows: rows.length,
						defaultValue_one: "{{rows}} in {{count}} flow",
					});

	return (
		<div className="space-y-2">
			<div className="flex items-baseline justify-between gap-3">
				<span id={`${id}-label`} className="text-sm font-medium">
					{t("eventStartPicker.title", "Start node")}
				</span>
				<span className="text-xs text-muted-foreground" aria-live="polite">
					{count}
				</span>
			</div>
			<div className="flex gap-2">
				<div className="relative min-w-0 flex-1">
					<Search
						className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground"
						aria-hidden="true"
					/>
					<Input
						type="search"
						aria-label={t(
							"eventStartPicker.searchPrompt",
							"Search start nodes, pages and flows",
						)}
						placeholder={t(
							"eventStartPicker.searchPrompt",
							"Search start nodes, pages and flows",
						)}
						className="h-10 pl-9"
						value={search}
						onChange={(event) => changeSearch(event.target.value)}
						disabled={disabled}
					/>
				</div>
				<FlowFilter
					flows={flowOptions}
					value={flow}
					onChange={changeFlow}
					disabled={disabled}
				/>
			</div>
			<div
				ref={listRef}
				role="radiogroup"
				aria-labelledby={`${id}-label`}
				aria-busy={loading || deferredSearch !== search}
				onKeyDown={moveFocus}
				className="max-h-[45vh] min-h-[min(280px,45vh)] overflow-y-auto rounded-md border bg-background p-1"
			>
				{view.groups.map((group) => (
					<div key={group.key || "pages"}>
						<div className="sticky top-0 z-10 flex items-center justify-between bg-background px-3 py-2 text-xs text-muted-foreground">
							<span className="truncate font-medium">{group.name}</span>
							{!group.pending && <span>{group.total}</span>}
						</div>
						{!group.pending && (
							<div className="[content-visibility:auto] [contain-intrinsic-size:auto_260px]">
								{group.shown.map((row) => (
									<StartRowItem
										key={row.key}
										row={row}
										checked={isSelected(row, selectedKeys)}
										tabbable={row.key === tabKey}
										disabled={disabled}
										unavailableLabel={t(
											"eventStartPicker.pageUnavailable",
											"Page unavailable",
										)}
										onPick={pick}
										onFocusRow={setActiveKey}
										onRender={onRowRender}
									/>
								))}
								{group.shown.length < group.total && (
									<div className="px-3 pb-2">
										<Button
											type="button"
											variant="ghost"
											size="sm"
											disabled={disabled}
											onClick={() => showMore(group.key, group.shown.length)}
										>
											{t("eventStartPicker.showMore", "Show {{count}} more", {
												count: Math.min(
													GROUP_PAGE_SIZE,
													group.total - group.shown.length,
												),
											})}
										</Button>
									</div>
								)}
							</div>
						)}
						{group.loadable && (
							<div className="px-3 pb-2">
								<Button
									type="button"
									variant="outline"
									size="sm"
									disabled={disabled}
									aria-label={t(
										"eventStartPicker.loadFlowNamed",
										"Load start nodes in {{flow}}",
										{ flow: group.name },
									)}
									onClick={() => loadFlow(group.key)}
								>
									{t("eventStartPicker.loadFlow", "Load start nodes")}
								</Button>
							</div>
						)}
					</div>
				))}
				{view.hiddenGroups > 0 && (
					<p className="px-3 py-3 text-center text-xs text-muted-foreground">
						{t(
							"eventStartPicker.capped",
							"{{count}} more flows not shown. Search or pick a flow to see them.",
							{
								count: view.hiddenGroups,
								defaultValue_one:
									"{{count}} more flow not shown. Search or pick a flow to see it.",
							},
						)}
					</p>
				)}
				{loading && (
					<output className="flex items-center justify-center gap-2 px-3 py-4 text-xs text-muted-foreground">
						<Loader2 aria-hidden="true" className="size-4 animate-spin" />
						{t("eventStartPicker.loading", "Loading start nodes and pages…")}
					</output>
				)}
				{!loading && view.groups.length === 0 && (
					<div className="px-3 py-6 text-center text-sm text-muted-foreground">
						{problems.length
							? t(
									"eventStartPicker.incomplete",
									"No matches in the available results. Retry the failed reads below.",
								)
							: filtering
								? t(
										"eventStartPicker.noMatches",
										"No start node or page matches.",
									)
								: t(
										"eventStartPicker.empty",
										"Add a start node or page to a flow to create an event.",
									)}
						{filtering && (
							<Button
								type="button"
								variant="link"
								size="sm"
								className="mx-auto block"
								disabled={disabled}
								onClick={() => {
									changeSearch("");
									changeFlow("");
								}}
							>
								{t("eventStartPicker.clear", "Clear search and filter")}
							</Button>
						)}
					</div>
				)}
			</div>
			{problems.map((problem) => (
				<div
					key={problem.key}
					role="alert"
					className="flex items-center justify-between gap-2 rounded-md border border-destructive/25 bg-destructive/5 px-3 py-2 text-xs"
				>
					<span>{problem.message}</span>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={disabled}
						onClick={problem.retry}
					>
						{t("retry", "Retry")}
					</Button>
				</div>
			))}
			{chosen && (
				<p className="text-xs text-muted-foreground">
					{t("eventStartPicker.chosen", "Chosen")}{" "}
					<span className="text-foreground">
						{chosen.flowName} <span aria-hidden="true">›</span> {chosen.label}
					</span>
				</p>
			)}
		</div>
	);
}

interface StartRowItemProps {
	row: StartRow;
	checked: boolean;
	tabbable: boolean;
	disabled: boolean;
	unavailableLabel: string;
	onPick(row: StartRow): void;
	onFocusRow(key: string): void;
	onRender?: (key: string) => void;
}

const StartRowItem = memo(function StartRowItem({
	row,
	checked,
	tabbable,
	disabled,
	unavailableLabel,
	onPick,
	onFocusRow,
	onRender,
}: Readonly<StartRowItemProps>) {
	onRender?.(row.key);
	const inactive = disabled || row.unavailable;
	return (
		<button
			type="button"
			// biome-ignore lint/a11y/useSemanticElements: arrow keys must move focus without selecting, which a native radio group cannot do
			role="radio"
			aria-checked={checked}
			aria-label={`${row.flowName}: ${row.label}`}
			data-start-row=""
			tabIndex={tabbable ? 0 : -1}
			disabled={inactive}
			onClick={() => onPick(row)}
			onFocus={() => onFocusRow(row.key)}
			className={cn(
				"flex min-h-11 w-full cursor-pointer items-center gap-3 rounded-md px-3 py-2 text-left outline-none [content-visibility:auto] [contain-intrinsic-size:auto_52px] hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
				checked && "bg-primary/8 hover:bg-primary/10",
				inactive && "cursor-not-allowed opacity-50",
			)}
		>
			<span
				aria-hidden="true"
				className={cn(
					"flex size-4 shrink-0 items-center justify-center rounded-full border border-muted-foreground/50",
					checked && "border-primary",
				)}
			>
				{checked && <span className="size-2 rounded-full bg-primary" />}
			</span>
			<span className="min-w-0 flex-1">
				<span
					className={cn("block truncate text-sm", checked && "font-medium")}
				>
					{row.label}
				</span>
				<span className="block truncate text-xs text-muted-foreground">
					{row.unavailable ? unavailableLabel : row.kind}
				</span>
			</span>
			{row.pageId && (
				<FileText aria-hidden="true" className="size-4 text-muted-foreground" />
			)}
		</button>
	);
});

function FlowFilter({
	flows,
	value,
	onChange,
	disabled,
}: Readonly<{
	flows: StartFlow[];
	value: string;
	onChange(flowId: string): void;
	disabled: boolean;
}>) {
	const { t } = useTranslation("common");
	const [open, setOpen] = useState(false);
	const allFlows = t("eventStartPicker.allFlows", "All flows");
	const current = flows.find((item) => item.id === value)?.name ?? allFlows;
	const choose = useCallback(
		(flowId: string) => {
			onChange(flowId);
			setOpen(false);
		},
		[onChange],
	);
	return (
		<Popover open={open} onOpenChange={setOpen} modal={false}>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="outline"
					// biome-ignore lint/a11y/useSemanticElements: the trigger opens a searchable popover list, which a native select cannot host
					role="combobox"
					aria-haspopup="listbox"
					aria-expanded={open}
					aria-label={`${t("eventStartPicker.flowFilter", "Filter by flow")}: ${current}`}
					disabled={disabled}
					className="h-10 w-44 max-w-[40%] shrink-0 justify-between px-3 font-normal"
				>
					<span className="truncate">{current}</span>
					<ChevronDown
						className="size-4 text-muted-foreground"
						aria-hidden="true"
					/>
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-72 p-0">
				<FlowOptions
					flows={flows}
					value={value}
					allFlows={allFlows}
					onChoose={choose}
				/>
			</PopoverContent>
		</Popover>
	);
}

function FlowOptions({
	flows,
	value,
	allFlows,
	onChoose,
}: Readonly<{
	flows: StartFlow[];
	value: string;
	allFlows: string;
	onChoose(flowId: string): void;
}>) {
	const { t } = useTranslation("common");
	const [query, setQuery] = useState("");
	const deferred = useDeferredValue(query);
	const matches = useSearch(flows, deferred, FLOW_FIELDS);
	const listed = matches.slice(0, FLOW_OPTION_CAP);
	return (
		<Command shouldFilter={false}>
			<CommandInput
				value={query}
				onValueChange={setQuery}
				placeholder={t("eventStartPicker.searchFlows", "Search flows")}
				className="h-9 text-sm"
			/>
			<CommandList className="max-h-64">
				<CommandEmpty className="text-sm text-muted-foreground">
					{t("eventStartPicker.noFlows", "No flow matches.")}
				</CommandEmpty>
				{!deferred.trim() && (
					<CommandItem value="" onSelect={() => onChoose("")}>
						<Check
							aria-hidden="true"
							className={cn("size-4", value ? "opacity-0" : "opacity-100")}
						/>
						<span className="truncate">{allFlows}</span>
					</CommandItem>
				)}
				{listed.map((item) => (
					<CommandItem
						key={item.id}
						value={item.id}
						onSelect={() => onChoose(item.id)}
					>
						<Check
							aria-hidden="true"
							className={cn(
								"size-4",
								value === item.id ? "opacity-100" : "opacity-0",
							)}
						/>
						<span className="truncate">{item.name}</span>
					</CommandItem>
				))}
				{matches.length > listed.length && (
					<p className="px-3 py-2 text-xs text-muted-foreground">
						{t(
							"eventStartPicker.moreFlows",
							"{{count}} more flows. Type to narrow the list.",
							{ count: matches.length - listed.length },
						)}
					</p>
				)}
			</CommandList>
		</Command>
	);
}

interface RowSources {
	flows: IBoardSummary[];
	pages: PageListItem[];
	graphs: ReadonlyMap<string, BoardGraph>;
	pinnedBoardId: string;
	pinnedVersion?: BoardVersion;
	eventConfig: IEventMapping;
	customStart: string;
	pageKind: string;
	pagesGroup: string;
}

function buildRows(sources: RowSources): StartRow[] {
	const rows: StartRow[] = [];
	const byId = new Map<string, IBoardSummary>();
	for (const summary of sources.flows) {
		byId.set(summary.id, summary);
		const version =
			summary.id === sources.pinnedBoardId ? sources.pinnedVersion : undefined;
		const graph = sources.graphs.get(summary.id);
		const fromGraph = !!version || isLegacy(summary);
		const source = fromGraph
			? graph
			: {
					entries: summary.entryNodes ?? [],
					executionMode: summary.executionMode,
				};
		if (!source) continue;
		for (const entry of source.entries) {
			const name =
				entry.friendlyName ||
				formatEventTypeLabel(entry.nodeType.replace(/^events?_/, ""));
			rows.push({
				key: `node:${summary.id}:${entry.nodeId}`,
				boardId: summary.id,
				nodeId: entry.nodeId,
				nodeType: entry.nodeType,
				boardVersion: version,
				boardExecutionMode: source.executionMode,
				name,
				label: name,
				flowName: summary.name,
				kind: sources.eventConfig[entry.nodeType]
					? formatEventTypeLabel(entry.nodeType.replace(/^events?_/, ""))
					: sources.customStart,
			});
		}
	}
	for (const page of sources.pages) {
		const summary = byId.get(page.boardId ?? "");
		rows.push({
			key: `page:${page.pageId}`,
			boardId: page.boardId ?? "",
			pageId: page.pageId,
			name: page.name,
			label: page.name,
			flowName: summary?.name ?? sources.pagesGroup,
			kind: sources.pageKind,
			boardExecutionMode: summary?.executionMode,
			unavailable: page.unavailable,
		});
	}
	rows.sort(
		(a, b) =>
			collator.compare(a.flowName, b.flowName) ||
			collator.compare(a.boardId, b.boardId) ||
			collator.compare(a.name, b.name) ||
			collator.compare(a.key, b.key),
	);
	const sameName = new Map<string, number>();
	for (const row of rows) {
		const nameKey = `${row.boardId}\u0000${row.name}`;
		sameName.set(nameKey, (sameName.get(nameKey) ?? 0) + 1);
	}
	for (const row of rows) {
		if ((sameName.get(`${row.boardId}\u0000${row.name}`) ?? 0) > 1)
			row.label = `${row.name} · ${(row.nodeId ?? row.pageId ?? row.key).slice(-6)}`;
	}
	return rows;
}

function buildView(
	matched: readonly StartRow[],
	pendingFlows: readonly StartFlow[],
	limits: Readonly<Record<string, number>>,
	needle: string,
): StartView {
	const grouped = new Map<string, { name: string; rows: StartRow[] }>();
	for (const row of matched) {
		const group = grouped.get(row.boardId) ?? { name: row.flowName, rows: [] };
		group.rows.push(row);
		grouped.set(row.boardId, group);
	}
	const loadable = new Set(pendingFlows.map((item) => item.id));
	const entries = [
		...[...grouped].map(([key, group]) => ({
			key,
			pending: false,
			loadable: loadable.has(key),
			...group,
		})),
		...pendingFlows
			.filter(
				(item) =>
					!grouped.has(item.id) &&
					(!needle || item.name.toLocaleLowerCase().includes(needle)),
			)
			.map((item) => ({
				key: item.id,
				name: item.name,
				rows: [] as StartRow[],
				pending: true,
				loadable: true,
			})),
	];
	const groups: ViewGroup[] = [];
	let used = 0;
	let hiddenGroups = 0;
	for (const entry of entries) {
		const budget = Math.min(GROUP_PAGE_SIZE, TOTAL_ROW_CAP - used);
		const limit = limits[entry.key] ?? budget;
		if (limit <= 0) {
			hiddenGroups += 1;
			continue;
		}
		used += entry.pending ? 1 : Math.min(entry.rows.length, limit);
		const shown = entry.rows.slice(0, limit);
		groups.push({
			key: entry.key,
			name: entry.name,
			total: entry.rows.length,
			shown,
			pending: entry.pending,
			loadable: entry.loadable,
		});
	}
	return { groups, hiddenGroups };
}
