"use client";

import { createId } from "@paralleldrive/cuid2";
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
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Textarea,
	VariableConfigCard,
	VariableTypeIndicator,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import type { IOAuthConsentStore } from "@flow-like/flow-like-ui/db/oauth-db";
import {
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
} from "@flow-like/flow-like-ui/lib/oauth/helpers";
import type {
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "@flow-like/flow-like-ui/lib/oauth/types";
import type { IHub } from "@flow-like/flow-like-ui/lib/schema/hub/hub";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "@flow-like/flow-like-ui/lib/uint8";
import type { PageListItem } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import {
	AlertTriangle,
	Cloud,
	CodeIcon,
	CogIcon,
	EditIcon,
	ExternalLinkIcon,
	FileTextIcon,
	FormInputIcon,
	GitBranchIcon,
	LayersIcon,
	Loader2,
	Monitor,
	Pause,
	Play,
	Plus,
	RefreshCw,
	SaveIcon,
	Settings,
	StickyNote,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
type ViewMode = "list" | "table";

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
	const [routeCleanupDone, setRouteCleanupDone] = useState(false);
	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);
	const normalizePath = useCallback((path: unknown): string => {
		const raw = String(path ?? "").trim();
		if (!raw) return "/";
		const withoutQuery = raw.split("?")[0] ?? raw;
		if (!withoutQuery || withoutQuery === "/") return "/";
		return withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
	}, []);

	const router = useRouter();
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id ?? ""],
		(id ?? "") !== "",
	);

	const boards = useInvoke(
		backend.boardState.getBoards,
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

	// Clean up orphaned routes (routes pointing to deleted events)
	useEffect(() => {
		if (!id || !events.data) return;

		const cleanupOrphanedRoutes = async () => {
			try {
				const routes = await backend.routeState.getRoutes(id);
				const eventIds = new Set(events.data?.map((e) => e.id) ?? []);
				let deletedAny = false;

				console.log(
					`[Route Cleanup] Found ${routes.length} routes, checking against ${eventIds.size} events`,
				);

				for (const route of routes) {
					// Delete routes that point to non-existent events
					if (!eventIds.has(route.eventId)) {
						console.log(
							`[Route Cleanup] Marking for deletion: route ${route.path} (eventId: ${route.eventId})`,
						);
						await backend.routeState.deleteRouteByPath(id, route.path);
						deletedAny = true;
					}
				}

				if (deletedAny) {
					console.log(
						"[Route Cleanup] Completed - deleted orphaned routes, invalidating cache",
					);
					await invalidate(backend.routeState.getRoutes, [id]);
				} else {
					console.log("[Route Cleanup] No orphaned routes found");
				}
			} catch (e) {
				console.error("Failed to clean up orphaned routes:", e);
			} finally {
				setRouteCleanupDone(true);
			}
		};

		cleanupOrphanedRoutes();
	}, [id, events.data, backend.routeState, invalidate]);

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
						const path = normalizePath((newEvent as any)?.path);
						await backend.routeState.setRoute(id, path, savedEvent.id);
						await invalidate(backend.routeState.getRoutes, [id]);
					} catch (error) {
						console.error("Failed to create route for UI event:", error);
					}
				}

				await invalidate(backend.eventState.getEvents, [id]);
				await events.refetch();
			} catch (error) {
				console.error("Failed to create event:", error);
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
			normalizePath,
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
				console.log(`Deleted event with ID: ${eventId}`);
			} catch (e) {
				console.error("Failed to delete event:", e);
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
							const path = normalizePath(pendingRoutePath);
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
			normalizePath,
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
		<div className="container mx-auto flex flex-col grow max-h-full">
			<div className="flex flex-col grow overflow-hidden max-h-full">
				<div className="flex flex-col overflow-auto overflow-x-visible grow h-full max-h-full">
					{events.data?.length === 0 ? (
						<Card>
							<CardContent className="py-12 text-center">
								<Settings className="h-12 w-12 text-muted-foreground mx-auto mb-4" />
								<h3 className="text-lg font-semibold mb-2">
									No events configured
								</h3>
								<p className="text-muted-foreground mb-4">
									Get started by creating your first event
								</p>
								<Button
									onClick={() => setIsCreateDialogOpen(true)}
									className="gap-2"
								>
									<Plus className="h-4 w-4" />
									Create Event
								</Button>
							</CardContent>
						</Card>
					) : (
						<EventsTable
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
						<DialogTitle>Create New Event</DialogTitle>
						<DialogDescription>
							Configure a new event with its properties and settings
						</DialogDescription>
					</DialogHeader>
					<DialogBody>
						{id && routeCleanupDone && (
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
				title="Create Event with Sink"
				description="This event requires a sink. Select or create a Personal Access Token to activate the event sink."
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
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [isEditing, setIsEditing] = useState(false);
	const [formData, setFormData] = useState<IEvent>(event);
	const [showPatDialog, setShowPatDialog] = useState(false);
	const [isOffline, setIsOffline] = useState<boolean | null>(null);
	const canExecuteLocally = backend.capabilities().canExecuteLocally;
	const [isRefreshingInputs, setIsRefreshingInputs] = useState(false);
	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);
	const normalizePath = useCallback((path: unknown): string => {
		const raw = String(path ?? "").trim();
		if (!raw) return "/";
		const withoutQuery = raw.split("?")[0] ?? raw;
		if (!withoutQuery || withoutQuery === "/") return "/";
		return withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
	}, []);
	const [routePathDraft, setRoutePathDraft] = useState<string>("/");
	const [routePathError, setRoutePathError] = useState<string | null>(null);

	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		(appId ?? "") !== "",
	);

	const routeForEvent = useMemo(() => {
		return routes.data?.find((r) => r.eventId === event.id) ?? null;
	}, [routes.data, event.id]);

	useEffect(() => {
		if (isEditing) return;
		setRoutePathDraft(routeForEvent?.path ?? "/");
		setRoutePathError(null);
	}, [routeForEvent?.path, isEditing]);

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
		backend.boardState.getBoards,
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
		[
			appId,
			formData.board_id,
			event.board_version as [number, number, number] | undefined,
		],
		!!event.board_id && !isPageTargetEvent,
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, formData.board_id],
		(formData.board_id ?? "") !== "" && isEditing && !isPageTargetEvent,
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
		console.dir({
			field,
			value,
		});
		setFormData((prev) => ({ ...prev, [field]: value }));
	};

	const checkRequiresSink = (): boolean => {
		const node = board.data?.nodes?.[formData.node_id];
		if (!node) return false;
		const eventTypeConfig = eventMapping[node?.name];
		return eventTypeConfig?.withSink.includes(formData.event_type);
	};

	const handleSave = async (
		selectedPat?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	) => {
		setRoutePathError(null);
		const isUiEvent = uiEventTypeSet.has(formData.event_type);
		const isPageTargetEvent = !!formData.default_page_id;
		const shouldHaveRoute = isUiEvent || isPageTargetEvent;
		const desiredRoutePath = shouldHaveRoute
			? normalizePath(routePathDraft)
			: null;

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
				const normalized = normalizePath(r.path);
				if (normalized !== desiredRoutePath) return false;
				// Allow if this event already owns this path
				return r.eventId !== event.id;
			});
			if (conflict) {
				setRoutePathError(`Route path already in use: ${desiredRoutePath}`);
				return;
			}
		}

		// Save the event with the PAT and OAuth tokens if provided
		await backend.eventState.upsertEvent(
			appId,
			formData,
			undefined,
			selectedPat,
			oauthTokens,
		);

		if (shouldHaveRoute && desiredRoutePath) {
			try {
				// If path changed, delete old route first
				if (routeForEvent && routeForEvent.path !== desiredRoutePath) {
					await backend.routeState.deleteRouteByPath(appId, routeForEvent.path);
				}
				// Set new route
				await backend.routeState.setRoute(appId, desiredRoutePath, formData.id);
				await routes.refetch();
			} catch (error) {
				console.error("Failed to upsert route for UI event:", error);
				setRoutePathError("Failed to save route path");
				return;
			}
		}
		onReload?.();
		setIsEditing(false);
		setShowPatDialog(false);
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
			await backend.eventState.upsertEvent(
				appId,
				event,
				undefined,
				undefined,
				undefined,
			);
			await invalidate(backend.eventState.getEvents, [appId]);
			onReload?.();
		} catch (error) {
			console.error("Failed to refresh inputs:", error);
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
		if (JSON.stringify(formData) !== JSON.stringify(event)) return true;
		if (routeForEvent && routePathDraft !== routeForEvent.path) return true;
		return false;
	}, [formData, event, routePathDraft, routeForEvent]);

	const enterEdit = useCallback(() => {
		setIsEditing(true);
	}, []);

	return (
		<div className="container mx-auto flex flex-col min-h-0">
			{/* Breadcrumbs */}
			<div className="flex items-center space-x-2 text-sm text-muted-foreground py-4">
				<Button
					variant="ghost"
					size="sm"
					onClick={onDone}
					className="p-0 h-auto font-normal hover:text-foreground"
				>
					Events
				</Button>
				<span>/</span>
				<span className="text-foreground font-medium">{event.name}</span>
			</div>

			{/* Content */}
			<div className="space-y-6 pb-24">
				{/* Status */}
				<div className="flex items-center gap-3 overflow-x-auto rounded-lg border bg-card/80 px-4 py-3">
					<div className="flex shrink-0 items-center gap-2.5 pr-3 border-r">
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
									Execution
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
												<Monitor className="h-3 w-3" /> Local
											</span>
										</SelectItem>
										<SelectItem value={IEventExecutionMode.Remote}>
											<span className="inline-flex items-center gap-1.5">
												<Cloud className="h-3 w-3" /> Remote
											</span>
										</SelectItem>
									</SelectContent>
								</Select>
							</div>
						);
					})()}
					<div className="ml-auto" />
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
								<Pause className="h-4 w-4" /> Deactivate
							</>
						) : (
							<>
								<Play className="h-4 w-4" /> Activate
							</>
						)}
					</Button>
				</div>

				{/* Main Configuration */}
				<div className="space-y-6">
					{/* Top Row - Essential Information */}
					<div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
						{/* Basic Information */}
						<Card>
							<CardHeader>
								<CardTitle className="flex items-center gap-2">
									<FileTextIcon className="h-5 w-5" />
									Basic Information
								</CardTitle>
							</CardHeader>
							<CardContent className="space-y-4">
								<div>
									<Label>Event Name</Label>
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
									<Label>Description</Label>
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
											{event.description || "Click to add a description"}
										</button>
									)}
								</div>
								{uiEventTypeSet.has(formData.event_type) ||
								isPageTargetEvent ? (
									<div>
										<Label>Route Path</Label>
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
													Used for path-based navigation. Must be unique.
												</p>
											</div>
										) : (
											<button
												type="button"
												className="mt-1 text-sm text-muted-foreground font-mono text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
												onClick={enterEdit}
											>
												{routeForEvent?.path ?? "No route configured"}
											</button>
										)}
									</div>
								) : null}
								<div>
									<Label>Event ID</Label>
									<p className="mt-1 text-sm text-muted-foreground font-mono">
										{event.id}
									</p>
								</div>
							</CardContent>
						</Card>

						{/* Flow Configuration */}
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
												title="Open Page Editor"
												className="flex flex-row items-center"
												href={`/library/config/page-editor?id=${appId}&pageId=${event.default_page_id}`}
											>
												Page
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
								</CardContent>
							)}
							{!isEditing && !event.default_page_id && (
								<CardContent className="space-y-4">
									<div>
										<Label>Flow</Label>
										<button
											type="button"
											className="mt-1 text-sm text-muted-foreground font-mono text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors block"
											onClick={enterEdit}
										>
											{board.data?.name ?? "BOARD NOT FOUND!"}
										</button>
									</div>
									<div>
										<Label>Flow Version</Label>
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
													title="Open Flow and Node"
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
													Node ID
													<span className="p-0! w-4 h-4 ml-1 mb-[0.1rem] inline-flex">
														<ExternalLinkIcon className="w-4 h-4 group-hover:text-primary" />
													</span>
												</button>
											) : (
												<Link
													title="Open Flow and Node"
													className="flex flex-row items-center"
													href={`/flow?id=${event.board_id}&app=${appId}&node=${event.node_id}${event.board_version ? `&version=${event.board_version.join("_")}` : ""}`}
												>
													Node ID
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
												"Node not found"}{" "}
											({event.node_id})
										</p>
									</div>
								</CardContent>
							)}
							{isEditing && isPageTargetEvent && (
								<CardContent className="space-y-4">
									{/* Page Selection */}
									<div className="space-y-2">
										<Label htmlFor="page">Page</Label>
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
											}}
										>
											<SelectTrigger>
												<SelectValue placeholder="Select a page" />
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
								</CardContent>
							)}
							{isEditing && !isPageTargetEvent && (
								<CardContent className="space-y-4">
									{/* Board Selection */}
									<div className="space-y-4">
										<div className="space-y-2">
											<Label htmlFor="board">Flow</Label>
											<Select
												value={formData.board_id}
												onValueChange={(value) => {
													handleInputChange("board_id", value);
													handleInputChange("board_version", undefined);
													handleInputChange("node_id", undefined);
												}}
											>
												<SelectTrigger>
													<SelectValue placeholder="Select a board" />
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
											<Label htmlFor="board">Flow Version</Label>
											<Select
												value={formData.board_version?.join(".") ?? ""}
												onValueChange={(value) => {
													handleInputChange(
														"board_version",
														value === "" || value === "none"
															? undefined
															: value.split(".").map(Number),
													);
													handleInputChange("node_id", undefined);
												}}
											>
												<SelectTrigger>
													<SelectValue placeholder="Latest" />
												</SelectTrigger>
												<SelectContent>
													{versions.data?.map((board) => (
														<SelectItem
															key={board.join(".")}
															value={board.join(".")}
														>
															v{board.join(".")}
														</SelectItem>
													))}
													<SelectItem key={""} value={"none"}>
														Latest
													</SelectItem>
												</SelectContent>
											</Select>
										</div>
									</div>

									{/* Node and Board Selection */}
									{board.data && (
										<div className="space-y-4">
											<div className="space-y-2">
												<Label htmlFor="node">Node</Label>
												<Select
													value={formData.node_id}
													onValueChange={(value) =>
														handleInputChange("node_id", value)
													}
												>
													<SelectTrigger>
														<SelectValue placeholder="Select a node" />
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
					</div>

					{/* Version Information - Single row for metadata */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<GitBranchIcon className="h-5 w-5" />
								Version Information
							</CardTitle>
						</CardHeader>
						<CardContent>
							<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
								<div>
									<Label>Event Version</Label>
									<p className="mt-1 text-sm text-muted-foreground">
										{event.event_version.join(".")}
									</p>
								</div>
								<div>
									<Label>Created</Label>
									<p className="mt-1 text-sm text-muted-foreground">
										{new Date(
											event.created_at.secs_since_epoch * 1000,
										).toLocaleString()}
									</p>
								</div>
								<div>
									<Label>Last Updated</Label>
									<p className="mt-1 text-sm text-muted-foreground">
										{new Date(
											event.updated_at.secs_since_epoch * 1000,
										).toLocaleString()}
									</p>
								</div>
							</div>
						</CardContent>
					</Card>

					{/* Inputs - Show saved inputs and drift detection */}
					{event.node_id && (
						<Card>
							<CardHeader>
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<FormInputIcon className="h-5 w-5" />
										<CardTitle>Inputs</CardTitle>
										{inputsDrift?.hasDrift && (
											<Badge variant="destructive" className="ml-2">
												<AlertTriangle className="h-3 w-3 mr-1" />
												Drift Detected
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
										Refresh from Node
									</Button>
								</div>
								<CardDescription>
									Input pins captured at publish time. Changes to the node since
									then are shown below.
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								{inputsDrift?.isEmpty && !inputsDrift?.hasDrift && (
									<p className="text-sm text-muted-foreground">
										No input pins were captured for this event. Click "Refresh
										from Node" to sync.
									</p>
								)}

								{inputsDrift?.hasDrift && (
									<div className="space-y-3 p-3 bg-destructive/10 rounded-md border border-destructive/20">
										<p className="text-sm font-medium text-destructive">
											The node's inputs have changed since this event was
											published:
										</p>
										{inputsDrift.added.length > 0 && (
											<div className="text-sm">
												<span className="font-medium text-green-600">
													Added:{" "}
												</span>
												{inputsDrift.added
													.map((p) => p.friendly_name || p.name)
													.join(", ")}
											</div>
										)}
										{inputsDrift.removed.length > 0 && (
											<div className="text-sm">
												<span className="font-medium text-red-600">
													Removed:{" "}
												</span>
												{inputsDrift.removed
													.map((i) => i.friendly_name || i.name)
													.join(", ")}
											</div>
										)}
										{inputsDrift.changed.length > 0 && (
											<div className="text-sm">
												<span className="font-medium text-yellow-600">
													Changed:{" "}
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
											Captured Inputs ({event.inputs?.length ?? 0})
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
					<Card>
						<CardHeader>
							<div className="flex items-center justify-between">
								<CardTitle className="flex flex-row items-center gap-2">
									<CodeIcon className="h-5 w-5" />
									<p>Variables</p>
								</CardTitle>
								{isEditing && (
									<Dialog>
										<DialogTrigger asChild>
											<Button variant="outline" className="gap-2 ml-2">
												<Plus className="h-4 w-4" />
												Add Flow Variables
											</Button>
										</DialogTrigger>
										<DialogContent className="max-w-lg">
											<DialogHeader>
												<DialogTitle>Add Flow Variables</DialogTitle>
												<DialogDescription>
													Select flow variables to override in this event
													configuration
												</DialogDescription>
											</DialogHeader>
											<div className="space-y-2 max-h-80 overflow-y-auto">
												{board.data?.variables &&
													Object.entries(board.data.variables)
														.filter(([_, variable]) => variable.exposed)
														.map(([key, variable]) => {
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
																		</div>
																		{variable.default_value && (
																			<div className="text-xs text-muted-foreground mt-1">
																				Default:{" "}
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
																		{isAlreadyAdded ? "Remove" : "Add"}
																	</Button>
																</div>
															);
														})}
												{(!board.data?.variables ||
													Object.keys(board.data.variables).length === 0) && (
													<div className="text-center py-8 text-muted-foreground">
														No board variables available
													</div>
												)}
											</div>
										</DialogContent>
									</Dialog>
								)}
							</div>
						</CardHeader>
						<CardContent>
							{Object.keys(formData.variables).length > 0 ? (
								<div className="space-y-2">
									{Object.entries(formData.variables).map(([key, value]) => (
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
									))}
								</div>
							) : (
								<p className="text-sm text-muted-foreground">
									{isEditing
										? "No variables configured. Click 'Add Flow Variables' to get started."
										: "No variables configured"}
								</p>
							)}
						</CardContent>
					</Card>

					{/* Node Specific Configuration - Full width due to potential size */}
					{board.data && (
						<Card>
							<CardHeader>
								<CardTitle className="flex items-center gap-2">
									<CogIcon className="h-5 w-5" />
									Node Configuration
								</CardTitle>
							</CardHeader>
							<CardContent
								className={`space-y-4 flex flex-col items-start ${!isEditing ? "cursor-pointer" : ""}`}
								onClick={!isEditing ? enterEdit : undefined}
							>
								<EventTranslation
									appId={appId}
									eventType={formData.event_type}
									eventConfig={eventMapping}
									editing={isEditing}
									config={parseUint8ArrayToJson(event.config ?? []) ?? {}}
									board={board.data}
									nodeId={formData.node_id}
									hub={hub}
									eventId={event.id}
									canExecuteLocally={canExecuteLocally}
									eventExecutionMode={
										formData.execution_mode ?? IEventExecutionMode.Local
									}
									onUpdate={(config) => {
										console.dir(config);
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

					{/* Notes */}
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2">
								<StickyNote className="h-5 w-5" />
								Notes
							</CardTitle>
						</CardHeader>
						<CardContent>
							{isEditing ? (
								<Textarea
									value={formData.notes?.NOTES ?? ""}
									onChange={(e) =>
										handleInputChange("notes", { NOTES: e.target.value })
									}
									placeholder="Add notes about this event..."
									rows={4}
								/>
							) : (
								<button
									type="button"
									className="text-sm text-muted-foreground whitespace-pre-wrap text-left w-full rounded px-2 py-1 -mx-2 hover:bg-muted/60 transition-colors"
									onClick={enterEdit}
								>
									{event.notes?.NOTES ?? "Click to add notes..."}
								</button>
							)}
						</CardContent>
					</Card>
				</div>
			</div>

			{/* Floating Save Bar */}
			{isEditing && (
				<div className="fixed bottom-0 left-0 right-0 z-50 border-t bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/80">
					<div className="container mx-auto flex items-center justify-between py-3 px-4">
						<p className="text-sm text-muted-foreground">
							{isDirty ? "You have unsaved changes" : "Editing mode"}
						</p>
						<div className="flex items-center gap-2">
							<Button variant="outline" size="sm" onClick={handleCancel}>
								Discard
							</Button>
							<Button
								size="sm"
								onClick={() => handleSave()}
								disabled={!isDirty}
							>
								<SaveIcon className="h-4 w-4 mr-1.5" />
								Save Changes
							</Button>
						</div>
					</div>
				</div>
			)}

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

// Helper component for activate sink button in table
function TableActivateSinkButton({
	event,
	appId,
	onActivated,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
}: {
	event: IEvent;
	appId: string;
	onActivated: () => void;
	tokenStore?: IOAuthTokenStoreWithPending;
	consentStore?: IOAuthConsentStore;
	hub?: IHub;
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}) {
	const backend = useBackend();
	const [showDialog, setShowDialog] = useState(false);

	// OAuth consent dialog state
	const [showOAuthConsent, setShowOAuthConsent] = useState(false);
	const [missingProviders, setMissingProviders] = useState<IOAuthProvider[]>(
		[],
	);
	const [authorizedProviders, setAuthorizedProviders] = useState<Set<string>>(
		new Set(),
	);
	const [preAuthorizedProviders, setPreAuthorizedProviders] = useState<
		Set<string>
	>(new Set());
	const [pendingOAuthTokens, setPendingOAuthTokens] = useState<
		Record<string, IOAuthToken>
	>({});

	const handleActivate = async (
		patOrOAuthTokens?: string | Record<string, IOAuthToken>,
	) => {
		try {
			// Determine if we got a PAT string or OAuth tokens
			const pat =
				typeof patOrOAuthTokens === "string" ? patOrOAuthTokens : undefined;
			const oauthTokens =
				typeof patOrOAuthTokens === "object" ? patOrOAuthTokens : undefined;

			// Ensure the event is set to active before upserting
			const activeEvent = { ...event, active: true };
			await backend.eventState.upsertEvent(
				appId,
				activeEvent,
				undefined,
				pat,
				oauthTokens,
			);
			setShowDialog(false);
			setShowOAuthConsent(false);
			onActivated();
		} catch (error) {
			console.error("Failed to activate sink:", error);
		}
	};

	const handleClick = async () => {
		const isOffline = await backend.isOffline(appId);

		// Check OAuth requirements if tokenStore is provided
		if (!isOffline && tokenStore) {
			try {
				let oauthResult:
					| Awaited<ReturnType<typeof checkOAuthTokens>>
					| undefined;

				// Try board first, fallback to prerun for execute-only permissions
				const board = await backend.boardState
					.getBoard(
						appId,
						event.board_id,
						event.board_version as [number, number, number] | undefined,
					)
					.catch(() => undefined);

				if (board) {
					oauthResult = await checkOAuthTokens(board, tokenStore, hub, {
						refreshToken: onRefreshToken,
					});
				} else if (backend.eventState.prerunEvent) {
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
				}

				if (oauthResult && oauthResult.requiredProviders.length > 0) {
					// Check consent for providers that have tokens but might not have consent for this app
					const consentedIds = consentStore
						? await consentStore.getConsentedProviderIds(appId)
						: new Set<string>();
					const providersNeedingConsent: IOAuthProvider[] = [];
					const hasTokenNeedsConsent: Set<string> = new Set();

					// Add providers that are missing tokens
					providersNeedingConsent.push(...oauthResult.missingProviders);

					// Also add providers that have tokens but no consent for this specific app
					for (const provider of oauthResult.requiredProviders) {
						const hasToken = oauthResult.tokens[provider.id] !== undefined;
						const hasConsent = consentedIds.has(provider.id);

						if (hasToken && !hasConsent) {
							hasTokenNeedsConsent.add(provider.id);
							providersNeedingConsent.push(provider);
						}
					}

					if (providersNeedingConsent.length > 0) {
						// Store tokens for later use and show OAuth consent dialog
						setPendingOAuthTokens(oauthResult.tokens);
						setMissingProviders(providersNeedingConsent);
						setPreAuthorizedProviders(hasTokenNeedsConsent);
						setAuthorizedProviders(new Set());
						setShowOAuthConsent(true);
						return;
					}

					// All OAuth is satisfied, proceed with activation
					if (Object.keys(oauthResult.tokens).length > 0) {
						await handleActivate(oauthResult.tokens);
						return;
					}
				}
			} catch (error) {
				console.error("Failed to check OAuth:", error);
			}
		}

		if (!isOffline) {
			// Online project - show PAT dialog
			setShowDialog(true);
		} else {
			// Offline project - directly activate without PAT
			await handleActivate();
		}
	};

	const handleOAuthAuthorize = async (providerId: string) => {
		const provider = missingProviders.find((p) => p.id === providerId);
		if (!provider || !onStartOAuth) return;
		await onStartOAuth(provider);
	};

	const handleOAuthConfirmAll = async (rememberConsent: boolean) => {
		if (rememberConsent && consentStore) {
			for (const provider of missingProviders) {
				await consentStore.setConsent(appId, provider.id, provider.scopes);
			}
		}

		setShowOAuthConsent(false);

		// Collect all tokens (pending + newly authorized)
		const allTokens = { ...pendingOAuthTokens };
		for (const providerId of authorizedProviders) {
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

		if (Object.keys(allTokens).length > 0) {
			await handleActivate(allTokens);
		} else {
			// No OAuth tokens needed, show PAT dialog
			setShowDialog(true);
		}
	};

	const handleOAuthCancel = () => {
		setShowOAuthConsent(false);
		setMissingProviders([]);
		setAuthorizedProviders(new Set());
		setPreAuthorizedProviders(new Set());
		setPendingOAuthTokens({});
	};

	// Poll for OAuth token updates while the consent dialog is open
	useEffect(() => {
		if (!showOAuthConsent || !tokenStore || missingProviders.length === 0) {
			return;
		}

		const checkTokens = async () => {
			const newlyAuthorized = new Set(authorizedProviders);
			const newTokens = { ...pendingOAuthTokens };

			for (const provider of missingProviders) {
				if (
					newlyAuthorized.has(provider.id) ||
					preAuthorizedProviders.has(provider.id)
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

			if (newlyAuthorized.size !== authorizedProviders.size) {
				setAuthorizedProviders(newlyAuthorized);
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
		missingProviders,
		authorizedProviders,
		preAuthorizedProviders,
		pendingOAuthTokens,
	]);

	return (
		<>
			<Button
				variant="ghost"
				size="sm"
				className="h-6 px-2 text-xs gap-1"
				onClick={handleClick}
			>
				<Play className="h-3 w-3" />
				Activate
			</Button>
			<PatSelectorDialog
				open={showDialog}
				onOpenChange={setShowDialog}
				onPatSelected={handleActivate}
				title="Activate Event Sink"
				description="Select or create a Personal Access Token to activate this event sink."
			/>
			<OAuthConsentDialog
				open={showOAuthConsent}
				onOpenChange={setShowOAuthConsent}
				providers={missingProviders}
				authorizedProviders={authorizedProviders}
				preAuthorizedProviders={preAuthorizedProviders}
				onAuthorize={handleOAuthAuthorize}
				onConfirmAll={handleOAuthConfirmAll}
				onCancel={handleOAuthCancel}
			/>
		</>
	);
}

interface IEventsTableProps {
	events: IEvent[];
	boardsMap: Map<string, string>;
	appId: string;
	eventMapping: IEventMapping;
	/** Optional list of event types that are UI-capable and should have a route path. */
	uiEventTypes?: string[];
	onEdit: (event: IEvent) => void;
	onDelete: (eventId: string) => void;
	onNavigateToNode: (event: IEvent, nodeId: string) => void;
	onCreateEvent: () => void;
	/** Token store for OAuth checks. If not provided, OAuth checks are skipped. */
	tokenStore?: IOAuthTokenStoreWithPending;
	/** Consent store for OAuth consent tracking. */
	consentStore?: IOAuthConsentStore;
	/** Hub configuration for OAuth provider resolution */
	hub?: IHub;
	/** Whether the app is offline (local-only) */
	isOffline?: boolean;
	/** Callback to start OAuth authorization for a provider */
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	/** Optional callback to refresh expired tokens */
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

function EventsTable({
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
}: Readonly<IEventsTableProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const uiEventTypeSet = useMemo(
		() => new Set(uiEventTypes ?? []),
		[uiEventTypes],
	);
	const normalizePath = useCallback((path: unknown): string => {
		const raw = String(path ?? "").trim();
		if (!raw) return "/";
		const withoutQuery = raw.split("?")[0] ?? raw;
		if (!withoutQuery || withoutQuery === "/") return "/";
		return withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
	}, []);
	const [currentPage, setCurrentPage] = useState(1);
	const [pageSize, setPageSize] = useState(50);
	const [searchTerm, setSearchTerm] = useState("");
	const [viewMode, setViewMode] = useState<ViewMode>("list");
	const [sinkStatuses, setSinkStatuses] = useState<Map<string, boolean>>(
		new Map(),
	);
	const [eventNodeNames, setEventNodeNames] = useState<Map<string, string>>(
		new Map(),
	);

	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		(appId ?? "") !== "",
	);

	const routeByEventId = useMemo(() => {
		const map = new Map<string, string>();
		routes.data?.forEach((r) => {
			map.set(r.eventId, normalizePath(r.path));
		});
		return map;
	}, [routes.data, normalizePath]);

	// Fetch boards to get node names for events
	useEffect(() => {
		const fetchNodeNames = async () => {
			const nodeNamesMap = new Map<string, string>();
			const uniqueBoardIds = [...new Set(events.map((e) => e.board_id))];

			for (const boardId of uniqueBoardIds) {
				try {
					const board = await backend.boardState.getBoard(appId, boardId);
					// Map each event to its node name
					events.forEach((event) => {
						if (event.board_id === boardId && event.node_id) {
							const node = board?.nodes?.[event.node_id];
							if (node?.name) {
								nodeNamesMap.set(event.id, node.name);
							}
						}
					});
				} catch (error) {
					console.error(`Failed to fetch board ${boardId}:`, error);
				}
			}
			setEventNodeNames(nodeNamesMap);
		};

		if (events.length > 0) {
			fetchNodeNames();
		}
	}, [events, appId, backend.boardState]);

	const filteredEvents = useMemo(() => {
		if (!searchTerm) return events;
		return events.filter(
			(event) =>
				event.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
				event.description.toLowerCase().includes(searchTerm.toLowerCase()) ||
				event.event_type.toLowerCase().includes(searchTerm.toLowerCase()) ||
				(routeByEventId.get(event.id) ?? "")
					.toLowerCase()
					.includes(searchTerm.toLowerCase()) ||
				(boardsMap.get(event.board_id) ?? "")
					.toLowerCase()
					.includes(searchTerm.toLowerCase()),
		);
	}, [events, searchTerm, boardsMap]);

	const orderedEvents = useMemo(() => {
		const copy = [...filteredEvents];
		copy.sort((a, b) => {
			const aIsUi = uiEventTypeSet.has(a.event_type);
			const bIsUi = uiEventTypeSet.has(b.event_type);
			if (aIsUi !== bIsUi) return aIsUi ? -1 : 1;
			return (a.priority ?? 0) - (b.priority ?? 0);
		});
		return copy;
	}, [filteredEvents, uiEventTypeSet]);

	const totalPages = Math.ceil(orderedEvents.length / pageSize);
	const startIndex = (currentPage - 1) * pageSize;
	const paginatedEvents = orderedEvents.slice(
		startIndex,
		startIndex + pageSize,
	);

	// Check sink status for events that require it
	useEffect(() => {
		const checkSinkStatuses = async () => {
			const statuses = new Map<string, boolean>();
			const eventIds = paginatedEvents.map((e) => e.id).join(",");

			for (const event of paginatedEvents) {
				const nodeName = eventNodeNames.get(event.id);
				if (eventRequiresSink(eventMapping, event, nodeName)) {
					try {
						const isActive = await backend.eventState.isEventSinkActive(
							event.id,
						);
						statuses.set(event.id, isActive);
					} catch (error) {
						console.error(
							`Failed to check sink status for event ${event.id}:`,
							error,
						);
						statuses.set(event.id, false);
					}
				}
			}
			setSinkStatuses(statuses);
		};

		if (paginatedEvents.length > 0 && eventNodeNames.size > 0) {
			checkSinkStatuses();
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [
		paginatedEvents.map((e) => e.id).join(","),
		eventNodeNames.size,
		backend.eventState,
	]);

	const formatRelativeTime = useCallback((timestamp: number) => {
		const now = Date.now();
		const eventTime = timestamp * 1000;
		const diffMs = now - eventTime;
		const diffHours = diffMs / (1000 * 60 * 60);
		const diffDays = diffMs / (1000 * 60 * 60 * 24);

		if (diffHours < 24) {
			return `${Math.floor(diffHours)}h ago`;
		}
		if (diffDays < 7) {
			return `${Math.floor(diffDays)}d ago`;
		}
		return new Date(eventTime).toLocaleDateString();
	}, []);

	const truncateText = useCallback(
		(text: string, maxLength = 50) =>
			text.length > maxLength ? `${text.slice(0, maxLength)}...` : text,
		[],
	);

	useEffect(() => {
		setCurrentPage(1);
	}, [searchTerm]);

	// Compact, non-card list item for all screens
	const ListEventItem = ({ event }: { event: IEvent }) => {
		const nodeName = eventNodeNames.get(event.id);
		const requiresSink = eventRequiresSink(eventMapping, event, nodeName);
		const sinkActive = sinkStatuses.get(event.id);
		const isPageTargetEvent = !!event.default_page_id;
		const isUiEvent = uiEventTypeSet.has(event.event_type) || isPageTargetEvent;
		const routePath = routeByEventId.get(event.id);
		const [showActivateDialog, setShowActivateDialog] = useState(false);
		const [isEditingRoute, setIsEditingRoute] = useState(false);
		const [routeInput, setRouteInput] = useState(routePath ?? "");
		const [routeSaving, setRouteSaving] = useState(false);

		// OAuth consent dialog state
		const [showOAuthConsent, setShowOAuthConsent] = useState(false);
		const [missingProviders, setMissingProviders] = useState<IOAuthProvider[]>(
			[],
		);
		const [authorizedProviders, setAuthorizedProviders] = useState<Set<string>>(
			new Set(),
		);
		const [preAuthorizedProviders, setPreAuthorizedProviders] = useState<
			Set<string>
		>(new Set());
		const [pendingOAuthTokens, setPendingOAuthTokens] = useState<
			Record<string, IOAuthToken>
		>({});

		const handleRouteSave = async () => {
			if (!appId) return;
			setRouteSaving(true);
			try {
				const normalized = normalizePath(routeInput.trim());
				if (normalized) {
					// Delete old route if it exists and is different
					if (routePath && routePath !== normalized) {
						await backend.routeState.deleteRouteByPath(appId, routePath);
					}
					await backend.routeState.setRoute(appId, normalized, event.id);
				} else if (routePath) {
					// Remove route if input is empty
					await backend.routeState.deleteRouteByPath(appId, routePath);
				}
				await invalidate(backend.routeState.getRoutes, [appId]);
			} catch (error) {
				console.error("Failed to save route:", error);
			} finally {
				setRouteSaving(false);
				setIsEditingRoute(false);
			}
		};

		const handleActivateSink = async (
			patOrOAuthTokens?: string | Record<string, IOAuthToken>,
		) => {
			try {
				// Determine if we got a PAT string or OAuth tokens
				const pat =
					typeof patOrOAuthTokens === "string" ? patOrOAuthTokens : undefined;
				const oauthTokens =
					typeof patOrOAuthTokens === "object" ? patOrOAuthTokens : undefined;

				// Ensure the event is set to active before upserting
				const activeEvent = { ...event, active: true };
				await backend.eventState.upsertEvent(
					appId,
					activeEvent,
					undefined,
					pat,
					oauthTokens,
				);
				setShowActivateDialog(false);
				setShowOAuthConsent(false);
				// Trigger a re-check of sink statuses
				const checkStatus = async () => {
					const statuses = new Map<string, boolean>();
					for (const ev of paginatedEvents) {
						const evNodeName = eventNodeNames.get(ev.id);
						if (eventRequiresSink(eventMapping, ev, evNodeName)) {
							try {
								const isActive = await backend.eventState.isEventSinkActive(
									ev.id,
								);
								statuses.set(ev.id, isActive);
							} catch (error) {
								console.error(
									`Failed to check sink status for event ${ev.id}:`,
									error,
								);
								statuses.set(ev.id, false);
							}
						}
					}
					setSinkStatuses(statuses);
				};
				await checkStatus();
			} catch (error) {
				console.error("Failed to activate sink:", error);
			}
		};

		const handleActivateClick = async () => {
			const isOffline = await backend.isOffline(appId);

			// Check OAuth requirements if tokenStore is provided
			if (!isOffline && tokenStore) {
				try {
					let oauthResult:
						| Awaited<ReturnType<typeof checkOAuthTokens>>
						| undefined;

					// Try board first, fallback to prerun for execute-only permissions
					const board = await backend.boardState
						.getBoard(
							appId,
							event.board_id,
							event.board_version as [number, number, number] | undefined,
						)
						.catch(() => undefined);

					if (board) {
						oauthResult = await checkOAuthTokens(board, tokenStore, hub, {
							refreshToken: onRefreshToken,
						});
					} else if (backend.eventState.prerunEvent) {
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
					}

					if (oauthResult && oauthResult.requiredProviders.length > 0) {
						// Check consent for providers that have tokens but might not have consent for this app
						const consentedIds = consentStore
							? await consentStore.getConsentedProviderIds(appId)
							: new Set<string>();
						const providersNeedingConsent: IOAuthProvider[] = [];
						const hasTokenNeedsConsent: Set<string> = new Set();

						// Add providers that are missing tokens
						providersNeedingConsent.push(...oauthResult.missingProviders);

						// Also add providers that have tokens but no consent for this specific app
						for (const provider of oauthResult.requiredProviders) {
							const hasToken = oauthResult.tokens[provider.id] !== undefined;
							const hasConsent = consentedIds.has(provider.id);

							if (hasToken && !hasConsent) {
								hasTokenNeedsConsent.add(provider.id);
								providersNeedingConsent.push(provider);
							}
						}

						if (providersNeedingConsent.length > 0) {
							// Store tokens for later use and show OAuth consent dialog
							setPendingOAuthTokens(oauthResult.tokens);
							setMissingProviders(providersNeedingConsent);
							setPreAuthorizedProviders(hasTokenNeedsConsent);
							setAuthorizedProviders(new Set());
							setShowOAuthConsent(true);
							return;
						}

						// All OAuth is satisfied, proceed with activation
						if (Object.keys(oauthResult.tokens).length > 0) {
							await handleActivateSink(oauthResult.tokens);
							return;
						}
					}
				} catch (error) {
					console.error("Failed to check OAuth:", error);
				}
			}

			if (!isOffline) {
				// Online project - show PAT dialog
				setShowActivateDialog(true);
			} else {
				// Offline project - directly activate without PAT
				await handleActivateSink();
			}
		};

		const handleOAuthAuthorize = async (providerId: string) => {
			const provider = missingProviders.find((p) => p.id === providerId);
			if (!provider || !onStartOAuth) return;
			await onStartOAuth(provider);
		};

		const handleOAuthConfirmAll = async (rememberConsent: boolean) => {
			if (rememberConsent && consentStore) {
				for (const provider of missingProviders) {
					await consentStore.setConsent(appId, provider.id, provider.scopes);
				}
			}

			setShowOAuthConsent(false);

			// Collect all tokens (pending + newly authorized)
			const allTokens = { ...pendingOAuthTokens };
			for (const providerId of authorizedProviders) {
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

			if (Object.keys(allTokens).length > 0) {
				await handleActivateSink(allTokens);
			} else {
				// No OAuth tokens needed, show PAT dialog
				setShowActivateDialog(true);
			}
		};

		const handleOAuthCancel = () => {
			setShowOAuthConsent(false);
			setMissingProviders([]);
			setAuthorizedProviders(new Set());
			setPreAuthorizedProviders(new Set());
			setPendingOAuthTokens({});
		};

		// Poll for OAuth token updates while the consent dialog is open
		useEffect(() => {
			if (!showOAuthConsent || !tokenStore || missingProviders.length === 0) {
				return;
			}

			const checkTokens = async () => {
				const newlyAuthorized = new Set(authorizedProviders);
				const newTokens = { ...pendingOAuthTokens };

				for (const provider of missingProviders) {
					if (
						newlyAuthorized.has(provider.id) ||
						preAuthorizedProviders.has(provider.id)
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

				if (newlyAuthorized.size !== authorizedProviders.size) {
					setAuthorizedProviders(newlyAuthorized);
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
			missingProviders,
			authorizedProviders,
			preAuthorizedProviders,
			pendingOAuthTokens,
		]);

		return (
			<>
				<div className="px-3 py-2.5 border-b hover:bg-muted/50 transition-colors">
					{/* Single row layout */}
					<div className="flex items-center gap-3">
						{/* Status indicator */}
						<div
							className={`w-2 h-2 rounded-full shrink-0 ${event.active ? "bg-green-500" : "bg-orange-500"}`}
						/>

						{/* Name + type */}
						<div className="min-w-0 flex-1">
							<div className="flex items-center gap-2">
								<span className="font-medium truncate">{event.name}</span>
								<span className="text-xs px-1.5 py-0.5 rounded bg-secondary text-secondary-foreground shrink-0">
									{event.event_type}
								</span>
								{requiresSink && (
									<span
										className={`text-xs px-1.5 py-0.5 rounded shrink-0 ${sinkActive ? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300" : "bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300"}`}
									>
										{sinkActive ? "Active" : "Inactive"}
									</span>
								)}
								{requiresSink && (
									<span
										className={`text-xs px-1.5 py-0.5 rounded shrink-0 inline-flex items-center gap-1 ${isOffline ? "bg-muted text-muted-foreground" : "bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300"}`}
									>
										{isOffline ? (
											<>
												<Monitor className="h-3 w-3" /> Local
											</>
										) : (
											<>
												<Cloud className="h-3 w-3" /> Online
											</>
										)}
									</span>
								)}
							</div>
							{event.description && (
								<div className="text-xs text-muted-foreground truncate mt-0.5">
									{truncateText(event.description, 80)}
								</div>
							)}
						</div>

						{/* Route (inline editable for UI events) */}
						{isUiEvent && (
							<div className="shrink-0 flex items-center gap-1">
								{isEditingRoute ? (
									<div className="flex items-center gap-1">
										<Input
											value={routeInput}
											onChange={(e) => setRouteInput(e.target.value)}
											onKeyDown={(e) => {
												if (e.key === "Enter") handleRouteSave();
												if (e.key === "Escape") {
													setRouteInput(routePath ?? "");
													setIsEditingRoute(false);
												}
											}}
											onBlur={handleRouteSave}
											placeholder="/route"
											className="h-7 w-32 font-mono text-xs"
											autoFocus
											disabled={routeSaving}
										/>
									</div>
								) : (
									<button
										type="button"
										onClick={() => {
											setRouteInput(routePath ?? "");
											setIsEditingRoute(true);
										}}
										className={`text-xs px-2 py-1 rounded font-mono cursor-pointer hover:opacity-80 transition-opacity ${
											routePath
												? "bg-primary/10 text-primary border border-primary/20"
												: "bg-destructive/10 text-destructive border border-destructive/20"
										}`}
										title="Click to edit route"
									>
										{routePath ?? "No route"}
									</button>
								)}
							</div>
						)}

						{/* Flow info */}
						<div className="text-xs text-muted-foreground shrink-0 text-right hidden md:block">
							{boardsMap.get(event.board_id) ?? "Unknown"}
						</div>

						{/* Actions */}
						<div className="flex items-center gap-1 shrink-0">
							{requiresSink && !sinkActive && (
								<Button
									variant="ghost"
									size="sm"
									className="h-7 px-2 text-xs gap-1"
									onClick={handleActivateClick}
								>
									<Play className="h-3 w-3" />
								</Button>
							)}
							<Button
								variant="ghost"
								size="sm"
								className="h-7 w-7 p-0"
								onClick={() => onEdit(event)}
								title="Edit event"
							>
								<EditIcon className="h-4 w-4" />
							</Button>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 w-7 p-0"
								onClick={() => onNavigateToNode(event, event.node_id)}
								title="Open in flow"
							>
								<ExternalLinkIcon className="h-4 w-4" />
							</Button>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 w-7 p-0 text-destructive hover:text-destructive hover:bg-destructive/10"
								onClick={() => onDelete(event.id)}
								title="Delete event"
							>
								<Trash2 className="h-4 w-4" />
							</Button>
						</div>
					</div>
				</div>

				{/* PAT Selector Dialog for Activation */}
				<PatSelectorDialog
					open={showActivateDialog}
					onOpenChange={setShowActivateDialog}
					onPatSelected={handleActivateSink}
					title="Activate Event Sink"
					description="Select or create a Personal Access Token to activate this event sink."
				/>

				{/* OAuth Consent Dialog for Activation */}
				<OAuthConsentDialog
					open={showOAuthConsent}
					onOpenChange={setShowOAuthConsent}
					providers={missingProviders}
					onAuthorize={handleOAuthAuthorize}
					onConfirmAll={handleOAuthConfirmAll}
					onCancel={handleOAuthCancel}
					authorizedProviders={authorizedProviders}
					preAuthorizedProviders={preAuthorizedProviders}
				/>
			</>
		);
	};

	return (
		<div className="flex flex-col h-full min-h-0">
			<div className="flex items-center justify-between gap-4 mb-4 shrink-0 flex-wrap">
				<div className="flex items-center gap-2 flex-1 min-w-60">
					<Input
						placeholder="Search events..."
						value={searchTerm}
						onChange={(e) => setSearchTerm(e.target.value)}
						className="w-full sm:w-64"
					/>
					<div className="text-sm text-muted-foreground hidden sm:block">
						{filteredEvents.length} of {events.length} events
					</div>
				</div>
				<div className="flex items-center gap-2 flex-wrap">
					<div className="text-sm text-muted-foreground sm:hidden w-full">
						{filteredEvents.length} of {events.length} events
					</div>
					<Button onClick={onCreateEvent} className="gap-2 w-full sm:w-auto">
						<Plus className="h-4 w-4" />
						Create Event
					</Button>
					<div className="hidden sm:flex items-center gap-2">
						<Label htmlFor="pageSize" className="text-sm">
							Show:
						</Label>
						<Select
							value={pageSize.toString()}
							onValueChange={(value) => setPageSize(Number(value))}
						>
							<SelectTrigger className="w-20">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="25">25</SelectItem>
								<SelectItem value="50">50</SelectItem>
								<SelectItem value="100">100</SelectItem>
								<SelectItem value="200">200</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center">
						<div className="inline-flex rounded-md border p-1">
							<Button
								variant={viewMode === "list" ? "default" : "ghost"}
								size="sm"
								onClick={() => setViewMode("list")}
							>
								List
							</Button>
							<Button
								variant={viewMode === "table" ? "default" : "ghost"}
								size="sm"
								onClick={() => setViewMode("table")}
							>
								Table
							</Button>
						</div>
					</div>
				</div>
			</div>

			<div className="flex-1 min-h-0 rounded-md overflow-hidden flex flex-col">
				{filteredEvents.length === 0 ? (
					<div className="flex-1 flex items-center justify-center text-sm text-muted-foreground">
						No matching events
					</div>
				) : viewMode === "list" ? (
					<div className="flex-1 min-h-0 overflow-auto">
						<div className="divide-y">
							{(() => {
								const isUiOrPageEvent = (e: IEvent) =>
									uiEventTypeSet.has(e.event_type) || !!e.default_page_id;
								const ui = paginatedEvents.filter(isUiOrPageEvent);
								const backendOnly = paginatedEvents.filter(
									(e) => !isUiOrPageEvent(e),
								);

								return (
									<>
										{ui.length > 0 && (
											<div className="px-3 py-2 text-xs font-semibold text-muted-foreground bg-muted/30">
												UI Events
											</div>
										)}
										{ui.map((event) => (
											<ListEventItem key={event.id} event={event} />
										))}
										{backendOnly.length > 0 && (
											<div className="px-3 py-2 text-xs font-semibold text-muted-foreground bg-muted/30">
												Backend-only Events
											</div>
										)}
										{backendOnly.map((event) => (
											<ListEventItem key={event.id} event={event} />
										))}
									</>
								);
							})()}
						</div>
					</div>
				) : (
					<div className="flex-1 min-h-0 overflow-auto">
						<div className="overflow-x-auto">
							<Table>
								<TableHeader className="sticky top-0 bg-background z-10 border-b">
									<TableRow>
										<TableHead className="w-12">Status</TableHead>
										<TableHead className="min-w-[200px]">Name</TableHead>
										<TableHead className="min-w-[300px] hidden xl:table-cell">
											Description
										</TableHead>
										<TableHead className="min-w-[150px] hidden lg:table-cell">
											Flow
										</TableHead>
										<TableHead className="w-32">Event Type</TableHead>
										<TableHead className="w-32">Last Updated</TableHead>
										<TableHead className="w-24">Actions</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{paginatedEvents.map((event) => {
										const nodeName = eventNodeNames.get(event.id);
										const requiresSink = eventRequiresSink(
											eventMapping,
											event,
											nodeName,
										);
										const sinkActive = sinkStatuses.get(event.id);
										const isPageTargetEvent = !!event.default_page_id;
										const isUiEvent =
											uiEventTypeSet.has(event.event_type) || isPageTargetEvent;
										const routePath = routeByEventId.get(event.id);

										return (
											<TableRow key={event.id} className="hover:bg-muted/50">
												<TableCell>
													<div className="flex items-center">
														<div
															className={`w-2 h-2 rounded-full ${event.active ? "bg-green-500" : "bg-orange-500"}`}
														/>
													</div>
												</TableCell>
												<TableCell>
													<div className="font-medium">{event.name}</div>
													<div className="text-xs text-muted-foreground font-mono">
														{event.id.slice(0, 8)}...
													</div>
													{isUiEvent && (
														<div className="mt-1">
															<span
																className={`text-xs px-2 py-0.5 rounded-full font-mono inline-block ${routePath ? "bg-secondary text-secondary-foreground" : "bg-destructive/10 text-destructive"}`}
															>
																{routePath ?? "No route"}
															</span>
														</div>
													)}
													{!isUiEvent && (
														<div className="mt-1">
															<span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground inline-block">
																Backend-only
															</span>
														</div>
													)}
													{requiresSink && (
														<div className="flex items-center gap-2 mt-1">
															<div
																className={`text-xs px-2 py-0.5 rounded-full inline-block ${sinkActive ? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300" : "bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300"}`}
															>
																{sinkActive ? "Sink Active" : "Sink Inactive"}
															</div>
															<div
																className={`text-xs px-2 py-0.5 rounded-full inline-flex items-center gap-1 ${isOffline ? "bg-muted text-muted-foreground" : "bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300"}`}
															>
																{isOffline ? (
																	<>
																		<Monitor className="h-3 w-3" /> Local
																	</>
																) : (
																	<>
																		<Cloud className="h-3 w-3" /> Online
																	</>
																)}
															</div>
															{!sinkActive && (
																<TableActivateSinkButton
																	event={event}
																	appId={appId}
																	tokenStore={tokenStore}
																	consentStore={consentStore}
																	hub={hub}
																	onStartOAuth={onStartOAuth}
																	onRefreshToken={onRefreshToken}
																	onActivated={async () => {
																		// Refresh sink status after activation
																		try {
																			const isActive =
																				await backend.eventState.isEventSinkActive(
																					event.id,
																				);
																			setSinkStatuses((prev) =>
																				new Map(prev).set(event.id, isActive),
																			);
																		} catch (error) {
																			console.error(
																				"Failed to refresh sink status:",
																				error,
																			);
																		}
																	}}
																/>
															)}
														</div>
													)}
												</TableCell>
												<TableCell className="hidden xl:table-cell">
													<div className="text-sm text-muted-foreground">
														{event.description
															? truncateText(event.description, 80)
															: "No description"}
													</div>
												</TableCell>
												<TableCell className="hidden lg:table-cell">
													<div className="text-sm">
														{boardsMap.get(event.board_id) ?? "Unknown"}
													</div>
													<div className="text-xs text-muted-foreground">
														{event.board_version
															? `v${event.board_version.join(".")}`
															: "Latest"}
													</div>
												</TableCell>
												<TableCell>
													<div className="inline-flex items-center px-2 py-1 rounded-full text-xs font-medium bg-secondary text-secondary-foreground">
														{event.event_type}
													</div>
												</TableCell>
												<TableCell>
													<div className="text-sm text-muted-foreground">
														{formatRelativeTime(
															event.updated_at.secs_since_epoch,
														)}
													</div>
												</TableCell>
												<TableCell>
													<div className="flex items-center gap-1">
														<Button
															variant="ghost"
															size="sm"
															onClick={() => onEdit(event)}
															className="h-8 w-8 p-0"
															aria-label="Edit"
														>
															<EditIcon className="h-4 w-4" />
														</Button>
														<Button
															variant="ghost"
															size="sm"
															onClick={() =>
																onNavigateToNode(event, event.node_id)
															}
															className="h-8 w-8 p-0"
															aria-label="Open"
														>
															<ExternalLinkIcon className="h-4 w-4" />
														</Button>
														<Button
															variant="ghost"
															size="sm"
															onClick={() => onDelete(event.id)}
															className="h-8 w-8 p-0 text-destructive hover:text-destructive"
															aria-label="Delete"
														>
															<Trash2 className="h-4 w-4" />
														</Button>
													</div>
												</TableCell>
											</TableRow>
										);
									})}
								</TableBody>
							</Table>
						</div>
					</div>
				)}

				{totalPages > 1 && (
					<div className="border-t bg-background p-4 shrink-0">
						<div className="flex items-center justify-between">
							<div className="text-sm text-muted-foreground">
								Showing {startIndex + 1} to{" "}
								{Math.min(startIndex + pageSize, filteredEvents.length)} of{" "}
								{filteredEvents.length} results
							</div>
							<div className="flex items-center gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setCurrentPage((prev) => Math.max(1, prev - 1))
									}
									disabled={currentPage === 1}
								>
									Previous
								</Button>
								<div className="flex items-center gap-1">
									{Array.from({ length: Math.min(5, totalPages) }, (_, i) => {
										let pageNum: number;
										if (totalPages <= 5) pageNum = i + 1;
										else if (currentPage <= 3) pageNum = i + 1;
										else if (currentPage >= totalPages - 2)
											pageNum = totalPages - 4 + i;
										else pageNum = currentPage - 2 + i;

										return (
											<Button
												key={pageNum}
												variant={
													currentPage === pageNum ? "default" : "outline"
												}
												size="sm"
												onClick={() => setCurrentPage(pageNum)}
												className="w-8 h-8 p-0"
											>
												{pageNum}
											</Button>
										);
									})}
								</div>
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setCurrentPage((prev) => Math.min(totalPages, prev + 1))
									}
									disabled={currentPage === totalPages}
								>
									Next
								</Button>
							</div>
						</div>
					</div>
				)}
			</div>
		</div>
	);
}
