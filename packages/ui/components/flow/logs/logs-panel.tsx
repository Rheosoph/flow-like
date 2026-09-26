import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, PanelRightIcon } from "lucide-react";
import {
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type { ILogQuery } from "../../../lib/schema/flow/log-query";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { useLogAggregation } from "../../../state/log-aggregation-state";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../../ui/resizable";
import { usePanelToolbarSlot } from "../shell/board-panes";
import {
	buildBoardGraph,
	layerPath,
	missedUpstream,
	nodeLabel,
	upstreamNodes,
} from "./board-graph";
import type { ICopyEntry } from "./copy-format";
import {
	addChip,
	excludeNode,
	filterToQuery,
	hasTextInclude,
	includedGroups,
	isFilterEmpty,
	onlyLevel,
	queryKey,
	scopeToNode,
	toggleLevel,
} from "./filter-model";
import { FirstErrorStrip, type IMissedLabel } from "./first-error-strip";
import {
	foldHead,
	groupLabel,
	groupsByFingerprint,
	planFold,
} from "./fold-model";
import { type IDetailActions, LogDetailPane } from "./log-detail-pane";
import { LogEmptyState } from "./log-empty-state";
import { logKey, logStart } from "./log-format";
import { type ILogListHandle, type IRowView, LogList } from "./log-list";
import type { ILogQueryBarProps } from "./log-query-bar";
import type { IRowActions } from "./log-row";
import { LogRowMenu } from "./log-row-menu";
import { LogCopyBar, LogStatusBar } from "./log-status-bar";
import { LogToolbar } from "./log-toolbar";
import type { INodeRef } from "./query-tokens";
import type { ISuggestionContext } from "./suggestions";
import { useLogCopy } from "./use-log-copy";
import { useLogFilter } from "./use-log-filter";
import { useLogKeyboard } from "./use-log-keyboard";
import {
	type ILocateTarget,
	type ILocated,
	locateRow,
	targetOf,
	useLogNavigation,
} from "./use-log-navigation";
import { useLogSelection } from "./use-log-selection";
import {
	useLogWindow,
	useRunLogCount,
	useRunLogSummary,
} from "./use-log-window";

const EMPTY_QUERY: ILogQuery = {};

interface IFocused {
	/** Unknown once the query changes until the row is located again. */
	index?: number;
	log: ILog;
}

interface IMenuTarget {
	log: ILog;
	text: string;
}

const DetailToggle = memo(function DetailToggle({
	open,
	onToggle,
}: Readonly<{ open: boolean; onToggle: () => void }>) {
	const { t } = useTranslation("flow");
	return (
		<button
			type="button"
			aria-pressed={open}
			aria-label={t("logViewDetailPane", "Detail pane")}
			title={t("logViewDetailPane", "Detail pane")}
			onClick={onToggle}
			className={cn(
				"flex size-6 items-center justify-center rounded-md transition-colors",
				open
					? "bg-secondary text-foreground"
					: "text-muted-foreground hover:bg-secondary hover:text-foreground",
			)}
		>
			<PanelRightIcon className="size-3.5" />
		</button>
	);
});

export interface ILogsPanelProps {
	meta: ILogMetadata;
	board: RefObject<IBoard | undefined>;
	onFocusNode: (nodeId: string) => void;
	nodeIdFilter?: string;
	onClearNodeIdFilter?: () => void;
	variant: "card" | "panel";
}

export function LogsPanel({
	meta,
	board,
	onFocusNode,
	nodeIdFilter,
	onClearNodeIdFilter,
	variant,
}: Readonly<ILogsPanelProps>) {
	const { t } = useTranslation("flow");
	const boardState = useBackend().boardState;
	const setCurrentSummary = useLogAggregation(
		(state) => state.setCurrentSummary,
	);
	const toolbarSlot = usePanelToolbarSlot();

	const boardValue = board.current;
	const graph = useMemo(() => buildBoardGraph(boardValue), [boardValue]);
	const nodeRefs = useMemo<INodeRef[]>(
		() =>
			Object.values(boardValue?.nodes ?? {}).map((node) => ({
				id: node.id,
				name: node.name ?? "",
				friendlyName: node.friendly_name ?? "",
			})),
		[boardValue],
	);
	const nodeOf = useCallback(
		(id: string) => board.current?.nodes?.[id],
		[board],
	);
	const nodeName = useCallback(
		(id: string) => nodeLabel(board.current?.nodes?.[id]) ?? id.slice(0, 8),
		[board],
	);
	const upstreamOf = useCallback(
		(id: string) => upstreamNodes(graph, id),
		[graph],
	);
	const upstreamCount = useCallback(
		(id: string) => upstreamOf(id).length,
		[upstreamOf],
	);

	const summaryQuery = useRunLogSummary(boardState, meta);
	const summary = summaryQuery.data ?? undefined;
	const fingerprinted = !!summary?.fingerprinted;
	const groups = useMemo(() => groupsByFingerprint(summary), [summary]);

	useEffect(() => {
		if (summaryQuery.data !== undefined) {
			setCurrentSummary(meta.run_id, summaryQuery.data);
		}
	}, [summaryQuery.data, meta.run_id, setCurrentSummary]);

	const {
		filter,
		filterRef,
		setFilter,
		input,
		setInput,
		effective,
		clearFilters,
	} = useLogFilter({ nodes: nodeRefs, nodeIdFilter, onClearNodeIdFilter });

	const [fold, setFold] = useState(true);
	const [relative, setRelative] = useState(true);
	const [detailOpen, setDetailOpen] = useState(true);
	const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(
		() => new Set(),
	);

	const plan = useMemo(
		() =>
			planFold(summary, {
				enabled: fold,
				unfolded,
				textActive: hasTextInclude(effective.chips),
				onlyGroups: includedGroups(effective.chips),
			}),
		[summary, fold, unfolded, effective.chips],
	);
	const foldSet = useMemo(
		() => new Set(plan.fold.map((entry) => entry.fingerprint)),
		[plan],
	);

	const baseQuery = useMemo(
		() => filterToQuery(effective, { upstream: upstreamOf }),
		[effective, upstreamOf],
	);
	const listQuery = useMemo(
		() =>
			plan.fold.length > 0 ? { ...baseQuery, fold: plan.fold } : baseQuery,
		[baseQuery, plan],
	);

	const list = useLogWindow(boardState, meta, listQuery);
	const matchCount = useRunLogCount(
		boardState,
		meta,
		baseQuery,
		plan.fold.length > 0,
	);
	const totalCount = useRunLogCount(
		boardState,
		meta,
		EMPTY_QUERY,
		summaryQuery.isError,
	);
	const total =
		summary?.total ??
		(summaryQuery.data === null ? 0 : (totalCount.data ?? undefined));
	const matching = plan.fold.length > 0 ? matchCount.data : list.count;

	const selection = useLogSelection(list);
	const [focused, setFocused] = useState<IFocused>();
	const listHandle = useRef<ILogListHandle>(null);
	const inputRef = useRef<HTMLInputElement>(null);
	const clearSelection = selection.clear;

	useEffect(() => {
		if (!meta.run_id) return;
		setUnfolded(new Set());
		setFocused(undefined);
		clearSelection();
	}, [meta.run_id, clearSelection]);

	const latest = useRef({
		boardState,
		meta,
		list,
		focused,
		groups,
		foldSet,
		onFocusNode,
	});
	latest.current = {
		boardState,
		meta,
		list,
		focused,
		groups,
		foldSet,
		onFocusNode,
	};

	const goTo = useCallback((located: ILocated) => {
		setFocused(located);
		listHandle.current?.scrollToIndex(located.index);
	}, []);

	const relocate = useRef<ILocateTarget | undefined>(undefined);
	const viewKey = list.query ? `${meta.run_id}|${queryKey(list.query)}` : "";
	useEffect(() => {
		if (!viewKey) return;
		const { boardState, meta, list, focused } = latest.current;
		const target =
			relocate.current ?? (focused ? targetOf(focused.log) : undefined);
		relocate.current = undefined;
		setFocused((current) => (current ? { log: current.log } : current));
		if (!target || !list.query) {
			listHandle.current?.scrollToIndex(0);
			return;
		}
		locateRow(boardState, meta, list.query, target)
			.then((found) => {
				if (found) goTo(found);
				else listHandle.current?.scrollToIndex(0);
			})
			.catch(() => listHandle.current?.scrollToIndex(0));
	}, [viewKey, goTo]);

	const navigation = useLogNavigation({
		boardState,
		meta,
		query: list.query,
		firstError: summary?.first_error,
		current: focused?.log,
		goTo,
	});

	const entryOf = useCallback(
		(log: ILog): ICopyEntry => {
			const { foldSet, groups } = latest.current;
			return {
				log,
				node: log.node_id ? nodeName(log.node_id) : undefined,
				repeat: foldHead(log, foldSet, groups)?.count,
			};
		},
		[nodeName],
	);
	const copy = useLogCopy({
		boardState,
		meta,
		entryOf,
		title: `${nodeName(meta.node_id)} · ${meta.run_id}`,
	});

	const scopeNode = useCallback(
		(id: string) => setFilter(scopeToNode(filterRef.current, id)),
		[setFilter, filterRef],
	);
	const groupChip = useCallback(
		(fingerprint: string, negated: boolean) => {
			const template = latest.current.groups.get(fingerprint)?.template;
			setFilter(
				addChip(filterRef.current, {
					kind: "group",
					value: fingerprint,
					label: groupLabel(template ?? fingerprint),
					negated,
				}),
			);
		},
		[setFilter, filterRef],
	);

	const detailActions = useMemo<IDetailActions>(
		() => ({
			copyText: (log) => void copy.copyLogs([log], "text"),
			copyJson: (log) => void copy.copyLogs([log], "json"),
			scopeNode,
			excludeNode: (id) => setFilter(excludeNode(filterRef.current, id)),
			hideSimilar: (fingerprint) => groupChip(fingerprint, true),
			onlySimilar: (fingerprint) => groupChip(fingerprint, false),
			showOnBoard: (id) => latest.current.onFocusNode(id),
			close: () => setFocused(undefined),
		}),
		[copy.copyLogs, scopeNode, setFilter, filterRef, groupChip],
	);

	const setFolded = useCallback((fingerprint: string, folded: boolean) => {
		const group = latest.current.groups.get(fingerprint);
		if (group) {
			relocate.current = {
				start: group.first_start,
				node_id: group.node_id,
				level: group.log_level,
				message: group.sample,
				fingerprint,
			};
		}
		setUnfolded((prev) => {
			const next = new Set(prev);
			if (folded) next.delete(fingerprint);
			else next.add(fingerprint);
			return next;
		});
	}, []);

	const rowActions = useMemo<IRowActions>(
		() => ({
			toggleSelect: selection.toggle,
			focusRow: (index, log) => setFocused({ index, log }),
			scopeNode,
			setFolded,
		}),
		[selection.toggle, scopeNode, setFolded],
	);

	const focusedKey = focused ? logKey(focused.log) : undefined;
	const rowView = useMemo<IRowView>(
		() => ({
			selectedKeys: selection.keys,
			focusedKey,
			relative,
			base: meta.start,
			nodeOf,
			nodeName,
			headOf: (log) => {
				const fingerprint = log.fingerprint;
				if (!fingerprint || !fingerprinted) return undefined;
				const group = groups.get(fingerprint);
				if (!group || group.count < 2 || group.first_start !== logStart(log)) {
					return undefined;
				}
				if (foldSet.has(fingerprint)) return { group, folded: true };
				if (unfolded.has(fingerprint)) return { group, folded: false };
				return undefined;
			},
		}),
		[
			selection.keys,
			focusedKey,
			relative,
			meta.start,
			nodeOf,
			nodeName,
			fingerprinted,
			groups,
			foldSet,
			unfolded,
		],
	);

	const [menuTarget, setMenuTarget] = useState<IMenuTarget>();
	const onContextRow = useCallback(
		(_index: number, log: ILog, text: string) => setMenuTarget({ log, text }),
		[],
	);

	const suggestionContext = useMemo<ISuggestionContext>(() => {
		const nodeCounts: Record<string, number> = {};
		for (const [id, levels] of Object.entries(summary?.nodes ?? {})) {
			nodeCounts[id] = levels.reduce((sum, count) => sum + (count ?? 0), 0);
		}
		return {
			nodes: nodeRefs,
			nodeCounts,
			levelCounts: summary?.levels ?? [],
			groups: fingerprinted ? (summary?.groups ?? []) : [],
		};
	}, [summary, nodeRefs, fingerprinted]);

	const queryBar = useMemo<ILogQueryBarProps>(
		() => ({
			filter,
			onFilterChange: setFilter,
			input,
			onInputChange: setInput,
			nodes: nodeRefs,
			nodeOf,
			nodeName,
			upstreamCount,
			suggestionContext,
			inputRef,
		}),
		[
			filter,
			setFilter,
			input,
			setInput,
			nodeRefs,
			nodeOf,
			nodeName,
			upstreamCount,
			suggestionContext,
		],
	);

	const onToggleLevel = useCallback(
		(level: number) => setFilter(toggleLevel(filterRef.current, level)),
		[setFilter, filterRef],
	);
	const onOnlyLevel = useCallback(
		(level: number) => setFilter(onlyLevel(filterRef.current, level)),
		[setFilter, filterRef],
	);

	const firstError = summary?.first_error ?? undefined;
	const missed = useMemo<IMissedLabel[]>(() => {
		if (!firstError?.node_id || !summary?.visited) return [];
		return missedUpstream(
			graph,
			firstError.node_id,
			new Set(summary.visited),
		).map((entry) => ({
			name: nodeName(entry.nodeId),
			target: `${nodeName(entry.feeds.nodeId)} › ${entry.feeds.pin}`,
		}));
	}, [firstError, summary?.visited, graph, nodeName]);

	const moveFocus = useCallback((delta: number) => {
		const { list, focused } = latest.current;
		if (list.count === 0) return;
		const from = focused?.index ?? (delta > 0 ? -1 : list.count);
		const index = Math.min(list.count - 1, Math.max(0, from + delta));
		listHandle.current?.scrollToIndex(index);
		const log = list.row(index);
		if (log) setFocused({ index, log });
	}, []);

	const onKeyDown = useLogKeyboard({
		focused,
		selectedCount: selection.count,
		copySelected: () => void copy.copyLogs(selection.logs, "text"),
		stepError: (direction) => void navigation.stepError(direction),
		moveFocus,
		toggleSelect: selection.toggle,
		clearSelection: selection.clear,
		clearFocus: () => setFocused(undefined),
		focusQuery: () => inputRef.current?.focus(),
	});

	const filtered = !isFilterEmpty(effective);
	const panel = variant === "panel";
	const detailLog = focused?.log;
	const detailGroup = useMemo(() => {
		const fingerprint = detailLog?.fingerprint;
		if (!fingerprint || !fingerprinted) return undefined;
		const group = groups.get(fingerprint);
		return group && group.count >= 2 ? group : undefined;
	}, [detailLog, fingerprinted, groups]);
	const detailPath = useMemo(
		() => (detailLog?.node_id ? layerPath(boardValue, detailLog.node_id) : []),
		[detailLog, boardValue],
	);
	const menuLog = menuTarget?.log;
	const toggleDetail = useCallback(() => setDetailOpen((open) => !open), []);

	return (
		<section
			aria-label={t("logs", "Logs")}
			tabIndex={-1}
			onKeyDown={onKeyDown}
			className={cn(
				"flex h-full w-full min-h-0 flex-col overflow-hidden bg-background outline-none",
				!panel && "rounded-lg border",
			)}
		>
			{panel && toolbarSlot
				? createPortal(
						<DetailToggle open={detailOpen} onToggle={toggleDetail} />,
						toolbarSlot,
					)
				: null}
			<LogToolbar
				queryBar={queryBar}
				levelsOff={effective.levelsOff}
				levelCounts={summary?.levels}
				onToggleLevel={onToggleLevel}
				onOnlyLevel={onOnlyLevel}
				foldAvailable={fingerprinted}
				fold={fold}
				onFoldChange={setFold}
				relative={relative}
				onRelativeChange={setRelative}
			/>
			{firstError ? (
				<FirstErrorStrip
					error={firstError}
					base={meta.start}
					node={firstError.node_id ? nodeOf(firstError.node_id) : undefined}
					nodeName={
						firstError.node_id ? nodeName(firstError.node_id) : undefined
					}
					missed={missed}
					notice={navigation.notice}
					busy={navigation.busy}
					onJump={navigation.jumpToFirstError}
					onShowOnBoard={() =>
						firstError.node_id && onFocusNode(firstError.node_id)
					}
					onScope={() => firstError.node_id && scopeNode(firstError.node_id)}
					onPrev={() => navigation.stepError(-1)}
					onNext={() => navigation.stepError(1)}
					onClearFilters={() => {
						clearFilters();
						navigation.clearNotice();
					}}
				/>
			) : null}
			<ResizablePanelGroup direction="horizontal" className="min-h-0 flex-1">
				<ResizablePanel id="log-list" order={1} defaultSize={64} minSize={30}>
					<div className="flex h-full min-h-0 flex-col">
						{list.loading ? (
							<div className="flex flex-1 items-center justify-center gap-2 text-xs text-muted-foreground">
								<Loader2Icon className="size-3.5 animate-spin" />
								{t("logViewLoading", "Loading logs…")}
							</div>
						) : (
							<LogList
								handleRef={listHandle}
								window={list}
								view={rowView}
								actions={rowActions}
								onContextRow={onContextRow}
								empty={
									<LogEmptyState
										error={list.error}
										filtered={filtered}
										onRetry={list.retry}
										onClearFilters={clearFilters}
									/>
								}
								menu={
									menuLog ? (
										<LogRowMenu
											log={menuLog}
											selectedText={menuTarget?.text ?? ""}
											nodeName={
												menuLog.node_id ? nodeName(menuLog.node_id) : undefined
											}
											similar={
												menuLog.fingerprint
													? groups.get(menuLog.fingerprint)?.count
													: undefined
											}
											fingerprinted={fingerprinted}
											actions={detailActions}
											onCopySelection={copy.copyText}
										/>
									) : null
								}
							/>
						)}
					</div>
				</ResizablePanel>
				{detailOpen ? (
					<>
						<ResizableHandle />
						<ResizablePanel
							id="log-detail"
							order={2}
							defaultSize={36}
							minSize={20}
						>
							<LogDetailPane
								log={detailLog}
								base={meta.start}
								node={
									detailLog?.node_id ? nodeOf(detailLog.node_id) : undefined
								}
								nodeName={
									detailLog?.node_id ? nodeName(detailLog.node_id) : undefined
								}
								path={detailPath}
								group={detailGroup}
								fingerprinted={fingerprinted}
								actions={detailActions}
							/>
						</ResizablePanel>
					</>
				) : null}
			</ResizablePanelGroup>
			{selection.count > 0 ? (
				<LogCopyBar
					count={selection.count}
					onCopyText={() => void copy.copyLogs(selection.logs, "text")}
					onCopyJson={() => void copy.copyLogs(selection.logs, "json")}
					onCopyMarkdown={() => void copy.copyLogs(selection.logs, "markdown")}
					onClear={selection.clear}
				/>
			) : null}
			<LogStatusBar
				total={total}
				rows={list.count}
				matching={matching}
				partial={!!summary?.partial}
				folded={plan.fold.length > 0}
				foldPaused={plan.paused}
				filtered={filtered}
				copying={copy.copying}
				loadFailed={!!list.error || list.pageFailed}
				onClearFilters={clearFilters}
				onCopyVisible={() => void copy.copyVisible(list.query, list.count)}
				onRetry={list.retry}
			/>
		</section>
	);
}
