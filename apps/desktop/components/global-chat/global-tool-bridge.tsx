"use client";

import {
	IExecutionStage,
	ILogLevel,
	type IMetadata,
	IRole,
	Response,
	nowSystemTime,
	useAssistantSurface,
	useBackend,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import { createCopilotStreamParser } from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
import {
	flowPilotModelIdForProvider,
	normalizeAIProvider,
} from "@flow-like/flow-like-ui/components/flowpilot/types";
import { compactLogEvents } from "@flow-like/flow-like-ui/components/flowpilot/utils";
import {
	validateCanvasSettings,
	validateComponents,
} from "@flow-like/flow-like-ui/components/flowpilot/validateComponents";
import type {
	IAttachment,
	IMessage,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import { processChatEvents } from "@flow-like/flow-like-ui/components/interfaces/chat-default/event-processor";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import { createId } from "@paralleldrive/cuid2";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { EVENT_CONFIG, isChatEventType } from "../../lib/event-config";
import {
	type GlobalToolAsk,
	type GlobalToolAskChoice,
	type GlobalToolPromptResolution,
	SUB_STEP_PREFIX,
	useGlobalChatStore,
} from "../../lib/global-chat-store";
import {
	applyStreamEvent,
	createStreamAccumulator,
	orderedSteps,
	readUsageStat,
} from "./copilot-stream-steps";

const GLOBAL_FRONTEND_TOOL_EVENT = "flowpilot://global-tool-request";

/** Diagnostic prefix emitted by the FlowScript merge when a blocked edit would delete board items. */
const DELETION_DIAGNOSTIC_PREFIX = "FlowScript edit would delete ";

type ApprovalKind = "none" | "mutating" | "execute";

interface FrontendToolApproval {
	kind: ApprovalKind;
	title?: string;
	description?: string;
	sessionKey?: string;
}

interface FrontendToolRequest {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approval?: FrontendToolApproval;
}

interface FrontendToolResponse {
	requestId: string;
	approved: boolean;
	result?: unknown;
	error?: string;
}

/** Custom prompt copy for approvals raised mid-tool (e.g. the deletion gate), replacing the request's approval metadata. */
interface DialogOverride {
	title: string;
	description?: string;
}

type DialogState =
	| {
			type: "approval";
			request: FrontendToolRequest;
			override?: DialogOverride;
	  }
	| { type: "ask"; request: FrontendToolRequest };

function argString(args: Record<string, unknown>, key: string): string {
	const value = args[key];
	return typeof value === "string" ? value : "";
}

/** Read an optional boolean tool argument, tolerating the "true"/"false" string forms some backends emit. */
function argBool(
	args: Record<string, unknown>,
	key: string,
): boolean | undefined {
	const value = args[key];
	if (typeof value === "boolean") return value;
	if (value === "true") return true;
	if (value === "false") return false;
	return undefined;
}

/** Parse the `ask_user` arguments into the choice metadata that drives the inline prompt. */
function parseAsk(args: Record<string, unknown>): GlobalToolAsk {
	const rawMode = argString(args, "mode");
	const mode =
		rawMode === "single_choice" || rawMode === "multiple_choice"
			? rawMode
			: "freeform";
	const choices = Array.isArray(args.choices)
		? (args.choices as GlobalToolAskChoice[]).filter(
				(choice) => choice && typeof choice.label === "string",
			)
		: [];
	return {
		mode: mode === "freeform" || choices.length > 0 ? mode : "freeform",
		choices,
		defaultValue: args.default_value ?? args.defaultValue,
		placeholder: argString(args, "placeholder") || undefined,
	};
}

/** Event types that render a UI surface (have a use-interface in the desktop event config). */
const UI_EVENT_TYPES = new Set(
	Object.values(EVENT_CONFIG).flatMap((config) =>
		Object.keys(config.useInterfaces ?? {}),
	),
);

type EventInterfaceKind = "chat" | "page" | "headless";

/** How an event is consumed: inline chat, embeddable UI page, or headless execution. */
function classifyEvent(event: {
	event_type: string;
	default_page_id?: string | null;
}): EventInterfaceKind {
	if (isChatEventType(event.event_type)) return "chat";
	if (event.default_page_id || UI_EVENT_TYPES.has(event.event_type))
		return "page";
	return "headless";
}

/** Record that the current response acted on an app — attached to that message as a chip. */
function referenceApp(appId: string) {
	if (!appId) return;
	useGlobalChatStore.getState().addPendingAppRef(appId);
}

/** The board copilot's flowscript_workspace may be the raw source or a {source,status} JSON blob. */
function extractFlowScriptSource(
	workspace: string | undefined,
): string | undefined {
	const trimmed = workspace?.trim();
	if (!trimmed) return undefined;
	try {
		const parsed = JSON.parse(trimmed);
		if (
			parsed &&
			typeof parsed === "object" &&
			typeof parsed.source === "string"
		) {
			return parsed.source;
		}
	} catch {
		// not JSON — treat the whole string as the flowscript source
	}
	return trimmed;
}

/** Top-level routes that actually exist in the desktop app. */
const KNOWN_ROUTE_PREFIXES = [
	"/chat",
	"/flow",
	"/learn",
	"/library",
	"/settings",
	"/store",
	"/use",
];

const CUID_LIKE = /^[a-z0-9]{20,}$/i;

/** Build the app use-surface route, optionally deep-linking a page route path. */
function buildAppUseRoute(appId: string, pageRoute?: string): string {
	const route = pageRoute?.trim();
	return `/use?id=${appId}${
		route
			? `&route=${encodeURIComponent(route.startsWith("/") ? route : `/${route}`)}`
			: ""
	}`;
}

/**
 * The model sometimes invents router paths (e.g. '/view/<appId>/<pageId>') that don't exist in the
 * desktop app. Accept only known routes verbatim; otherwise recover the app id / page path and send
 * the user to the app's real use surface.
 */
function normalizeExplicitRoute(
	route: string,
	fallbackAppId: string,
): string | undefined {
	const trimmed = route.trim();
	if (!trimmed.startsWith("/")) return undefined;
	if (trimmed === "/") return trimmed;
	if (
		KNOWN_ROUTE_PREFIXES.some(
			(prefix) =>
				trimmed === prefix ||
				trimmed.startsWith(`${prefix}/`) ||
				trimmed.startsWith(`${prefix}?`),
		)
	) {
		return trimmed;
	}
	const segments = (trimmed.split("?")[0] ?? "").split("/").filter(Boolean);
	const appId =
		segments.find((segment) => CUID_LIKE.test(segment)) ?? fallbackAppId;
	if (!appId) return undefined;
	// A trailing human-readable segment is treated as the app-internal page route; trailing ids
	// (page/event ids the use surface can't resolve from a path) are dropped.
	const tail = segments[segments.length - 1];
	const pageRoute =
		tail && tail !== appId && !CUID_LIKE.test(tail) ? tail : undefined;
	return buildAppUseRoute(appId, pageRoute);
}

function routeForView(args: Record<string, unknown>): string {
	const appId = argString(args, "app_id") || argString(args, "appId");
	const pageRoute =
		argString(args, "page_route") || argString(args, "pageRoute");
	const explicit = argString(args, "route");
	if (explicit) {
		const normalized = normalizeExplicitRoute(explicit, appId);
		if (normalized) return normalized;
	}
	const view = argString(args, "view").toLowerCase();
	switch (view) {
		case "home":
			return "/";
		case "apps":
		case "library":
			return "/library";
		case "store":
		case "packages":
			return "/store/explore/apps";
		case "settings":
			return "/settings";
		case "profile":
		case "profiles":
			return "/settings/profiles";
		case "learn":
		case "university":
		case "courses":
			return "/learn";
		case "app":
		case "use":
		case "page":
		case "board":
		case "flow":
			// The app's use surface (its pages/interfaces) lives at /use — /library ignores ?id.
			return appId ? buildAppUseRoute(appId, pageRoute) : "/library";
		default:
			return appId ? buildAppUseRoute(appId, pageRoute) : "/";
	}
}

/**
 * Shared plumbing for nested copilot sub-runs (flowpilot_board / flowpilot_widget): parses the
 * sub-run's stream, accumulates its plan steps, and publishes them into the owning chat message
 * under the request's SUB_STEP_PREFIX block (owner-guarded so stale runs can't leak steps).
 */
function createSubRunStream(requestId: string) {
	const subParser = createCopilotStreamParser();
	const subAcc = createStreamAccumulator();
	const subPrefix = `${SUB_STEP_PREFIX}${requestId}:`;
	// If the sub-run outlives its owning response (bridge timeout, user moved on),
	// stop publishing — otherwise stale "↳" steps leak into the NEXT message.
	const ownerMessageId = useGlobalChatStore.getState().streamingMessage?.id;
	const runIsLive = () => {
		const store = useGlobalChatStore.getState();
		return (
			Boolean(store.streamingMessage) &&
			(!ownerMessageId || store.streamingMessage?.id === ownerMessageId)
		);
	};
	const publishSubSteps = () => {
		if (!runIsLive()) return;
		const store = useGlobalChatStore.getState();
		// Merge by run prefix, replacing this run's block IN PLACE so parallel
		// sub-runs keep a stable order instead of swapping on every chunk.
		const current = store.subPlanSteps;
		const firstIndex = current.findIndex((step) =>
			step.id.startsWith(subPrefix),
		);
		const others = current.filter((step) => !step.id.startsWith(subPrefix));
		const insertAt =
			firstIndex === -1 ? others.length : Math.min(firstIndex, others.length);
		const mine = orderedSteps(subAcc).map((step) => ({
			...step,
			id: `${subPrefix}${step.id}`,
			title: `↳ ${step.title}`,
		}));
		store.setSubPlanSteps([
			...others.slice(0, insertAt),
			...mine,
			...others.slice(insertAt),
		]);
	};
	/** Settle this run's published steps as failed so they aren't finalized green. */
	const failProgressSteps = () => {
		for (const id of subAcc.stepOrder) {
			const step = subAcc.steps.get(id);
			if (step?.status === "progress") {
				subAcc.steps.set(id, { ...step, status: "failed" });
			}
		}
		publishSubSteps();
	};
	return { subParser, subAcc, runIsLive, publishSubSteps, failProgressSteps };
}

function promptForDialog(
	dialog: DialogState,
	respond: (value: GlobalToolPromptResolution, promptId?: string) => void,
) {
	const request = dialog.request;
	// Unique per prompt INSTANCE (one request can spawn several prompts, e.g. tool approval
	// then deletion approval) — binds button clicks to exactly this prompt and remounts the
	// inline card so its local state (answer text, remember) never leaks between prompts.
	const promptId = createId();
	const bound = (value: GlobalToolPromptResolution) => respond(value, promptId);
	if (dialog.type === "ask") {
		return {
			id: promptId,
			kind: "ask" as const,
			toolName: request.toolName,
			title: "FlowPilot needs input",
			description:
				argString(request.arguments, "question") ||
				argString(request.arguments, "prompt") ||
				"Please provide the requested information.",
			ask: parseAsk(request.arguments),
			respond: bound,
		};
	}
	return {
		id: promptId,
		kind: "approval" as const,
		toolName: request.toolName,
		title:
			dialog.override?.title || request.approval?.title || "Approve action",
		description:
			dialog.override?.description ||
			request.approval?.description ||
			`FlowPilot wants to run '${request.toolName}'.`,
		// App-scoped tools (call_app_chat/call_app_event/flowpilot_board) carry the target app id
		// in their arguments — the card resolves it to the app's name + icon.
		appId:
			argString(request.arguments, "app_id") ||
			argString(request.arguments, "appId") ||
			undefined,
		respond: bound,
	};
}

/**
 * Listens for the global FlowPilot assistant's tool requests (a dedicated Tauri event, separate from
 * the board copilot's) and executes them in the app: navigation, app creation, and delegating board
 * work. Mutating/execute tools and ask_user surface an inline prompt card in the chat (via the
 * global-chat store) instead of a modal. The response is returned through the shared
 * `flowpilot_frontend_tool_result` command.
 */
export function GlobalToolBridge() {
	const router = useRouter();
	const pathname = usePathname();
	const backend = useBackend();
	const queryClient = useQueryClient();
	// Auth state gates online (cloud) app creation; keep it in a ref so the stable runTool
	// callback reads the latest value without re-creating on every token refresh.
	const auth = useAuth();
	const authRef = useRef(auth);
	useEffect(() => {
		authRef.current = auth;
	}, [auth]);
	const openOverlay = useGlobalChatStore((s) => s.openOverlay);
	const addInlineAppChat = useGlobalChatStore((s) => s.addInlineAppChat);
	const setToolPrompt = useGlobalChatStore((s) => s.setToolPrompt);

	// The full /chat page already renders the conversation — only dock the overlay elsewhere.
	const pathnameRef = useRef(pathname);
	useEffect(() => {
		pathnameRef.current = pathname;
	}, [pathname]);
	const showConversation = useCallback(() => {
		if (pathnameRef.current !== "/chat") openOverlay();
	}, [openOverlay]);
	const resolverRef = useRef<
		((value: GlobalToolPromptResolution) => void) | null
	>(null);
	// The agent loop executes tool calls in parallel (join_all in Rust), so multiple dialog
	// requests can arrive concurrently — queue them and show one at a time, or the orphaned
	// request would block the agent until its bridge timeout.
	const dialogQueueRef = useRef<
		Array<{
			dialog: DialogState;
			resolve: (value: GlobalToolPromptResolution) => void;
		}>
	>([]);
	const approvedKeysRef = useRef<Set<string>>(new Set());
	const executeRef = useRef<
		(request: FrontendToolRequest) => Promise<FrontendToolResponse>
	>(async (request) => ({ requestId: request.requestId, approved: false }));

	const resolveDialog = useCallback(
		(value: GlobalToolPromptResolution, promptId?: string) => {
			// The next queued prompt renders in the same spot the instant the current one
			// resolves — without this guard a double-click would answer it sight-unseen.
			if (
				promptId &&
				useGlobalChatStore.getState().toolPrompt?.id !== promptId
			) {
				return;
			}
			const resolver = resolverRef.current;
			resolverRef.current = null;
			resolver?.(value);
			const next = dialogQueueRef.current.shift();
			if (next) {
				resolverRef.current = next.resolve;
				setToolPrompt(promptForDialog(next.dialog, resolveDialogRef.current));
			} else {
				setToolPrompt(null);
			}
		},
		[setToolPrompt],
	);
	const resolveDialogRef = useRef(resolveDialog);
	useEffect(() => {
		resolveDialogRef.current = resolveDialog;
	}, [resolveDialog]);

	const openDialog = useCallback(
		(next: DialogState) =>
			new Promise<GlobalToolPromptResolution>((resolve) => {
				if (resolverRef.current) {
					dialogQueueRef.current.push({ dialog: next, resolve });
					return;
				}
				resolverRef.current = resolve;
				setToolPrompt(promptForDialog(next, resolveDialogRef.current));
				// The prompt lives inside the chat surface — make sure one is visible.
				showConversation();
			}),
		[setToolPrompt, showConversation],
	);

	const runTool = useCallback(
		async (request: FrontendToolRequest): Promise<unknown> => {
			const args = request.arguments ?? {};
			// Only apps visible in the CURRENT profile are eligible for app-interface tools.
			const getProfileAppIds = async (): Promise<Set<string>> => {
				try {
					const profile = await backend.userState.getSettingsProfile();
					return new Set(
						(profile?.hub_profile?.apps ?? []).map((entry) => entry.app_id),
					);
				} catch {
					return new Set<string>();
				}
			};
			switch (request.toolName) {
				case "list_apps": {
					// Selection is driven by app + EVENT metadata only (no board loading): each app's
					// active events and their event_type tell the agent which interfaces it can call.
					const profileAppIds = await getProfileAppIds();
					const apps = await backend.appState.getApps();
					// Sort by display name so the output is stable across calls (getApps returns
					// object-store order, i.e. app id) and truncation, if any, is deterministic.
					const visible = apps
						.filter(([app]) => profileAppIds.has(app.id))
						.sort(([, a], [, b]) =>
							(a?.name ?? "").localeCompare(b?.name ?? ""),
						);
					// Safety bound for pathologically large profiles only. Real profiles list in
					// full; when this ever trips it is reported so the agent never concludes an
					// app is absent just because it fell past the cap.
					const MAX_LISTED_APPS = 250;
					const truncated = visible.length > MAX_LISTED_APPS;
					const detailed = await Promise.all(
						visible.slice(0, MAX_LISTED_APPS).map(async ([app, meta]) => {
							let events: Array<{
								id: string;
								name: string;
								description: string;
								event_type: string;
								kind: EventInterfaceKind;
							}> = [];
							try {
								const appEvents = await backend.eventState.getEvents(app.id);
								events = appEvents
									.filter((event) => event.active)
									.map((event) => ({
										id: event.id,
										name: event.name,
										description: event.description,
										event_type: event.event_type,
										// Tells the agent which tool consumes this interface: open_app_chat /
										// call_app_chat ("chat"), open_app_page ("page"), call_app_event ("headless").
										kind: classifyEvent(event),
									}));
							} catch {
								// ignore apps whose events cannot be listed
							}
							return {
								app_id: app.id,
								name: meta?.name ?? app.id,
								description: meta?.description ?? "",
								events,
							};
						}),
					);
					return {
						status: "ok",
						total: visible.length,
						returned: detailed.length,
						...(truncated
							? {
									truncated: true,
									note: `Only the first ${MAX_LISTED_APPS} of ${visible.length} profile apps are listed (sorted by name). If the user references an app not shown, it may fall past this cap rather than not exist.`,
								}
							: {}),
						apps: detailed,
					};
				}
				case "navigate_view": {
					const route = routeForView(args);
					router.push(route);
					// Morph the conversation into the docked overlay so the user keeps chatting
					// while the destination view is shown — decided by the TARGET route (the
					// pre-navigation pathname may still be /chat when this runs).
					if (!route.startsWith("/chat")) openOverlay();
					referenceApp(argString(args, "app_id") || argString(args, "appId"));
					return { status: "ok", route };
				}
				case "describe_app_interface": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					if (!appId || !eventId)
						return {
							status: "error",
							message: "describe_app_interface requires app_id and event_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const events = await backend.eventState.getEvents(appId);
					const event = events.find((candidate) => candidate.id === eventId);
					if (!event)
						return {
							status: "error",
							message: `Event '${eventId}' not found in app '${appId}'.`,
						};
					referenceApp(appId);
					// The event configuration is the user-readable interface contract (chat
					// settings, REST routes, MCP tools, …) — expose it verbatim, size-capped.
					let config = parseUint8ArrayToJson(event.config) ?? {};
					const serialized = JSON.stringify(config);
					if (serialized.length > 12_000) {
						config = { truncated: true, preview: serialized.slice(0, 12_000) };
					}
					return {
						status: "ok",
						event: {
							id: event.id,
							name: event.name,
							description: event.description,
							event_type: event.event_type,
							active: event.active,
							inputs: event.inputs ?? [],
						},
						config,
					};
				}
				case "open_app_chat": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: "open_app_chat requires an app_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					const events = await backend.eventState.getEvents(appId);
					const chatEvent = eventId
						? events.find(
								(event) =>
									event.id === eventId && isChatEventType(event.event_type),
							)
						: events.find(
								(event) => event.active && isChatEventType(event.event_type),
							);
					if (!chatEvent)
						return {
							status: "error",
							message: `App '${appId}' has no chat event.`,
						};
					addInlineAppChat({
						appId,
						eventId: chatEvent.id,
						name: chatEvent.name || appId,
					});
					showConversation();
					referenceApp(appId);
					return {
						status: "ok",
						message: `Opened '${chatEvent.name}' inline — the user can now chat with the app directly.`,
					};
				}
				case "open_app_page": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: "open_app_page requires an app_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					const events = await backend.eventState.getEvents(appId);
					const isPageEvent = (event: (typeof events)[number]) =>
						event.active && classifyEvent(event) === "page";
					const pageEvent = eventId
						? events.find((event) => event.id === eventId && isPageEvent(event))
						: events.find(isPageEvent);
					if (!pageEvent)
						return {
							status: "error",
							message: eventId
								? `Event '${eventId}' in app '${appId}' is not an embeddable UI page. Use call_app_event for headless events or open_app_chat for chats.`
								: `App '${appId}' has no embeddable UI page event.`,
						};
					useGlobalChatStore.getState().addInlineAppPage({
						appId,
						eventId: pageEvent.id,
						name: pageEvent.name || appId,
					});
					showConversation();
					referenceApp(appId);
					return {
						status: "ok",
						message: `Embedded the page '${pageEvent.name}' inline — the user can now use the app's UI directly in the chat.`,
					};
				}
				case "call_app_event": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					if (!appId || !eventId)
						return {
							status: "error",
							message: "call_app_event requires app_id and event_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const events = await backend.eventState.getEvents(appId);
					const event = events.find((candidate) => candidate.id === eventId);
					if (!event)
						return {
							status: "error",
							message: `Event '${eventId}' not found in app '${appId}'.`,
						};
					if (!event.active)
						return {
							status: "error",
							message: `Event '${eventId}' in app '${appId}' is not active.`,
						};
					if (isChatEventType(event.event_type))
						return {
							status: "error",
							message: `Event '${eventId}' is a chat interface — use call_app_chat instead.`,
						};

					const payload =
						args.payload && typeof args.payload === "object"
							? (args.payload as Record<string, unknown>)
							: {};
					const logs: unknown[] = [];
					let runId: string | undefined;
					const metadata = await backend.eventState.executeEvent(
						appId,
						event.id,
						{ id: event.node_id, payload } as Parameters<
							typeof backend.eventState.executeEvent
						>[2],
						true,
						(id) => {
							runId = id;
						},
						(batch) => {
							logs.push(...batch);
						},
					);
					referenceApp(appId);
					return {
						status: "ok",
						app_id: appId,
						event_id: event.id,
						event_type: event.event_type,
						run_id: runId,
						metadata,
						log_count: logs.length,
						logs: compactLogEvents(logs),
					};
				}
				case "create_app": {
					const name = argString(args, "name") || "Untitled app";
					const description = argString(args, "description");
					const meta: IMetadata = {
						name,
						description,
						tags: [],
						use_case: "",
						created_at: nowSystemTime(),
						updated_at: nowSystemTime(),
						preview_media: [],
					};
					// Default to a cloud app when signed in (mirrors the library's create dialog),
					// let the model force local via online:false, but never attempt online without
					// auth — createApp's remote PUT would fail without a token.
					const authenticated = Boolean(authRef.current?.isAuthenticated);
					const online =
						(argBool(args, "online") ?? authenticated) && authenticated;
					const app = await backend.appState.createApp(meta, [], online);
					// Associate the app with the current profile so it surfaces in list_apps
					// (which is profile-scoped) and the user's library, matching the other
					// create-app entry points.
					try {
						const profile = await backend.userState.getSettingsProfile();
						if (profile) {
							await backend.userState.updateProfileApp(
								profile,
								{ app_id: app.id, favorite: false, pinned: false },
								"Upsert",
							);
						}
					} catch (error) {
						console.error(
							"[global-tool-bridge] create_app: profile registration failed",
							error,
						);
					}
					queryClient.invalidateQueries({ queryKey: ["getApps"] });
					queryClient.invalidateQueries({ queryKey: ["getSettingsProfile"] });
					referenceApp(app.id);
					return { status: "ok", app_id: app.id, name, online };
				}
				case "flowpilot_board": {
					const instruction = argString(args, "instruction");
					if (!instruction)
						return {
							status: "error",
							message: "flowpilot_board requires an instruction.",
						};
					// Read-only mode: the board copilot answers a question about the board and
					// makes no edits (no FlowScript, no apply, no approval).
					const readOnly = argString(args, "mode") === "explain";
					const appIdArg =
						argString(args, "app_id") || argString(args, "appId");
					const boardIdArg =
						argString(args, "board_id") || argString(args, "boardId");
					// Prefer the live board surface (open canvas): its applyFlowScript is
					// layer-aware and routes through the board's command pipeline (undo history,
					// refetch, awareness). Detached fetch/apply stays as fallback.
					const boardSurface = useAssistantSurface.getState().boardSurface;
					const liveSurface =
						boardSurface &&
						(!appIdArg || appIdArg === boardSurface.appId) &&
						(!boardIdArg || boardIdArg === boardSurface.boardId)
							? boardSurface
							: null;
					const appId = liveSurface?.appId ?? appIdArg;
					if (!appId)
						return {
							status: "error",
							message: "flowpilot_board requires an app_id.",
						};
					let boardId = liveSurface?.boardId ?? boardIdArg;
					let createdBoard = false;
					if (!liveSurface) {
						if (!boardId) {
							const boards = await backend.boardState.getBoards(appId);
							boardId = boards?.[0]?.id ?? "";
						}
						// New apps have no board yet — create one instead of bouncing the task back
						// to the user.
						if (!boardId) {
							boardId = createId();
							await backend.boardState.upsertBoard(
								appId,
								boardId,
								argString(args, "board_name") || "Main Board",
								instruction.slice(0, 140),
								ILogLevel.Debug,
								IExecutionStage.Dev,
							);
							createdBoard = true;
						}
					}

					console.debug("[global-tool-bridge] flowpilot_board: loading board", {
						appId,
						boardId,
						createdBoard,
					});
					const [board, catalog] = await Promise.all([
						liveSurface?.board
							? Promise.resolve(liveSurface.board)
							: backend.boardState.getBoard(appId, boardId, undefined, true),
						liveSurface?.catalogNodes?.length
							? Promise.resolve(liveSurface.catalogNodes)
							: backend.boardState.getCatalog(appId),
					]);

					// Run the board copilot as a sub-agent, using the global chat's selected model.
					const chat = useGlobalChatStore.getState();
					const modelId = flowPilotModelIdForProvider(
						normalizeAIProvider(chat.provider),
						chat.selectedModelId,
					);
					console.debug(
						"[global-tool-bridge] flowpilot_board: starting nested copilot_chat",
						{ modelId, boardId },
					);

					// Consume the sub-run's stream: it carries the board copilot's inner tool/plan
					// activity (surfaced live in the parent chat) AND — for the codex/claude-code
					// backends — the ONLY copy of the FlowScript workspace (their final response
					// hard-codes flowscript_workspace to None).
					const {
						subParser,
						subAcc,
						runIsLive,
						publishSubSteps,
						failProgressSteps,
					} = createSubRunStream(request.requestId);
					let streamedWorkspace: { source?: string; status?: string } = {};
					const onToken = (chunk: string) => {
						let stepsChanged = false;
						for (const event of subParser.push(chunk)) {
							if (event.type === "flowscript_workspace") {
								const payload =
									(event.data as { source?: unknown; status?: unknown }) ?? {};
								const source =
									typeof payload.source === "string"
										? payload.source
										: typeof event.data === "string"
											? event.data
											: undefined;
								streamedWorkspace = {
									source: source ?? streamedWorkspace.source,
									status:
										typeof payload.status === "string"
											? payload.status
											: streamedWorkspace.status,
								};
								// Mirror the workspace live into the chat's FlowScript panel, like
								// the board FlowPilot does.
								if (streamedWorkspace.source && runIsLive()) {
									useGlobalChatStore.getState().setFlowscriptWorkspace({
										source: streamedWorkspace.source,
										status: streamedWorkspace.status,
									});
								}
								continue;
							}
							// Roll the sub-agent's own token usage into the owning message's stats.
							if (event.type === "usage_stat") {
								const stat = readUsageStat(event.data);
								if (stat)
									useGlobalChatStore.getState().addSubUsageStats([stat]);
								continue;
							}
							if (event.type === "text") continue;
							applyStreamEvent(subAcc, event);
							stepsChanged = true;
						}
						if (stepsChanged) publishSubSteps();
					};

					let response: Awaited<
						ReturnType<typeof backend.boardState.copilot_chat>
					>;
					let appliedCommands = 0;
					// null = no apply ran; true = applied through the live surface callbacks.
					let appliedViaLive: boolean | null = null;
					let blockedDeletion = false;
					let deletionApproved = false;
					let diagnostics: string[] = [];
					let source: string | undefined;
					let workspaceStatus: string | undefined;
					// Attached run/log context (e.g. the user inspecting a failed run) lets the board
					// copilot pull the run's logs via its query tools.
					const surfaceRunContext = liveSurface?.runContext
						? {
								run_id: liveSurface.runContext.run_id,
								app_id: liveSurface.runContext.app_id,
								board_id: liveSurface.runContext.board_id,
							}
						: undefined;
					// Weaker models tend to loop on analysis tools and end without ever submitting
					// an edit — make the success criterion explicit in the sub-agent's instruction.
					// In read-only mode the criterion is inverted: answer, and change nothing.
					const boardInstruction = readOnly
						? `${instruction}

Answer the user's question about this board clearly and concisely, grounded in its actual nodes and connections. Do NOT modify the board — make no edits and submit no FlowScript.`
						: `${instruction}

Execute the change NOW in this run: draft the complete FlowScript workspace for this request and submit it via your edit tools. Do not stop after analysis and do not merely describe a plan — the run only counts as successful once a FlowScript workspace has been submitted.`;
					try {
						response = await backend.boardState.copilot_chat(
							"Board",
							board,
							catalog,
							liveSurface?.selectedNodeIds ?? [],
							null,
							[],
							boardInstruction,
							[],
							undefined /* images */,
							onToken,
							modelId,
							undefined /* token */,
							surfaceRunContext,
							undefined /* actionContext */,
							true /* nested: isolate from the pending parent session */,
							readOnly /* explain mode: answer, don't edit */,
						);
						console.debug(
							"[global-tool-bridge] flowpilot_board: nested copilot_chat finished",
							{ commands: response.commands?.length ?? 0, readOnly },
						);

						// Read-only explain: nothing is applied. Surface the board (navigating only
						// when it isn't already the live canvas) and relay the copilot's answer.
						if (readOnly) {
							if (!liveSurface) {
								router.push(`/flow?id=${boardId}&app=${appId}`);
								openOverlay();
							}
							referenceApp(appId);
							return {
								status: "ok",
								mode: "explain",
								message: response.message,
								...(createdBoard ? { created_board_id: boardId } : {}),
							};
						}

						// Apply the resulting FlowScript ADDITIVELY: allowDeletions=false, so the
						// backend merge blocks destructive edits instead of silently deleting existing
						// board work. Prefer the final response's workspace (bits/copilot backends),
						// fall back to the streamed one (codex/claude-code only stream it).
						source =
							extractFlowScriptSource(response.flowscript_workspace) ??
							extractFlowScriptSource(streamedWorkspace.source);
						workspaceStatus = streamedWorkspace.status;
						const applicable =
							workspaceStatus !== "validation_errors" &&
							workspaceStatus !== "no_changes";
						if (source && applicable) {
							const flowscript = source;
							const applyOnce = async (allowDeletions: boolean) => {
								// The sub-run can outlast the open board (closed/navigated mid-run):
								// re-resolve the surface at apply time; a stale captured surface would
								// apply through dead closures (lost awareness ping, no user feedback).
								const surfaceNow = useAssistantSurface.getState().boardSurface;
								const applyLive =
									liveSurface &&
									surfaceNow &&
									surfaceNow.appId === appId &&
									surfaceNow.boardId === boardId
										? surfaceNow
										: null;
								if (applyLive) {
									// Live path: the surface callback already handles layer targeting,
									// undo history and board refetch — no query invalidation needed.
									const applyResult = await applyLive.applyFlowScript(
										flowscript,
										{ allowDeletions, suppressBlockedToast: true },
									);
									appliedCommands = applyResult?.commands?.length ?? 0;
									diagnostics = applyResult?.diagnostics ?? [];
								} else {
									const applyResult = await backend.boardState.applyFlowScript(
										appId,
										boardId,
										flowscript,
										undefined,
										catalog,
										allowDeletions,
									);
									appliedCommands = applyResult.commands?.length ?? 0;
									diagnostics = applyResult.diagnostics ?? [];
								}
								blockedDeletion =
									diagnostics[0]?.startsWith(DELETION_DIAGNOSTIC_PREFIX) ??
									false;
								return applyLive !== null;
							};
							appliedViaLive = await applyOnce(false);
							if (blockedDeletion) {
								// Destructive edits are NEVER auto-applied: ask the user inline and
								// only re-apply with deletions allowed after an explicit approve.
								const diagnostic = diagnostics[0] ?? "";
								const outcome = await openDialog({
									type: "approval",
									request,
									override: {
										title: "Approve deletion",
										description: `${
											diagnostic.length > 200
												? `${diagnostic.slice(0, 200)}…`
												: diagnostic
										} Re-apply allowing these deletions?`,
									},
								});
								if (outcome && "approved" in outcome && outcome.approved) {
									deletionApproved = true;
									appliedViaLive = await applyOnce(true);
								}
							}
							// Refresh only the board-related queries so an already-open canvas shows
							// the new nodes without a manual reload (and without a global refetch storm).
							if (appliedViaLive !== true && appliedCommands > 0) {
								void queryClient.invalidateQueries({
									predicate: (query) => {
										const key = query.queryKey;
										return (
											Array.isArray(key) &&
											typeof key[0] === "string" &&
											["getBoard", "getBoards", "getCatalog"].includes(
												key[0],
											) &&
											key.includes(appId)
										);
									},
								});
							}
						}
					} catch (error) {
						failProgressSteps();
						throw error;
					}

					const applyFailed =
						appliedCommands === 0 && diagnostics.length > 0 && !blockedDeletion;
					// Publish the final workspace too — bits/copilot backends only carry it in the
					// final response, not the stream.
					if (source && runIsLive()) {
						useGlobalChatStore.getState().setFlowscriptWorkspace({
							source,
							status: workspaceStatus,
						});
					}
					// Close the run with a summary step; the FlowScript itself is expandable.
					if (source) {
						subAcc.stepOrder.push("flowscript");
						subAcc.steps.set("flowscript", {
							id: "flowscript",
							title: "FlowScript",
							description:
								workspaceStatus === "validation_errors"
									? "Not applied — validation errors"
									: workspaceStatus === "no_changes"
										? "No changes needed"
										: applyFailed
											? `Not applied — ${diagnostics[0]?.slice(0, 120) ?? "apply failed"}`
											: `${appliedCommands} command${appliedCommands === 1 ? "" : "s"} applied${blockedDeletion ? " (deletions blocked)" : deletionApproved ? " (deletions approved)" : ""}`,
							status:
								workspaceStatus === "validation_errors" || applyFailed
									? "failed"
									: "done",
							reasoning: `\`\`\`\n${source.slice(0, 6000)}\n\`\`\``,
							timestamp: Date.now(),
						});
						publishSubSteps();
					}

					// Show the board and keep chatting in the docked overlay (target is /flow, so
					// the conversation always needs the dock). With a live surface the user is
					// already looking at the board — no navigation. If the board was closed
					// mid-run (apply fell back to detached), navigate so the change is visible.
					if (!liveSurface || appliedViaLive === false) {
						router.push(`/flow?id=${boardId}&app=${appId}`);
						openOverlay();
					}
					referenceApp(appId);

					return {
						status: "ok",
						message: response.message,
						applied_commands: appliedCommands,
						...(createdBoard ? { created_board_id: boardId } : {}),
						...(!source
							? {
									flowscript_status: "no_flowscript",
									note: "IMPORTANT: the board copilot ended WITHOUT submitting a FlowScript — the board was NOT modified and contains no new nodes. Do not tell the user the workflow was built. Retry flowpilot_board once with a more explicit, step-by-step instruction, or tell the user honestly that the edit failed.",
								}
							: {}),
						...(workspaceStatus === "validation_errors"
							? {
									flowscript_status: "validation_errors",
									note: "The board copilot produced a FlowScript draft with validation errors — nothing was applied. Consider retrying with a clearer instruction.",
								}
							: {}),
						...(applyFailed && workspaceStatus !== "validation_errors"
							? {
									flowscript_status: "apply_failed",
									diagnostics: diagnostics.slice(0, 5),
									note: "The FlowScript draft could not be applied to the board — report the diagnostics honestly and consider retrying with a clearer instruction.",
								}
							: {}),
						...(deletionApproved ? { deletion_approved: true } : {}),
						...(blockedDeletion
							? {
									blocked_deletion: true,
									note: "Some edits would delete existing board items and were blocked. The user was asked inline and declined — deletions remain blocked. Do not re-apply them.",
								}
							: {}),
					};
				}
				case "flowpilot_widget": {
					const instruction = argString(args, "instruction");
					if (!instruction)
						return {
							status: "error",
							message: "flowpilot_widget requires an instruction.",
						};
					// The widget copilot only targets a LIVE builder surface — there is no
					// detached path: generated components are staged for review and applied
					// through the surface's own pipeline.
					const widgetSurface = useAssistantSurface.getState().widgetSurface;
					if (!widgetSurface)
						return {
							status: "error",
							message:
								"No widget/page surface is open. Ask the user to open the page/widget builder first (navigate_view or open the builder), then retry.",
						};

					// Run the widget copilot as a sub-agent, using the global chat's selected model.
					const chat = useGlobalChatStore.getState();
					const modelId = flowPilotModelIdForProvider(
						normalizeAIProvider(chat.provider),
						chat.selectedModelId,
					);

					const {
						subParser,
						subAcc,
						runIsLive,
						publishSubSteps,
						failProgressSteps,
					} = createSubRunStream(request.requestId);
					// `components` frames stream in batches (codex/claude-code); the final
					// response's components (bits/copilot backends) supersede them — mirroring
					// the board FlowPilot's handling.
					const streamedComponents: SurfaceComponent[] = [];
					const warnings: string[] = [];
					let canvasSettings: CanvasSettings | undefined;
					const collectComponents = (raw: unknown): SurfaceComponent[] => {
						if (!Array.isArray(raw) || raw.length === 0) return [];
						const result = validateComponents(raw as SurfaceComponent[]);
						if (result.warnings.length > 0) warnings.push(...result.warnings);
						return result.components;
					};
					const onToken = (chunk: string) => {
						let stepsChanged = false;
						for (const event of subParser.push(chunk)) {
							if (event.type === "components") {
								streamedComponents.push(...collectComponents(event.data));
								continue;
							}
							if (event.type === "canvas_settings") {
								canvasSettings =
									validateCanvasSettings(event.data) ?? canvasSettings;
								continue;
							}
							if (event.type === "usage_stat") {
								const stat = readUsageStat(event.data);
								if (stat)
									useGlobalChatStore.getState().addSubUsageStats([stat]);
								continue;
							}
							if (event.type === "text") continue;
							applyStreamEvent(subAcc, event);
							stepsChanged = true;
						}
						if (stepsChanged) publishSubSteps();
					};

					let response: Awaited<
						ReturnType<typeof backend.boardState.copilot_chat>
					>;
					try {
						response = await backend.boardState.copilot_chat(
							"Frontend",
							null,
							undefined,
							[],
							widgetSurface.currentComponents,
							widgetSurface.selectedComponentIds,
							instruction,
							[],
							undefined /* images */,
							onToken,
							modelId,
							undefined /* token */,
							undefined /* runContext */,
							undefined /* actionContext */,
							true /* nested: isolate from the pending parent session */,
						);
					} catch (error) {
						failProgressSteps();
						throw error;
					}

					const finalComponents = collectComponents(response.components);
					const components =
						finalComponents.length > 0 ? finalComponents : streamedComponents;
					canvasSettings =
						validateCanvasSettings(response.canvas_settings) ?? canvasSettings;

					let staged = false;
					if (components.length > 0) {
						// Stage for the user's inline review — NEVER auto-applied to the canvas.
						// Bound to the generating builder so Apply can't target another surface.
						if (runIsLive()) {
							useGlobalChatStore.getState().setPendingComponents({
								components,
								canvasSettings,
								warnings: warnings.length > 0 ? warnings : undefined,
								surfaceId: widgetSurface.surfaceId,
								appId: widgetSurface.appId,
							});
							staged = true;
						}
						// Close the run with a summary step, like the board case's FlowScript step.
						subAcc.stepOrder.push("components");
						subAcc.steps.set("components", {
							id: "components",
							title: "UI components",
							description: `${components.length} component${components.length === 1 ? "" : "s"} ready for review`,
							status: "done",
							timestamp: Date.now(),
						});
						publishSubSteps();
					}

					if (widgetSurface.appId) referenceApp(widgetSurface.appId);

					if (components.length === 0)
						return {
							status: "ok",
							message: response.message,
							component_count: 0,
							note: "IMPORTANT: the widget copilot ended WITHOUT generating any UI components — the surface was NOT changed. Do not tell the user the UI was built; retry once with a clearer instruction or tell the user honestly that nothing was generated.",
						};
					if (!staged)
						return {
							status: "ok",
							message: response.message,
							component_count: components.length,
							staged: false,
							note: "IMPORTANT: components were generated but the conversation moved on before they could be staged — they were DISCARDED and there is no review card. Do not tell the user to review anything; offer to regenerate.",
						};
					return {
						status: "ok",
						message: response.message,
						component_count: components.length,
						staged: true,
						note: "Components are pending user review in the chat — they are NOT applied yet. Tell the user to review and apply them.",
					};
				}
				case "call_app_chat": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const message =
						argString(args, "message") || argString(args, "prompt");
					if (!appId)
						return {
							status: "error",
							message: "call_app_chat requires an app_id.",
						};
					if (!message)
						return {
							status: "error",
							message: "call_app_chat requires a message.",
						};

					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};

					// Call the specific chat event the agent selected from list_apps metadata
					// (falling back to the app's first chat event) — events, not boards.
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					const events = await backend.eventState.getEvents(appId);
					const chatEvent = eventId
						? events.find(
								(event) =>
									event.id === eventId && isChatEventType(event.event_type),
							)
						: events.find(
								(event) => event.active && isChatEventType(event.event_type),
							);
					if (!chatEvent)
						return {
							status: "error",
							message: eventId
								? `App '${appId}' has no chat event '${eventId}'.`
								: `App '${appId}' has no chat event.`,
						};

					// Forward the current turn's user attachments into the app chat, so "check this
					// file with app X" reaches the app agent instead of silently dropping the files.
					const forwardedAttachments = (() => {
						const msgs = useGlobalChatStore.getState().messages;
						for (let i = msgs.length - 1; i >= 0; i--) {
							if (msgs[i]?.inner.role === IRole.User)
								return msgs[i].files ?? [];
						}
						return [];
					})();

					// Invoke the app's chat event through the SAME pipeline the simple chat uses
					// (executeEvent + processChatEvents), so it runs with full app-chat behavior.
					const chatId = createId();
					const runPayload = {
						id: chatEvent.node_id,
						payload: {
							chat_id: chatId,
							messages: [{ role: "user", content: message }],
							local_session: {},
							global_session: {},
							actions: [],
							tools: [],
							attachments: forwardedAttachments,
						},
					};

					const responseMessage: IMessage = {
						id: createId(),
						appId,
						sessionId: chatId,
						inner: { role: IRole.Assistant, content: "" },
						files: [],
						tools: [],
						actions: [],
						timestamp: Date.now(),
					};
					let intermediate = Response.default();
					const attachments = new Map<string, IAttachment>();

					// Surface the app chat's own plan steps as nested "↳" sub-steps in the
					// global chat, the same way flowpilot_board/flowpilot_widget fold their
					// sub-run activity into the owning message. Without this the user only
					// sees the outer call_app_chat step, never the app agent's inner work.
					const { subAcc, publishSubSteps, failProgressSteps } =
						createSubRunStream(request.requestId);
					const syncSubSteps = () => {
						const steps = responseMessage.plan_steps;
						if (!steps?.length) return;
						for (const step of steps) {
							if (!subAcc.steps.has(step.id)) subAcc.stepOrder.push(step.id);
							subAcc.steps.set(step.id, step);
						}
						publishSubSteps();
					};

					try {
						await backend.eventState.executeEvent(
							appId,
							chatEvent.id,
							runPayload as Parameters<
								typeof backend.eventState.executeEvent
							>[2],
							false,
							undefined,
							(batch) => {
								const result = processChatEvents(batch, {
									intermediateResponse: intermediate,
									responseMessage,
									attachments,
									tmpLocalState: null,
									tmpGlobalState: null,
									done: false,
									appId,
									eventId: chatEvent.id,
									sessionId: chatId,
								});
								intermediate = result.intermediateResponse;
								syncSubSteps();
								// Surface app-chat dialogs (single/multiple choice, form) inline so the
								// user can answer — respond_to_interaction unblocks the app workflow
								// while this call_app_chat tool call is still awaiting its result.
								if (result.interactions?.length) {
									useGlobalChatStore
										.getState()
										.addInteractions(result.interactions);
								}
							},
						);
					} catch (error) {
						failProgressSteps();
						throw error;
					}

					const text =
						typeof responseMessage.inner.content === "string"
							? responseMessage.inner.content
							: "";

					// Fold any attachments the app chat produced into the owning message so they
					// render, and report them back to the assistant so it can relay/reference them.
					const files = responseMessage.files ?? [];
					if (files.length > 0) {
						useGlobalChatStore.getState().addSubAttachments(files);
					}
					const attachmentSummaries = files.map((file) =>
						typeof file === "string"
							? { url: file }
							: { url: file.url, name: file.name, type: file.type },
					);

					// Surface the called app's own model usage (its chat_usage_stat events land on
					// responseMessage.usage_stats) in the global chat's stats badge.
					if (responseMessage.usage_stats?.length) {
						useGlobalChatStore
							.getState()
							.addSubUsageStats(responseMessage.usage_stats);
					}

					referenceApp(appId);
					return {
						status: "ok",
						app_id: appId,
						response: text || "(the app chat returned no text)",
						attachments:
							attachmentSummaries.length > 0 ? attachmentSummaries : undefined,
					};
				}
				default:
					throw new Error(`Unsupported global tool '${request.toolName}'.`);
			}
		},
		[
			backend.appState,
			backend.boardState,
			backend.eventState,
			backend.userState,
			router,
			queryClient,
			openOverlay,
			showConversation,
			addInlineAppChat,
			openDialog,
		],
	);

	const execute = useCallback(
		async (request: FrontendToolRequest): Promise<FrontendToolResponse> => {
			try {
				if (request.toolName === "ask_user") {
					const resolution = await openDialog({ type: "ask", request });
					if (!resolution || !("answer" in resolution))
						return {
							requestId: request.requestId,
							approved: false,
							error: "User dismissed the question.",
						};
					return {
						requestId: request.requestId,
						approved: true,
						result: { status: "ok", answer: resolution.answer },
					};
				}

				const approval = request.approval;
				const sessionKey =
					approval?.sessionKey ||
					`${request.toolName}:${approval?.kind ?? "none"}`;
				const needsApproval =
					approval?.kind === "mutating" || approval?.kind === "execute";

				if (needsApproval && !approvedKeysRef.current.has(sessionKey)) {
					const outcome = await openDialog({ type: "approval", request });
					if (!outcome || !("approved" in outcome) || !outcome.approved) {
						return {
							requestId: request.requestId,
							approved: false,
							error: "User denied the request.",
						};
					}
					if (outcome.remember) approvedKeysRef.current.add(sessionKey);
				}

				const result = await runTool(request);
				return { requestId: request.requestId, approved: true, result };
			} catch (error) {
				// approved:true + error => the bridge reports status:"error" (not a user denial).
				return {
					requestId: request.requestId,
					approved: true,
					error: error instanceof Error ? error.message : String(error),
				};
			}
		},
		[openDialog, runTool],
	);

	useEffect(() => {
		executeRef.current = execute;
	}, [execute]);

	useEffect(() => {
		if (typeof window === "undefined") return;
		let disposed = false;
		let unlisten: (() => void) | undefined;

		void (async () => {
			try {
				const [{ listen }, { invoke }] = await Promise.all([
					import("@tauri-apps/api/event"),
					import("@tauri-apps/api/core"),
				]);
				const stop = await listen<FrontendToolRequest>(
					GLOBAL_FRONTEND_TOOL_EVENT,
					async (event) => {
						const request = event.payload;
						if (!request?.requestId || !request.toolName) return;
						console.debug(
							"[global-tool-bridge] request",
							request.toolName,
							request.requestId,
						);
						const response = await executeRef.current(request);
						console.debug(
							"[global-tool-bridge] responding",
							request.toolName,
							request.requestId,
							{ approved: response.approved, error: response.error },
						);
						await invoke("flowpilot_frontend_tool_result", { response });
					},
				);
				if (disposed) stop();
				else unlisten = stop;
			} catch {
				// Not running under Tauri (e.g. web build) — the global tool bridge is desktop-only.
			}
		})();

		return () => {
			disposed = true;
			unlisten?.();
		};
	}, []);

	// The pending prompt is rendered inline by the chat surfaces (InlineToolPrompt) via the store.
	return null;
}
