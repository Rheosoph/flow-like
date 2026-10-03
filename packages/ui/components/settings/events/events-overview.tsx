"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import {
	AlertTriangleIcon,
	CheckIcon,
	CircleSlashIcon,
	ClipboardListIcon,
	ClockIcon,
	CodeIcon,
	CogIcon,
	CopyIcon,
	EllipsisIcon,
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
import { useRouter, useSearchParams } from "next/navigation";
import type { ComponentType } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type { IOAuthConsentStore } from "../../../db/oauth-db";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { useSearch } from "../../../hooks/use-search-index";
import { describeEventEntry } from "../../../lib/event-entry";
import { getEventTypeGlyph } from "../../../lib/event-sections";
import { isDeviceEventSource } from "../../../lib/event-source";
import { formatEventTypeLabel } from "../../../lib/event-type-label";
import type {
	IOAuthProvider,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../../lib/oauth/types";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { asArray } from "../../../lib/response-shape";
import { normalizeRoutePath } from "../../../lib/route-path";
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
import {
	type EventsDevicesHarness,
	EventsDevicesProvider,
	useEventsDevices,
	useOptionalEventsDevices,
} from "../devices/events/events-devices";
import {
	EVENTS_BLOCK_ID,
	EventsDevicesBanner,
	OnDevicesStrip,
} from "../devices/events/on-devices-strip";
import { RunsOnCell, RunsOnColumnHeader } from "../devices/events/runs-on-cell";
import { scheduleOnDevice } from "../devices/events/runs-on-model";
import {
	type RunOnDevice,
	useRunOnDevice,
} from "../devices/events/use-run-on-device";
import { Banner } from "../devices/primitives/banner";
import { DvButton } from "../devices/primitives/dv-button";
import { useCopy } from "../devices/primitives/use-copy";
import { PermissionNotice } from "../permission/permission-notice";
import { EventDeleteDialog } from "./event-delete-dialog";
import { type EventStatus, getEventStatus } from "./event-status";
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

/** Blocking setup issues take precedence over activation state. */
type StatusFilter = "all" | EventStatus;

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
	/** Saved for devices only: this app and hub register no trigger for it. */
	deviceOnly: boolean;
	requiresSink: boolean;
	sinkActive?: boolean;
	sinkStatusLoading: boolean;
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
	/** Whether the signed-in role may write this project's events. */
	canEdit?: boolean;
	/** Whether it may read the flows behind them (board names, sink lookup). */
	canReadBoards?: boolean;
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
	/** Tests and the visual harness: what the device workspace reads instead of the host. */
	devicesHarness?: EventsDevicesHarness;
}

/** The list switches its columns on its own width: the config sidebar makes the viewport a poor guide. */
const ROW_GRID = cn(
	"grid gap-x-3 pr-3",
	"grid-cols-[3px_30px_minmax(0,1fr)_28px]",
	"@min-[600px]/events:grid-cols-[3px_30px_minmax(150px,1fr)_minmax(84px,180px)_minmax(172px,176px)_58px]",
	"@min-[900px]/events:grid-cols-[3px_30px_minmax(170px,1fr)_minmax(112px,180px)_minmax(160px,200px)_minmax(104px,140px)_156px]",
	"@min-[1100px]/events:grid-cols-[3px_30px_minmax(180px,1fr)_minmax(130px,200px)_200px_148px_112px_156px]",
);
const FROM_600 = "hidden @min-[600px]/events:block";
const FROM_900 = "hidden @min-[900px]/events:block";

const eventRowId = (eventId: string) => `event-row-${eventId}`;

function eventRequiresSink(
	eventMapping: IEventMapping,
	event: IEvent,
	nodeName?: string,
): boolean {
	if (!nodeName) return false;
	return eventMapping[nodeName]?.withSink.includes(event.event_type) ?? false;
}

