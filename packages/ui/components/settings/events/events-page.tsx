"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	EventForm,
	EventTranslation,
	EventTypeConfiguration,
	type IEvent,
	IEventExecutionMode,
	IEventExposure,
	type IEventInput,
	type IEventMapping,
	type IOAuthProvider,
	type IOAuthToken,
	Input,
	Label,
	OAuthConsentDialog,
	PatSelectorDialog,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Textarea,
	VariableConfigCard,
	VariableTypeIndicator,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
	useIsMobile,
} from "@flow-like/flow-like-ui";
import type { IOAuthConsentStore } from "@flow-like/flow-like-ui/db/oauth-db";
import type { EventSectionId } from "@flow-like/flow-like-ui/lib/event-sections";
import {
	getEventSections,
	isTriggerSection,
} from "@flow-like/flow-like-ui/lib/event-sections";
import {
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
} from "@flow-like/flow-like-ui/lib/oauth/helpers";
import type {
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "@flow-like/flow-like-ui/lib/oauth/types";
import { normalizeRoutePath } from "@flow-like/flow-like-ui/lib/route-path";
import {
	isEventOverridable,
	isRuntimeConfigured,
} from "@flow-like/flow-like-ui/lib/runtime-vars-utils";
import { normalizeBoardVersion } from "@flow-like/flow-like-ui/lib/schema/flow/board-version";
import type { IHub } from "@flow-like/flow-like-ui/lib/schema/hub/hub";
import { stableStringify } from "@flow-like/flow-like-ui/lib/stable-stringify";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "@flow-like/flow-like-ui/lib/uint8";
import type { PageListItem } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import { Trans, useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	AlertTriangle,
	Cloud,
	CodeIcon,
	CogIcon,
	ExternalLinkIcon,
	FileTextIcon,
	FormInputIcon,
	GitBranchIcon,
	Globe,
	LayersIcon,
	Loader2,
	Lock,
	Monitor,
	Pause,
	Play,
	Plus,
	RefreshCw,
	Settings,
	StickyNote,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { EventAttentionStrip } from "./event-attention-strip";
import { EventSaveBar } from "./event-save-bar";
import { EventSectionRail } from "./event-section-rail";
import { EventsOverview } from "./events-overview";
import { SectionGuidance } from "./section-guidance";
import { SetupChecklist } from "./setup-checklist";
import { isHeadlessEventType, useEventIssues } from "./use-event-issues";

function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	return "Unexpected error";
}

// Helper function to check if an event requires a sink based on eventMapping
function eventRequiresSink(
	eventMapping: IEventMapping,
	event: IEvent,
	nodeName?: string,
): boolean {
	if (!nodeName) return false;
	const eventTypeConfig = eventMapping[nodeName];
	return eventTypeConfig?.withSink.includes(event.event_type) ?? false;
}

export interface EventsPageProps {
	eventMapping: IEventMapping;
	/** Optional list of event types that are UI-capable and should request a unique route path on creation. */
	uiEventTypes?: string[];
	/** Token store for OAuth checks. If not provided, OAuth checks are skipped. */
	tokenStore?: IOAuthTokenStoreWithPending;
	/** Consent store for OAuth consent tracking. */
	consentStore?: IOAuthConsentStore;
	/** Hub configuration for OAuth provider resolution */
	hub?: IHub;
	/** Callback to start OAuth authorization for a provider */
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	/** Optional callback to refresh expired tokens */
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
	/** Base path for routing (defaults to /library/config/events) */
	basePath?: string;
	appId?: string | null;
	eventId?: string | null;
	embedded?: boolean;
	onEventIdChange?: (eventId: string | null) => void;
	onNavigateToFlow?: (target: {
		boardId: string;
		appId: string;
		nodeId?: string;
		version?: [number, number, number];
	}) => void;
	/**
	 * Optional pre-filled template used to seed the "Create event" dialog
	 * (e.g. driven by ?newEvent=... deep links from the University runtime).
	 * When provided, the create dialog opens automatically on mount with the
	 * template applied.
	 */
	newEventTemplate?: Partial<IEvent>;
}

export default function EventsPage({
	eventMapping,
	uiEventTypes,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
	basePath = "/library/config/events",
	appId: appIdProp,
	eventId: eventIdProp,
	embedded = false,
	onEventIdChange,
	onNavigateToFlow,
	newEventTemplate,
}: Readonly<EventsPageProps>) {
	const { t } = useTranslation("settings");
	const searchParams = useSearchParams();
	const id = appIdProp ?? searchParams.get("id");
	const eventId = eventIdProp ?? searchParams.get("eventId");

	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
	const newEventTemplateKey = useMemo(
		() => (newEventTemplate ? JSON.stringify(newEventTemplate) : ""),
		[newEventTemplate],
	);
	const newEventTemplateAppliedRef = useRef("");
	useEffect(() => {
		if (!newEventTemplate) return;
		if (newEventTemplateAppliedRef.current === newEventTemplateKey) return;
		newEventTemplateAppliedRef.current = newEventTemplateKey;
		setIsCreateDialogOpen(true);
	}, [newEventTemplate, newEventTemplateKey]);
	const [isCreating, setIsCreating] = useState(false);
	const [editingEvent, setEditingEvent] = useState<IEvent | null>(null);
	const [showCreatePatDialog, setShowCreatePatDialog] = useState(false);
	const [pendingEvent, setPendingEvent] = useState<IEvent | null>(null);
	const [pendingRoutePath, setPendingRoutePath] = useState<string | null>(null);
	const [isOffline, setIsOffline] = useState<boolean | null>(null);
	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);
	const router = useRouter();
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id ?? ""],
		(id ?? "") !== "",
	);

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[id ?? ""],
		(id ?? "") !== "",
	);

	const boardsMap = useMemo(() => {
		const map = new Map<string, string>();
		boards.data?.forEach((board) => map.set(board.id, board.name));
		return map;
	}, [boards.data]);

	useEffect(() => {
		setEditingEvent(events.data?.find((event) => event.id === eventId) ?? null);
	}, [editingEvent, id, eventId, events.data]);

	// A route cannot be orphaned: server-side it is the `route` column on the
	// event row itself, so it disappears with the event. Reconciling the two
	// lists here only ever deleted live routes, because `GET /routes` is a
	// superset of `GET /events` (which filters by type, active flag and
	// permission) and because a failed event fetch is indistinguishable from an
	// app with no events.

	// Check if app is offline
	useEffect(() => {
		const checkOffline = async () => {
			if (id) {
				const offline = await backend.isOffline(id);
				setIsOffline(offline);
			}
		};
		checkOffline();
	}, [id, backend]);

	const handleCreateEvent = useCallback(
		async (
			newEvent: Partial<IEvent>,
			selectedPatOrOAuthTokens?: string | Record<string, IOAuthToken>,
		) => {
			if (!id) {
				console.error("App ID is required to create an event");
				return;
			}
			if (isCreating) {
				return;
			}
			setIsCreating(true);

			// Determine if we got a PAT string or OAuth tokens
			const selectedPat =
				typeof selectedPatOrOAuthTokens === "string"
					? selectedPatOrOAuthTokens
					: undefined;
			const oauthTokens =
				typeof selectedPatOrOAuthTokens === "object"
					? selectedPatOrOAuthTokens
					: undefined;

			const event: IEvent = {
				id: createId(),
				name: newEvent.name ?? "New Event",
				description: newEvent.description ?? "",
				active: true,
				board_id: newEvent.board_id ?? "",
				board_version: newEvent.board_version ?? undefined,
				config: newEvent.config ?? [],
				created_at: {
					secs_since_epoch: Math.floor(Date.now() / 1000),
					nanos_since_epoch: 0,
				},
				updated_at: {
					secs_since_epoch: Math.floor(Date.now() / 1000),
					nanos_since_epoch: 0,
				},
				event_version: [0, 0, 0],
				node_id: newEvent.node_id ?? "",
				variables: newEvent.variables ?? {},
				event_type: newEvent.event_type ?? "default",
				default_page_id: (newEvent as any)?.default_page_id ?? undefined,
				priority: events.data?.length ?? 0,
				canary: null,
				notes: null,
				execution_mode:
					(newEvent as any)?.execution_mode ?? IEventExecutionMode.Local,
				exposure: (newEvent as any)?.exposure ?? IEventExposure.Public,
			};

			let savedEvent: IEvent | null = null;
			try {
				// Check if the event requires a sink and PAT is needed
				if (event.board_id && event.node_id) {
					try {
						const board = await backend.boardState.getBoard(
							id,
							event.board_id,
							event.board_version as [number, number, number] | undefined,
						);
						const node = board?.nodes?.[event.node_id];
						if (node?.name) {
							const requiresSink = eventRequiresSink(
								eventMapping,
								event,
								node.name,
							);

							if (requiresSink && !isOffline && !selectedPat) {
								// Store the event and route path, then show PAT dialog
								setPendingEvent(event);
								setPendingRoutePath((newEvent as any)?.path ?? null);
								setShowCreatePatDialog(true);
								return;
							}
						}
					} catch (error) {
						console.error("Failed to fetch board for sink check:", error);
					}
				}

				savedEvent = await backend.eventState.upsertEvent(
					id,
					event,
					undefined,
					selectedPat,
					oauthTokens,
				);

				// If this is a UI event (including page-target events), create a path-based route pointing to it.
				// Use savedEvent.id since the backend may generate a new ID for new events
				if (
					uiEventTypeSet.has(savedEvent.event_type) ||
					!!savedEvent.default_page_id
				) {
					try {
						const path = normalizeRoutePath((newEvent as any)?.path);
						await backend.routeState.setRoute(id, path, savedEvent.id);
						await invalidate(backend.routeState.getRoutes, [id]);
					} catch (error) {
						console.error("Failed to create route for UI event:", error);
					}
				}

				await invalidate(backend.eventState.getEvents, [id]);
				await events.refetch();
				toast.success(`Event "${savedEvent.name}" created`);
			} catch (error) {
				console.error("Failed to create event:", error);
				toast.error(`Failed to create event: ${errorMessage(error)}`);
			} finally {
				if (savedEvent) {
					setIsCreateDialogOpen(false);
					setShowCreatePatDialog(false);
					setPendingEvent(null);
					setPendingRoutePath(null);
				}
				setIsCreating(false);
			}
		},
		[
			id,
			events,
			backend.eventState,
			backend.boardState,
			backend.routeState,
			eventMapping,
			isOffline,
			uiEventTypeSet,
			invalidate,
			isCreating,
		],
	);

	const handleDeleteEvent = useCallback(
		async (eventId: string) => {
			if (!id) {
				console.error("App ID is required to delete an event");
				return;
			}
			try {
				// Delete route pointing to this event (non-fatal)
				try {
					await backend.routeState.deleteRouteByEvent(id, eventId);
					await invalidate(backend.routeState.getRoutes, [id]);
				} catch (routeError) {
					console.warn("Failed to delete route for event:", routeError);
				}

				await backend.eventState.deleteEvent(id, eventId);
				toast.success("Event deleted");
			} catch (e) {
				console.error("Failed to delete event:", e);
				toast.error(`Failed to delete event: ${errorMessage(e)}`);
			} finally {
				if (editingEvent?.id === eventId) {
					setEditingEvent(null);
				}
				await invalidate(backend.eventState.getEvents, [id]);
				await events.refetch();
			}
		},
		[
			id,
			editingEvent,
			events,
			backend.eventState,
			backend.routeState,
			invalidate,
		],
	);

	const handleEditingEvent = useCallback(
		(event?: IEvent) => {
			if (embedded) {
				onEventIdChange?.(event?.id ?? null);
				return;
			}
			let additionalParams = "";
			if (event?.id) {
				additionalParams = `&eventId=${event.id}`;
			}

			router.push(`${basePath}?id=${id}${additionalParams}`);
		},
		[id, router, basePath, embedded, onEventIdChange],
	);

	const handleNavigateToNode = useCallback(
		(event: IEvent, nodeId: string) => {
			if (embedded && id && event.board_id) {
				onNavigateToFlow?.({
					boardId: event.board_id,
					appId: id,
					nodeId,
					version: event.board_version as [number, number, number] | undefined,
				});
				return;
			}
			router.push(
				`/flow?id=${event.board_id}&app=${id}&node=${nodeId}${event.board_version ? `&version=${event.board_version.join("_")}` : ""}`,
			);
		},
		[id, router, embedded, onNavigateToFlow],
	);

	const handleCreateWithPat = useCallback(
		async (selectedPat: string) => {
			if (pendingEvent && id) {
				if (isCreating) {
					return;
				}
				setIsCreating(true);
				let savedEvent: IEvent | null = null;
				try {
					savedEvent = await backend.eventState.upsertEvent(
						id,
						pendingEvent,
						undefined,
						selectedPat,
					);

					// Create route for UI events - use savedEvent.id since backend may generate new ID
					if (
						uiEventTypeSet.has(savedEvent.event_type) ||
						!!savedEvent.default_page_id
					) {
						try {
							const path = normalizeRoutePath(pendingRoutePath);
							await backend.routeState.setRoute(id, path, savedEvent.id);
							await invalidate(backend.routeState.getRoutes, [id]);
						} catch (error) {
							console.error("Failed to create route for UI event:", error);
						}
					}

					await invalidate(backend.eventState.getEvents, [id]);
					await events.refetch();
				} catch (error) {
					console.error("Failed to create event with PAT:", error);
				} finally {
					if (savedEvent) {
						setIsCreateDialogOpen(false);
						setShowCreatePatDialog(false);
						setPendingEvent(null);
						setPendingRoutePath(null);
					}
					setIsCreating(false);
				}
			}
		},
		[
			pendingEvent,
			pendingRoutePath,
			id,
			backend.eventState,
			backend.routeState,
			events,
			uiEventTypeSet,
			invalidate,
			isCreating,
		],
	);

	if (id && editingEvent) {
		return (
			<EventConfiguration
				eventMapping={eventMapping}
				uiEventTypes={uiEventTypes}
				appId={id}
				event={editingEvent}
				onDone={() => handleEditingEvent()}
				onReload={async () => {
					await events.refetch();
				}}
				tokenStore={tokenStore}
				consentStore={consentStore}
				hub={hub}
				onStartOAuth={onStartOAuth}
				onRefreshToken={onRefreshToken}
				onNavigateToFlow={onNavigateToFlow}
			/>
		);
	}

	return (
		<div className="container mx-auto flex max-h-full grow flex-col px-3 md:px-0">
			<div className="flex flex-col grow overflow-hidden max-h-full">
				<div className="flex flex-col overflow-auto overflow-x-visible grow h-full max-h-full">
					{events.data?.length === 0 ? (
						<Card>
							<CardContent className="py-12 text-center">
								<Settings className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
								<h3 className="text-lg font-semibold mb-2">
									{t("noEventsConfigured", "No events configured")}
								</h3>
								<p className="text-muted-foreground mb-4">
									{t(
										"getStartedByCreatingYourFirstEvent",
										"Get started by creating your first event",
									)}
								</p>
								<Button
									onClick={() => setIsCreateDialogOpen(true)}
									className="gap-2"
								>
									<Plus className="h-4 w-4" />
									{t("createEvent", "Create Event")}
								</Button>
							</CardContent>
						</Card>
					) : (
						<EventsOverview
							events={events.data ?? []}
							boardsMap={boardsMap}
							appId={id ?? ""}
							eventMapping={eventMapping}
							uiEventTypes={uiEventTypes}
							onEdit={handleEditingEvent}
							onDelete={handleDeleteEvent}
							onNavigateToNode={handleNavigateToNode}
							onCreateEvent={() => setIsCreateDialogOpen(true)}
							tokenStore={tokenStore}
							consentStore={consentStore}
							hub={hub}
							onStartOAuth={onStartOAuth}
							onRefreshToken={onRefreshToken}
							isOffline={isOffline ?? undefined}
						/>
					)}
				</div>
			</div>

			<Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
				<DialogContent className="max-w-2xl">
					<DialogHeader>
						<DialogTitle>{t("createNewEvent", "Create New Event")}</DialogTitle>
						<DialogDescription>
							{`Configure a new event with its properties and settings`}
						</DialogDescription>
					</DialogHeader>
					<DialogBody>
						{id && (
							<EventForm
								eventConfig={eventMapping}
								uiEventTypes={uiEventTypes}
								appId={id}
								event={newEventTemplate as IEvent | undefined}
								onSubmit={handleCreateEvent}
								onCancel={() => setIsCreateDialogOpen(false)}
								isSubmitting={isCreating}
								tokenStore={tokenStore}
								consentStore={consentStore}
								hub={hub}
								onStartOAuth={onStartOAuth}
								onRefreshToken={onRefreshToken}
							/>
						)}
					</DialogBody>
				</DialogContent>
			</Dialog>

			{/* PAT Selector Dialog for Event Creation */}
			<PatSelectorDialog
				open={showCreatePatDialog}
				onOpenChange={setShowCreatePatDialog}
				onPatSelected={handleCreateWithPat}
				title={t("createEventWithSink", "Create Event with Sink")}
				description={t(
					"thisEventRequiresASinkSelectOrCreateAPersonalAccessTokenToActivateTheEventSink",
					"This event requires a sink. Select or create a Personal Access Token to activate the event sink.",
				)}
			/>
		</div>
	);
}

