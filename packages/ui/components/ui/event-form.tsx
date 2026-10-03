"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	Cloud,
	Info,
	Loader2,
	Monitor,
	Server,
	Zap,
} from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { IOAuthConsentStore } from "../../db/oauth-db";
import { useInvoke } from "../../hooks/use-invoke";
import type { IEvent, IOAuthProvider, IOAuthToken } from "../../lib";
import { eventKind } from "../../lib/device-management/deployment";
import {
	isServerOnlyEventType,
	serverEventBlocker,
	serverEventBlockerMessage,
	sinkSupportsEventExecution,
} from "../../lib/event-definitions";
import { getEventSections, isTriggerSection } from "../../lib/event-sections";
import {
	eventTriggerConfig,
	mergeEventTriggerConfig,
} from "../../lib/event-source";
import { formatEventTypeLabel } from "../../lib/event-type-label";
import { checkOAuthTokens } from "../../lib/oauth/helpers";
import type {
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../lib/oauth/types";
import { asArray } from "../../lib/response-shape";
import { normalizeRoutePath } from "../../lib/route-path";
import { IExecutionMode } from "../../lib/schema/flow/board";
import {
	type BoardVersion,
	normalizeBoardVersion,
} from "../../lib/schema/flow/board-version";
import {
	IEventExecutionMode,
	IEventExposure,
} from "../../lib/schema/flow/event";
import type { IHub } from "../../lib/schema/hub/hub";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../lib/uint8";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IEventMapping } from "../interfaces/interfaces";
import { OAuthConsentDialog } from "../oauth/oauth-consent-dialog";
import type { DeviceWorkspaceOverrides } from "../settings/devices/workspace/device-workspace-provider";
import { NewEventDeployment } from "../settings/events/new-event-deployment";
import { Button } from "./button";
import { EventStartPicker, type EventStartTarget } from "./event-start-picker";
import { Input } from "./input";
import { Label } from "./label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "./select";
import { Textarea } from "./textarea";

interface EventFormProps {
	deviceWorkspaceOverrides?: DeviceWorkspaceOverrides;
	deviceEventId?: string;
	event?: IEvent;
	eventConfig: IEventMapping;
	uiEventTypes?: string[];
	appId: string;
	onSubmit: (
		event: Partial<IEvent>,
		oauthTokens?: Record<string, IOAuthToken>,
	) => void | Promise<void>;
	onCreateDevice?: (event: Partial<IEvent>) => Promise<IEvent>;
	onDeploymentComplete?: (event: IEvent) => void;
	onNavigateDeployment?: (href: string) => void;
	onBusyChange?: (busy: boolean) => void;
	onCancel: () => void;
	isSubmitting?: boolean;
	tokenStore?: IOAuthTokenStoreWithPending;
	consentStore?: IOAuthConsentStore;
	hub?: IHub;
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

type Destination = "computer" | "hub" | "device";
interface EventFormData {
	name: string;
	description: string;
	board_version?: BoardVersion;
	node_id?: string;
	board_id: string;
	default_page_id?: string;
	target_kind: "board" | "page";
	event_type?: string;
	config: number[];
	path: string;
	execution_mode: IEventExecutionMode;
	exposure: IEventExposure;
}

export function EventForm({
	deviceWorkspaceOverrides,
	deviceEventId,
	eventConfig,
	uiEventTypes,
	appId,
	event,
	onSubmit,
	onCreateDevice,
	onDeploymentComplete,
	onNavigateDeployment,
	onBusyChange,
	onCancel,
	isSubmitting = false,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
}: Readonly<EventFormProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const formId = useId();
	const formRef = useRef<HTMLFormElement>(null);
	const [footerContainer, setFooterContainer] = useState<HTMLDivElement | null>(
		null,
	);
	const canExecuteLocally = backend.capabilities().canExecuteLocally;
	const [destination, setDestination] = useState<Destination>(
		onCreateDevice ? "device" : canExecuteLocally ? "computer" : "hub",
	);
	const [deploymentBusy, setDeploymentBusy] = useState(false);
	const [deploymentSaved, setDeploymentSaved] = useState(false);
	const [checkingOAuth, setCheckingOAuth] = useState(false);
	const [submitError, setSubmitError] = useState<string | null>(null);
	const [configSection, setConfigSection] = useState<string | undefined>();
	const [formData, setFormData] = useState<EventFormData>({
		name: event?.name ?? "",
		description: event?.description ?? "",
		board_version: normalizeBoardVersion(event?.board_version),
		node_id: event?.node_id ?? "",
		board_id: event?.board_id ?? "",
		default_page_id: event?.default_page_id ?? undefined,
		target_kind: event?.default_page_id ? "page" : "board",
		event_type: event?.event_type,
		config: event?.config ?? [],
		path:
			typeof event?.route === "string"
				? event.route
				: typeof event?.path === "string"
					? event.path
					: "/",
		execution_mode: event?.execution_mode ?? IEventExecutionMode.Local,
		exposure: event?.exposure ?? IEventExposure.Public,
	});
	const [pathError, setPathError] = useState<string | null>(null);
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
	const offline = useInvoke(backend.isOffline, backend, [appId], !!appId);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		!!appId,
	);
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, formData.board_id, formData.board_version],
		!!formData.board_id,
	);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, formData.board_id],
		!!formData.board_id,
	);
	const node = board.data?.nodes?.[formData.node_id ?? ""];
	const mapping = node ? eventConfig[node.name] : undefined;
	const boardExecutionMode = board.data?.execution_mode;
	const isServerEvent = isServerOnlyEventType(formData.event_type);
	const serverBlocker = serverEventBlocker(offline.data, boardExecutionMode);
	const serverEventError =
		isServerEvent && serverBlocker
			? serverEventBlockerMessage(t, serverBlocker, formData.event_type ?? "")
			: undefined;
	const isPageEvent = formData.target_kind === "page";
	const shouldRequireRoutePath =
		isPageEvent || !!uiEventTypes?.includes(formData.event_type ?? "");
	const config = useMemo(
		() => parseUint8ArrayToJson(eventTriggerConfig(formData.config)) ?? {},
		[formData.config],
	);
	const triggerLabels: Record<string, string> = {
		quick_action: t("eventCreation.quickAction", "Quick action"),
		api: t("eventCreation.apiEndpoint", "API endpoint"),
		cron: t("eventCreation.schedule", "Schedule"),
		daemon: t("eventCreation.backgroundService", "Background service"),
		deeplink: t("eventCreation.deepLink", "Deep link"),
		rest: t("eventCreation.restServer", "REST server"),
		mcp: t("eventCreation.mcpServer", "MCP server"),
	};
	const triggerLabel = (type: string) =>
		triggerLabels[type] ?? formatEventTypeLabel(type);
	const typeLabel = formData.event_type
		? triggerLabel(formData.event_type)
		: t("event", "Event");
	const deviceSupported =
		!formData.event_type ||
		eventKind({
			event_type: formData.event_type,
			default_page_id: formData.default_page_id,
		}) !== null;
	const supportsDestination = (target: Destination) => {
		if (target === "device") return !!onCreateDevice && deviceSupported;
		if (
			target === "computer" &&
			(!canExecuteLocally ||
				boardExecutionMode === IExecutionMode.Remote ||
				isServerEvent)
		)
			return false;
		if (
			target === "hub" &&
			(offline.data !== false || boardExecutionMode === IExecutionMode.Local)
		)
			return false;
		if (mapping?.withSink.includes(formData.event_type ?? "")) {
			const mode =
				target === "computer"
					? IEventExecutionMode.Local
					: IEventExecutionMode.Remote;
			if (
				!sinkSupportsEventExecution(
					mapping.sinkAvailability?.[formData.event_type ?? ""],
					mode,
					canExecuteLocally,
				)
			)
				return false;
			if (
				target === "hub" &&
				isServerEvent &&
				hub &&
				hub.supported_sinks?.[formData.event_type ?? ""] !== true
			)
				return false;
		}
		return true;
	};
	const destinationAvailable = supportsDestination(destination);
	const busy = isSubmitting || checkingOAuth || deploymentBusy;
	const locked = busy || deploymentSaved;
	const availableTriggers = isPageEvent
		? ["page"]
		: (mapping?.eventTypes ?? (node?.start ? ["default"] : []));
	const validTrigger = availableTriggers.includes(formData.event_type ?? "");
	const validTarget =
		!!formData.board_id &&
		(isPageEvent ? !!formData.default_page_id : !!node?.start);
	const canSubmit =
		!!formData.name.trim() &&
		validTarget &&
		validTrigger &&
		!!formData.event_type &&
		destinationAvailable &&
		!serverEventError &&
		!board.isLoading &&
		!board.isError &&
		(!shouldRequireRoutePath || routes.isSuccess) &&
		!(isServerEvent && offline.isLoading);
	useEffect(() => {
		onBusyChange?.(busy);
	}, [busy, onBusyChange]);
	useEffect(() => {
		if (deploymentBusy || deploymentSaved) return;
		const mode =
			destination === "hub" || boardExecutionMode === IExecutionMode.Remote
				? IEventExecutionMode.Remote
				: IEventExecutionMode.Local;
		setFormData((previous) =>
			previous.execution_mode === mode
				? previous
				: { ...previous, execution_mode: mode },
		);
	}, [destination, boardExecutionMode, deploymentBusy, deploymentSaved]);
	useEffect(() => {
		if (deploymentBusy || deploymentSaved || !isServerEvent || serverBlocker)
			return;
		setDestination("hub");
	}, [isServerEvent, serverBlocker, deploymentBusy, deploymentSaved]);
	function handleInputChange<K extends keyof EventFormData>(
		field: K,
		value: EventFormData[K],
	) {
		setSubmitError(null);
		setFormData((previous) => ({
			...previous,
			[field]:
				field === "config"
					? mergeEventTriggerConfig(previous.config, value as number[])
					: value,
		}));
	}
	function selectTarget(target: EventStartTarget) {
		const nextMapping = eventConfig[target.nodeType ?? ""];
		const type = target.pageId
			? "page"
			: (nextMapping?.defaultEventType ?? "default");
		setPathError(null);
		setConfigSection(undefined);
		setSubmitError(null);
		setFormData((previous) => ({
			...previous,
			board_id: target.boardId,
			node_id: target.nodeId,
			default_page_id: target.pageId,
			board_version: target.boardVersion,
			target_kind: target.pageId ? "page" : "board",
			event_type: type,
			config: convertJsonToUint8Array(nextMapping?.configs[type] ?? {}) ?? [],
			name: previous.name || target.name,
		}));
	}
	function selectType(type: string) {
		setConfigSection(undefined);
		setPathError(null);
		setFormData((previous) => ({
			...previous,
			event_type: type,
			config: convertJsonToUint8Array(mapping?.configs[type] ?? {}) ?? [],
		}));
	}
	function buildEventData(): Partial<IEvent> {
		const { target_kind, path, ...data } = formData;
		return {
			...data,
			name: data.name.trim(),
			config: eventTriggerConfig(data.config),
			...(shouldRequireRoutePath
				? { path: normalizeRoutePath(path), route: normalizeRoutePath(path) }
				: {}),
			variables: event?.variables ?? {},
		};
	}
	function validate(): boolean {
		if (!canSubmit || !formRef.current?.reportValidity()) return false;
		if (shouldRequireRoutePath) {
			if (routes.isLoading || routes.isError) {
				setPathError(
					t(
						"routeCheckUnavailable",
						"Wait for route availability to load, then try again.",
					),
				);
				return false;
			}
			if (
				asArray(routes.data).some(
					(route) =>
						normalizeRoutePath(route.path) ===
							normalizeRoutePath(formData.path) &&
						(destination !== "device" || route.eventId !== deviceEventId),
				)
			) {
				setPathError(
					t("routeAlreadyUsed", "This path is already used by another route."),
				);
				return false;
			}
		}
		setPathError(null);
		return true;
	}
	const handleSubmit = async (e: React.FormEvent) => {
		e.preventDefault();
		if (locked || destination === "device" || !validate()) return;
		setCheckingOAuth(true);
		setSubmitError(null);
		try {
			await submitToSource();
		} catch (error) {
			setSubmitError(error instanceof Error ? error.message : String(error));
		} finally {
			setCheckingOAuth(false);
		}
	};
	const submitToSource = async () => {
		const eventData = buildEventData();
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
					await onSubmit(eventData, oauthResult.tokens);
					return;
				}
			}
		}

		await onSubmit(eventData);
	};

	const handleOAuthAuthorize = async (providerId: string) => {
		const provider = missingProviders.find((p) => p.id === providerId);
		if (!provider || !onStartOAuth) return;
		await onStartOAuth(provider);
	};

	const handleOAuthConfirmAll = async (rememberConsent: boolean) => {
		if (busy || !validate()) return;
		setCheckingOAuth(true);
		setSubmitError(null);
		try {
			if (rememberConsent && consentStore) {
				for (const provider of missingProviders) {
					await consentStore.setConsent(appId, provider.id, provider.scopes);
				}
			}

			setShowOAuthConsent(false);

			const eventData = buildEventData();

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
				await onSubmit(eventData, allTokens);
			} else {
				await onSubmit(eventData);
			}
		} catch (error) {
			setSubmitError(error instanceof Error ? error.message : String(error));
		} finally {
			setCheckingOAuth(false);
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

	const ConfigInterface = mapping?.configInterfaces[formData.event_type ?? ""];
	const triggerSections = getEventSections(buildEventData() as IEvent).filter(
		(section) => isTriggerSection(section.id),
	);
	const activeSection = configSection ?? triggerSections[0]?.id;
	const configEditor =
		ConfigInterface && node ? (
			<ConfigInterface
				isEditing={!locked}
				appId={appId}
				boardId={formData.board_id}
				nodeId={node.id}
				node={node}
				config={config}
				onConfigUpdate={(payload) =>
					handleInputChange("config", convertJsonToUint8Array(payload) ?? [])
				}
				hub={hub}
				canExecuteLocally={destination === "device" || canExecuteLocally}
				eventExecutionMode={formData.execution_mode}
				section={activeSection}
			/>
		) : null;
	const destinations = [
		{
			id: "computer" as const,
			icon: Monitor,
			title: t("thisComputer", "This computer"),
			detail: t("runsInDesktopApp", "Desktop app"),
		},
		{
			id: "hub" as const,
			icon: Cloud,
			title: t("hub", "Hub"),
			detail: t("runsOnServer", "On the server"),
		},
		{
			id: "device" as const,
			icon: Server,
			title: t("device", "Device"),
			detail: t("chooseDevices", "Choose devices"),
		},
	];
	return (
		<div className="flex min-h-0 flex-1 flex-col">
			<div
				className={cn(
					"grid min-h-0 flex-1 overflow-y-auto",
					!deploymentSaved &&
						"lg:grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)]",
				)}
			>
				<form
					id={formId}
					ref={formRef}
					onSubmit={handleSubmit}
					className={cn(
						"min-w-0 space-y-5 p-5 sm:p-7",
						deploymentSaved && "hidden",
					)}
				>
					<fieldset disabled={locked} className="min-w-0 space-y-5">
						<EventStartPicker
							appId={appId}
							eventConfig={eventConfig}
							selected={{
								boardId: formData.board_id,
								nodeId: formData.node_id,
								pageId: formData.default_page_id,
								boardVersion: formData.board_version,
							}}
							onSelect={selectTarget}
							disabled={locked}
						/>
						{board.isError && (
							<div role="alert" className="text-sm text-destructive">
								{t(
									"flowCouldNotBeLoaded",
									"The selected flow could not be loaded.",
								)}{" "}
								<button
									type="button"
									className="underline"
									onClick={() => void board.refetch()}
								>
									{t("retry", "Retry")}
								</button>
							</div>
						)}
						{formData.board_id &&
							!board.isLoading &&
							!isPageEvent &&
							!node?.start && (
								<p role="alert" className="text-sm text-destructive">
									{t(
										"chooseStartNodeInVersion",
										"Choose a start node from this flow version.",
									)}
								</p>
							)}
						{validTarget && !validTrigger && (
							<p role="alert" className="text-sm text-destructive">
								{t(
									"chooseTriggerInVersion",
									"Choose a trigger supported by this flow version.",
								)}
							</p>
						)}
						{availableTriggers.length > 0 && (
							<section className="space-y-3 border-t pt-5">
								<Label>{t("triggeredBy", "Triggered by")}</Label>
								<div
									className="flex flex-wrap gap-1.5"
									aria-roledescription="choices"
									aria-label={t("eventType", "Event type")}
								>
									{availableTriggers.map((type) => (
										<button
											key={type}
											type="button"
											aria-pressed={formData.event_type === type}
											onClick={() => selectType(type)}
											className={cn(
												"rounded-md border px-3 py-2 text-xs font-medium transition-colors hover:border-primary/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
												formData.event_type === type
													? "border-primary bg-primary/8 text-foreground"
													: "border-border bg-background text-muted-foreground",
											)}
										>
											{triggerLabel(type)}
										</button>
									))}
								</div>
								{formData.event_type === "api" ? (
									<div className="grid grid-cols-[110px_minmax(0,1fr)] gap-3 rounded-lg bg-muted/40 p-3">
										<div className="space-y-1.5">
											<Label htmlFor={`${formId}-method`} className="text-xs">
												{t("method", "Method")}
											</Label>
											<select
												id={`${formId}-method`}
												value={config.method ?? "POST"}
												onChange={(e) =>
													handleInputChange(
														"config",
														convertJsonToUint8Array({
															...config,
															method: e.target.value,
														}) ?? [],
													)
												}
												className="h-10 w-full rounded-md border bg-background px-2 font-mono text-xs"
											>
												{[
													"GET",
													"POST",
													"PUT",
													"PATCH",
													"DELETE",
													"HEAD",
													"OPTIONS",
												].map((method) => (
													<option key={method}>{method}</option>
												))}
											</select>
										</div>
										<div className="space-y-1.5">
											<Label htmlFor={`${formId}-endpoint`} className="text-xs">
												{t("path", "Path")}
											</Label>
											<Input
												id={`${formId}-endpoint`}
												required
												pattern="/.*"
												value={config.path ?? "/webhook"}
												onChange={(e) =>
													handleInputChange(
														"config",
														convertJsonToUint8Array({
															...config,
															path: e.target.value,
														}) ?? [],
													)
												}
												className="h-10 bg-background font-mono text-xs"
											/>
										</div>
									</div>
								) : (
									configEditor && (
										<details
											className="rounded-lg border bg-muted/20"
											open={undefined}
										>
											<summary className="cursor-pointer px-3 py-3 text-xs font-medium">
												{t("triggerSettings", "Trigger settings")}
											</summary>
											<div className="space-y-3 border-t p-3">
												{triggerSections.length > 1 && (
													<select
														aria-label={t(
															"settingsSection",
															"Settings section",
														)}
														value={activeSection}
														onChange={(e) => setConfigSection(e.target.value)}
														className="h-9 w-full rounded-md border bg-background px-2 text-xs"
													>
														{triggerSections.map((section) => (
															<option key={section.id} value={section.id}>
																{section.label}
															</option>
														))}
													</select>
												)}
												{configEditor}
											</div>
										</details>
									)
								)}
								{shouldRequireRoutePath && (
									<div className="space-y-1.5">
										<Label htmlFor={`${formId}-path`}>
											{t("routePath", "Route path")}
										</Label>
										<Input
											id={`${formId}-path`}
											value={formData.path}
											onChange={(e) => {
												setPathError(null);
												handleInputChange("path", e.target.value);
											}}
											className="h-10 font-mono text-sm"
										/>
										{pathError && (
											<p role="alert" className="text-xs text-destructive">
												{pathError}
											</p>
										)}
										{routes.isError && (
											<button
												type="button"
												onClick={() => void routes.refetch()}
												className="text-xs text-destructive underline"
											>
												{t("retryRouteCheck", "Retry route availability check")}
											</button>
										)}
									</div>
								)}
							</section>
						)}
						<section className="grid grid-cols-[minmax(0,1fr)_140px] gap-3 border-t pt-5">
							<div className="space-y-1.5">
								<Label htmlFor={`${formId}-name`}>
									{t("eventName", "Event name")}
								</Label>
								<Input
									id={`${formId}-name`}
									value={formData.name}
									onChange={(e) => handleInputChange("name", e.target.value)}
									placeholder={t("nameThisEvent", "Name this event")}
									required
									className="h-10"
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor={`${formId}-version`}>
									{t("flowVersion", "Flow version")}
								</Label>
								<Select
									value={formData.board_version?.join(".") ?? "latest"}
									disabled={locked || !formData.board_id}
									onValueChange={(value) => {
										handleInputChange(
											"board_version",
											value === "latest"
												? undefined
												: normalizeBoardVersion(value.split(".").map(Number)),
										);
									}}
								>
									<SelectTrigger
										id={`${formId}-version`}
										className="h-10 w-full"
									>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="latest">
											{t("latest", "Latest")}
										</SelectItem>
										{asArray(versions.data).map((version) => (
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
							<p className="col-span-2 text-xs leading-relaxed text-muted-foreground">
								{destination === "device"
									? t(
											"deviceVersionSnapshot",
											"Devices run the flow as it was when you last deployed or updated it.",
										)
									: t(
											"latestOrPinnedFlow",
											"Latest follows the current flow. A pinned version keeps the same snapshot.",
										)}
							</p>
						</section>
						<details className="text-xs">
							<summary className="cursor-pointer font-medium text-muted-foreground">
								{t("addDescription", "Add a description")}
							</summary>
							<Textarea
								aria-label={t("description", "Description")}
								value={formData.description}
								onChange={(e) =>
									handleInputChange("description", e.target.value)
								}
								rows={2}
								className="mt-2"
							/>
						</details>
					</fieldset>
				</form>
				<section
					className="min-w-0 space-y-5 border-t bg-muted/25 p-5 sm:p-7 lg:border-l lg:border-t-0"
					aria-label={t("whereItRuns", "Where it runs")}
				>
					{!deploymentSaved && (
						<>
							<div className="space-y-3">
								<Label>{t("whereItRuns", "Where it runs")}</Label>
								<div
									aria-roledescription="choices"
									aria-label={t(
										"executionDestination",
										"Execution destination",
									)}
									className="grid grid-cols-3 gap-2"
								>
									{destinations.map((target) => {
										const Icon = target.icon;
										const enabled = supportsDestination(target.id);
										return (
											<button
												type="button"
												key={target.id}
												aria-pressed={destination === target.id}
												disabled={locked || !enabled}
												onClick={() => {
													setDestination(target.id);
													setPathError(null);
												}}
												className={cn(
													"flex min-h-24 flex-col items-start gap-2 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45",
													destination === target.id
														? "border-primary bg-primary/8 ring-1 ring-primary"
														: "border-border bg-background hover:border-primary/50",
												)}
											>
												<div className="flex w-full items-center justify-between">
													<Icon
														aria-hidden
														className="size-4 text-muted-foreground"
													/>
													{destination === target.id && (
														<Check
															aria-hidden
															className="size-3.5 text-primary"
														/>
													)}
												</div>
												<span className="text-xs font-semibold">
													{target.title}
												</span>
												<span className="text-[11px] text-muted-foreground">
													{target.detail}
												</span>
											</button>
										);
									})}
								</div>
							</div>
							{!destinationAvailable && (
								<p role="alert" className="text-xs text-destructive">
									{t(
										"chooseSupportedDestination",
										"Choose an available destination for this event and flow.",
									)}
								</p>
							)}
							{serverEventError && (
								<p role="alert" className="text-xs text-destructive">
									{serverEventError}
								</p>
							)}
						</>
					)}
					{destination === "device" && onCreateDevice ? (
						<NewEventDeployment
							overrides={deviceWorkspaceOverrides}
							footerContainer={footerContainer}
							onNavigate={onNavigateDeployment}
							appId={appId}
							draftEvent={buildEventData()}
							disabled={!canSubmit || isSubmitting || checkingOAuth}
							onBusyChange={setDeploymentBusy}
							onSavedChange={setDeploymentSaved}
							onCreate={async () => {
								if (!validate())
									throw new Error(
										t(
											"completeEventDetails",
											"Complete the event details before deploying.",
										),
									);
								return onCreateDevice(buildEventData());
							}}
							onComplete={onDeploymentComplete}
						/>
					) : (
						<div className="rounded-xl border bg-background p-5">
							<div className="flex items-center gap-2 text-sm font-medium">
								<Zap aria-hidden className="size-4 text-primary" />
								{t("whatWillHappen", "What will happen")}
							</div>
							<p className="mt-3 text-sm leading-relaxed">
								{destination === "computer"
									? t(
											"eventRunsOnComputer",
											"This event runs in the desktop app on this computer.",
										)
									: t(
											"eventRunsOnHub",
											"This event runs on the hub, independently of this computer.",
										)}
							</p>
							{formData.name && (
								<p className="mt-3 text-xs text-muted-foreground">
									{formData.name} · {typeLabel}
								</p>
							)}
							<div className="mt-5 flex items-start gap-2 border-t pt-4 text-xs leading-relaxed text-muted-foreground">
								<Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
								<p>
									{destination === "computer"
										? t(
												"keepDesktopRunning",
												"Keep the desktop app running to receive triggers.",
											)
										: t(
												"sourceCredentialsNext",
												"Any required authorization is checked before the event is created.",
											)}
								</p>
							</div>
						</div>
					)}
				</section>
			</div>
			<div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t bg-background px-5 py-4 sm:px-7">
				<div
					className="min-w-0 flex-1 text-xs text-muted-foreground"
					aria-live="polite"
				>
					{submitError ? (
						<span role="alert" className="text-destructive">
							{submitError}
						</span>
					) : deploymentSaved ? (
						t(
							"finishDeploymentHere",
							"Finish deployment here. Your event definition is saved.",
						)
					) : destination === "device" ? (
						t(
							"createDirectlyForDevices",
							"Create the event and deploy its flow to your selected devices.",
						)
					) : (
						t("eventCanBeChangedLater", "You can change these settings later.")
					)}
				</div>
				<Button
					type="button"
					variant="outline"
					onClick={onCancel}
					disabled={busy}
					className="h-10"
				>
					{deploymentSaved ? t("close", "Close") : t("cancel", "Cancel")}
				</Button>
				{destination === "device" && (
					<div
						ref={setFooterContainer}
						className="flex flex-wrap items-center gap-2"
					/>
				)}
				{destination !== "device" && (
					<Button
						type="submit"
						form={formId}
						disabled={
							locked ||
							!canSubmit ||
							(shouldRequireRoutePath && (routes.isLoading || routes.isError))
						}
						className="h-10"
					>
						{locked ? (
							<Loader2 aria-hidden className="size-4 animate-spin" />
						) : (
							<Zap aria-hidden className="size-4" />
						)}
						{locked
							? t("creatingEvent", "Creating…")
							: t("createEvent", "Create event")}
					</Button>
				)}
			</div>
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
		</div>
	);
}
