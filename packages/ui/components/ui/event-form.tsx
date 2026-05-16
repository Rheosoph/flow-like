"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import type { IOAuthConsentStore } from "../../db/oauth-db";
import { useInvoke } from "../../hooks";
import type { IEvent, IOAuthProvider, IOAuthToken } from "../../lib";
import { checkOAuthTokens } from "../../lib/oauth/helpers";
import type { IOAuthTokenStoreWithPending } from "../../lib/oauth/types";
import type { IStoredOAuthToken } from "../../lib/oauth/types";
import { IExecutionMode } from "../../lib/schema/flow/board";
import { IEventExecutionMode } from "../../lib/schema/flow/event";
import type { IHub } from "../../lib/schema/hub/hub";
import { convertJsonToUint8Array } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import type { PageListItem } from "../../state/backend-state/page-state";
import type { IEventMapping } from "../interfaces";
import { OAuthConsentDialog } from "../oauth/oauth-consent-dialog";
import { Button } from "./button";
import { EventTypeConfig } from "./event-type-config";
import { Input } from "./input";
import { Label } from "./label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "./select";
import { Separator } from "./separator";
import { Textarea } from "./textarea";

interface EventFormProps {
	event?: IEvent;
	eventConfig: IEventMapping;
	/** Optional list of event types that should be treated as UI-capable and require a unique route path. */
	uiEventTypes?: string[];
	appId: string;
	onSubmit: (
		event: Partial<IEvent>,
		oauthTokens?: Record<string, IOAuthToken>,
	) => void;
	onCancel: () => void;
	isSubmitting?: boolean;
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
}

