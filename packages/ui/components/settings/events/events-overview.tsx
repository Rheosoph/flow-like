"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	ClipboardListIcon,
	ClockIcon,
	CodeIcon,
	CogIcon,
	ExternalLinkIcon,
	FileTextIcon,
	FormInputIcon,
	GitBranchIcon,
	GlobeIcon,
	HashIcon,
	LayersIcon,
	LayoutIcon,
	LinkIcon,
	ListFilterIcon,
	Loader2Icon,
	MailIcon,
	MessageSquareIcon,
	PauseIcon,
	PlayIcon,
	PlugIcon,
	PlusIcon,
	SearchIcon,
	SendIcon,
	ServerIcon,
	SettingsIcon,
	SlidersHorizontalIcon,
	Trash2Icon,
	ZapIcon,
} from "lucide-react";
import type { ComponentType } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { IOAuthConsentStore } from "../../../db/oauth-db";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { useSearch } from "../../../hooks/use-search-index";
import { describeEventEntry } from "../../../lib/event-entry";
import { getEventTypeGlyph } from "../../../lib/event-sections";
import { formatEventTypeLabel } from "../../../lib/event-type-label";
import type {
	IOAuthProvider,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../../lib/oauth/types";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IHub } from "../../../lib/schema/hub/hub";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type { IEventMapping } from "../../interfaces";
import { OAuthConsentDialog } from "../../oauth/oauth-consent-dialog";
import { PatSelectorDialog } from "../../pat-selector-dialog";
import { Button } from "../../ui/button";
import {
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Input } from "../../ui/input";
import { useProjectRuns } from "../dashboard/use-project-runs";
import type { SurfaceRunHealth } from "../dashboard/use-project-runs";
import { computeEventIssues } from "./use-event-issues";
import type { IEventIssue } from "./use-event-issues";
import { useSinkActivation } from "./use-sink-activation";

const TYPE_ICONS: Record<string, ComponentType<{ className?: string }>> = {
	clock: ClockIcon,
	globe: GlobeIcon,
	server: ServerIcon,
	plug: PlugIcon,
	"message-square": MessageSquareIcon,
	hash: HashIcon,
	send: SendIcon,
	mail: MailIcon,
	link: LinkIcon,
	zap: ZapIcon,
	"clipboard-list": ClipboardListIcon,
	layout: LayoutIcon,
	cog: CogIcon,
	layers: LayersIcon,
	"form-input": FormInputIcon,
	code: CodeIcon,
	"git-branch": GitBranchIcon,
	"file-text": FileTextIcon,
};

/**
 * The three questions this page is opened to answer. `attention` deliberately
 * outranks the others: an event with a blocking issue is not "paused", it is
 * broken, and grouping it with the things you switched off on purpose is how it
 * stays broken.
 */
type StatusFilter = "all" | "live" | "paused" | "attention";

type EventStatus = "live" | "paused" | "attention";

/**
 * The two things an event can be, from the point of view of the person using
 * the app rather than the person who built it: something you open, or something
 * that fires on its own.
 */
type EventGroup = "entry" | "trigger";

interface EventRowModel {
	event: IEvent;
	group: EventGroup;
	status: EventStatus;
	/** The issue worth putting on the row — blocking first, then check. */
	topIssue: IEventIssue | null;
	blocking: boolean;
	requiresSink: boolean;
	sinkActive: boolean;
	routePath?: string;
	isRouted: boolean;
	entry: ReturnType<typeof describeEventEntry>;
	health?: SurfaceRunHealth;
	glyph: { label: string; icon: string };
}

export interface EventsOverviewProps {
	events: IEvent[];
	boardsMap: Map<string, string>;
	appId: string;
	eventMapping: IEventMapping;
	/** Event types that render a UI and therefore own a route path. */
	uiEventTypes?: string[];
	onEdit: (event: IEvent) => void;
	onDelete: (eventId: string) => void;
	onNavigateToNode: (event: IEvent, nodeId: string) => void;
	onCreateEvent: () => void;
	tokenStore?: IOAuthTokenStoreWithPending;
	consentStore?: IOAuthConsentStore;
	hub?: IHub;
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
	/** Whether the app is local-only, which changes where a sink can run. */
	isOffline?: boolean;
}

function eventRequiresSink(
	eventMapping: IEventMapping,
	event: IEvent,
	nodeName?: string,
): boolean {
	if (!nodeName) return false;
	return eventMapping[nodeName]?.withSink.includes(event.event_type) ?? false;
}

function normalizePath(path: unknown): string {
	const raw = String(path ?? "").trim();
	if (!raw) return "/";
	const withoutQuery = raw.split("?")[0] ?? raw;
	if (!withoutQuery || withoutQuery === "/") return "/";
	return withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
}

export function EventsOverview({
	events,
	boardsMap,
	appId,
	eventMapping,
	uiEventTypes,
	onEdit,
	onDelete,
	onNavigateToNode,
	onCreateEvent,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
	isOffline,
}: Readonly<EventsOverviewProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [search, setSearch] = useState("");
	const [status, setStatus] = useState<StatusFilter>("all");
	const [typeFilter, setTypeFilter] = useState<Set<string>>(new Set());
	const [sinkStatuses, setSinkStatuses] = useState<Map<string, boolean>>(
		new Map(),
	);
	const [nodeNames, setNodeNames] = useState<Map<string, string>>(new Map());

	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);

	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		appId !== "",
	);

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		appId !== "",
	);

	// Run health is read from the run log rather than analytics, so it works for
	// offline projects too. Same source as the dashboard's Surfaces table.
	const runs = useProjectRuns(appId || undefined, boards.data);

	const routeByEventId = useMemo(() => {
		const map = new Map<string, string>();
		for (const route of routes.data ?? []) {
			map.set(route.eventId, normalizePath(route.path));
		}
		return map;
	}, [routes.data]);

	const boardIdsKey = useMemo(
		() => [...new Set(events.map((event) => event.board_id))].sort().join(","),
		[events],
	);

	// The event stores a node id; the sink registry is keyed by node name, so
	// deciding whether a row even has a sink means reading the board.
	useEffect(() => {
		let cancelled = false;
		const load = async () => {
			const boardIds = boardIdsKey ? boardIdsKey.split(",") : [];
			const names = new Map<string, string>();
			for (const boardId of boardIds) {
				if (!boardId) continue;
				try {
					const board = await backend.boardState.getBoard(appId, boardId);
					for (const event of events) {
						if (event.board_id !== boardId || !event.node_id) continue;
						const name = board?.nodes?.[event.node_id]?.name;
						if (name) names.set(event.id, name);
					}
				} catch (error) {
					console.error(`Failed to read board ${boardId}:`, error);
				}
			}
			if (!cancelled) setNodeNames(names);
		};
		if (events.length > 0) load();
		return () => {
			cancelled = true;
		};
	}, [appId, backend.boardState, boardIdsKey, events]);

	const sinkEventIdsKey = useMemo(
		() =>
			events
				.filter((event) =>
					eventRequiresSink(eventMapping, event, nodeNames.get(event.id)),
				)
				.map((event) => event.id)
				.join(","),
		[events, eventMapping, nodeNames],
	);

	const refreshSinkStatuses = useCallback(async () => {
		const ids = sinkEventIdsKey ? sinkEventIdsKey.split(",") : [];
		const statuses = new Map<string, boolean>();
		for (const id of ids) {
			if (!id) continue;
			try {
				statuses.set(id, await backend.eventState.isEventSinkActive(id));
			} catch (error) {
				console.error(`Failed to read sink status for event ${id}:`, error);
				statuses.set(id, false);
			}
		}
		setSinkStatuses(statuses);
	}, [backend.eventState, sinkEventIdsKey]);

	useEffect(() => {
		refreshSinkStatuses();
	}, [refreshSinkStatuses]);

	const { requestToggle, pendingId, dialogProps } = useSinkActivation({
		appId,
		tokenStore,
		consentStore,
		hub,
		onStartOAuth,
		onRefreshToken,
		onChanged: async () => {
			await invalidate(backend.eventState.getEvents, [appId]);
			await refreshSinkStatuses();
		},
	});

	const rows = useMemo<EventRowModel[]>(() => {
		return events.map((event) => {
			const config =
				(parseUint8ArrayToJson(event.config ?? []) as Record<
					string,
					unknown
				> | null) ?? {};
			const requiresSink = eventRequiresSink(
				eventMapping,
				event,
				nodeNames.get(event.id),
			);
			const issues = computeEventIssues({ event, config, requiresSink });
			const blockingIssue = issues.find((i) => i.severity === "blocking");
			const sinkActive = sinkStatuses.get(event.id) ?? false;
			const isRouted =
				uiEventTypeSet.has(event.event_type) || !!event.default_page_id;

			const notRunning = requiresSink && !sinkActive;
			const status: EventStatus = blockingIssue
				? "attention"
				: !event.active
					? "paused"
					: notRunning
						? "attention"
						: "live";

			return {
				event,
				group: isRouted ? "entry" : "trigger",
				status,
				topIssue: blockingIssue ?? issues[0] ?? null,
				blocking: !!blockingIssue,
				requiresSink,
				sinkActive,
				routePath: routeByEventId.get(event.id),
				isRouted,
				entry: isRouted ? null : describeEventEntry(event, config),
				health: runs.byEvent.get(event.id),
				glyph: getEventTypeGlyph(event),
			};
		});
	}, [
		events,
		eventMapping,
		nodeNames,
		sinkStatuses,
		uiEventTypeSet,
		routeByEventId,
		runs.byEvent,
	]);

	const statusCounts = useMemo(() => {
		const counts = { all: rows.length, live: 0, paused: 0, attention: 0 };
		for (const row of rows) counts[row.status] += 1;
		return counts;
	}, [rows]);

	const availableTypes = useMemo(() => {
		const set = new Set(rows.map((row) => row.event.event_type));
		return [...set].sort((a, b) =>
			formatEventTypeLabel(a).localeCompare(formatEventTypeLabel(b)),
		);
	}, [rows]);

	// The board name and the entry summary are derived, so they get folded into
	// an explicit haystack field the index can read.
	const searchableRows = useMemo(
		() =>
			rows.map((row) => ({
				row,
				haystack: [
					formatEventTypeLabel(row.event.event_type),
					row.entry?.text ?? "",
					boardsMap.get(row.event.board_id) ?? "",
				].join(" "),
			})),
		[rows, boardsMap],
	);

	const matchedRows = useSearch(searchableRows, search, {
		fields: [
			"row.event.name",
			"row.event.description",
			"row.event.event_type",
			"row.routePath",
			"haystack",
		],
		boost: { "row.event.name": 3 },
	});

	const visible = useMemo(
		() =>
			matchedRows
				.map(({ row }) => row)
				.filter((row) => {
					if (status !== "all" && row.status !== status) return false;
					if (typeFilter.size > 0 && !typeFilter.has(row.event.event_type)) {
						return false;
					}
					return true;
				})
				.sort((a, b) => (a.event.priority ?? 0) - (b.event.priority ?? 0)),
		[matchedRows, status, typeFilter],
	);

	const blocked = useMemo(() => rows.filter((row) => row.blocking), [rows]);

	const filtersActive =
		search.trim() !== "" || status !== "all" || typeFilter.size > 0;

	const entryRows = visible.filter((row) => row.group === "entry");
	const triggerRows = visible.filter((row) => row.group === "trigger");

	const handleToggleActive = useCallback(
		async (row: EventRowModel) => {
			await requestToggle(row.event, {
				active: !row.event.active,
				requiresSink: row.requiresSink,
			});
		},
		[requestToggle],
	);

	const handleRouteChange = useCallback(
		async (eventId: string, previous: string | undefined, next: string) => {
			const normalized = next.trim() ? normalizePath(next) : "";
			if (normalized === (previous ?? "")) return;
			try {
				if (previous && previous !== normalized) {
					await backend.routeState.deleteRouteByPath(appId, previous);
				}
				if (normalized) {
					await backend.routeState.setRoute(appId, normalized, eventId);
				}
				await invalidate(backend.routeState.getRoutes, [appId]);
			} catch (error) {
				console.error(`Failed to save route for event ${eventId}:`, error);
			}
		},
		[appId, backend.routeState, invalidate],
	);

	return (
		<div className="flex h-full min-h-0 flex-col gap-3">
			<div className="flex shrink-0 flex-wrap items-center gap-2">
				<div className="relative min-w-52 max-w-xs flex-1">
					<SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						placeholder={t('searchEvents', 'Search events…')}
						aria-label={t('searchEvents2', 'Search events')}
						className="h-9 pl-8"
					/>
				</div>

				<StatusFilterBar
					value={status}
					counts={statusCounts}
					onChange={setStatus}
				/>

				<div className="flex-1" />

				<TypeFilterMenu
					types={availableTypes}
					selected={typeFilter}
					onChange={setTypeFilter}
				/>

				<Button onClick={onCreateEvent} className="h-9 gap-2">
					<PlusIcon className="h-4 w-4" />
					{t('newEvent', 'New event')}
				</Button>
			</div>

			{blocked.length > 0 && status !== "attention" && (
				<AttentionBand rows={blocked} onSelect={onEdit} />
			)}

			<div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto">
				{visible.length === 0 ? (
					<div className="flex flex-1 flex-col items-center justify-center gap-3 py-12 text-sm text-muted-foreground">
						{filtersActive ? t('noEventMatchesThisSearch', 'No event matches this search.') : t('noEventsYet', 'No events yet.')}
						{filtersActive && (
							<Button
								variant="outline"
								size="sm"
								onClick={() => {
									setSearch("");
									setStatus("all");
									setTypeFilter(new Set());
								}}
							>
								{t('clearFilters', 'Clear filters')}
							</Button>
						)}
					</div>
				) : (
					<>
						<EventGroupSection
							title={t('entryPoints', 'Entry points')}
							blurb="People open these — a chat, a page, a form, a palette command."
							rows={entryRows}
							boardsMap={boardsMap}
							isOffline={isOffline}
							pendingActive={pendingId}
							onEdit={onEdit}
							onDelete={onDelete}
							onNavigateToNode={onNavigateToNode}
							onToggleActive={handleToggleActive}
							onRouteChange={handleRouteChange}
						/>
						<EventGroupSection
							title="Triggers"
							blurb="These fire on their own — a request, a schedule, a message, a mailbox."
							rows={triggerRows}
							boardsMap={boardsMap}
							isOffline={isOffline}
							pendingActive={pendingId}
							onEdit={onEdit}
							onDelete={onDelete}
							onNavigateToNode={onNavigateToNode}
							onToggleActive={handleToggleActive}
							onRouteChange={handleRouteChange}
						/>
					</>
				)}
			</div>

			<PatSelectorDialog
				{...dialogProps.pat}
				title={t('authorizeThisChange', 'Authorize this change')}
				description={t('registeringOrRemovingAnEventSinkNeedsAPersonalAccessToken', 'Registering or removing an event sink needs a Personal Access Token.')}
			/>
			<OAuthConsentDialog {...dialogProps.consent} />
		</div>
	);
}