function EventConfiguration({
	eventMapping,
	uiEventTypes,
	event,
	appId,
	onDone,
	onReload,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
	onNavigateToFlow,
}: Readonly<{
	eventMapping: IEventMapping;
	/** Optional list of event types that are UI-capable and should have a route path. */
	uiEventTypes?: string[];
	event: IEvent;
	appId: string;
	onDone?: () => void;
	onReload?: () => void;
	/** Token store for OAuth checks. If not provided, OAuth checks are skipped. */
	tokenStore?: IOAuthTokenStoreWithPending;
	/** Consent store for OAuth consent tracking. */
	consentStore?: IOAuthConsentStore;
	/** Hub configuration for OAuth provider resolution */
	hub?: IHub;
	/** Callback to start OAuth authorization for a provider */
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	/** Optional callback to refresh expired tokens */
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
	onNavigateToFlow?: (target: {
		boardId: string;
		appId: string;
		nodeId?: string;
		version?: [number, number, number];
	}) => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const isMobile = useIsMobile();
	const [isEditing, setIsEditing] = useState(false);
	const [formData, setFormData] = useState<IEvent>(event);
	const [showPatDialog, setShowPatDialog] = useState(false);
	const [isSaving, setIsSaving] = useState(false);
	const [isOffline, setIsOffline] = useState<boolean | null>(null);
	const canExecuteLocally = backend.capabilities().canExecuteLocally;
	const [isRefreshingInputs, setIsRefreshingInputs] = useState(false);
	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);
	const [routePathDraft, setRoutePathDraft] = useState<string>("/");
	const [routePathError, setRoutePathError] = useState<string | null>(null);
	// Case-key mapping rows are edited locally (index-stable, so typing a key
	// name never collides mid-edit) and committed to formData on every change.
	const [caseKeyRows, setCaseKeyRows] = useState<
		Array<{ key: string; path: string }>
	>([]);

	useEffect(() => {
		setCaseKeyRows(
			Object.entries(event.correlation_mappings ?? {}).map(([key, path]) => ({
				key,
				path,
			})),
		);
	}, [event.id, event.correlation_mappings]);

	const commitCaseKeyRows = useCallback(
		(rows: Array<{ key: string; path: string }>) => {
			setCaseKeyRows(rows);
			const mappings: Record<string, string> = {};
			for (const row of rows) {
				const key = row.key.trim();
				const path = row.path.trim();
				if (key && path) mappings[key] = path;
			}
			setFormData((previous) => ({
				...previous,
				correlation_mappings:
					Object.keys(mappings).length > 0 ? mappings : null,
			}));
		},
		[],
	);

	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		(appId ?? "") !== "",
	);

	const routeForEvent = useMemo(() => {
		return routes.data?.find((r) => r.eventId === event.id) ?? null;
	}, [routes.data, event.id]);

	// Until the route list has actually loaded, `routeForEvent` is null for an
	// event that does have a route. Seeding the draft with "/" then would let a
	// save overwrite the real path with the placeholder.
	useEffect(() => {
		if (isEditing || !routes.isSuccess) return;
		setRoutePathDraft(routeForEvent?.path ?? "/");
		setRoutePathError(null);
	}, [routeForEvent?.path, isEditing, routes.isSuccess]);

	// OAuth consent state
	const [showOAuthConsent, setShowOAuthConsent] = useState(false);
	const [oauthMissingProviders, setOauthMissingProviders] = useState<
		IOAuthProvider[]
	>([]);
	const [oauthAuthorizedProviders, setOauthAuthorizedProviders] = useState<
		Set<string>
	>(new Set());
	const [oauthPreAuthorizedProviders, setOauthPreAuthorizedProviders] =
		useState<Set<string>>(new Set());
	const [pendingOAuthTokens, setPendingOAuthTokens] = useState<
		Record<string, IOAuthToken>
	>({});

	const isPageTargetEvent = !!formData.default_page_id;
	const shouldShowRoutePath =
		uiEventTypeSet.has(formData.event_type) || isPageTargetEvent;

	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId],
		!!appId && isEditing && !isPageTargetEvent,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		!!appId && isEditing,
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, formData.board_id, normalizeBoardVersion(formData.board_version)],
		!!formData.board_id && !isPageTargetEvent,
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, formData.board_id],
		(formData.board_id ?? "") !== "" && isEditing,
	);

	// Check if app is offline
	useEffect(() => {
		const checkOffline = async () => {
			const offline = await backend.isOffline(appId);
			setIsOffline(offline);
		};
		if (appId) {
			checkOffline();
		}
	}, [appId, backend]);

	// Poll for OAuth token updates while the consent dialog is open
	useEffect(() => {
		if (
			!showOAuthConsent ||
			!tokenStore ||
			oauthMissingProviders.length === 0
		) {
			return;
		}

		const checkTokens = async () => {
			const newlyAuthorized = new Set(oauthAuthorizedProviders);
			const newTokens = { ...pendingOAuthTokens };

			for (const provider of oauthMissingProviders) {
				if (
					newlyAuthorized.has(provider.id) ||
					oauthPreAuthorizedProviders.has(provider.id)
				) {
					continue;
				}

				const token = await tokenStore.getToken(provider.id);
				if (token && !tokenStore.isExpired(token)) {
					newlyAuthorized.add(provider.id);
					newTokens[provider.id] = {
						access_token: token.access_token,
						refresh_token: token.refresh_token,
						expires_at: token.expires_at
							? Math.floor(token.expires_at / 1000)
							: undefined,
						token_type: token.token_type ?? "Bearer",
					};
				}
			}

			if (newlyAuthorized.size !== oauthAuthorizedProviders.size) {
				setOauthAuthorizedProviders(newlyAuthorized);
				setPendingOAuthTokens(newTokens);
			}
		};

		// Check immediately and then poll every second
		checkTokens();
		const interval = setInterval(checkTokens, 1000);
		return () => clearInterval(interval);
	}, [
		showOAuthConsent,
		tokenStore,
		oauthMissingProviders,
		oauthAuthorizedProviders,
		oauthPreAuthorizedProviders,
		pendingOAuthTokens,
	]);

	const handleInputChange = (field: keyof IEvent, value: any) => {
		setFormData((prev) => ({ ...prev, [field]: value }));
	};

	const checkRequiresSink = (): boolean => {
		const node = board.data?.nodes?.[formData.node_id];
		if (!node) return false;
		const eventTypeConfig = eventMapping[node?.name];
		return eventTypeConfig?.withSink.includes(formData.event_type);
	};

	const runSave = async (
		selectedPat?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	) => {
		setRoutePathError(null);
		const isUiEvent = uiEventTypeSet.has(formData.event_type);
		const isPageTargetEvent = !!formData.default_page_id;
		const shouldHaveRoute = isUiEvent || isPageTargetEvent;
		const desiredRoutePath = shouldHaveRoute
			? normalizeRoutePath(routePathDraft)
			: null;

		// The draft is only trustworthy once the existing routes are known —
		// otherwise the placeholder "/" would be written over the real path.
		if (shouldHaveRoute && !routes.isSuccess) {
			setRoutePathError("Route path is still loading, please retry");
			toast.error("Route path is still loading, please retry");
			return;
		}

		const requiresSink = checkRequiresSink();

		// Check OAuth requirements first if we have the stores
		if (tokenStore && consentStore && onStartOAuth && !oauthTokens) {
			let oauthResult: Awaited<ReturnType<typeof checkOAuthTokens>> | undefined;

			// Try board first, fallback to prerun for execute-only permissions
			if (board.data) {
				oauthResult = await checkOAuthTokens(board.data, tokenStore, hub, {
					refreshToken: onRefreshToken,
				});
			} else if (backend.eventState.prerunEvent && formData.board_id) {
				try {
					const prerun = await backend.eventState.prerunEvent(
						appId,
						event.id,
						event.board_version as [number, number, number] | undefined,
					);
					oauthResult = await checkOAuthTokensFromPrerun(
						prerun.oauth_requirements,
						tokenStore,
						hub,
						{ refreshToken: onRefreshToken },
					);
				} catch {
					// Prerun not available, skip OAuth check
				}
			}

			if (oauthResult && oauthResult.requiredProviders.length > 0) {
				// Check consent for providers that have tokens but might not have consent for this app
				const consentedIds = await consentStore.getConsentedProviderIds(appId);
				const providersNeedingConsent: IOAuthProvider[] = [];
				const hasTokenNeedsConsent: Set<string> = new Set();
				const alreadyAuthorized: Set<string> = new Set();

				// Add providers that are missing tokens
				providersNeedingConsent.push(...oauthResult.missingProviders);

				// Also add providers that have tokens but no consent for this specific app
				for (const provider of oauthResult.requiredProviders) {
					const hasToken = oauthResult.tokens[provider.id] !== undefined;
					const hasConsent = consentedIds.has(provider.id);

					if (hasToken && !hasConsent) {
						hasTokenNeedsConsent.add(provider.id);
						providersNeedingConsent.push(provider);
					} else if (hasToken && hasConsent) {
						alreadyAuthorized.add(provider.id);
					}
				}

				if (providersNeedingConsent.length > 0) {
					setOauthMissingProviders(providersNeedingConsent);
					setOauthAuthorizedProviders(alreadyAuthorized);
					setOauthPreAuthorizedProviders(hasTokenNeedsConsent);
					setPendingOAuthTokens(oauthResult.tokens);
					setShowOAuthConsent(true);
					return;
				}
			}

			// If we have tokens but no missing providers, use those tokens
			if (oauthResult && Object.keys(oauthResult.tokens).length > 0) {
				oauthTokens = oauthResult.tokens;
			}
		}

		if (requiresSink && !isOffline && !selectedPat) {
			// Show PAT selector dialog
			setShowPatDialog(true);
			return;
		}

		if (shouldHaveRoute) {
			const existingRoutes =
				routes.data ?? (await backend.routeState.getRoutes(appId));
			const conflict = existingRoutes.find((r) => {
				const normalized = normalizeRoutePath(r.path);
				if (normalized !== desiredRoutePath) return false;
				// Allow if this event already owns this path
				return r.eventId !== event.id;
			});
			if (conflict) {
				const message = t(
					"routePathAlreadyInUseDesiredroutepath",
					"Route path already in use: {{desiredRoutePath}}",
					{ desiredRoutePath },
				);
				setRoutePathError(message);
				toast.error(message);
				return;
			}
		}

		// Save the event with the PAT and OAuth tokens if provided
		const saved = await backend.eventState.upsertEvent(
			appId,
			formData,
			undefined,
			selectedPat,
			oauthTokens,
		);

		// The backend stamps `updated_at` and re-populates `inputs` on every
		// upsert, so the draft can never match the stored event again. Without
		// adopting the response the dirty check stays true forever: the bar keeps
		// claiming unsaved changes right after a successful save, which reads as
		// "saving is broken" and invites repeated taps.
		setFormData(saved);

		if (shouldHaveRoute && desiredRoutePath) {
			try {
				// If path changed, delete old route first
				if (routeForEvent && routeForEvent.path !== desiredRoutePath) {
					await backend.routeState.deleteRouteByPath(appId, routeForEvent.path);
				}
				// Set new route
				await backend.routeState.setRoute(appId, desiredRoutePath, saved.id);
				await routes.refetch();
			} catch (error) {
				console.error("Failed to upsert route for UI event:", error);
				setRoutePathError("Failed to save route path");
				toast.error(
					t(
						"eventSavedButTheRoutePathCouldNotBeUpdatedVal",
						"Event saved, but the route path could not be updated: {{val}}",
						{ val: errorMessage(error) },
					),
				);
				return;
			}
		}
		onReload?.();
		setIsEditing(false);
		setShowPatDialog(false);
		toast.success(`"${saved.name}" saved`);
	};

	// Saving is the one action on this page with real consequences, and until now
	// it reported nothing: a rejected upsert surfaced as an unhandled rejection
	// and the bar just stayed up. Every outcome is now visible, and the in-flight
	// flag stops a double tap from firing two writes.
	const savingRef = useRef(false);
	const handleSave = async (
		selectedPat?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	) => {
		if (savingRef.current) return;
		savingRef.current = true;
		setIsSaving(true);
		try {
			await runSave(selectedPat, oauthTokens);
		} catch (error) {
			console.error("Failed to save event:", error);
			toast.error(`Failed to save event: ${errorMessage(error)}`);
		} finally {
			savingRef.current = false;
			setIsSaving(false);
		}
	};

	const handleOAuthAuthorize = async (providerId: string) => {
		const provider = oauthMissingProviders.find((p) => p.id === providerId);
		if (!provider || !onStartOAuth) return;
		await onStartOAuth(provider);
	};

	const handleOAuthConfirmAll = async (rememberConsent: boolean) => {
		if (rememberConsent && consentStore) {
			for (const provider of oauthMissingProviders) {
				await consentStore.setConsent(appId, provider.id, provider.scopes);
			}
		}

		setShowOAuthConsent(false);

		// Collect all tokens (pending + newly authorized)
		const allTokens = { ...pendingOAuthTokens };
		for (const providerId of oauthAuthorizedProviders) {
			if (tokenStore) {
				const token = await tokenStore.getToken(providerId);
				if (token && !tokenStore.isExpired(token)) {
					allTokens[providerId] = {
						access_token: token.access_token,
						refresh_token: token.refresh_token,
						expires_at: token.expires_at
							? Math.floor(token.expires_at / 1000)
							: undefined,
						token_type: token.token_type ?? "Bearer",
					};
				}
			}
		}

		// Continue with save, passing the OAuth tokens
		await handleSave(undefined, allTokens);
	};

	const handleOAuthCancel = () => {
		setShowOAuthConsent(false);
		setOauthMissingProviders([]);
		setOauthAuthorizedProviders(new Set());
		setOauthPreAuthorizedProviders(new Set());
		setPendingOAuthTokens({});
	};

	const handleCancel = () => {
		setFormData(event);
		setIsEditing(false);
	};

	// Refresh inputs from the current node definition
	const handleRefreshInputs = async () => {
		setIsRefreshingInputs(true);
		try {
			// Re-upsert the event to trigger populate_inputs on the backend
			const saved = await backend.eventState.upsertEvent(
				appId,
				event,
				undefined,
				undefined,
				undefined,
			);
			setFormData(saved);
			await invalidate(backend.eventState.getEvents, [appId]);
			onReload?.();
			toast.success("Inputs refreshed");
		} catch (error) {
			console.error("Failed to refresh inputs:", error);
			toast.error(`Failed to refresh inputs: ${errorMessage(error)}`);
		} finally {
			setIsRefreshingInputs(false);
		}
	};

	// Compute inputs drift by comparing event.inputs with current node pins
	const inputsDrift = useMemo(() => {
		if (!board.data || !event.node_id) return null;

		const node = board.data.nodes?.[event.node_id];
		if (!node) return null;

		// For page-target events (A2UI/generic form), check Input pins
		// For regular events, check Output pins
		const targetPinType = event.default_page_id ? "Input" : "Output";

		const currentPins = Object.values(node.pins ?? {})
			.filter(
				(pin: any) =>
					pin.pin_type === targetPinType && pin.data_type !== "Execution",
			)
			.sort((a: any, b: any) => a.index - b.index);

		const savedInputs = event.inputs ?? [];

		// Check for differences
		const added: Array<{ id: string; name: string; friendly_name: string }> =
			[];
		const removed: IEventInput[] = [];
		const changed: Array<{
			id: string;
			name: string;
			field: string;
			oldValue: string;
			newValue: string;
		}> = [];

		const savedInputsMap = new Map(savedInputs.map((i) => [i.id, i]));
		const currentPinsMap = new Map(currentPins.map((p: any) => [p.id, p]));

		// Find added pins (in current but not in saved)
		for (const pin of currentPins as any[]) {
			if (!savedInputsMap.has(pin.id)) {
				added.push({
					id: pin.id,
					name: pin.name,
					friendly_name: pin.friendly_name,
				});
			}
		}

		// Find removed pins (in saved but not in current)
		for (const input of savedInputs) {
			if (!currentPinsMap.has(input.id)) {
				removed.push(input);
			}
		}

		// Find changed pins
		for (const input of savedInputs) {
			const pin = currentPinsMap.get(input.id) as any;
			if (!pin) continue;

			if (pin.name !== input.name) {
				changed.push({
					id: input.id,
					name: input.name,
					field: "name",
					oldValue: input.name,
					newValue: pin.name,
				});
			}
			if (pin.friendly_name !== input.friendly_name) {
				changed.push({
					id: input.id,
					name: input.friendly_name,
					field: "friendly_name",
					oldValue: input.friendly_name,
					newValue: pin.friendly_name,
				});
			}
			const pinDataType = String(pin.data_type);
			if (pinDataType !== input.data_type) {
				changed.push({
					id: input.id,
					name: input.name,
					field: "data_type",
					oldValue: input.data_type,
					newValue: pinDataType,
				});
			}
		}

		const hasDrift =
			added.length > 0 || removed.length > 0 || changed.length > 0;
		const isEmpty = savedInputs.length === 0;

		return {
			hasDrift,
			isEmpty,
			added,
			removed,
			changed,
			savedInputs,
			currentPins: currentPins.length,
		};
	}, [board.data, event.node_id, event.inputs, event.default_page_id]);

	const isDirty = useMemo(() => {
		// `config` is a byte blob whose encoding depends on key insertion order, so
		// comparing the raw arrays reports edits that never happened. Compare the
		// decoded config with sorted keys, and the rest of the event separately.
		// `updated_at` is stamped by the backend on every upsert and is never a
		// user edit, so comparing it would leave the editor permanently dirty.
		const {
			config: draftConfig,
			updated_at: _draftStamp,
			...draftRest
		} = formData;
		const {
			config: savedConfig,
			updated_at: _savedStamp,
			...savedRest
		} = event;
		if (stableStringify(draftRest) !== stableStringify(savedRest)) return true;
		if (
			stableStringify(parseUint8ArrayToJson(draftConfig ?? []) ?? {}) !==
			stableStringify(parseUint8ArrayToJson(savedConfig ?? []) ?? {})
		) {
			return true;
		}
		if (routeForEvent && routePathDraft !== routeForEvent.path) return true;
		return false;
	}, [formData, event, routePathDraft, routeForEvent]);

	// The section rail is generated from the event type, so a mailbox shows a
	// different shape from an MCP server without the layout knowing either exists.
	const sections = useMemo(() => getEventSections(formData), [formData]);
	const [activeSection, setActiveSection] = useState<EventSectionId>(
		() => sections[0]?.id ?? "flow",
	);
	// The rail is type-derived, so changing type (or opening a page event, which
	// has no trigger section at all) can strand the selection on a section that
	// no longer exists.
	useEffect(() => {
		if (!sections.some((section) => section.id === activeSection)) {
			setActiveSection(sections[0]?.id ?? "flow");
		}
	}, [sections, activeSection]);
	const activeSectionDef =
		sections.find((section) => section.id === activeSection) ?? sections[0];

	const parsedConfig = useMemo(
		() => parseUint8ArrayToJson(formData.config ?? []) ?? {},
		[formData.config],
	);

	const overridableVariables = useMemo(
		() =>
			Object.entries(board.data?.variables ?? {}).filter(([_, variable]) =>
				isEventOverridable(variable),
			),
		[board.data?.variables],
	);

	/**
	 * Runtime-configured variables the flow needs but this event does not supply.
	 * They normally come from the user's device, which a headless trigger has no
	 * access to — so left unset they read null and the flow misbehaves silently.
	 * Interactive events still prompt, so there is nothing to warn about there.
	 */
	const unsetRuntimeVariables = useMemo(
		() =>
			isHeadlessEventType(formData.event_type)
				? overridableVariables.filter(
						([key, variable]) =>
							isRuntimeConfigured(variable) && !formData.variables[key],
					)
				: [],
		[overridableVariables, formData.variables, formData.event_type],
	);

	const issues = useEventIssues({
		event: formData,
		config: parsedConfig,
		drift: inputsDrift,
		requiresSink: eventRequiresSink(
			eventMapping,
			formData,
			board.data?.nodes?.[formData.node_id]?.name,
		),
		routeError: routePathError,
		boardVariables: board.data?.variables,
	});

	const enterEdit = useCallback(() => {
		setIsEditing(true);
	}, []);

	const showSaveBar = isDirty || isEditing;
	const saveBar = (
		<EventSaveBar
			placement={isMobile ? "top" : "bottom"}
			isDirty={isDirty}
			isSaving={isSaving}
			error={routePathError}
			onSave={() => handleSave()}
			onDiscard={handleCancel}
		/>
	);

	return (
		// The desktop shell drops all padding below md, so the editor supplies its
		// own gutter instead of running its cards into the screen edges.
		<div className="container mx-auto flex min-h-0 flex-col px-3 md:px-0">
			{/* Breadcrumbs */}
			<div className="flex min-w-0 items-center gap-2 py-3 text-sm text-muted-foreground sm:py-4">
				<Button
					variant="ghost"
					size="sm"
					onClick={onDone}
					className="h-auto shrink-0 p-0 font-normal hover:text-foreground"
				>
					{t("events", "Events")}
				</Button>
				<span className="shrink-0">/</span>
				<span className="truncate font-medium text-foreground">
					{event.name}
				</span>
			</div>

			{isMobile && showSaveBar && saveBar}

			{/* Content */}
			<div className="space-y-6 pb-6">
				{/* Status */}
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2.5 rounded-lg border bg-card/80 px-3 py-3 sm:px-4">
					<div className="flex shrink-0 items-center gap-2.5">
						<div
							className={`w-2.5 h-2.5 rounded-full ${formData.active ? "bg-green-500" : "bg-orange-500"}`}
						/>
						<span className="text-sm font-medium">
							{formData.active ? "Active" : "Inactive"}
						</span>
					</div>
					{(() => {
						const boardMode = board.data?.execution_mode;
						const locked = boardMode === "Local" || boardMode === "Remote";
						const currentMode =
							formData.execution_mode ?? IEventExecutionMode.Local;
						// Only gate Local on platform capability — Remote is always a
						// valid choice (the backend rejects configurations the hub
						// can't host at save time).
						const localDisabled =
							!canExecuteLocally &&
							currentMode !== IEventExecutionMode.Local &&
							!locked;
						return (
							<div className="flex shrink-0 items-center gap-2">
								<Label className="text-xs text-muted-foreground">
									{t("execution", "Execution")}
								</Label>
								<Select
									value={currentMode}
									onValueChange={(value) => {
										if (!isEditing) enterEdit();
										handleInputChange(
											"execution_mode",
											value as IEventExecutionMode,
										);
									}}
									disabled={!isEditing || locked}
								>
									<SelectTrigger size="sm" className="w-32 text-xs">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem
											value={IEventExecutionMode.Local}
											disabled={localDisabled}
										>
											<span className="inline-flex items-center gap-1.5">
												<Monitor className="h-3 w-3" /> {t("local", "Local")}
											</span>
										</SelectItem>
										<SelectItem value={IEventExecutionMode.Remote}>
											<span className="inline-flex items-center gap-1.5">
												<Cloud className="h-3 w-3" /> {t("remote", "Remote")}
											</span>
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
						);
					})()}
					{(formData.event_type === "rest" || formData.event_type === "mcp") &&
						(() => {
							const currentExposure =
								formData.exposure ?? IEventExposure.Public;
							return (
								<div className="flex shrink-0 items-center gap-2">
									<Label className="text-xs text-muted-foreground">
										{t("exposure", "Exposure")}
									</Label>
									<Select
										value={currentExposure}
										onValueChange={(value) => {
											if (!isEditing) enterEdit();
											handleInputChange("exposure", value as IEventExposure);
										}}
										disabled={!isEditing}
									>
										<SelectTrigger
											size="sm"
											className="w-32 text-xs"
											title={
												currentExposure === IEventExposure.Internal
													? t(
															"onlyCallableByConnectedAppsNoPublicEndpoint",
															"Only callable by connected apps — no public endpoint.",
														)
													: t(
															"reachableOnItsPublicEndpointWithTheConfiguredAuth",
															"Reachable on its public endpoint with the configured auth.",
														)
											}
										>
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value={IEventExposure.Public}>
												<span className="inline-flex items-center gap-1.5">
													<Globe className="h-3 w-3" /> {t("public", "Public")}
												</span>
											</SelectItem>
											<SelectItem value={IEventExposure.Internal}>
												<span className="inline-flex items-center gap-1.5">
													<Lock className="h-3 w-3" />{" "}
													{t("internal", "Internal")}
												</span>
											</SelectItem>
										</SelectContent>
									</Select>
								</div>
							);
						})()}
					<div className="flex w-full shrink-0 items-center justify-end gap-2 sm:ml-auto sm:w-auto">
						{board.data?.nodes?.[formData.node_id] && formData.node_id && (
							<EventTypeConfiguration
								eventConfig={eventMapping}
								disabled={!isEditing}
								node={board.data?.nodes?.[formData.node_id]}
								event={formData}
								onUpdate={(type) => {
									if (!isEditing) enterEdit();
									handleInputChange("event_type", type);
								}}
								hub={hub}
								canExecuteLocally={canExecuteLocally}
								eventExecutionMode={
									formData.execution_mode ?? IEventExecutionMode.Local
								}
								compact
							/>
						)}
						<Button
							variant="outline"
							size="sm"
							onClick={() => {
								if (!isEditing) enterEdit();
								handleInputChange("active", !formData.active);
							}}
							className="shrink-0 gap-2"
						>
							{formData.active ? (
								<>
									<Pause className="h-4 w-4" /> {t("deactivate", "Deactivate")}
								</>
							) : (
								<>
									<Play className="h-4 w-4" /> {t("activate", "Activate")}
								</>
							)}
						</Button>
					</div>
					{(formData.event_type === "rest" ||
						formData.event_type === "mcp") && (
						<p className="basis-full text-[0.7rem] leading-tight text-muted-foreground">
							{(formData.exposure ?? IEventExposure.Public) ===
							IEventExposure.Internal
								? t(
										"internalOnlyCallableByConnectedAppsThroughTheAppconnectionProxyNoPublicEndpoint",
										"Internal — only callable by connected apps through the app-connection proxy; no public endpoint.",
									)
								: t(
										"publicReachableOnItsPublicEndpointWithTheConfiguredAuth",
										"Public — reachable on its public endpoint with the configured auth.",
									)}
						</p>
					)}
				</div>

				{/* Workspace: section rail, one section at a time, guidance */}
				<div className="grid grid-cols-1 gap-6 lg:grid-cols-[188px_minmax(0,1fr)] xl:grid-cols-[188px_minmax(0,1fr)_296px]">
					<EventSectionRail
						sections={sections}
						active={activeSection}
						onSelect={setActiveSection}
						issues={issues}
					/>

					<div className="min-w-0 space-y-6">
						<div>
							<h2 className="text-lg font-semibold tracking-tight">
								{activeSectionDef.label}
							</h2>
							<p className="mt-1 max-w-[74ch] text-sm text-muted-foreground">
								{activeSectionDef.blurb}
							</p>
						</div>

						<EventAttentionStrip
							issues={issues}
							sections={sections}
							onNavigate={setActiveSection}
						/>

						<SectionGuidance event={formData} section={activeSection} />

						{activeSection === "identity" && (
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<FileTextIcon className="h-5 w-5" />
										{t("basicInformation", "Basic Information")}
									</CardTitle>
								</CardHeader>
								<CardContent className="space-y-4">
									<div>
										<Label>{t("eventName", "Event Name")}</Label>
										{isEditing ? (
											<Input
												type="text"
												value={formData.name}
												onChange={(e) =>
													handleInputChange("name", e.target.value)
												}
											/>
										) : (
											<button
												type="button"
												className="mt-1 text-sm text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
												onClick={enterEdit}
											>
												{event.name}
											</button>
										)}
									</div>
									<div>
										<Label>{t("description", "Description")}</Label>
										{isEditing ? (
											<Textarea
												value={formData.description}
												onChange={(e) =>
													handleInputChange("description", e.target.value)
												}
												rows={3}
											/>
										) : (
											<button
												type="button"
												className="mt-1 text-sm text-muted-foreground text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
												onClick={enterEdit}
											>
												{event.description ||
													t(
														"clickToAddADescription",
														"Click to add a description",
													)}
											</button>
										)}
									</div>
									{uiEventTypeSet.has(formData.event_type) ||
									isPageTargetEvent ? (
										<div>
											<Label>{t("routePath", "Route Path")}</Label>
											{isEditing ? (
												<div className="space-y-1">
													<Input
														value={routePathDraft}
														onChange={(e) => setRoutePathDraft(e.target.value)}
														placeholder="/"
													/>
													{routePathError && (
														<p className="text-xs text-destructive">
															{routePathError}
														</p>
													)}
													<p className="text-xs text-muted-foreground">
														{t(
															"usedForPathbasedNavigationMustBeUnique",
															"Used for path-based navigation. Must be unique.",
														)}
													</p>
												</div>
											) : (
												<button
													type="button"
													className="mt-1 text-sm text-muted-foreground font-mono text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
													onClick={enterEdit}
												>
													{routeForEvent?.path ??
														t("noRouteConfigured", "No route configured")}
												</button>
											)}
										</div>
									) : null}
									<div>
										<Label>{t("eventId", "Event ID")}</Label>
										<p className="mt-1 text-sm text-muted-foreground font-mono">
											{event.id}
										</p>
									</div>
									<div>
										<Label>{t("caseKeys", "Case Keys")}</Label>
										<p className="mt-0.5 text-xs text-muted-foreground">
											{`Tie every run to a business object for process mining: each key is read from the payload at the given path (e.g.`}{" "}
											<Trans i18nKey="spanClassnamefontmonoorderidspanAndGroupsRunsIntoCasesAcrossApps">
												<span className="font-mono">order.id</span>) and groups
												runs into cases across apps.
											</Trans>
										</p>
										{isEditing ? (
											<div className="mt-2 space-y-2">
												{caseKeyRows.map((row, index) => (
													<div
														key={`case-key-${String(index)}`}
														className="flex items-center gap-2"
													>
														<Input
															value={row.key}
															placeholder="order_id"
															className="h-8 w-36 font-mono text-xs"
															onChange={(e) => {
																const rows = [...caseKeyRows];
																rows[index] = { ...row, key: e.target.value };
																commitCaseKeyRows(rows);
															}}
														/>
														<span className="text-xs text-muted-foreground">
															←
														</span>
														<Input
															value={row.path}
															placeholder="order.id"
															className="h-8 flex-1 font-mono text-xs"
															onChange={(e) => {
																const rows = [...caseKeyRows];
																rows[index] = { ...row, path: e.target.value };
																commitCaseKeyRows(rows);
															}}
														/>
														<Button
															variant="ghost"
															size="icon"
															className="h-8 w-8 shrink-0 text-destructive hover:text-destructive"
															aria-label={t("removeCaseKey", "Remove case key")}
															onClick={() =>
																commitCaseKeyRows(
																	caseKeyRows.filter(
																		(_, rowIndex) => rowIndex !== index,
																	),
																)
															}
														>
															<Trash2 className="h-3.5 w-3.5" />
														</Button>
													</div>
												))}
												<Button
													variant="outline"
													size="sm"
													disabled={caseKeyRows.length >= 8}
													onClick={() =>
														commitCaseKeyRows([
															...caseKeyRows,
															{ key: "", path: "" },
														])
													}
												>
													<Plus className="mr-1.5 h-3.5 w-3.5" />
													{t("addCaseKey", "Add case key")}
												</Button>
											</div>
										) : (
											<button
												type="button"
												className="mt-1 w-full rounded px-2 py-1 -mx-2 text-left transition-colors hover:bg-muted/60"
												onClick={enterEdit}
											>
												{Object.keys(event.correlation_mappings ?? {}).length >
												0 ? (
													<span className="flex flex-wrap gap-1">
														{Object.entries(
															event.correlation_mappings ?? {},
														).map(([key, path]) => (
															<Badge
																key={key}
																variant="secondary"
																className="gap-1 font-mono text-[10px] font-normal"
															>
																{key}
																<span className="text-muted-foreground">{`← ${path}`}</span>
															</Badge>
														))}
													</span>
												) : (
													<span className="text-sm text-muted-foreground">
														{t(
															"noCaseKeysClickToConfigureProcessMining",
															"No case keys — click to configure process mining",
														)}
													</span>
												)}
											</button>
										)}
									</div>
								</CardContent>
							</Card>
						)}

						{activeSection === "flow" && (
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<LayersIcon className="h-5 w-5" />
										{event.default_page_id
											? "Page Configuration"
											: "Flow Configuration"}
									</CardTitle>
								</CardHeader>
								{!isEditing && event.default_page_id && (
									<CardContent className="space-y-4">
										<div>
											<Label className="group flex items-center hover:underline">
												<Link
													title={t("openPageEditor", "Open Page Editor")}
													className="flex flex-row items-center"
													href={`/library/config/page-editor?id=${appId}&pageId=${event.default_page_id}`}
												>
													{t("page", "Page")}
													<Button
														size={"icon"}
														variant={"ghost"}
														className="p-0! w-4 h-4 ml-1 mb-[0.1rem]"
													>
														<ExternalLinkIcon className="w-4 h-4 group-hover:text-primary" />
													</Button>
												</Link>
											</Label>
											<p className="mt-1 text-sm text-muted-foreground font-mono">
												{event.default_page_id}
											</p>
										</div>
										<div>
											<Label>{t("flowVersion", "Flow Version")}</Label>
											<button
												type="button"
												className="mt-1 block w-full rounded px-2 py-1 -mx-2 text-left text-sm text-muted-foreground hover:bg-muted/60 transition-colors"
												onClick={enterEdit}
											>
												{event.board_version
													? `v${event.board_version.join(".")}`
													: "Latest"}
											</button>
										</div>
									</CardContent>
								)}
								{!isEditing && !event.default_page_id && (
									<CardContent className="space-y-4">
										<div>
											<Label>{t("flow", "Flow")}</Label>
											<button
												type="button"
												className="mt-1 text-sm text-muted-foreground font-mono text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors block"
												onClick={enterEdit}
											>
												{board.data?.name ??
													t("boardNotFound", "BOARD NOT FOUND!")}
											</button>
										</div>
										<div>
											<Label>{t("flowVersion", "Flow Version")}</Label>
											<button
												type="button"
												className="mt-1 text-sm text-muted-foreground text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors block"
												onClick={enterEdit}
											>
												{event.board_version
													? event.board_version.join(".")
													: "Latest"}
											</button>
										</div>
										<div>
											<Label className="group flex items-center hover:underline">
												{onNavigateToFlow ? (
													<button
														type="button"
														title={t("openFlowAndNode", "Open Flow and Node")}
														className="flex flex-row items-center"
														onClick={() =>
															onNavigateToFlow({
																boardId: event.board_id,
																appId,
																nodeId: event.node_id,
																version: event.board_version as
																	| [number, number, number]
																	| undefined,
															})
														}
													>
														{t("nodeId", "Node ID")}
														<span className="p-0! w-4 h-4 ml-1 mb-[0.1rem] inline-flex">
															<ExternalLinkIcon className="w-4 h-4 group-hover:text-primary" />
														</span>
													</button>
												) : (
													<Link
														title={t("openFlowAndNode", "Open Flow and Node")}
														className="flex flex-row items-center"
														href={`/flow?id=${event.board_id}&app=${appId}&node=${event.node_id}${event.board_version ? `&version=${event.board_version.join("_")}` : ""}`}
													>
														{t("nodeId", "Node ID")}
														<Button
															size={"icon"}
															variant={"ghost"}
															className="p-0! w-4 h-4 ml-1 mb-[0.1rem]"
														>
															<ExternalLinkIcon className="w-4 h-4 group-hover:text-primary" />
														</Button>
													</Link>
												)}
											</Label>
											<p className="mt-1 text-sm text-muted-foreground font-mono">
												{board.data?.nodes?.[event.node_id]?.friendly_name ??
													t("nodeNotFound", "Node not found")}{" "}
												{`(${event.node_id})`}
											</p>
										</div>
									</CardContent>
								)}
								{isEditing && isPageTargetEvent && (
									<CardContent className="space-y-4">
										{/* Page Selection */}
										<div className="space-y-2">
											<Label htmlFor="page">{t("page", "Page")}</Label>
											<Select
												value={formData.default_page_id ?? ""}
												onValueChange={(value) => {
													handleInputChange("default_page_id", value);
													const page = (pages.data ?? []).find(
														(p: PageListItem) => p.pageId === value,
													);
													if (page?.boardId) {
														handleInputChange("board_id", page.boardId);
													}
													handleInputChange("board_version", undefined);
												}}
											>
												<SelectTrigger>
													<SelectValue
														placeholder={t("selectAPage", "Select a page")}
													/>
												</SelectTrigger>
												<SelectContent>
													{(pages.data ?? []).map((p: PageListItem) => (
														<SelectItem key={p.pageId} value={p.pageId}>
															{p.name}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
										<div className="space-y-2">
											<Label>{t("flowVersion", "Flow Version")}</Label>
											<Select
												value={formData.board_version?.join(".") ?? "latest"}
												onValueChange={(value) =>
													handleInputChange(
														"board_version",
														value === "latest"
															? undefined
															: normalizeBoardVersion(
																	value.split(".").map(Number),
																),
													)
												}
											>
												<SelectTrigger>
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													<SelectItem value="latest">
														{t("latest", "Latest")}
													</SelectItem>
													{versions.data?.map((version) => (
														<SelectItem
															key={version.join(".")}
															value={version.join(".")}
														>
															v{version.join(".")}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										</div>
									</CardContent>
								)}
								{isEditing && !isPageTargetEvent && (
									<CardContent className="space-y-4">
										{/* Board Selection */}
										<div className="space-y-4">
											<div className="space-y-2">
												<Label htmlFor="board">{t("flow", "Flow")}</Label>
												<Select
													value={formData.board_id}
													onValueChange={(value) => {
														handleInputChange("board_id", value);
														handleInputChange("board_version", undefined);
														handleInputChange("node_id", undefined);
													}}
												>
													<SelectTrigger>
														<SelectValue
															placeholder={t("selectABoard", "Select a board")}
														/>
													</SelectTrigger>
													<SelectContent>
														{boards.data?.map((board) => (
															<SelectItem key={board.id} value={board.id}>
																{board.name}
															</SelectItem>
														))}
													</SelectContent>
												</Select>
											</div>
										</div>
										{/* Board Version Selection */}
										<div className="space-y-4">
											<div className="space-y-2">
												<Label htmlFor="board">
													{t("flowVersion", "Flow Version")}
												</Label>
												<Select
													value={formData.board_version?.join(".") ?? "latest"}
													onValueChange={(value) => {
														handleInputChange(
															"board_version",
															value === "latest"
																? undefined
																: normalizeBoardVersion(
																		value.split(".").map(Number),
																	),
														);
														handleInputChange("node_id", undefined);
													}}
												>
													<SelectTrigger>
														<SelectValue />
													</SelectTrigger>
													<SelectContent>
														<SelectItem value="latest">
															{t("latest", "Latest")}
														</SelectItem>
														{versions.data?.map((board) => (
															<SelectItem
																key={board.join(".")}
																value={board.join(".")}
															>
																v{board.join(".")}
															</SelectItem>
														))}
													</SelectContent>
												</Select>
											</div>
										</div>

										{/* Node and Board Selection */}
										{board.data && (
											<div className="space-y-4">
												<div className="space-y-2">
													<Label htmlFor="node">{t("node", "Node")}</Label>
													<Select
														value={formData.node_id}
														onValueChange={(value) =>
															handleInputChange("node_id", value)
														}
													>
														<SelectTrigger>
															<SelectValue
																placeholder={t("selectANode", "Select a node")}
															/>
														</SelectTrigger>
														<SelectContent>
															{Object.values(board.data.nodes)
																.filter((node) => node.start)
																.map((node) => (
																	<SelectItem key={node.id} value={node.id}>
																		{node?.friendly_name || node?.name}
																	</SelectItem>
																))}
														</SelectContent>
													</Select>
												</div>
											</div>
										)}
									</CardContent>
								)}
							</Card>
						)}

						{activeSection === "release" && (
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<GitBranchIcon className="h-5 w-5" />
										{t("versionInformation", "Version Information")}
									</CardTitle>
								</CardHeader>
								<CardContent>
									<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
										<div>
											<Label>{t("eventVersion", "Event Version")}</Label>
											<p className="mt-1 text-sm text-muted-foreground">
												{event.event_version.join(".")}
											</p>
										</div>
										<div>
											<Label>{t("created", "Created")}</Label>
											<p className="mt-1 text-sm text-muted-foreground">
												{new Date(
													event.created_at.secs_since_epoch * 1000,
												).toLocaleString()}
											</p>
										</div>
										<div>
											<Label>{t("lastUpdated", "Last Updated")}</Label>
											<p className="mt-1 text-sm text-muted-foreground">
												{new Date(
													event.updated_at.secs_since_epoch * 1000,
												).toLocaleString()}
											</p>
										</div>
									</div>
								</CardContent>
							</Card>
						)}

						{/* Inputs - Show saved inputs and drift detection */}
						{activeSection === "inputs" && event.node_id && (
							<Card>
								<CardHeader>
									<div className="flex items-center justify-between">
										<div className="flex items-center gap-2">
											<FormInputIcon className="h-5 w-5" />
											<CardTitle>{t("inputs", "Inputs")}</CardTitle>
											{inputsDrift?.hasDrift && (
												<Badge variant="destructive" className="ml-2">
													<AlertTriangle className="h-3 w-3 mr-1" />
													{t("driftDetected", "Drift Detected")}
												</Badge>
											)}
										</div>
										<Button
											variant="outline"
											size="sm"
											onClick={handleRefreshInputs}
											disabled={isRefreshingInputs}
											className="gap-2"
										>
											{isRefreshingInputs ? (
												<Loader2 className="h-4 w-4 animate-spin" />
											) : (
												<RefreshCw className="h-4 w-4" />
											)}
											{`Refresh from Node`}
										</Button>
									</div>
									<CardDescription>
										{t(
											"inputPinsCapturedAtPublishTimeChangesToTheNodeSinceThenAreShownBelow",
											"Input pins captured at publish time. Changes to the node since then are shown below.",
										)}
									</CardDescription>
								</CardHeader>
								<CardContent className="space-y-4">
									{inputsDrift?.isEmpty && !inputsDrift?.hasDrift && (
										<p className="text-sm text-muted-foreground">
											{`No input pins were captured for this event. Click "Refresh from Node" to sync.`}
										</p>
									)}

									{inputsDrift?.hasDrift && (
										<div className="space-y-3 p-3 bg-destructive/10 rounded-md border border-destructive/20">
											<p className="text-sm font-medium text-destructive">
												{t(
													"theNodesInputsHaveChangedSinceThisEventWasPublished",
													"The node's inputs have changed since this event was published:",
												)}
											</p>
											{inputsDrift.added.length > 0 && (
												<div className="text-sm">
													<span className="font-medium text-green-600">
														{t("added", "Added:")}{" "}
													</span>
													{inputsDrift.added
														.map((p) => p.friendly_name || p.name)
														.join(", ")}
												</div>
											)}
											{inputsDrift.removed.length > 0 && (
												<div className="text-sm">
													<span className="font-medium text-red-600">
														{t("removed", "Removed:")}{" "}
													</span>
													{inputsDrift.removed
														.map((i) => i.friendly_name || i.name)
														.join(", ")}
												</div>
											)}
											{inputsDrift.changed.length > 0 && (
												<div className="text-sm">
													<span className="font-medium text-yellow-600">
														{t("changed", "Changed:")}{" "}
													</span>
													{inputsDrift.changed
														.map((c) => `${c.name} (${c.field})`)
														.join(", ")}
												</div>
											)}
										</div>
									)}

									{(event.inputs ?? []).length > 0 && (
										<div className="space-y-2">
											<Label className="text-sm font-medium">
												{t(
													"capturedInputsLength",
													"Captured Inputs ({{length}})",
													{
														length: event.inputs?.length ?? 0,
													},
												)}
											</Label>
											<div className="grid gap-2">
												{(event.inputs ?? []).map((input) => {
													// Don't show description if it looks like an ID (all digits) or is too long
													const showDescription =
														input.description &&
														!/^\d+$/.test(input.description) &&
														input.description.length < 100;
													return (
														<div
															key={input.id}
															className="flex items-start gap-3 p-3 bg-muted/50 rounded-md text-sm"
														>
															<div className="flex items-center gap-2 shrink-0">
																<span className="font-medium">
																	{input.friendly_name || input.name}
																</span>
																<Badge variant="secondary" className="text-xs">
																	{input.data_type}
																</Badge>
																{input.value_type !== "Normal" && (
																	<Badge variant="outline" className="text-xs">
																		{input.value_type}
																	</Badge>
																)}
															</div>
															{showDescription && (
																<span className="text-muted-foreground text-xs">
																	{input.description}
																</span>
															)}
														</div>
													);
												})}
											</div>
										</div>
									)}
								</CardContent>
							</Card>
						)}

						{/* Variables - Full width due to potential size */}
						{activeSection === "variables" && (
							<Card>
								<CardHeader>
									<div className="flex items-center justify-between">
										<CardTitle className="flex flex-row items-center gap-2">
											<CodeIcon className="h-5 w-5" />
											<p>{t("variables", "Variables")}</p>
										</CardTitle>
										{isEditing && (
											<Dialog>
												<DialogTrigger asChild>
													<Button variant="outline" className="gap-2 ml-2">
														<Plus className="h-4 w-4" />
														{t("addFlowVariables", "Add Flow Variables")}
													</Button>
												</DialogTrigger>
												<DialogContent className="max-w-lg">
													<DialogHeader>
														<DialogTitle>
															{t("addFlowVariables", "Add Flow Variables")}
														</DialogTitle>
														<DialogDescription>
															{t(
																"selectFlowVariablesToOverrideInThisEventConfiguration",
																"Select flow variables to override in this event configuration",
															)}
														</DialogDescription>
													</DialogHeader>
													<div className="space-y-2 max-h-80 overflow-y-auto">
														{overridableVariables.map(([key, variable]) => {
															const isAlreadyAdded =
																formData.variables.hasOwnProperty(key);
															return (
																<div
																	key={key}
																	className="flex items-center justify-between p-3 border rounded"
																>
																	<div className="flex-1">
																		<div className="flex flex-row items-center gap-2">
																			<VariableTypeIndicator
																				valueType={variable.data_type}
																				type={variable.value_type}
																			/>
																			<div className="font-medium text-sm">
																				{variable.name}
																			</div>
																			{isRuntimeConfigured(variable) && (
																				<Badge
																					variant="outline"
																					className="gap-1 text-[10px]"
																				>
																					{variable.secret && (
																						<Lock className="h-2.5 w-2.5" />
																					)}
																					{t(
																						"runtimeConfigured",
																						"Runtime configured",
																					)}
																				</Badge>
																			)}
																		</div>
																		{/* Secrets reach the browser blank from the API,
																		    but desktop reads boards straight off disk —
																		    don't print the value there either. */}
																		{variable.default_value &&
																			!variable.secret && (
																				<div className="text-xs text-muted-foreground mt-1">
																					{t("default2", "Default:")}{" "}
																					<span>
																						{String(
																							parseUint8ArrayToJson(
																								variable.default_value,
																							),
																						)}
																					</span>
																				</div>
																			)}
																	</div>
																	<Button
																		variant={
																			isAlreadyAdded ? "outline" : "default"
																		}
																		size="sm"
																		onClick={() => {
																			if (isAlreadyAdded) {
																				const newVars = {
																					...formData.variables,
																				};
																				delete newVars[key];
																				handleInputChange("variables", newVars);
																			} else {
																				handleInputChange("variables", {
																					...formData.variables,
																					[key]: variable,
																				});
																			}
																		}}
																	>
																		{isAlreadyAdded
																			? "Remove"
																			: t("add", "Add")}
																	</Button>
																</div>
															);
														})}
														{overridableVariables.length === 0 && (
															<div className="text-center py-8 text-muted-foreground">
																{t(
																	"noBoardVariablesAvailable",
																	"No board variables available",
																)}
															</div>
														)}
													</div>
												</DialogContent>
											</Dialog>
										)}
									</div>
								</CardHeader>
								<CardContent className="space-y-4">
									{unsetRuntimeVariables.length > 0 && (
										<div className="flex gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3">
											<AlertTriangle className="h-4 w-4 shrink-0 mt-0.5 text-amber-600 dark:text-amber-500" />
											<div className="space-y-1 text-sm">
												<p className="font-medium">
													{t(
														"runtimeVariablesUnset",
														"Runtime variables have no value here",
													)}
												</p>
												<p className="text-muted-foreground">
													{t(
														"runtimeVariablesUnsetDetail",
														"These are normally filled in from your device when you run the flow yourself. A trigger runs without you, so unless you set them here they read as empty:",
													)}{" "}
													<span className="font-medium text-foreground">
														{unsetRuntimeVariables
															.map(([, variable]) => variable.name)
															.join(", ")}
													</span>
												</p>
											</div>
										</div>
									)}
									{Object.values(formData.variables).some(
										(variable) => variable.secret,
									) && (
										<div className="flex gap-3 rounded-lg border bg-muted/40 p-3">
											<Lock className="h-4 w-4 shrink-0 mt-0.5 text-muted-foreground" />
											<p className="text-sm text-muted-foreground">
												{t(
													"eventSecretStorageNotice",
													"Secret values set here are saved with the event so this trigger can run without you — unlike runtime variables, which stay on your device. They are never shown back to you, and leaving a secret field blank keeps the stored value.",
												)}
											</p>
										</div>
									)}
									{Object.keys(formData.variables).length > 0 ? (
										<div className="space-y-2">
											{Object.entries(formData.variables).map(
												([key, value]) => (
													<VariableConfigCard
														disabled={!isEditing}
														key={key}
														variable={value}
														onUpdate={async (variable) => {
															if (!isEditing) setIsEditing(true);
															const newVars = {
																...formData.variables,
																[key]: {
																	...variable,
																	default_value: variable.default_value,
																},
															};
															handleInputChange("variables", newVars);
														}}
													/>
												),
											)}
										</div>
									) : (
										<p className="text-sm text-muted-foreground">
											{isEditing
												? t(
														"noVariablesConfiguredClickAddFlowVariablesToGetStarted",
														"No variables configured. Click 'Add Flow Variables' to get started.",
													)
												: t("noVariablesConfigured", "No variables configured")}
										</p>
									)}
								</CardContent>
							</Card>
						)}

						{/* Node Specific Configuration - Full width due to potential size */}
						{isTriggerSection(activeSection) && board.data && (
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<CogIcon className="h-5 w-5" />
										{t("nodeConfiguration", "Node Configuration")}
									</CardTitle>
								</CardHeader>
								{/* Always interactive. Gating these behind edit mode made every
								    control `disabled`, and disabled controls emit no pointer
								    events — so clicking one to "start editing" did nothing at
								    all. Edit mode now begins at the first real change. */}
								<CardContent className="space-y-4 flex flex-col items-start">
									<EventTranslation
										section={activeSection}
										appId={appId}
										eventType={formData.event_type}
										eventConfig={eventMapping}
										editing
										// The working copy, not the saved event — otherwise the
										// fields render the last-saved values and Discard has
										// nothing to reset them to.
										config={parsedConfig}
										board={board.data}
										nodeId={formData.node_id}
										hub={hub}
										eventId={event.id}
										canExecuteLocally={canExecuteLocally}
										eventExecutionMode={
											formData.execution_mode ?? IEventExecutionMode.Local
										}
										onUpdate={(config) => {
											if (!isEditing) setIsEditing(true);
											handleInputChange(
												"config",
												convertJsonToUint8Array(config),
											);
										}}
									/>
								</CardContent>
							</Card>
						)}

						{/* Trigger is the landing section, so it must never be blank. */}
						{isTriggerSection(activeSection) && !board.data && (
							<Card>
								<CardContent className="py-10 text-center text-sm text-muted-foreground">
									{formData.board_id
										? t(
												"loadingTheFlowThisEventIsBoundTo",
												"Loading the flow this event is bound to…",
											)
										: t(
												"thisEventIsntBoundToAFlowYetSoThereIsNothingTypespecificToConfigurePickOneUnderFlowTarget",
												"This event isn't bound to a flow yet, so there is nothing type-specific to configure. Pick one under Flow & target.",
											)}
								</CardContent>
							</Card>
						)}

						{/* Notes — release notes live with the version they describe */}
						{activeSection === "release" && (
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<StickyNote className="h-5 w-5" />
										{t("notes", "Notes")}
									</CardTitle>
								</CardHeader>
								<CardContent>
									{isEditing ? (
										<Textarea
											value={formData.notes?.NOTES ?? ""}
											onChange={(e) =>
												handleInputChange("notes", { NOTES: e.target.value })
											}
											placeholder={t(
												"addNotesAboutThisEvent",
												"Add notes about this event...",
											)}
											rows={4}
										/>
									) : (
										<button
											type="button"
											className="text-sm text-muted-foreground whitespace-pre-wrap text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
											onClick={enterEdit}
										>
											{event.notes?.NOTES ??
												t("clickToAddNotes", "Click to add notes...")}
										</button>
									)}
								</CardContent>
							</Card>
						)}
					</div>

					<aside className="hidden min-w-0 flex-col gap-3 xl:flex">
						<SetupChecklist
							event={formData}
							config={parsedConfig}
							onNavigate={setActiveSection}
						/>
					</aside>
				</div>
			</div>

			{/* Save bar — driven by actual changes, since the config surface is
			    always interactive and "edit mode" no longer means the user has
			    changed anything. */}
			{!isMobile && showSaveBar && saveBar}

			{/* PAT Selector Dialog */}
			<PatSelectorDialog
				open={showPatDialog}
				onOpenChange={setShowPatDialog}
				onPatSelected={(token) => {
					handleSave(token);
				}}
			/>
			{/* OAuth Consent Dialog */}
			<OAuthConsentDialog
				open={showOAuthConsent}
				onOpenChange={setShowOAuthConsent}
				providers={oauthMissingProviders}
				authorizedProviders={oauthAuthorizedProviders}
				preAuthorizedProviders={oauthPreAuthorizedProviders}
				onAuthorize={handleOAuthAuthorize}
				onConfirmAll={handleOAuthConfirmAll}
				onCancel={handleOAuthCancel}
			/>
		</div>
	);
}