export function EventForm({
	eventConfig,
	uiEventTypes,
	appId,
	event,
	onSubmit,
	onCancel,
	isSubmitting = false,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
}: Readonly<EventFormProps>) {
	const backend = useBackend();
	const inferredDefaultPageId = (event as any)?.default_page_id as
		| string
		| null
		| undefined;
	const [formData, setFormData] = useState({
		name: event?.name ?? "",
		description: event?.description ?? "",
		board_version: undefined,
		node_id: event?.node_id ?? "",
		board_id: event?.board_id ?? "",
		default_page_id: inferredDefaultPageId ?? undefined,
		target_kind: inferredDefaultPageId ? ("page" as const) : ("board" as const),
		event_type: undefined,
		config: [],
		path: (event as any)?.path ?? "/",
		execution_mode:
			(event?.execution_mode as IEventExecutionMode | undefined) ??
			IEventExecutionMode.Local,
	});
	const [pathError, setPathError] = useState<string | null>(null);

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

	const boards = useInvoke(backend.boardState.getBoards, backend.boardState, [
		appId,
	]);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		appId !== "",
	);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		appId !== "",
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, formData.board_id, formData.board_version],
		(formData.board_id ?? "") !== "",
	);

	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, formData.board_id],
		(formData.board_id ?? "") !== "",
	);

	const [selectedNodeType, setSelectedNodeType] = useState<string>("");
	const [eventTypeConfig, setEventTypeConfig] = useState<any>({});

	const boardExecutionMode = board.data?.execution_mode;
	const canExecuteLocally = backend.capabilities().canExecuteLocally;

	// Lock the event's execution mode when the board constrains it. Boards in
	// Hybrid mode let the user pick; Local/Remote boards propagate to events.
	useEffect(() => {
		if (!boardExecutionMode) return;
		if (boardExecutionMode === IExecutionMode.Local) {
			if (formData.execution_mode !== IEventExecutionMode.Local) {
				setFormData((prev) => ({
					...prev,
					execution_mode: IEventExecutionMode.Local,
				}));
			}
		} else if (boardExecutionMode === IExecutionMode.Remote) {
			if (formData.execution_mode !== IEventExecutionMode.Remote) {
				setFormData((prev) => ({
					...prev,
					execution_mode: IEventExecutionMode.Remote,
				}));
			}
		}
	}, [boardExecutionMode, formData.execution_mode]);

	const executionModeLocked =
		boardExecutionMode === IExecutionMode.Local ||
		boardExecutionMode === IExecutionMode.Remote;

	// Local is disabled only when the device can't execute locally; Remote
	// is always a valid choice (the backend rejects unsupported hub setups).
	const canPickLocal = canExecuteLocally;

	const normalizedPath = (path: string | undefined): string => {
		const raw = (path ?? "").trim();
		if (!raw) return "/";
		const withoutQuery = raw.split("?")[0] ?? raw;
		if (!withoutQuery || withoutQuery === "/") return "/";
		return withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
	};

	const isUiEventType = (eventType: string | undefined): boolean => {
		if (!eventType) return false;
		if (!uiEventTypes || uiEventTypes.length === 0) return false;
		return uiEventTypes.includes(eventType);
	};

	const shouldRequireRoutePath = useMemo(() => {
		if (formData.target_kind === "page") return true;
		return isUiEventType(formData.event_type);
	}, [formData.target_kind, formData.event_type, uiEventTypes]);

	const handleInputChange = (field: string, value: any) => {
		setFormData((prev) => ({ ...prev, [field]: value }));
	};

	const handleSubmit = async (e: React.FormEvent) => {
		e.preventDefault();

		// Validate route path for UI events
		if (shouldRequireRoutePath) {
			const path = normalizedPath(formData.path);
			if (!path.startsWith("/")) {
				setPathError("Path must start with '/'");
				return;
			}
			const existing = routes.data?.find((r) => {
				if (normalizedPath(r.path) !== path) return false;
				if (event?.id && r.eventId === event.id) return false;
				return true;
			});
			if (existing) {
				setPathError("This path is already used by another route");
				return;
			}
			setPathError(null);
		}

		if (formData.target_kind === "page") {
			if (!formData.default_page_id) return;
		}

		const eventData: Partial<IEvent> = {
			...formData,
			variables: event?.variables || {},
			...(selectedNodeType && { eventTypeConfig }),
		};

		// Check OAuth requirements if tokenStore is provided and board is loaded
		if (tokenStore && board.data) {
			const oauthResult = await checkOAuthTokens(board.data, tokenStore, hub, {
				refreshToken: onRefreshToken,
			});

			if (oauthResult.requiredProviders.length > 0) {
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
					// Store tokens for later use
					setPendingOAuthTokens(oauthResult.tokens);
					setMissingProviders(providersNeedingConsent);
					setPreAuthorizedProviders(hasTokenNeedsConsent);
					setAuthorizedProviders(new Set());
					setShowOAuthConsent(true);
					return;
				}

				// All OAuth is satisfied, pass tokens
				if (Object.keys(oauthResult.tokens).length > 0) {
					onSubmit(eventData, oauthResult.tokens);
					return;
				}
			}
		}

		onSubmit(eventData);
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

		const eventData: Partial<IEvent> = {
			...formData,
			variables: event?.variables || {},
			...(selectedNodeType && { eventTypeConfig }),
		};

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
			onSubmit(eventData, allTokens);
		} else {
			onSubmit(eventData);
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

	const isEditing = !!event;
	const isPageEvent = formData.target_kind === "page";

	const handleSelectPage = useCallback(
		(pageId: string) => {
			setPathError(null);
			const page = (pages.data ?? []).find(
				(p: PageListItem) => p.pageId === pageId,
			);
			handleInputChange("default_page_id", pageId);
			handleInputChange("event_type", "page");
			handleInputChange("node_id", "");
			handleInputChange("board_version", undefined);
			setSelectedNodeType("");
			setEventTypeConfig({});
			if (page?.boardId) {
				handleInputChange("board_id", page.boardId);
			} else {
				handleInputChange("board_id", "");
			}
		},
		[pages.data],
	);

	return (
		<form onSubmit={handleSubmit} className="space-y-6 pb-4">
			{/* Basic Information */}
			<div className="space-y-4">
				<div className="space-y-2">
					<Label htmlFor="name">Event Name</Label>
					<Input
						id="name"
						value={formData.name}
						onChange={(e) => handleInputChange("name", e.target.value)}
						placeholder="Enter event name"
						required
					/>
				</div>

				<div className="space-y-2">
					<Label htmlFor="description">Description</Label>
					<Textarea
						id="description"
						value={formData.description}
						onChange={(e) => handleInputChange("description", e.target.value)}
						placeholder="Enter event description"
						rows={3}
					/>
				</div>
			</div>

			<Separator />

			{/* Target Selection */}
			<div className="space-y-4">
				<div className="space-y-2">
					<Label>Target</Label>
					<Select
						value={formData.target_kind}
						onValueChange={(value) => {
							setPathError(null);
							handleInputChange("target_kind", value as "board" | "page");
							if (value === "page") {
								handleInputChange("event_type", "page");
								return;
							}
							handleInputChange("default_page_id", undefined);
						}}
					>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="board">Board Event</SelectItem>
							<SelectItem value="page">Page</SelectItem>
						</SelectContent>
					</Select>
				</div>
			</div>

			{isPageEvent ? (
				<div className="space-y-4">
					<div className="space-y-2">
						<Label>Page</Label>
						<Select
							value={(formData.default_page_id ?? "") as string}
							onValueChange={handleSelectPage}
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

					<div className="space-y-2">
						<Label htmlFor="path">Route Path</Label>
						<Input
							id="path"
							value={formData.path}
							onChange={(e) => {
								setPathError(null);
								handleInputChange("path", e.target.value);
							}}
							placeholder="/"
						/>
						{pathError ? (
							<p className="text-sm text-destructive">{pathError}</p>
						) : (
							<p className="text-xs text-muted-foreground">
								Default is <span className="font-mono">/</span>. Each path can
								only be used once.
							</p>
						)}
					</div>
				</div>
			) : (
				<>
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
									setSelectedNodeType("");
									setEventTypeConfig({});
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
								value={formData.board_version ?? ""}
								onValueChange={(value) => {
									handleInputChange(
										"board_version",
										value === "" ? undefined : value.split(".").map(Number),
									);
									handleInputChange("node_id", undefined);
								}}
							>
								<SelectTrigger>
									<SelectValue placeholder="Latest" />
								</SelectTrigger>
								<SelectContent>
									{versions.data?.map((board) => (
										<SelectItem key={board.join(".")} value={board.join(".")}>
											v{board.join(".")}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					</div>

					{/* Execution mode — where the event runs. Locked to match the
					    board when the board is Local/Remote; user-selectable when
					    the board is Hybrid. */}
					{formData.board_id && (
						<div className="space-y-2">
							<Label htmlFor="execution_mode">Execution Mode</Label>
							<Select
								value={formData.execution_mode}
								onValueChange={(value) =>
									handleInputChange(
										"execution_mode",
										value as IEventExecutionMode,
									)
								}
								disabled={executionModeLocked}
							>
								<SelectTrigger id="execution_mode">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem
										value={IEventExecutionMode.Local}
										disabled={!canPickLocal && !executionModeLocked}
									>
										Local — runs on this device
									</SelectItem>
									<SelectItem value={IEventExecutionMode.Remote}>
										Remote — runs on the server
									</SelectItem>
								</SelectContent>
							</Select>
							<p className="text-xs text-muted-foreground">
								{executionModeLocked
									? `Locked by the flow's execution mode (${boardExecutionMode}).`
									: "An event always runs in exactly one environment — the UI, endpoints, and credentials differ between the two."}
							</p>
						</div>
					)}
				</>
			)}

			{/* Node and Board Selection */}
			{!isPageEvent && board.data && (
				<div className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="node">Node</Label>
						<Select
							value={formData.node_id}
							onValueChange={(value) => {
								handleInputChange("node_id", value);
								const node = board.data.nodes[value];
								if (!node) return;
								if (node) {
									const eventType = eventConfig[node?.name];
									if (eventType) {
										handleInputChange("event_type", eventType.defaultEventType);
										handleInputChange(
											"config",
											convertJsonToUint8Array(
												eventType.configs[eventType.defaultEventType] ?? {},
											) ?? [],
										);
									}
								}
							}}
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

					{/* Event Type Selector - shown when node has multiple event types.
					    Types are filtered by the chosen execution mode so a Remote
					    event never shows offline-only choices (email IMAP, Discord). */}
					{formData.node_id &&
						board.data.nodes[formData.node_id] &&
						(() => {
							const node = board.data.nodes[formData.node_id];
							const nodeEventConfig = eventConfig[node?.name];

							if (!nodeEventConfig) return null;

							const isLocalEvent =
								formData.execution_mode === IEventExecutionMode.Local;
							const visibleTypes = nodeEventConfig.eventTypes.filter((type) => {
								if (!nodeEventConfig.withSink?.includes(type)) return true;
								const availability =
									nodeEventConfig.sinkAvailability?.[type]?.availability;
								if (!availability || availability === "both") return true;
								return isLocalEvent
									? availability === "local"
									: availability === "remote";
							});

							if (visibleTypes.length <= 1) return null;

							return (
								<div className="space-y-2">
									<Label htmlFor="event_type">Event Type</Label>
									<Select
										value={
											formData.event_type || nodeEventConfig.defaultEventType
										}
										onValueChange={(value) => {
											handleInputChange("event_type", value);
											setPathError(null);
											handleInputChange(
												"config",
												convertJsonToUint8Array(
													nodeEventConfig.configs[value] ?? {},
												) ?? [],
											);
										}}
									>
										<SelectTrigger>
											<SelectValue placeholder="Select event type" />
										</SelectTrigger>
										<SelectContent>
											{visibleTypes.map((type) => (
												<SelectItem key={type} value={type}>
													{type
														.replace(/_/g, " ")
														.replace(/\b\w/g, (c) => c.toUpperCase())}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
								</div>
							);
						})()}

					{shouldRequireRoutePath && (
						<div className="space-y-2">
							<Label htmlFor="path">Route Path</Label>
							<Input
								id="path"
								value={formData.path}
								onChange={(e) => {
									setPathError(null);
									handleInputChange("path", e.target.value);
								}}
								placeholder="/"
							/>
							{pathError ? (
								<p className="text-sm text-destructive">{pathError}</p>
							) : (
								<p className="text-xs text-muted-foreground">
									Default is <span className="font-mono">/</span>. Each path can
									only be used once.
								</p>
							)}
						</div>
					)}
				</div>
			)}

			{/* Type-specific Configuration */}
			{selectedNodeType && (
				<>
					<Separator />
					<EventTypeConfig
						type={selectedNodeType}
						config={eventTypeConfig}
						onChange={setEventTypeConfig}
					/>
				</>
			)}

			{/* Form Actions */}
			<div className="flex justify-end space-x-2 pt-4 border-t">
				<Button type="button" variant="outline" onClick={onCancel}>
					Cancel
				</Button>
				<Button
					type="submit"
					disabled={
						isSubmitting ||
						!formData.name ||
						(isPageEvent
							? !formData.default_page_id
							: !formData.board_id || !formData.node_id)
					}
				>
					{isEditing ? "Update Event" : "Create Event"}
				</Button>
			</div>

			{/* OAuth Consent Dialog */}
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
		</form>
	);
}