export function EventsOverview({
	events,
	boardsMap,
	appId,
	eventMapping,
	uiEventTypes,
	canEdit = true,
	canReadBoards = true,
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
	devicesHarness,
}: Readonly<EventsOverviewProps>) {
	const { t } = useTranslation("settings");
	const { t: td } = useTranslation("devices");
	const backend = useBackend();
	const router = useRouter();
	const invalidate = useInvalidateInvoke();
	const queryClient = useQueryClient();
	const [search, setSearch] = useState("");
	const [status, setStatus] = useState<StatusFilter>("all");
	const [typeFilter, setTypeFilter] = useState<Set<string>>(new Set());
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
		appId !== "" && canReadBoards,
	);

	// Run health is read from the run log rather than analytics, so it works for
	// offline projects too. Same source as the dashboard's Surfaces table.
	const runs = useProjectRuns(appId || undefined, boards.data);

	const routeByEventId = useMemo(() => {
		const map = new Map<string, string>();
		for (const route of asArray(routes.data)) {
			map.set(route.eventId, normalizeRoutePath(route.path));
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
		if (!canReadBoards) return;
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
	}, [appId, backend.boardState, boardIdsKey, canReadBoards, events]);

	const sinkEvents = useMemo(
		() =>
			events.filter(
				(event) =>
					!isDeviceEventSource(event) &&
					eventRequiresSink(eventMapping, event, nodeNames.get(event.id)),
			),
		[events, eventMapping, nodeNames],
	);

	const sinkQueries = useQueries({
		queries: sinkEvents.map((event) => ({
			queryKey: [
				"eventSinkStatus",
				appId,
				event.id,
				{
					active: event.active,
					event_type: event.event_type,
					execution_mode: event.execution_mode,
					config: event.config,
					event_version: event.event_version,
				},
			],
			queryFn: () =>
				backend.eventState.isEventSinkActive(event.id, { appId, event }),
			enabled: !!appId && event.active,
			retry: false,
			staleTime: 0,
			refetchInterval: 30_000,
		})),
	});
	const sinkStatuses = new Map(
		sinkEvents.map((event, index) => [event.id, sinkQueries[index]]),
	);

	const { requestToggle, pendingId, dialogProps } = useSinkActivation({
		appId,
		tokenStore,
		consentStore,
		hub,
		onStartOAuth,
		onRefreshToken,
		onChanged: async () => {
			await invalidate(backend.eventState.getEvents, [appId]);
			await queryClient.invalidateQueries({
				queryKey: ["eventSinkStatus", appId],
			});
		},
	});

	const rows = useMemo<EventRowModel[]>(() => {
		return events.map((event) => {
			const config =
				(parseUint8ArrayToJson(event.config ?? []) as Record<
					string,
					unknown
				> | null) ?? {};
			const deviceOnly = isDeviceEventSource(event);
			const requiresSink =
				!deviceOnly &&
				eventRequiresSink(eventMapping, event, nodeNames.get(event.id));
			const issues = computeEventIssues({ event, config, requiresSink });
			const blockingIssue = issues.find((i) => i.severity === "blocking");
			const sinkQuery = sinkStatuses.get(event.id);
			const sinkActive = !event.active
				? false
				: sinkQuery?.isError
					? undefined
					: sinkQuery?.data;
			const isRouted =
				uiEventTypeSet.has(event.event_type) || !!event.default_page_id;

			const status = getEventStatus({
				active: event.active,
				blocking: !!blockingIssue,
				requiresSink,
				sinkActive,
				deviceOnly,
			});

			return {
				event,
				group: isRouted ? "entry" : "trigger",
				status,
				topIssue: blockingIssue ?? issues[0] ?? null,
				blocking: !!blockingIssue,
				deviceOnly,
				requiresSink,
				sinkActive,
				sinkStatusLoading: sinkQuery?.isPending ?? true,
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
		const counts = {
			all: rows.length,
			live: 0,
			paused: 0,
			attention: 0,
			unknown: 0,
			device: 0,
		};
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
			if (!canEdit) return;
			const pausing = row.event.active;
			await requestToggle(row.event, {
				active: !pausing,
				requiresSink: row.requiresSink,
				onSettled: (error) => {
					if (!error && pausing && row.deviceOnly)
						toast.info(
							t(
								"devicePauseKeepsRunning",
								"Devices that already run it keep running until you update or stop their service.",
							),
						);
				},
			});
		},
		[canEdit, requestToggle, t],
	);

	const [deleting, setDeleting] = useState<IEvent | null>(null);
	const requestDelete = useCallback(
		(eventId: string) =>
			setDeleting(events.find((event) => event.id === eventId) ?? null),
		[events],
	);

	const handleRouteChange = useCallback(
		async (eventId: string, previous: string | undefined, next: string) => {
			if (!canEdit) return;
			const normalized = next.trim() ? normalizeRoutePath(next) : "";
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
		[appId, backend.routeState, canEdit, invalidate],
	);

	const writeDeniedMessage = t(
		"yourRoleCannotChangeThisProjectsEvents",
		"Your role cannot create, change or delete this project's events.",
	);

	const openEvent = useCallback(
		(eventId: string) => {
			const event = events.find((candidate) => candidate.id === eventId);
			if (event) onEdit(event);
		},
		[events, onEdit],
	);
	const navigate = useCallback((href: string) => router.push(href), [router]);
	const renderTile = useCallback(
		(eventId: string) => {
			const row = rows.find((candidate) => candidate.event.id === eventId);
			return row ? <EventTile icon={row.glyph.icon} small /> : null;
		},
		[rows],
	);
	const handlers: EventRowHandlers = {
		appId,
		boardsMap,
		isOffline,
		canEdit,
		writeDeniedMessage,
		onEdit,
		onDelete: requestDelete,
		onNavigateToNode,
		onToggleActive: handleToggleActive,
		onRouteChange: handleRouteChange,
	};

	return (
		<EventsDevicesProvider
			appId={appId}
			events={events}
			hub={hub}
			canReadBoards={canReadBoards}
			onNavigate={navigate}
			onOpenEvent={openEvent}
			renderTile={renderTile}
			harness={devicesHarness}
		>
			<div className="flex h-full min-h-0 flex-col gap-3">
				<EventDeepLink appId={appId} events={events} />
				<div className="flex shrink-0 flex-wrap items-center gap-2">
					<div className="relative min-w-52 max-w-xs flex-1">
						<SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={search}
							onChange={(e) => setSearch(e.target.value)}
							placeholder={t("searchEvents", "Search events…")}
							aria-label={t("searchEvents2", "Search events")}
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

					<Button
						onClick={onCreateEvent}
						className="h-9 gap-2"
						disabled={!canEdit}
						title={canEdit ? undefined : writeDeniedMessage}
					>
						<PlusIcon className="h-4 w-4" />
						{t("newEvent", "New event")}
					</Button>
				</div>

				{!canEdit && (
					<PermissionNotice
						tone="readOnly"
						className="shrink-0"
						title={t("eventsAreReadonly", "Events are read-only for you")}
						description={writeDeniedMessage}
						missing={[RolePermissions.WriteEvents]}
					/>
				)}
				{!canReadBoards && <FlowNamesNotice appId={appId} />}
				<EventsDevicesBanner className="shrink-0" />

				{blocked.length > 0 && status !== "attention" && (
					<AttentionBand rows={blocked} onSelect={onEdit} />
				)}

				<OnDevicesStrip className="shrink-0" />

				<div className="@container/events flex min-h-0 flex-1 flex-col gap-4 overflow-auto">
					{visible.length === 0 ? (
						<div className="flex flex-1 flex-col items-center justify-center gap-3 py-12 text-sm text-muted-foreground">
							{filtersActive
								? t("noEventMatchesThisSearch", "No event matches this search.")
								: t("noEventsYet", "No events yet.")}
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
									{t("clearFilters", "Clear filters")}
								</Button>
							)}
						</div>
					) : (
						<>
							<EventGroupSection
								title={t("entryPoints", "Entry points")}
								blurb={td(
									"events.section.entryBlurb",
									"People open these — a chat, a page, a form, a palette command.",
								)}
								entryLabel={td("events.column.route", "Route")}
								rows={entryRows}
								pendingActive={pendingId}
								handlers={handlers}
							/>
							<EventGroupSection
								title={td("events.section.triggers", "Triggers")}
								blurb={td(
									"events.section.triggerBlurb",
									"These fire on their own — a request, a schedule, a message, a mailbox.",
								)}
								entryLabel={td("events.column.trigger", "Trigger")}
								rows={triggerRows}
								pendingActive={pendingId}
								handlers={handlers}
							/>
						</>
					)}
				</div>

				<PatSelectorDialog
					{...dialogProps.pat}
					title={t("authorizeThisChange", "Authorize this change")}
					description={t(
						"registeringOrRemovingAnEventSinkNeedsAPersonalAccessToken",
						"Registering or removing an event sink needs a Personal Access Token.",
					)}
				/>
				<OAuthConsentDialog {...dialogProps.consent} />
				<EventDeleteDialog
					event={deleting}
					onCancel={() => setDeleting(null)}
					onConfirm={(eventId) => {
						setDeleting(null);
						onDelete(eventId);
					}}
				/>
			</div>
		</EventsDevicesProvider>
	);
}

function EventTile({
	icon,
	small = false,
}: Readonly<{ icon: string; small?: boolean }>) {
	const Icon = TYPE_ICONS[icon] ?? CogIcon;
	return (
		<span
			className={cn(
				"grid shrink-0 place-items-center rounded-md border bg-muted/50 text-muted-foreground",
				small ? "size-6" : "size-7.5",
			)}
		>
			<Icon className={small ? "size-3.5" : "size-3.75"} />
		</span>
	);
}

/**
 * No ReadBoards: flow names, sink status and the Devices column are all
 * unknown. While that is why Run on a device… is off, this notice is the
 * reason every gated control points at (APP §4.5).
 */
function FlowNamesNotice({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const { t: td } = useTranslation("devices");
	const backend = useBackend();
	const { block } = useEventsDevices();
	const { copied, copy } = useCopy();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		appId !== "",
	);
	const request = td(
		"events.blind.request",
		"Hi, could you give me the Read boards permission on {{app}} ({{id}})? I need it to see where its events run on devices and to deploy them. Thanks!",
		{ app: meta.data?.name ?? appId, id: appId },
	);
	return (
		<div
			id={block === "blind" ? EVENTS_BLOCK_ID : undefined}
			className="shrink-0"
		>
			<PermissionNotice
				title={t("flowNamesUnavailable", "Flow names unavailable")}
				description={`${t(
					"theFlowColumnStaysEmptyWithoutWorkflowAccess",
					"Your role cannot read this project's flows, so the flow column and sink status stay blank rather than reporting a flow that isn't there.",
				)} ${td(
					"events.blind.devices",
					"This page also can't tell which devices run its events, and Run on a device… is off. That doesn't mean they run nowhere.",
				)}`}
				missing={[RolePermissions.ReadBoards]}
				action={
					<DvButton
						size="sm"
						icon={copied ? CheckIcon : CopyIcon}
						className="shrink-0"
						onClick={() => void copy(request)}
					>
						{copied
							? td("events.blind.copied", "Copied")
							: td("events.blind.copy", "Copy a request for the owner")}
					</DvButton>
				}
			/>
		</div>
	);
}

/**
 * `event=<id>` (APP §4.1): highlights that row, scrolls to it and opens its
 * Devices popover. The page's own `eventId=` keeps opening the editor.
 */
function EventDeepLink({
	appId,
	events,
}: Readonly<{ appId: string; events: IEvent[] }>) {
	const { t } = useTranslation("devices");
	const backend = useBackend();
	const router = useRouter();
	const params = useSearchParams();
	const { showOnDevices } = useEventsDevices();
	const [missing, setMissing] = useState<string | null>(null);
	const target = params?.get("event") ?? null;
	const known = target !== null && events.some((event) => event.id === target);
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		appId !== "" && missing !== null,
	);

	useEffect(() => {
		if (!target) return;
		if (known) {
			setMissing(null);
			showOnDevices(target);
			document
				.getElementById(eventRowId(target))
				?.scrollIntoView({ block: "center" });
			return;
		}
		setMissing(target);
		const next = new URLSearchParams(params?.toString() ?? "");
		next.delete("event");
		const query = next.toString();
		router.replace(`${window.location.pathname}${query ? `?${query}` : ""}`, {
			scroll: false,
		});
	}, [target, known, showOnDevices, params, router]);

	if (!missing) return null;
	return (
		<Banner tone="warning" className="shrink-0">
			{meta.data?.name
				? t(
						"events.deepLink.missing",
						"No event with the ID {{id}} in {{app}}. Showing all its events.",
						{ id: missing, app: meta.data.name },
					)
				: t(
						"events.deepLink.missingPlain",
						"No event with the ID {{id}} in this app. Showing all its events.",
						{ id: missing },
					)}
		</Banner>
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
	const candidates: Array<{
		key: StatusFilter;
		label: string;
		dot?: string;
	}> = [
		{ key: "all", label: t("all", "All") },
		{ key: "live", label: t("live", "Live"), dot: "bg-emerald-500" },
		{
			key: "paused",
			label: t("paused", "Paused"),
			dot: "bg-muted-foreground/50",
		},
		{
			key: "attention",
			label: t("needsSetup", "Needs setup"),
			dot: "bg-destructive",
		},
		{
			key: "unknown",
			label: t("statusUnknown", "Unknown"),
			dot: "bg-muted-foreground/50",
		},
		{
			key: "device",
			label: t("devicesOnly", "Devices only"),
			dot: "bg-info-solid",
		},
	];
	const options = candidates.filter(
		(option) =>
			option.key !== "device" || counts.device > 0 || value === "device",
	);

	return (
		<div className="inline-flex max-w-full flex-wrap items-center gap-0.5 rounded-md border bg-muted/40 p-0.5">
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
					{t("eventType", "Event type")}
					{selected.size > 0 && (
						<span className="grid h-4 min-w-4 place-items-center rounded-full bg-primary px-1 text-[10px] font-bold text-primary-foreground">
							{selected.size}
						</span>
					)}
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-52">
				<DropdownMenuLabel>{t("eventType", "Event type")}</DropdownMenuLabel>
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
				{t("countEventsCantRun", {
					defaultValue_one: "1 event can't run",
					defaultValue_other: "{{count}} events can't run",
					count: rows.length,
				})}
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

interface EventRowHandlers {
	appId: string;
	boardsMap: Map<string, string>;
	isOffline?: boolean;
	canEdit: boolean;
	writeDeniedMessage: string;
	onEdit: (event: IEvent) => void;
	onDelete: (eventId: string) => void;
	onNavigateToNode: (event: IEvent, nodeId: string) => void;
	onToggleActive?: (row: EventRowModel) => void | Promise<void>;
	onRouteChange: (
		eventId: string,
		previous: string | undefined,
		next: string,
	) => Promise<void>;
}

interface RowActionProps {
	row: EventRowModel;
	/** Its activation change is being saved. */
	busy: boolean;
	handlers: EventRowHandlers;
}

function ColumnHeader({ entryLabel }: Readonly<{ entryLabel: string }>) {
	const { t } = useTranslation("devices");
	return (
		<div
			aria-hidden
			className={cn(
				ROW_GRID,
				"hidden items-center border-b py-1.5 text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground @min-[600px]/events:grid",
			)}
		>
			<span />
			<span />
			<span>{t("events.column.event", "Event")}</span>
			<span>{entryLabel}</span>
			<RunsOnColumnHeader />
			<span className={FROM_900}>{t("events.column.flow", "Flow")}</span>
			<span className="hidden @min-[1100px]/events:block">
				{t("events.column.runs", "Runs · 24 h")}
			</span>
			<span className="text-right">
				{t("events.column.actions", "Actions")}
			</span>
		</div>
	);
}

function EventGroupSection({
	title,
	blurb,
	entryLabel,
	rows,
	pendingActive,
	handlers,
}: Readonly<{
	title: string;
	blurb: string;
	/** Head of the route or trigger column. */
	entryLabel: string;
	rows: EventRowModel[];
	pendingActive: string | null;
	handlers: EventRowHandlers;
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
				<ColumnHeader entryLabel={entryLabel} />
				{rows.map((row) => (
					<EventRow
						key={row.event.id}
						row={row}
						busy={pendingActive === row.event.id}
						handlers={handlers}
					/>
				))}
			</div>
		</section>
	);
}

/** APP §4.5: first in the row's actions; off with the reason the Devices cell or the banner shows (R7). */
function RunOnDeviceButton({
	run,
	className,
}: Readonly<{ run: RunOnDevice; className?: string }>) {
	const { t } = useTranslation("devices");
	if (run.off)
		return (
			<DvButton
				variant="ghost"
				size="sm"
				iconOnly
				icon={ServerIcon}
				aria-disabled
				aria-describedby={run.off.by}
				aria-label={t(
					"events.row.runOff",
					"{{action}} Unavailable: {{reason}}",
					{
						action: run.name,
						reason: run.off.reason,
					},
				)}
				title={run.off.reason}
				data-run-on-device="off"
				className={className}
			/>
		);
	return (
		<DvButton
			variant="ghost"
			size="sm"
			iconOnly
			icon={ServerIcon}
			asChild
			className={className}
		>
			<a
				{...run.link}
				aria-label={run.name}
				title={run.label}
				data-run-on-device="on"
			/>
		</DvButton>
	);
}

/** The device items of the row menu: Run on a device… with its reason when off, then the Devices popover. */
function RowMenuDeviceItems({
	run,
	onExplain,
}: Readonly<{ run: RunOnDevice; onExplain(): void }>) {
	const { t } = useTranslation("devices");
	const ExplainIcon = run.eligible ? LayersIcon : CircleSlashIcon;
	return (
		<>
			<DropdownMenuItem
				disabled={run.off !== null}
				onSelect={run.open}
				className="items-start gap-2"
			>
				<ServerIcon className="mt-0.5 h-4 w-4" />
				<span className="flex min-w-0 flex-col">
					{run.label}
					{run.off ? (
						<span className="text-xs text-muted-foreground">
							{run.off.reason}
						</span>
					) : null}
				</span>
			</DropdownMenuItem>
			{run.explains ? (
				<DropdownMenuItem onSelect={onExplain}>
					<ExplainIcon className="h-4 w-4" />
					{run.eligible
						? t("events.row.where", "Where it runs")
						: t("events.row.why", "Why it can't run on devices")}
				</DropdownMenuItem>
			) : null}
		</>
	);
}

/** The row's own four actions as menu items. */
function RowMenuOwnItems({ row, busy, handlers }: Readonly<RowActionProps>) {
	const { t } = useTranslation("settings");
	const { event } = row;
	const { canEdit, onToggleActive } = handlers;
	const ToggleIcon = event.active ? PauseIcon : PlayIcon;
	return (
		<>
			{onToggleActive ? (
				<DropdownMenuItem
					disabled={busy || !canEdit}
					onSelect={() => void onToggleActive(row)}
				>
					<ToggleIcon className="h-4 w-4" />
					{event.active
						? t("pauseEvent", "Pause event")
						: t("resumeEvent", "Resume event")}
				</DropdownMenuItem>
			) : null}
			<DropdownMenuItem onSelect={() => handlers.onEdit(event)}>
				<SettingsIcon className="h-4 w-4" />
				{t("configureEvent", "Configure event")}
			</DropdownMenuItem>
			<DropdownMenuItem
				onSelect={() => handlers.onNavigateToNode(event, event.node_id)}
			>
				<ExternalLinkIcon className="h-4 w-4" />
				{t("openInFlow", "Open in flow")}
			</DropdownMenuItem>
			<DropdownMenuItem
				disabled={!canEdit}
				variant="destructive"
				onSelect={() => handlers.onDelete(event.id)}
			>
				<Trash2Icon className="h-4 w-4" />
				{t("deleteEvent", "Delete event")}
			</DropdownMenuItem>
		</>
	);
}

/** Below 900 px of list width the row's actions collapse into one menu, Run on a device… first (APP §4.7). */
function RowMenu({
	row,
	run,
	busy,
	handlers,
	className,
}: Readonly<RowActionProps & { run: RunOnDevice; className?: string }>) {
	const { t } = useTranslation("devices");
	/* The popover opens once the menu has closed: opening it earlier loses it to the menu's focus return. */
	const afterClose = useRef<(() => void) | null>(null);
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="sm"
					className={cn("h-7 w-7 p-0", className)}
					aria-label={t("events.row.actions", "Actions for {{event}}", {
						event: row.event.name,
					})}
				>
					<EllipsisIcon className="h-4 w-4" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="end"
				className="w-64"
				onCloseAutoFocus={(closing) => {
					const next = afterClose.current;
					afterClose.current = null;
					if (!next) return;
					closing.preventDefault();
					next();
				}}
			>
				<RowMenuDeviceItems
					run={run}
					onExplain={() => {
						afterClose.current = run.show;
					}}
				/>
				<DropdownMenuSeparator />
				<RowMenuOwnItems row={row} busy={busy} handlers={handlers} />
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

const STRIPE: Record<EventStatus, string> = {
	attention: "bg-destructive",
	live: "bg-emerald-500/60",
	paused: "bg-transparent",
	unknown: "bg-muted-foreground/30",
	device: "bg-muted-foreground/30",
};

const DOT: Record<EventStatus, string> = {
	attention: "bg-destructive",
	live: "bg-emerald-500",
	paused: "bg-muted-foreground/50",
	unknown: "bg-muted-foreground/50",
	device: "bg-info-solid",
};

/** The row's own four actions as an icon strip, from 900 px of list width. */
function RowIconActions({ row, busy, handlers }: Readonly<RowActionProps>) {
	const { t } = useTranslation("settings");
	const { event } = row;
	const { canEdit, writeDeniedMessage, onToggleActive } = handlers;
	const toggleActiveLabel = event.active
		? t("pauseEvent", "Pause event")
		: t("resumeEvent", "Resume event");
	const ToggleIcon = busy ? Loader2Icon : event.active ? PauseIcon : PlayIcon;
	return (
		<div className="hidden gap-0.5 @min-[900px]/events:flex">
			{onToggleActive && (
				<Button
					variant="ghost"
					size="sm"
					className="h-7 w-7 p-0"
					disabled={busy || !canEdit}
					title={canEdit ? toggleActiveLabel : writeDeniedMessage}
					aria-label={toggleActiveLabel}
					onClick={() => onToggleActive(row)}
				>
					<ToggleIcon className={cn("h-4 w-4", busy && "animate-spin")} />
				</Button>
			)}
			<Button
				variant="ghost"
				size="sm"
				className="h-7 w-7 p-0"
				title={t("configureEvent", "Configure event")}
				aria-label={t("configureEvent", "Configure event")}
				onClick={() => handlers.onEdit(event)}
			>
				<SettingsIcon className="h-4 w-4" />
			</Button>
			<Button
				variant="ghost"
				size="sm"
				className="h-7 w-7 p-0"
				title={t("openInFlow", "Open in flow")}
				aria-label={t("openInFlow", "Open in flow")}
				onClick={() => handlers.onNavigateToNode(event, event.node_id)}
			>
				<ExternalLinkIcon className="h-4 w-4" />
			</Button>
			<Button
				variant="ghost"
				size="sm"
				className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
				disabled={!canEdit}
				title={canEdit ? t("delete", "Delete") : writeDeniedMessage}
				aria-label={t("deleteEvent", "Delete event")}
				onClick={() => handlers.onDelete(event.id)}
			>
				<Trash2Icon className="h-4 w-4" />
			</Button>
		</div>
	);
}

/** Name, type and sink chips on line 1; the top issue or the description on line 2. */
function EventNameBlock({
	row,
	isOffline,
	onEdit,
}: Readonly<
	Pick<EventRowHandlers, "isOffline" | "onEdit"> & { row: EventRowModel }
>) {
	const { t } = useTranslation("settings");
	const devices = useOptionalEventsDevices();
	const { event, status, topIssue } = row;
	const chip = "shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium";
	// A schedule or bot this place doesn't run may run on a device: "Not running" would be wrong for it.
	const onDevice =
		row.requiresSink && row.sinkActive === false
			? scheduleOnDevice(devices?.live?.rows.get(event.id))
			: null;
	return (
		<div className="min-w-0 overflow-hidden">
			<div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
				<span
					className={cn("h-1.5 w-1.5 shrink-0 rounded-full", DOT[status])}
					aria-hidden
				/>
				<button
					type="button"
					onClick={() => onEdit(event)}
					className="min-w-0 text-left text-sm font-semibold wrap-anywhere hover:underline"
				>
					{event.name}
				</button>
				<span className={cn(chip, "bg-secondary text-secondary-foreground")}>
					{formatEventTypeLabel(event.event_type)}
				</span>
				{row.deviceOnly && (
					<span data-device-only className={cn(chip, "bg-info-bg text-info")}>
						{t("devicesOnly", "Devices only")}
					</span>
				)}
				{onDevice && (
					<span
						data-sink-chip={onDevice}
						className={cn(chip, "bg-muted text-muted-foreground")}
					>
						{onDevice === "runs"
							? t("onADevice", "On a device")
							: onDevice === "ran"
								? t("ranOnADevice", "Ran on a device")
								: t("assignedToADevice", "Assigned to a device")}
					</span>
				)}
				{row.requiresSink && row.sinkActive === false && !onDevice && (
					<span
						data-sink-chip="off"
						className={cn(
							chip,
							"bg-amber-500/15 text-amber-700 dark:text-amber-400",
						)}
					>
						{t("notRunning", "Not running")}
					</span>
				)}
				{row.requiresSink && row.sinkActive === undefined && (
					<span className={cn(chip, "bg-muted text-muted-foreground")}>
						{row.sinkStatusLoading
							? t("checkingSinkStatus", "Checking status…")
							: t("sinkStatusUnavailable", "Status unavailable")}
					</span>
				)}
				{row.requiresSink && row.sinkActive && (
					<span
						className={cn(
							chip,
							"hidden bg-muted text-muted-foreground lg:inline",
						)}
					>
						{isOffline ? "Local" : "Online"}
					</span>
				)}
			</div>
			<div className="mt-0.5 line-clamp-2 text-xs">
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
	);
}

/** The route of an entry point (edited in place) or how a trigger fires. */
function EventEntryCell({
	row,
	handlers,
}: Readonly<{ row: EventRowModel; handlers: EventRowHandlers }>) {
	const { event, entry } = row;
	if (row.isRouted)
		return (
			<RouteChip
				path={row.routePath}
				canEdit={handlers.canEdit}
				writeDeniedMessage={handlers.writeDeniedMessage}
				onSave={(next) => handlers.onRouteChange(event.id, row.routePath, next)}
			/>
		);
	if (!entry) return null;
	return (
		<span
			title={entry.title ?? entry.text}
			className={cn(
				"inline-block max-w-full truncate rounded border px-1.5 py-0.5 font-mono text-[11.5px] leading-[1.45]",
				entry.muted
					? "border-border bg-muted/50 text-muted-foreground"
					: "border-primary/25 bg-primary/10 text-primary",
			)}
		>
			{entry.text}
		</span>
	);
}

/** A phone row has no route or trigger column: the same fact as one line above the Devices summary (APP §4.7). */
function EventEntryLine({ row }: Readonly<{ row: EventRowModel }>) {
	const { t } = useTranslation("settings");
	const { t: td } = useTranslation("devices");
	const line = "truncate text-[12.5px]/[17px] text-muted-foreground";
	if (!row.isRouted)
		return row.entry ? (
			<span className={cn(line, "font-mono text-xs")}>{row.entry.text}</span>
		) : null;
	return (
		<span className={line}>
			{row.routePath ? (
				<>
					{td("events.column.route", "Route")}{" "}
					<span className="font-mono text-xs">{row.routePath}</span>
				</>
			) : (
				t("noRoute", "No route")
			)}
		</span>
	);
}

function EventRow({ row, busy, handlers }: Readonly<RowActionProps>) {
	const { t } = useTranslation("settings");
	const { t: td } = useTranslation("devices");
	const { event, status, glyph } = row;
	const run = useRunOnDevice(event.id);
	const boardName = handlers.boardsMap.get(event.board_id);
	const version = event.board_version
		? `v${event.board_version.join(".")}`
		: t("latest", "Latest");
	const runsFailed = row.health?.failed ?? 0;

	return (
		<div
			id={eventRowId(event.id)}
			data-event-target={run.targeted || undefined}
			className={cn(
				ROW_GRID,
				"group min-h-12.5 items-start gap-y-0.5 border-b py-2.5 last:border-b-0",
				"transition-colors hover:bg-muted/50",
				run.targeted && "bg-row-selected",
			)}
		>
			<div
				className={cn(
					"row-span-2 -my-2.5 self-stretch @min-[600px]/events:row-span-1",
					STRIPE[status],
				)}
				aria-hidden
			/>

			<EventTile icon={glyph.icon} />

			<EventNameBlock
				row={row}
				isOffline={handlers.isOffline}
				onEdit={handlers.onEdit}
			/>

			<div className={cn(FROM_600, "min-w-0 overflow-hidden pt-0.5")}>
				<EventEntryCell row={row} handlers={handlers} />
			</div>

			<div className="col-start-3 row-start-2 min-w-0 @min-[600px]/events:col-auto @min-[600px]/events:row-auto @min-[600px]/events:pt-0.5">
				<div className={FROM_600}>
					<RunsOnCell appId={handlers.appId} eventId={event.id} />
				</div>
				<div className="flex flex-col gap-0.5 @min-[600px]/events:hidden">
					<EventEntryLine row={row} />
					<RunsOnCell appId={handlers.appId} eventId={event.id} compact />
				</div>
			</div>

			<div className={cn(FROM_900, "min-w-0 overflow-hidden pt-0.5")}>
				<div className="truncate text-[13px]">
					{boardName ?? td("events.flow.unknown", "Unknown flow")}
				</div>
				<div className="truncate font-mono text-[11px] text-muted-foreground">
					{version}
				</div>
			</div>

			<div className="hidden min-w-0 items-center gap-1.5 pt-1 text-xs tabular-nums text-muted-foreground @min-[1100px]/events:flex">
				<RunSparkline trend={row.health?.trend} failed={runsFailed} />
				{runs24hLabel(row.health?.total ?? 0, runsFailed)}
			</div>

			<div className="flex items-center justify-end gap-0.5 opacity-60 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
				<RunOnDeviceButton
					run={run}
					className="hidden @min-[600px]/events:inline-flex"
				/>
				<RowMenu
					row={row}
					run={run}
					busy={busy}
					handlers={handlers}
					className="@min-[900px]/events:hidden"
				/>
				<RowIconActions row={row} busy={busy} handlers={handlers} />
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
			title={
				empty
					? t("noRunsInTheLast24Hours", "No runs in the last 24 hours")
					: undefined
			}
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
	canEdit,
	writeDeniedMessage,
	onSave,
}: Readonly<{
	path?: string;
	canEdit: boolean;
	writeDeniedMessage: string;
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
				aria-label={t("routePath2", "Route path")}
				disabled={saving}
				autoFocus
				className="h-7 w-full font-mono text-xs"
			/>
		);
	}

	return (
		<button
			type="button"
			disabled={!canEdit}
			onClick={() => setEditing(true)}
			title={
				!canEdit
					? writeDeniedMessage
					: path
						? t("pathClickToEdit", "{{path}} — click to edit", { path })
						: t("clickToSetARoute", "Click to set a route")
			}
			className={cn(
				"inline-flex max-w-full items-center gap-1.5 truncate rounded border px-1.5 py-0.5 font-mono text-[11.5px] leading-[1.45] transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-60",
				path
					? "border-primary/25 bg-primary/10 text-primary"
					: "border-destructive/25 bg-destructive/10 text-destructive",
			)}
		>
			<SlidersHorizontalIcon className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-60" />
			<span className="truncate">{path ?? t("noRoute", "No route")}</span>
		</button>
	);
}