function StatusFilterBar({
	value,
	counts,
	onChange,
}: Readonly<{
	value: StatusFilter;
	counts: Record<StatusFilter, number>;
	onChange: (next: StatusFilter) => void;
}>) {
	const { t } = useTranslation("settings");
	const options: Array<{
		key: StatusFilter;
		label: string;
		dot?: string;
	}> = [
		{ key: "all", label: t('all', 'All') },
		{ key: "live", label: t('live', 'Live'), dot: "bg-emerald-500" },
		{ key: "paused", label: t('paused', 'Paused'), dot: "bg-muted-foreground/50" },
		{ key: "attention", label: t('needsSetup', 'Needs setup'), dot: "bg-destructive" },
	];

	return (
		<div className="inline-flex items-center gap-0.5 rounded-md border bg-muted/40 p-0.5">
			{options.map((option) => {
				const active = value === option.key;
				return (
					<button
						key={option.key}
						type="button"
						aria-pressed={active}
						onClick={() => onChange(option.key)}
						className={cn(
							"inline-flex h-7 items-center gap-1.5 rounded px-2.5 text-xs font-medium transition-colors",
							active
								? "bg-card text-foreground"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						{option.dot && (
							<span
								className={cn("h-1.5 w-1.5 rounded-full", option.dot)}
								aria-hidden
							/>
						)}
						{option.label}
						<span className="text-[11px] tabular-nums opacity-60">
							{counts[option.key]}
						</span>
					</button>
				);
			})}
		</div>
	);
}

function TypeFilterMenu({
	types,
	selected,
	onChange,
}: Readonly<{
	types: string[];
	selected: Set<string>;
	onChange: (next: Set<string>) => void;
}>) {
	const { t } = useTranslation("settings");
	if (types.length < 2) return null;

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="outline" className="h-9 gap-2">
					<ListFilterIcon className="h-4 w-4" />
					Type
					{selected.size > 0 && (
						<span className="grid h-4 min-w-4 place-items-center rounded-full bg-primary px-1 text-[10px] font-bold text-primary-foreground">
							{selected.size}
						</span>
					)}
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-52">
				<DropdownMenuLabel>{t('eventType', 'Event type')}</DropdownMenuLabel>
				<DropdownMenuSeparator />
				{types.map((type) => (
					<DropdownMenuCheckboxItem
						key={type}
						checked={selected.has(type)}
						onCheckedChange={(checked) => {
							const next = new Set(selected);
							if (checked) next.add(type);
							else next.delete(type);
							onChange(next);
						}}
						onSelect={(e) => e.preventDefault()}
					>
						{formatEventTypeLabel(type)}
					</DropdownMenuCheckboxItem>
				))}
				{selected.size > 0 && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem onClick={() => onChange(new Set())}>
							Clear
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function AttentionBand({
	rows,
	onSelect,
}: Readonly<{
	rows: EventRowModel[];
	onSelect: (event: IEvent) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2">
			<span className="inline-flex shrink-0 items-center gap-1.5 text-sm font-semibold text-destructive">
				<AlertTriangleIcon className="h-3.5 w-3.5" />
				{t('countEventsCantRun', { defaultValue_one: '1 event can\'t run', defaultValue_other: '{{count}} events can\'t run', count: rows.length })}
			</span>
			{rows.map((row) => (
				<button
					key={row.event.id}
					type="button"
					onClick={() => onSelect(row.event)}
					className="inline-flex items-center gap-1.5 rounded-full border border-destructive/30 bg-card px-2 py-0.5 text-xs transition-colors hover:bg-muted"
				>
					<span
						className="h-1.5 w-1.5 rounded-full bg-destructive"
						aria-hidden
					/>
					<span className="font-medium">{row.event.name}</span>
					<span className="text-muted-foreground">— {row.topIssue?.title}</span>
				</button>
			))}
		</div>
	);
}

function EventGroupSection({
	title,
	blurb,
	rows,
	boardsMap,
	isOffline,
	pendingActive,
	onEdit,
	onDelete,
	onNavigateToNode,
	onToggleActive,
	onRouteChange,
}: Readonly<{
	title: string;
	blurb: string;
	rows: EventRowModel[];
	boardsMap: Map<string, string>;
	isOffline?: boolean;
	pendingActive: string | null;
	onEdit: (event: IEvent) => void;
	onDelete: (eventId: string) => void;
	onNavigateToNode: (event: IEvent, nodeId: string) => void;
	onToggleActive?: (row: EventRowModel) => void | Promise<void>;
	onRouteChange: (
		eventId: string,
		previous: string | undefined,
		next: string,
	) => Promise<void>;
}>) {
	if (rows.length === 0) return null;

	return (
		<section>
			<div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 px-0.5 pb-1.5">
				<h3 className="text-[13px] font-semibold">{title}</h3>
				<span className="text-xs tabular-nums text-muted-foreground">
					{rows.length}
				</span>
				<span className="text-xs text-muted-foreground">{blurb}</span>
			</div>
			<div className="overflow-hidden rounded-md border bg-card">
				{rows.map((row) => (
					<EventRow
						key={row.event.id}
						row={row}
						boardsMap={boardsMap}
						isOffline={isOffline}
						busy={pendingActive === row.event.id}
						onEdit={onEdit}
						onDelete={onDelete}
						onNavigateToNode={onNavigateToNode}
						onToggleActive={onToggleActive}
						onRouteChange={onRouteChange}
					/>
				))}
			</div>
		</section>
	);
}

const STRIPE: Record<EventStatus, string> = {
	attention: "bg-destructive",
	live: "bg-emerald-500/60",
	paused: "bg-transparent",
};

const DOT: Record<EventStatus, string> = {
	attention: "bg-destructive",
	live: "bg-emerald-500",
	paused: "bg-muted-foreground/50",
};

function EventRow({
	row,
	boardsMap,
	isOffline,
	busy,
	onEdit,
	onDelete,
	onNavigateToNode,
	onToggleActive,
	onRouteChange,
}: Readonly<{
	row: EventRowModel;
	boardsMap: Map<string, string>;
	isOffline?: boolean;
	busy: boolean;
	onEdit: (event: IEvent) => void;
	onDelete: (eventId: string) => void;
	onNavigateToNode: (event: IEvent, nodeId: string) => void;
	onToggleActive?: (row: EventRowModel) => void | Promise<void>;
	onRouteChange: (
		eventId: string,
		previous: string | undefined,
		next: string,
	) => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const { event, status, topIssue, glyph } = row;
	const Icon = TYPE_ICONS[glyph.icon] ?? CogIcon;
	const boardName = boardsMap.get(event.board_id);
	const version = event.board_version
		? `v${event.board_version.join(".")}`
		: "Latest";
	const runsFailed = row.health?.failed ?? 0;
	const runsTotal = row.health?.total ?? 0;

	return (
		<div
			className={cn(
				"group grid min-h-12.5 items-center gap-3 border-b py-2 pr-3 last:border-b-0",
				"grid-cols-[3px_30px_minmax(0,1fr)_126px]",
				"md:grid-cols-[3px_30px_minmax(0,1fr)_190px_126px]",
				"xl:grid-cols-[3px_30px_minmax(0,1fr)_200px_148px_122px_126px]",
				"transition-colors hover:bg-muted/50",
			)}
		>
			<div className={cn("h-full self-stretch", STRIPE[status])} aria-hidden />

			<div className="grid size-7.5 place-items-center rounded-md border bg-muted/50 text-muted-foreground">
				<Icon className="size-3.75" />
			</div>

			<div className="min-w-0 overflow-hidden">
				<div className="flex items-center gap-2">
					<span
						className={cn("h-1.5 w-1.5 shrink-0 rounded-full", DOT[status])}
						aria-hidden
					/>
					<button
						type="button"
						onClick={() => onEdit(event)}
						className="truncate text-left text-sm font-semibold hover:underline"
					>
						{event.name}
					</button>
					<span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[11px] font-medium text-secondary-foreground">
						{formatEventTypeLabel(event.event_type)}
					</span>
					{row.requiresSink && !row.sinkActive && (
						<span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-400">
							{t('notRunning', 'Not running')}
						</span>
					)}
					{row.requiresSink && row.sinkActive && (
						<span className="hidden shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground lg:inline">
							{isOffline ? "Local" : "Online"}
						</span>
					)}
				</div>
				<div className="mt-0.5 truncate text-xs">
					{topIssue ? (
						<span
							className={cn(
								topIssue.severity === "blocking"
									? "text-destructive"
									: "text-amber-700 dark:text-amber-400",
							)}
						>
							<AlertTriangleIcon className="mr-1 inline h-3 w-3 align-[-1px]" />
							<span className="font-medium">{topIssue.title}</span>
							<span className="opacity-80">{` — ${topIssue.detail}`}</span>
						</span>
					) : (
						<span className="text-muted-foreground">{event.description}</span>
					)}
				</div>
			</div>

			<div className="hidden min-w-0 overflow-hidden md:block">
				{row.isRouted ? (
					<RouteChip
						path={row.routePath}
						onSave={(next) => onRouteChange(event.id, row.routePath, next)}
					/>
				) : row.entry ? (
					<span
						title={row.entry.title ?? row.entry.text}
						className={cn(
							"inline-block max-w-full truncate rounded border px-1.5 py-0.5 font-mono text-[11.5px] leading-[1.45]",
							row.entry.muted
								? "border-border bg-muted/50 text-muted-foreground"
								: "border-primary/25 bg-primary/10 text-primary",
						)}
					>
						{row.entry.text}
					</span>
				) : null}
			</div>

			<div className="hidden min-w-0 overflow-hidden xl:block">
				<div className="truncate text-[13px]">
					{boardName ?? "Unknown flow"}
				</div>
				<div className="truncate font-mono text-[11px] text-muted-foreground">
					{version}
				</div>
			</div>

			<div className="hidden min-w-0 items-center gap-1.5 text-xs tabular-nums text-muted-foreground xl:flex">
				<RunSparkline trend={row.health?.trend} failed={runsFailed} />
				{runs24hLabel(runsTotal, runsFailed)}
			</div>

			<div className="flex justify-end gap-0.5 opacity-60 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
				{onToggleActive && (
					<Button
						variant="ghost"
						size="sm"
						className="h-7 w-7 p-0"
						disabled={busy}
						title={event.active ? "Pause event" : "Resume event"}
						aria-label={event.active ? "Pause event" : "Resume event"}
						onClick={() => onToggleActive(row)}
					>
						{busy ? (
							<Loader2Icon className="h-4 w-4 animate-spin" />
						) : event.active ? (
							<PauseIcon className="h-4 w-4" />
						) : (
							<PlayIcon className="h-4 w-4" />
						)}
					</Button>
				)}
				<Button
					variant="ghost"
					size="sm"
					className="h-7 w-7 p-0"
					title="Configure"
					aria-label={t('configureEvent', 'Configure event')}
					onClick={() => onEdit(event)}
				>
					<SettingsIcon className="h-4 w-4" />
				</Button>
				<Button
					variant="ghost"
					size="sm"
					className="h-7 w-7 p-0"
					title={t('openInFlow', 'Open in flow')}
					aria-label={t('openInFlow', 'Open in flow')}
					onClick={() => onNavigateToNode(event, event.node_id)}
				>
					<ExternalLinkIcon className="h-4 w-4" />
				</Button>
				<Button
					variant="ghost"
					size="sm"
					className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
					title={t('delete', 'Delete')}
					aria-label={t('deleteEvent', 'Delete event')}
					onClick={() => onDelete(event.id)}
				>
					<Trash2Icon className="h-4 w-4" />
				</Button>
			</div>
		</div>
	);
}

function runs24hLabel(total: number, failed: number) {
	if (total === 0) return <span className="opacity-60">—</span>;
	return (
		<span>
			{total.toLocaleString()}
			{failed > 0 && (
				<span className="ml-1 font-semibold text-destructive">{`${failed}✕`}</span>
			)}
		</span>
	);
}

function RunSparkline({
	trend,
	failed,
}: Readonly<{ trend?: number[]; failed: number }>) {
	const { t } = useTranslation("settings");
	const buckets = trend ?? [];
	const max = Math.max(1, ...buckets);
	const empty = buckets.length === 0 || buckets.every((v) => v === 0);

	return (
		<span
			className="flex h-4 shrink-0 items-end gap-[1.5px]"
			aria-hidden
			title={empty ? t('noRunsInTheLast24Hours', 'No runs in the last 24 hours') : undefined}
		>
			{(buckets.length > 0 ? buckets : new Array(12).fill(0)).map(
				(value, index) => (
					<span
						key={`${index}-${value}`}
						className={cn(
							"w-0.75 rounded-xs",
							empty
								? "bg-muted-foreground/25"
								: failed > 0
									? "bg-emerald-500/50"
									: "bg-emerald-500/55",
						)}
						style={{
							height: empty ? 2 : Math.max(2, Math.round((value / max) * 16)),
						}}
					/>
				),
			)}
		</span>
	);
}

/**
 * The route is the only field on this screen that is edited in place, because
 * it is the one people change most and opening the whole event to rename a path
 * is disproportionate.
 */
function RouteChip({
	path,
	onSave,
}: Readonly<{
	path?: string;
	onSave: (next: string) => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const [editing, setEditing] = useState(false);
	const [draft, setDraft] = useState(path ?? "");
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		if (!editing) setDraft(path ?? "");
	}, [path, editing]);

	const commit = async () => {
		if (saving) return;
		setSaving(true);
		try {
			await onSave(draft);
		} finally {
			setSaving(false);
			setEditing(false);
		}
	};

	if (editing) {
		return (
			<Input
				value={draft}
				onChange={(e) => setDraft(e.target.value)}
				onBlur={commit}
				onKeyDown={(e) => {
					if (e.key === "Enter") commit();
					if (e.key === "Escape") {
						setDraft(path ?? "");
						setEditing(false);
					}
				}}
				placeholder="/route"
				aria-label={t('routePath2', 'Route path')}
				disabled={saving}
				autoFocus
				className="h-7 w-full font-mono text-xs"
			/>
		);
	}

	return (
		<button
			type="button"
			onClick={() => setEditing(true)}
			title={path ? t('pathClickToEdit', '{{path}} — click to edit', { path }) : t('clickToSetARoute', 'Click to set a route')}
			className={cn(
				"inline-flex max-w-full items-center gap-1.5 truncate rounded border px-1.5 py-0.5 font-mono text-[11.5px] leading-[1.45] transition-opacity hover:opacity-80",
				path
					? "border-primary/25 bg-primary/10 text-primary"
					: "border-destructive/25 bg-destructive/10 text-destructive",
			)}
		>
			<SlidersHorizontalIcon className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-60" />
			<span className="truncate">{path ?? t('noRoute', 'No route')}</span>
		</button>
	);
}
