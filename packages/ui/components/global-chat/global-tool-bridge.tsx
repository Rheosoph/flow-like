"use client";

import { createId } from "@paralleldrive/cuid2";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { useFrontendRuntimeToolExecutor } from "../../hooks/use-frontend-runtime-tool-executor";
import {
	type IEvent,
	IEventExecutionMode,
	IExecutionStage,
	ILogLevel,
	type IMetadata,
	IRole,
	Response,
	nowSystemTime,
	useAssistantSurface,
	useBackend,
	useQueryClient,
} from "../../index";
import { captureInlineAppPageSnapshots } from "../../lib/app-page-snapshot";
import { shouldSkipUnavailableCreateTableApproval } from "../../lib/database-capability-session";
import { getErrorMessage } from "../../lib/error-message";
import { EVENT_CONFIG, isChatEventType } from "../../lib/event-config";
import { flowPilotDebugLog } from "../../lib/flowpilot-debug";
import type { FlowIrCommitToken } from "../../lib/schema/copilot";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../lib/uint8";
import type {
	IApplyFlowIrCommitResponse,
	IBoardState,
} from "../../state/backend-state/board-state";
import {
	type IAgentDebugEvent,
	agentDebugPreview,
	agentGenerationReviewDispositionEvent,
	createAgentDebugStreamRecorder,
	nestedAgentRunEvent,
} from "../../state/global-chat/agent-debug-report";
import {
	applyStreamEvent,
	createStreamAccumulator,
	orderedSteps,
	readUsageStat,
} from "../../state/global-chat/copilot-stream-steps";
import {
	type GlobalToolAsk,
	type GlobalToolAskChoice,
	type GlobalToolPrompt,
	type GlobalToolPromptResolution,
	SUB_STEP_PREFIX,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { registerGlobalChatToolExecutor } from "../../state/global-chat/global-chat-tool-registry";
import { foldA2UIServerMessage } from "../a2ui/fold-surfaces";
import type {
	A2UIServerMessage,
	CanvasSettings,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import {
	BoardEditRecoveryStore,
	BoardZeroProgressRetryGuard,
	CreatedArtifactJournal,
	type FlowScriptBaselineFingerprint,
	FrontendRequestExecutionFence,
	type FrontendRequestExecutionLease,
	assessFlowScriptCorrectionReadback,
	assessFlowScriptReadback,
	boardEditCoordinator,
	boardEditInterruptionResult,
	boardEditLockKey,
	boardEditRecoveryKey,
	flowScriptSnapshotChanged,
	flowScriptSnapshotFingerprint,
	hasActiveFrontendRequestOwnership,
	isCreatedAppBuildTargetMismatch,
	resolveFrontendToolExecutionDeadline,
	retainedFlowScriptRecoveryInstruction,
	retainedFlowScriptReferenceInstruction,
	retryCreatedAppReadiness,
	safeFlowScriptPlanReasoning,
} from "../flowpilot/board-edit-guard";
import { composeDelegatedRawUserPrompt } from "../flowpilot/copilot-request-context";
import {
	type FlowScriptWorkspaceCandidate,
	isFlowScriptWorkspaceApplicable,
	isPartialFlowScriptWorkspace,
	parseFlowScriptWorkspaceCandidate,
	rememberFlowScriptWorkspaceCandidate,
	resolveFinalFlowScriptWorkspaceCandidate,
	selectBestRecoverableFlowScriptCandidate,
	shouldPromoteFlowScriptWorkspaceEvents,
} from "../flowpilot/flowscript-workspace-candidates";
import {
	flowPilotModelIdForProvider,
	normalizeAIProvider,
} from "../flowpilot/types";
import { compactLogEvents } from "../flowpilot/utils";
import {
	validateCanvasSettings,
	validateComponents,
} from "../flowpilot/validateComponents";
import type { IAttachment, IMessage } from "../interfaces/chat-default/chat-db";
import { processChatEvents } from "../interfaces/chat-default/event-processor";
import {
	type RunnableWorkflowEventEntry,
	WORKFLOW_EVENT_ENTRY_NODE_NAMES,
	collectRunnableWorkflowEventEntries,
	isRunnableWorkflowEventEntry,
} from "./workflow-event-entries";

const GLOBAL_FRONTEND_TOOL_EVENT = "flowpilot://global-tool-request";
const GLOBAL_FRONTEND_TOOL_CANCEL_EVENT = "flowpilot://frontend-tool-cancel";
const GLOBAL_FRONTEND_TOOL_LIFECYCLE_EVENT =
	"flowpilot://frontend-tool-lifecycle";

/** Diagnostic prefix emitted by the FlowScript merge when a blocked edit would delete board items. */
const DELETION_DIAGNOSTIC_PREFIX = "FlowScript edit would delete ";
const FLOW_IR_DISMISS_RETRY_DELAYS_MS = [0, 250, 1_000, 3_000] as const;
const FLOWSCRIPT_DRAFT_PREVIEW_INTERVAL_MS = 80;
const activeFlowIrDismissals = new Map<string, Promise<boolean>>();

function dismissFlowIrCommitWithRetry(
	boardState: IBoardState,
	token: FlowIrCommitToken,
): Promise<boolean> {
	const key = `${token.board_id}:${token.draft_id}:${token.revision}:${token.claim_id}`;
	const existing = activeFlowIrDismissals.get(key);
	if (existing) return existing;
	const dismissal = (async () => {
		const resolveDisposition = boardState.flowIrCommitDisposition;
		if (!resolveDisposition) return false;
		for (const delayMs of FLOW_IR_DISMISS_RETRY_DELAYS_MS) {
			if (delayMs > 0) {
				await new Promise<void>((resolveDelay) =>
					setTimeout(resolveDelay, delayMs),
				);
			}
			try {
				const result = await resolveDisposition.call(
					boardState,
					token,
					"dismissed",
				);
				if (
					result.status === "dismissed" ||
					result.code === "IR_COMMIT_TOKEN_INVALID"
				) {
					return true;
				}
			} catch (error) {
				console.error(
					"[global-tool-bridge] compiled workflow review dismissal attempt failed",
					error,
				);
			}
		}
		return false;
	})().finally(() => activeFlowIrDismissals.delete(key));
	activeFlowIrDismissals.set(key, dismissal);
	return dismissal;
}

type ApprovalKind = "none" | "mutating" | "execute";

interface FrontendToolApproval {
	kind: ApprovalKind;
	title?: string;
	description?: string;
	sessionKey?: string;
}

export interface FrontendToolRequest {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approval?: FrontendToolApproval;
	/** Backend dispatch/deadline metadata used to settle before its receiver disappears. */
	dispatchedAtMs?: number;
	deadlineAtMs?: number;
	dispatched_at_ms?: number;
	deadline_at_ms?: number;
	timeoutMs?: number;
	parentRequestId?: string;
	/** Nested tools inherit their parent request so cancellation/diagnostics remain one tree. */
	context?: {
		parentRequestId?: string;
		parent_request_id?: string;
		conversationId?: string;
		conversation_id?: string;
		sourceUserPrompt?: string;
		source_user_prompt?: string;
	};
}

export interface FrontendToolResponse {
	requestId: string;
	approved: boolean;
	result?: unknown;
	error?: string;
}

/** Custom prompt copy for approvals raised mid-tool (e.g. the deletion gate), replacing the request's approval metadata. */
interface DialogOverride {
	title: string;
	description?: string;
	/** Marks a gate that must never be answered without the user (auto mode, batch approvers). */
	destructive?: boolean;
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

function parentRequestId(request: FrontendToolRequest) {
	return (
		request.parentRequestId ??
		request.context?.parentRequestId ??
		request.context?.parent_request_id
	);
}

function sourceUserPrompt(request: FrontendToolRequest): string | undefined {
	const owned =
		request.context?.sourceUserPrompt ?? request.context?.source_user_prompt;
	if (owned?.trim()) return owned.trim();
	const messages = useGlobalChatStore.getState().messages;
	for (let index = messages.length - 1; index >= 0; index -= 1) {
		const message = messages[index];
		const content = message?.inner.content;
		if (
			message?.inner.role === IRole.User &&
			typeof content === "string" &&
			content.trim()
		) {
			return content.trim();
		}
	}
	return undefined;
}

/**
 * Conversation id that scopes a delegated run's retained-draft identity. Prefer the id carried by
 * the owning request; fall back to the active conversation (the same source `sourceUserPrompt`
 * falls back to) so nested and follow-up repair runs of one conversation share identity while
 * other conversations never can.
 */
function conversationScopeId(request: FrontendToolRequest): string | undefined {
	const owned =
		request.context?.conversationId ?? request.context?.conversation_id;
	if (owned?.trim()) return owned.trim();
	const active = useGlobalChatStore.getState().activeConversationId;
	return active?.trim() ? active : undefined;
}

function requestDeadline(request: FrontendToolRequest) {
	return resolveFrontendToolExecutionDeadline({
		toolName: request.toolName,
		backendDeadlineAtMs: request.deadlineAtMs ?? request.deadline_at_ms,
	});
}

/** Turn a page name/route into a leading-slash URL slug (e.g. "My Page" -> "/my-page"). */
function slugifyRoute(value: string): string {
	const slug = value
		.trim()
		.toLowerCase()
		.replace(/^\/+/, "")
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "");
	return `/${slug || "page"}`;
}

interface InlineWidgetInstance {
	instanceId: string;
	copilotWidgetId: string;
	inlineDef: Record<string, unknown>;
	/** The live widgetInstance component object, so the caller can remap/strip it in place. */
	component: Record<string, unknown>;
}

/**
 * Collect the `widgetInstance` components that carry an inline widget definition (the copilot embeds
 * a reusable widget's tree there). The caller persists each unique widget once and wires the page's
 * instances to it via `widgetRefs`.
 */
function collectInlineWidgets(
	components: SurfaceComponent[],
): InlineWidgetInstance[] {
	const out: InlineWidgetInstance[] = [];
	for (const comp of components) {
		const inner = comp.component as unknown as
			| Record<string, unknown>
			| undefined;
		if (!inner || inner.type !== "widgetInstance") continue;
		const inlineDef = inner.inlineWidgetDef;
		if (!inlineDef || typeof inlineDef !== "object") continue;
		const instanceId =
			(typeof inner.instanceId === "string" && inner.instanceId) || comp.id;
		const copilotWidgetId =
			(typeof inner.widgetId === "string" && inner.widgetId) || instanceId;
		out.push({
			instanceId,
			copilotWidgetId,
			inlineDef: inlineDef as Record<string, unknown>,
			component: inner,
		});
	}
	return out;
}

/**
 * Ensure a component tree has a root with id "root" (the page/widget renderers look up "root"
 * verbatim). If the copilot rooted the tree under a different id (e.g. "page-root"), rename that
 * top-level (unreferenced) component to "root". No-op when a "root" already exists.
 */
function ensureRootId(components: SurfaceComponent[]): SurfaceComponent[] {
	if (
		components.length === 0 ||
		components.some((comp) => comp.id === "root")
	) {
		return components;
	}
	const referenced = new Set<string>();
	for (const comp of components) {
		const inner = comp.component as unknown as
			| Record<string, unknown>
			| undefined;
		const children = inner?.children as Record<string, unknown> | undefined;
		if (Array.isArray(children?.explicitList)) {
			for (const id of children.explicitList as unknown[]) {
				if (typeof id === "string") referenced.add(id);
			}
		}
		const template = children?.template as Record<string, unknown> | undefined;
		if (typeof template?.componentId === "string") {
			referenced.add(template.componentId);
		}
	}
	// The root is the one component nothing else references as a child.
	const root = components.find((comp) => !referenced.has(comp.id));
	if (!root) return components;
	return components.map((comp) =>
		comp.id === root.id ? { ...comp, id: "root" } : comp,
	);
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

function argObject(
	args: Record<string, unknown>,
	key: string,
): Record<string, unknown> | undefined {
	const value = args[key];
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
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

type EventInterfaceKind = "chat" | "page" | "headless";

/** How an event is consumed: inline chat, embeddable UI page, or headless execution. */
function classifyEvent(event: {
	event_type: string;
	default_page_id?: string | null;
}): EventInterfaceKind {
	if (isChatEventType(event.event_type)) return "chat";
	if (event.default_page_id) return "page";
	return "headless";
}

/** Record that the current response acted on an app — attached to that message as a chip. */
function referenceApp(appId: string) {
	if (!appId) return;
	useGlobalChatStore.getState().addPendingAppRef(appId);
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
			return "/store/explore/apps";
		case "packages":
			return "/store/packages";
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
function createSubRunStream(options: {
	requestId: string;
	parentRequestId: string;
	recordDebugEvent: (event: IAgentDebugEvent) => void;
}) {
	const debugStream = createAgentDebugStreamRecorder({
		scope: "nested",
		requestId: options.requestId,
		parentRequestId: options.parentRequestId,
		record: options.recordDebugEvent,
	});
	const subAcc = createStreamAccumulator();
	const subPrefix = `${SUB_STEP_PREFIX}${options.parentRequestId}:`;
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
	return {
		pushSubRunChunk: debugStream.push,
		flushSubRunStream: debugStream.flush,
		subAcc,
		runIsLive,
		publishSubSteps,
		failProgressSteps,
	};
}

const FLOWSCRIPT_VALIDATION_TOOL_SUFFIXES = [
	"write_flowscript",
	"patch_flowscript",
	"check_flowscript",
	"commit_flowscript",
	"edit_flowscript",
] as const;

interface NestedFlowScriptValidationEvidence {
	/** Structured diagnostics from the latest FlowScript validation tool result (may be empty). */
	diagnostics: unknown[];
	draftId?: string;
	revision?: number | string;
}

/** Keep the actionable diagnostic fields and bound free text so the result stays compact. */
function compactFlowScriptDiagnostic(diagnostic: unknown): unknown {
	if (typeof diagnostic === "string") return diagnostic.slice(0, 400);
	if (
		!diagnostic ||
		typeof diagnostic !== "object" ||
		Array.isArray(diagnostic)
	) {
		return diagnostic;
	}
	const record = diagnostic as Record<string, unknown>;
	const compacted: Record<string, unknown> = {};
	for (const key of [
		"code",
		"phase",
		"severity",
		"message",
		"line",
		"column",
		"span",
		"path",
		"function",
	]) {
		const value = record[key];
		if (value === undefined) continue;
		compacted[key] = typeof value === "string" ? value.slice(0, 400) : value;
	}
	return Object.keys(compacted).length > 0 ? compacted : record;
}

/**
 * Pull structured diagnostics and the retained draft identity out of a nested FlowScript
 * validation tool result (write/patch/check/commit/edit_flowscript), whether the result arrives
 * as plain JSON, tagged text, or an MCP content envelope. This is what lets the outer agent see
 * the concrete defect list when the sub-run ends with `validation_errors`.
 */
function extractNestedFlowScriptValidationEvidence(
	data: unknown,
): NestedFlowScriptValidationEvidence | undefined {
	if (!data || typeof data !== "object") return undefined;
	const record = data as Record<string, unknown>;
	const toolName = String(
		record.tool_name ?? record.toolName ?? record.tool ?? record.name ?? "",
	).toLowerCase();
	if (
		!FLOWSCRIPT_VALIDATION_TOOL_SUFFIXES.some((suffix) =>
			toolName.endsWith(suffix),
		)
	) {
		return undefined;
	}
	const found: {
		diagnostics?: unknown[];
		draftId?: string;
		revision?: number | string;
	} = {};
	const visit = (value: unknown, depth: number) => {
		if (depth > 6 || value === null || value === undefined) return;
		if (typeof value === "string") {
			const tagged = value.match(
				/<structured_diagnostics>([\s\S]*?)<\/structured_diagnostics>/,
			);
			if (tagged?.[1]) {
				try {
					const parsed = JSON.parse(tagged[1]);
					if (Array.isArray(parsed)) found.diagnostics = parsed;
				} catch {
					// Malformed tag payloads are ignored; the raw text stays in the debug report.
				}
			}
			const trimmed = value.trim();
			if (
				(trimmed.startsWith("{") && trimmed.endsWith("}")) ||
				(trimmed.startsWith("[") && trimmed.endsWith("]"))
			) {
				try {
					visit(JSON.parse(trimmed), depth + 1);
				} catch {
					// Not a JSON document.
				}
			}
			return;
		}
		if (Array.isArray(value)) {
			for (const entry of value) visit(entry, depth + 1);
			return;
		}
		if (typeof value !== "object") return;
		const container = value as Record<string, unknown>;
		const structured = Array.isArray(container.structured_diagnostics)
			? container.structured_diagnostics
			: Array.isArray(container.diagnostics)
				? container.diagnostics
				: undefined;
		if (structured) found.diagnostics = structured;
		if (typeof container.draft_id === "string" && container.draft_id) {
			found.draftId = container.draft_id;
		}
		if (
			typeof container.revision === "number" ||
			(typeof container.revision === "string" && container.revision)
		) {
			found.revision = container.revision;
		}
		if (typeof container.text === "string") visit(container.text, depth + 1);
		if (container.content !== undefined) visit(container.content, depth + 1);
	};
	visit(record.result_preview ?? record.result ?? record.output, 0);
	if (!found.diagnostics && found.draftId === undefined) return undefined;
	return {
		diagnostics: found.diagnostics ?? [],
		draftId: found.draftId,
		revision: found.revision,
	};
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
		destructive: dialog.override?.destructive ?? false,
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

function dialogPromptDebugInput(prompt: GlobalToolPrompt) {
	return {
		kind: prompt.kind,
		tool_name: prompt.toolName,
		title: prompt.title,
		description: prompt.description,
		app_id: prompt.appId,
		ask: prompt.ask,
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
	const executeRuntimeTool = useFrontendRuntimeToolExecutor();
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

	// Perform a tool-requested navigation only AFTER the agent turn ends — navigating mid-stream
	// tears down the run. Tools stash the target via setPendingNavigation; we execute it here once
	// streaming stops.
	const pendingNavigation = useGlobalChatStore((s) => s.pendingNavigation);
	const isStreaming = useGlobalChatStore((s) => s.isStreaming);
	useEffect(() => {
		if (isStreaming || !pendingNavigation) return;
		const target = pendingNavigation;
		useGlobalChatStore.getState().setPendingNavigation(null);
		router.push(target);
		// Dock the conversation alongside the destination view so the user keeps chatting there.
		// Deferred to the navigation moment (not fired when the tool ran) so the dock never pops
		// open over the full /chat page mid-stream — /chat renders the conversation itself.
		if (!target.startsWith("/chat")) openOverlay();
	}, [isStreaming, pendingNavigation, router, openOverlay]);

	// The full /chat page already renders the conversation — only dock the overlay elsewhere.
	const pathnameRef = useRef(pathname);
	useEffect(() => {
		pathnameRef.current = pathname;
	}, [pathname]);
	const showConversation = useCallback(() => {
		if (pathnameRef.current !== "/chat") openOverlay();
	}, [openOverlay]);
	const resolverRef = useRef<{
		request: FrontendToolRequest;
		promptId: string;
		resolve: (value: GlobalToolPromptResolution) => void;
	} | null>(null);
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
	const requestExecutionFenceRef = useRef(
		new FrontendRequestExecutionFence<FrontendToolRequest>(),
	);
	const requestExecutionLeasesRef = useRef<
		WeakMap<
			FrontendToolRequest,
			FrontendRequestExecutionLease<FrontendToolRequest>
		>
	>(new WeakMap());
	const requestOwnerMessageIdsRef = useRef<Map<string, string>>(new Map());
	const requestOwnerCleanupTimersRef = useRef<
		Map<string, ReturnType<typeof setTimeout>>
	>(new Map());
	// A create_app result is authoritative for the rest of its owning assistant turn. This prevents
	// a transient board fetch failure from redirecting mutations into an older, similarly named app.
	const createdAppTargetsByOwnerRef = useRef<Map<string, string>>(new Map());
	// Failed repair candidates are board-scoped (not message-scoped), so a retry in a new turn can
	// continue the closest source after a provider deadline or lost MCP response.
	const boardRecoveryRef = useRef(new BoardEditRecoveryStore());
	const boardZeroProgressRetryRef = useRef(new BoardZeroProgressRetryGuard());
	// Crash-durable record of artifacts created per conversation. A retried creating tool (after a
	// crash, reload, or lost tool response) is answered with the recorded ids instead of a duplicate.
	const createdArtifactJournalRef = useRef(new CreatedArtifactJournal());
	const boardRecoveryScopeByRequestRef = useRef<
		Map<
			string,
			{
				key: string;
				baselineFingerprint?: FlowScriptBaselineFingerprint;
			}
		>
	>(new Map());
	const requestOwnershipIsActive = useCallback(
		(requestId: string) =>
			hasActiveFrontendRequestOwnership(
				requestId,
				requestExecutionFenceRef.current.activeExecutions().map((active) => ({
					requestId: active.requestId,
					parentRequestId: active.parentRequestId,
				})),
			),
		[],
	);
	const rememberRequestOwner = useCallback(
		(requestId: string, messageId: string) => {
			requestOwnerMessageIdsRef.current.set(requestId, messageId);
			while (requestOwnerMessageIdsRef.current.size > 512) {
				let oldestInactive: string | undefined;
				for (const candidate of requestOwnerMessageIdsRef.current.keys()) {
					// The new request is remembered immediately before its controller is registered.
					if (candidate === requestId || requestOwnershipIsActive(candidate)) {
						continue;
					}
					oldestInactive = candidate;
					break;
				}
				if (!oldestInactive) break;
				requestOwnerMessageIdsRef.current.delete(oldestInactive);
				const timer = requestOwnerCleanupTimersRef.current.get(oldestInactive);
				if (timer !== undefined) clearTimeout(timer);
				requestOwnerCleanupTimersRef.current.delete(oldestInactive);
			}

			const existingTimer = requestOwnerCleanupTimersRef.current.get(requestId);
			if (existingTimer !== undefined) clearTimeout(existingTimer);
			const scheduleCleanup = () => {
				const timer = setTimeout(() => {
					requestOwnerCleanupTimersRef.current.delete(requestId);
					if (requestOwnerMessageIdsRef.current.get(requestId) !== messageId) {
						return;
					}
					if (requestOwnershipIsActive(requestId)) {
						scheduleCleanup();
						return;
					}
					requestOwnerMessageIdsRef.current.delete(requestId);
				}, 15 * 60_000);
				requestOwnerCleanupTimersRef.current.set(requestId, timer);
			};
			scheduleCleanup();
		},
		[requestOwnershipIsActive],
	);
	const markRequestExpired = useCallback((requestId: string) => {
		requestExecutionFenceRef.current.invalidate(requestId);
	}, []);
	const ownerMessageIdForRequest = useCallback(
		(request: FrontendToolRequest) => {
			const parentId = parentRequestId(request);
			return (
				requestOwnerMessageIdsRef.current.get(request.requestId) ??
				(parentId
					? requestOwnerMessageIdsRef.current.get(parentId)
					: undefined) ??
				useGlobalChatStore.getState().streamingMessage?.id
			);
		},
		[],
	);
	const recordRequestDebug = useCallback(
		(
			request: FrontendToolRequest,
			event: Omit<
				IAgentDebugEvent,
				"request_id" | "parent_request_id" | "timestamp_ms"
			> & { timestamp_ms?: number },
		) => {
			const ownerMessageId = ownerMessageIdForRequest(request);
			if (!ownerMessageId) return;
			useGlobalChatStore.getState().recordDebugEvent(ownerMessageId, {
				...event,
				request_id: request.requestId,
				parent_request_id: parentRequestId(request),
				timestamp_ms: event.timestamp_ms ?? Date.now(),
			});
		},
		[ownerMessageIdForRequest],
	);
	const recordNestedDebug = useCallback(
		(request: FrontendToolRequest, event: IAgentDebugEvent) => {
			const ownerMessageId = ownerMessageIdForRequest(request);
			if (!ownerMessageId) return;
			useGlobalChatStore.getState().recordDebugEvent(ownerMessageId, event);
		},
		[ownerMessageIdForRequest],
	);
	const isRequestExpired = useCallback((request: FrontendToolRequest) => {
		const execution = requestExecutionLeasesRef.current.get(request);
		const deadline = requestDeadline(request);
		return (
			Boolean(
				execution && requestExecutionFenceRef.current.isInvalidated(execution),
			) ||
			(deadline !== undefined && Date.now() >= deadline)
		);
	}, []);
	const assertRequestActive = useCallback(
		(request: FrontendToolRequest, stage: string) => {
			if (!isRequestExpired(request)) return;
			markRequestExpired(request.requestId);
			throw new Error(
				`Frontend tool request '${request.requestId}' expired before ${stage}; late side effects were blocked.`,
			);
		},
		[isRequestExpired, markRequestExpired],
	);
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
			if (resolver) {
				recordRequestDebug(resolver.request, {
					id: `frontend:${resolver.request.requestId}:dialog:${resolver.promptId}`,
					kind: "approval",
					stage: "dialog_answered",
					status:
						value &&
						(("approved" in value && value.approved) || "answer" in value)
							? "done"
							: "denied",
					name: resolver.request.toolName,
					ended_at_ms: Date.now(),
					result_summary: value
						? agentDebugPreview(value, 500)
						: "Dialog dismissed",
					result_preview: agentDebugPreview(value),
				});
				resolver.resolve(value);
			}
			const next = dialogQueueRef.current.shift();
			if (next) {
				const prompt = promptForDialog(next.dialog, resolveDialogRef.current);
				resolverRef.current = {
					request: next.dialog.request,
					promptId: prompt.id,
					resolve: next.resolve,
				};
				recordRequestDebug(next.dialog.request, {
					id: `frontend:${next.dialog.request.requestId}:dialog:${prompt.id}`,
					kind: "approval",
					stage: "dialog_shown",
					status: "progress",
					name: next.dialog.request.toolName,
					started_at_ms: Date.now(),
					arguments_preview: agentDebugPreview(dialogPromptDebugInput(prompt)),
				});
				setToolPrompt(prompt);
			} else {
				setToolPrompt(null);
			}
		},
		[recordRequestDebug, setToolPrompt],
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
					recordRequestDebug(next.request, {
						id: `frontend:${next.request.requestId}:dialog:queued`,
						kind: "approval",
						stage: "dialog_queued",
						status: "planned",
						name: next.request.toolName,
						summary: `${next.type} dialog queued behind another request.`,
					});
					return;
				}
				const prompt = promptForDialog(next, resolveDialogRef.current);
				resolverRef.current = {
					request: next.request,
					promptId: prompt.id,
					resolve,
				};
				recordRequestDebug(next.request, {
					id: `frontend:${next.request.requestId}:dialog:${prompt.id}`,
					kind: "approval",
					stage: "dialog_shown",
					status: "progress",
					name: next.request.toolName,
					started_at_ms: Date.now(),
					summary: `${next.type} dialog shown.`,
					arguments_preview: agentDebugPreview(dialogPromptDebugInput(prompt)),
				});
				setToolPrompt(prompt);
				// The prompt lives inside the chat surface — make sure one is visible.
				showConversation();
			}),
		[recordRequestDebug, setToolPrompt, showConversation],
	);

	const cancelRequestDialogs = useCallback(
		(requestId: string, reason: string) => {
			const active = resolverRef.current;
			if (
				active &&
				(active.request.requestId === requestId ||
					parentRequestId(active.request) === requestId)
			) {
				resolverRef.current = null;
				recordRequestDebug(active.request, {
					id: `frontend:${active.request.requestId}:dialog:${active.promptId}`,
					kind: "approval",
					stage: "dialog_expired",
					status: "cancelled",
					name: active.request.toolName,
					ended_at_ms: Date.now(),
					error: reason,
				});
				active.resolve(null);
				setToolPrompt(null);
			}
			const retained: typeof dialogQueueRef.current = [];
			for (const queued of dialogQueueRef.current) {
				if (
					queued.dialog.request.requestId !== requestId &&
					parentRequestId(queued.dialog.request) !== requestId
				) {
					retained.push(queued);
					continue;
				}
				recordRequestDebug(queued.dialog.request, {
					id: `frontend:${queued.dialog.request.requestId}:dialog:queued`,
					kind: "approval",
					stage: "dialog_expired",
					status: "cancelled",
					name: queued.dialog.request.toolName,
					ended_at_ms: Date.now(),
					error: reason,
				});
				queued.resolve(null);
			}
			dialogQueueRef.current = retained;
			if (!resolverRef.current) {
				const next = dialogQueueRef.current.shift();
				if (next) {
					const prompt = promptForDialog(next.dialog, resolveDialogRef.current);
					resolverRef.current = {
						request: next.dialog.request,
						promptId: prompt.id,
						resolve: next.resolve,
					};
					recordRequestDebug(next.dialog.request, {
						id: `frontend:${next.dialog.request.requestId}:dialog:${prompt.id}`,
						kind: "approval",
						stage: "dialog_shown",
						status: "progress",
						name: next.dialog.request.toolName,
						started_at_ms: Date.now(),
						summary: `${next.dialog.type} dialog shown after a previous request was cancelled.`,
						arguments_preview: agentDebugPreview(
							dialogPromptDebugInput(prompt),
						),
					});
					setToolPrompt(prompt);
				}
			}
		},
		[recordRequestDebug, setToolPrompt],
	);

	const runTool = useCallback(
		async (request: FrontendToolRequest): Promise<unknown> => {
			assertRequestActive(request, "tool execution");
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
				case "database_tool":
				case "storage_tool":
				case "ui_inspect":
				case "execute_event":
				case "execute_node":
				case "query_execution_logs":
				case "graph_overlay_tool":
				case "graph_query_tool":
				case "graph_element_tool":
				case "ontology_action_tool":
					return executeRuntimeTool(request.toolName, args);
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
					// Defer the route change until the turn ends — navigating mid-stream tears down
					// the run. The bridge performs it once streaming stops.
					useGlobalChatStore.getState().setPendingNavigation(route);
					// The bridge docks the overlay alongside the destination once streaming stops.
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
					const snapshot = await captureInlineAppPageSnapshots(
						appId,
						pageEvent.id,
					);
					assertRequestActive(request, "app page screenshot capture");
					const uploadedSnapshots = (
						await Promise.all(
							snapshot.images.map(async (image, index) => {
								const extension =
									image.mediaType === "image/webp"
										? "webp"
										: image.mediaType === "image/jpeg"
											? "jpg"
											: "png";
								const file = new File(
									[image.blob],
									`flowpilot-page-${index + 1}.${extension}`,
									{ type: image.mediaType },
								);
								try {
									const temporaryFile = backend.helperState.fileToTemporaryFile
										? await backend.helperState.fileToTemporaryFile(
												file,
												false,
												undefined,
												"remote",
											)
										: {
												url: await backend.helperState.fileToUrl(
													file,
													false,
													undefined,
													"remote",
												),
											};
									if (!/^https?:\/\//i.test(temporaryFile.url)) {
										throw new Error(
											"Temporary upload did not return a remotely readable URL.",
										);
									}
									return {
										url: temporaryFile.url,
										media_type: image.mediaType,
									};
								} catch (error) {
									console.warn(
										"[global-tool-bridge] failed to upload app page capture",
										error,
									);
									return null;
								}
							}),
						)
					).filter((image): image is { url: string; media_type: string } =>
						Boolean(image),
					);
					assertRequestActive(request, "app page screenshot upload");
					const screenshotCount = uploadedSnapshots.length;
					const screenshotComplete =
						snapshot.complete && screenshotCount === snapshot.images.length;
					return {
						status: "ok",
						message:
							screenshotCount > 0
								? `Embedded the page '${pageEvent.name}' inline and attached ${screenshotCount} visual capture${screenshotCount === 1 ? "" : "s"} of its rendered content for inspection.`
								: `Embedded the page '${pageEvent.name}' inline, but its rendered content could not be captured. Do not claim to have read the page visually.`,
						screenshot_count: screenshotCount,
						screenshot_complete: screenshotComplete,
						...(screenshotCount > 0
							? { _flowpilot_image_urls: uploadedSnapshots }
							: {}),
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
					const name = argString(args, "name").trim();
					if (!name)
						return {
							status: "error",
							message:
								'create_app requires a `name`. Derive a short name from the request (e.g. "Weather App") and call create_app once with it — do not call it again with empty arguments.',
						};
					const description = argString(args, "description");
					const idempotencyKey =
						argString(args, "idempotency_key") ||
						argString(args, "idempotencyKey");
					const creationConversationId = conversationScopeId(request);
					const creationIdentity = creationConversationId
						? {
								conversationId: creationConversationId,
								toolName: "create_app",
								instruction: `${name}\n${description}`,
								...(idempotencyKey ? { idempotencyKey } : {}),
							}
						: undefined;
					const journaled = creationIdentity
						? createdArtifactJournalRef.current.find(creationIdentity)
						: undefined;
					if (journaled?.artifacts.appId) {
						const existingAppId = journaled.artifacts.appId;
						const ownerMessageId = ownerMessageIdForRequest(request);
						if (ownerMessageId) {
							createdAppTargetsByOwnerRef.current.set(
								ownerMessageId,
								existingAppId,
							);
						}
						referenceApp(existingAppId);
						return {
							status: "ok",
							app_id: existingAppId,
							name,
							already_created: true,
							note: "An app for this exact request was already created earlier in this conversation; its app_id is returned instead of creating a duplicate. Continue building on this app_id. Only if the user truly wants a second, separate app, call create_app again with a distinct `idempotency_key`.",
						};
					}
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
					const ownerMessageId = ownerMessageIdForRequest(request);
					if (ownerMessageId) {
						createdAppTargetsByOwnerRef.current.set(ownerMessageId, app.id);
						while (createdAppTargetsByOwnerRef.current.size > 128) {
							const oldest = createdAppTargetsByOwnerRef.current
								.keys()
								.next().value;
							if (typeof oldest !== "string") break;
							createdAppTargetsByOwnerRef.current.delete(oldest);
						}
					}
					referenceApp(app.id);
					if (creationIdentity) {
						createdArtifactJournalRef.current.record(
							creationIdentity,
							{ appId: app.id },
							request.requestId,
						);
					}
					return { status: "ok", app_id: app.id, name, online };
				}
				case "upsert_event": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: "upsert_event requires an app_id.",
						};
					const name = argString(args, "name").trim();
					if (!name)
						return {
							status: "error",
							message: "upsert_event requires a name.",
						};
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					let existingEvent: IEvent | undefined;
					if (eventId) {
						try {
							existingEvent = await backend.eventState.getEvent(appId, eventId);
						} catch (error) {
							return {
								status: "error",
								message: `Cannot update event '${eventId}': ${error instanceof Error ? error.message : String(error)}`,
							};
						}
					}

					const pageId =
						argString(args, "page_id") ||
						argString(args, "pageId") ||
						existingEvent?.default_page_id ||
						"";
					const eventBoardId =
						argString(args, "board_id") ||
						argString(args, "boardId") ||
						existingEvent?.board_id ||
						"";
					const eventNodeId =
						argString(args, "node_id") ||
						argString(args, "nodeId") ||
						existingEvent?.node_id ||
						"";
					// A page event binds default_page_id (board/node optional); a workflow Event
					// needs a compatible entry node in a board.
					if (!pageId && (!eventBoardId || !eventNodeId))
						return {
							status: "error",
							message:
								"Provide page_id for a page event, OR board_id + node_id for an events_simple/events_generic/events_chat entry node returned by flowpilot_board.",
						};

					let entryNodeName: string | undefined;
					let entryConfig: (typeof EVENT_CONFIG)[string] | undefined;
					let boardExecutionMode: string | undefined;
					if (eventBoardId && eventNodeId) {
						let eventBoard: Awaited<
							ReturnType<typeof backend.boardState.getBoard>
						>;
						try {
							eventBoard = await backend.boardState.getBoard(
								appId,
								eventBoardId,
								undefined,
								true,
							);
						} catch (error) {
							return {
								status: "error",
								message: `Failed to load the Event's board: ${error instanceof Error ? error.message : String(error)}`,
							};
						}
						const entryNode = eventBoard?.nodes?.[eventNodeId];
						entryNodeName = entryNode?.name;
						boardExecutionMode = eventBoard?.execution_mode;
						entryConfig = entryNodeName
							? EVENT_CONFIG[entryNodeName]
							: undefined;
						if (!entryNodeName || !entryConfig) {
							return {
								status: "error",
								message: `Node '${eventNodeId}' is not a supported Event entry. Use flowpilot_board to create eventsSimple(), eventsGeneric(payload: Struct, fieldName: string, ...), or eventsChat(...), then pass the returned event_nodes id.`,
							};
						}
						if (!isRunnableWorkflowEventEntry(eventBoard, eventNodeId)) {
							return {
								status: "error",
								message: `Node '${eventNodeId}' is an empty or unconnected Event entry. Build and connect the board logic first, then use the exact runnable event_nodes id returned by flowpilot_board. No Event was registered.`,
							};
						}
					}

					const requestedEventType = argString(args, "event_type").trim();
					const eventType = pageId
						? requestedEventType || existingEvent?.event_type || "quick_action"
						: requestedEventType ||
							(existingEvent &&
							entryConfig?.eventTypes.includes(existingEvent.event_type)
								? existingEvent.event_type
								: entryConfig?.defaultEventType) ||
							"quick_action";
					if (entryConfig && !entryConfig.eventTypes.includes(eventType)) {
						return {
							status: "error",
							message: `Event type '${eventType}' is incompatible with ${entryNodeName}. Supported types: ${entryConfig.eventTypes.join(", ")}. Cron setup requires an events_simple entry.`,
						};
					}

					const requestedExecutionMode =
						argString(args, "execution_mode") ||
						argString(args, "executionMode");
					let executionMode =
						requestedExecutionMode.toLowerCase() === "remote"
							? IEventExecutionMode.Remote
							: requestedExecutionMode.toLowerCase() === "local"
								? IEventExecutionMode.Local
								: (existingEvent?.execution_mode ?? IEventExecutionMode.Local);
					// Core enforces a concrete board mode on its Events. Resolve it here too so
					// sink_execution and the persisted Event cannot contradict one another.
					if (boardExecutionMode === "Local")
						executionMode = IEventExecutionMode.Local;
					if (boardExecutionMode === "Remote")
						executionMode = IEventExecutionMode.Remote;

					const existingConfig = parseUint8ArrayToJson(existingEvent?.config);
					const defaultConfig = entryConfig?.configs[eventType] ?? {};
					const keepExistingConfig =
						existingEvent?.event_type === eventType &&
						existingConfig &&
						typeof existingConfig === "object";
					let eventConfig: Record<string, unknown> = {
						...(keepExistingConfig
							? (existingConfig as Record<string, unknown>)
							: (defaultConfig as Record<string, unknown>)),
						...(argObject(args, "config") ?? {}),
					};
					if (eventType === "cron") {
						const expression =
							argString(args, "cron_expression") ||
							argString(args, "cronExpression") ||
							(typeof eventConfig.expression === "string"
								? eventConfig.expression.trim()
								: "");
						const scheduledFor =
							argObject(args, "scheduled_for") ||
							argObject(args, "scheduledFor") ||
							(eventConfig.scheduled_for &&
							typeof eventConfig.scheduled_for === "object"
								? (eventConfig.scheduled_for as Record<string, unknown>)
								: undefined);
						if (!expression && !scheduledFor) {
							return {
								status: "error",
								message:
									"A cron Event requires cron_expression for a recurring schedule OR scheduled_for {date, time} for a one-time run.",
							};
						}
						if (
							scheduledFor &&
							(typeof scheduledFor.date !== "string" ||
								typeof scheduledFor.time !== "string")
						) {
							return {
								status: "error",
								message:
									"scheduled_for requires string fields date (YYYY-MM-DD) and time (HH:mm).",
							};
						}
						const timezone =
							argString(args, "timezone") ||
							(typeof eventConfig.timezone === "string"
								? eventConfig.timezone
								: "UTC");
						eventConfig = {
							...eventConfig,
							sink_type: "cron",
							timezone,
							last_fired: null,
							sink_execution:
								executionMode === IEventExecutionMode.Remote
									? "REMOTE"
									: "LOCAL",
						};
						if (expression) {
							eventConfig.expression = expression;
							eventConfig.scheduled_for = undefined;
						} else {
							eventConfig.scheduled_for = scheduledFor;
							eventConfig.expression = undefined;
						}
					}

					const now = nowSystemTime();
					const event: IEvent = {
						...(existingEvent ?? {}),
						id: eventId || createId(),
						name,
						description:
							argString(args, "description") ||
							existingEvent?.description ||
							"",
						board_id: eventBoardId,
						node_id: eventNodeId,
						config: convertJsonToUint8Array(eventConfig) ?? [],
						active: argBool(args, "active") ?? existingEvent?.active ?? true,
						event_type: eventType,
						event_version: existingEvent?.event_version ?? [0, 0, 0],
						priority: existingEvent?.priority ?? 0,
						variables: existingEvent?.variables ?? {},
						created_at: existingEvent?.created_at ?? now,
						updated_at: now,
						execution_mode: executionMode,
						...(pageId ? { default_page_id: pageId } : {}),
					};
					let savedEvent: IEvent;
					try {
						savedEvent = await backend.eventState.upsertEvent(appId, event);
					} catch (error) {
						return {
							status: "error",
							message: `Failed to upsert event: ${error instanceof Error ? error.message : String(error)}`,
						};
					}
					// Optional URL route mapping (path -> eventId) so the event is reachable.
					const rawRoute = argString(args, "route");
					let routePath: string | undefined;
					if (rawRoute) {
						routePath = rawRoute.startsWith("/") ? rawRoute : `/${rawRoute}`;
						try {
							await backend.routeState.setRoute(
								appId,
								routePath,
								savedEvent.id,
							);
						} catch (error) {
							console.error(
								"[global-tool-bridge] upsert_event: setRoute failed",
								error,
							);
						}
					}
					referenceApp(appId);
					return {
						status: "ok",
						event_id: savedEvent.id,
						event_type: savedEvent.event_type,
						...(entryNodeName ? { entry_node_type: entryNodeName } : {}),
						execution_mode: savedEvent.execution_mode,
						...(pageId ? { page_id: pageId } : {}),
						...(routePath ? { route: routePath } : {}),
						note: pageId
							? "Page event upserted (bound to the page)."
							: eventType === "cron"
								? "Cron setup attached to the Simple Event entry."
								: "Compatible Event setup attached to the workflow entry.",
					};
				}
				case "delete_event": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					if (!appId || !eventId)
						return {
							status: "error",
							message: "delete_event requires app_id and event_id.",
						};
					try {
						await backend.eventState.deleteEvent(appId, eventId);
					} catch (error) {
						return {
							status: "error",
							message: `Failed to delete event: ${error instanceof Error ? error.message : String(error)}`,
						};
					}
					try {
						await backend.routeState.deleteRouteByEvent(appId, eventId);
					} catch {
						// best-effort route cleanup
					}
					referenceApp(appId);
					return { status: "ok", note: "Event deleted." };
				}
				case "set_page_load_event": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const pageId =
						argString(args, "page_id") || argString(args, "pageId");
					if (!appId || !pageId)
						return {
							status: "error",
							message: "set_page_load_event requires app_id and page_id.",
						};
					const boardId =
						argString(args, "board_id") ||
						argString(args, "boardId") ||
						undefined;
					let page: Awaited<ReturnType<typeof backend.pageState.getPage>>;
					try {
						page = await backend.pageState.getPage(appId, pageId, boardId);
					} catch (error) {
						return {
							status: "error",
							message: `Page not found: ${error instanceof Error ? error.message : String(error)}`,
						};
					}
					// onLoad/onUnload/onInterval are board NODE ids (events_simple), e.g. from a
					// flowpilot_board result's event_nodes.
					const onLoad =
						argString(args, "on_load_event_id") ||
						argString(args, "onLoadEventId");
					const onUnload =
						argString(args, "on_unload_event_id") ||
						argString(args, "onUnloadEventId");
					const onInterval =
						argString(args, "on_interval_event_id") ||
						argString(args, "onIntervalEventId");
					const pageBoardId = boardId || page.boardId;
					const configuredEntryIds = [
						["on_load_event_id", onLoad],
						["on_unload_event_id", onUnload],
						["on_interval_event_id", onInterval],
					].filter((entry): entry is [string, string] => Boolean(entry[1]));
					if (configuredEntryIds.length > 0) {
						if (!pageBoardId) {
							return {
								status: "error",
								message:
									"The page has no board_id. Build the board first and pass the exact board_id plus runnable Simple Event node returned by flowpilot_board.",
							};
						}
						let pageBoard: Awaited<
							ReturnType<typeof backend.boardState.getBoard>
						>;
						try {
							pageBoard = await backend.boardState.getBoard(
								appId,
								pageBoardId,
								undefined,
								true,
							);
						} catch (error) {
							return {
								status: "error",
								message: `Failed to load the page board: ${error instanceof Error ? error.message : String(error)}`,
							};
						}
						for (const [field, nodeId] of configuredEntryIds) {
							const entryNode = pageBoard?.nodes?.[nodeId];
							if (
								entryNode?.name !== "events_simple" ||
								!isRunnableWorkflowEventEntry(pageBoard, nodeId)
							) {
								return {
									status: "error",
									message: `${field} '${nodeId}' is not a connected events_simple entry on board '${pageBoardId}'. Build and connect the board logic first; the page was not changed.`,
								};
							}
						}
					}
					page.onLoadEventId = onLoad || undefined;
					if (onUnload) page.onUnloadEventId = onUnload;
					if (onInterval) {
						page.onIntervalEventId = onInterval;
						const secs = args.on_interval_seconds ?? args.onIntervalSeconds;
						if (typeof secs === "number" && secs > 0)
							page.onIntervalSeconds = secs;
					}
					try {
						await backend.pageState.updatePage(appId, page);
					} catch (error) {
						return {
							status: "error",
							message: `Failed to update page: ${error instanceof Error ? error.message : String(error)}`,
						};
					}
					referenceApp(appId);
					return {
						status: "ok",
						note: onLoad
							? "Page onLoad event wired — it runs when the page opens."
							: "Page onLoad event cleared.",
					};
				}
				case "data_studio_agent": {
					const instruction = argString(args, "instruction");
					if (!instruction)
						return {
							status: "error",
							message: "data_studio_agent requires an instruction.",
						};
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message:
								"data_studio_agent requires an app_id. Use list_apps to find one, or open a Data Studio page.",
						};
					const overlayId =
						argString(args, "overlay_id") || argString(args, "overlayId");

					const chat = useGlobalChatStore.getState();
					const owningUserPrompt = sourceUserPrompt(request);
					const owningConversationId = conversationScopeId(request);
					const rawSpecialistPrompt = composeDelegatedRawUserPrompt(
						owningUserPrompt,
						instruction,
					);
					const modelId = flowPilotModelIdForProvider(
						normalizeAIProvider(chat.provider),
						chat.selectedModelId,
					);

					const nestedRunRequestId = `${request.requestId}:agent`;
					const {
						pushSubRunChunk,
						flushSubRunStream,
						subAcc,
						publishSubSteps,
						failProgressSteps,
					} = createSubRunStream({
						requestId: nestedRunRequestId,
						parentRequestId: request.requestId,
						recordDebugEvent: (event) => recordNestedDebug(request, event),
					});
					const consumeSubRunEvents = (
						events: ReturnType<typeof pushSubRunChunk>,
					) => {
						let stepsChanged = false;
						for (const event of events) {
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
					const onToken = (chunk: string) =>
						consumeSubRunEvents(pushSubRunChunk(chunk));
					let subRunFlushed = false;
					const flushSubRun = () => {
						if (subRunFlushed) return;
						subRunFlushed = true;
						consumeSubRunEvents(flushSubRunStream());
					};

					recordNestedDebug(
						request,
						nestedAgentRunEvent({
							requestId: nestedRunRequestId,
							parentRequestId: request.requestId,
							toolName: "data_studio_agent",
							stage: "started",
							input: {
								scope: "DataStudio",
								app_id: appId,
								overlay_id: overlayId,
								instruction,
							},
							summary: "Delegated Data Studio sub-agent started.",
						}),
					);

					try {
						const response = await backend.boardState.copilot_chat(
							"DataStudio",
							null /* board */,
							undefined /* catalog */,
							[] /* selectedNodeIds */,
							null /* currentSurface */,
							[] /* selectedComponentIds */,
							instruction,
							[] /* history */,
							undefined /* images */,
							onToken,
							modelId,
							chat.reasoningEffort || undefined,
							undefined /* token */,
							undefined /* runContext */,
							undefined /* actionContext */,
							true /* nested: isolate from the pending parent session */,
							false /* readOnly */,
							{
								appId,
								overlayId,
								parentRequestId: request.requestId,
								conversationId: owningConversationId,
								sourceUserPrompt: owningUserPrompt,
							},
							nestedRunRequestId,
							rawSpecialistPrompt,
							appId,
						);
						flushSubRun();
						return {
							status: "ok",
							app_id: appId,
							overlay_id: overlayId,
							response: response.message,
						};
					} catch (error) {
						failProgressSteps();
						flushSubRun();
						return {
							status: "error",
							message: error instanceof Error ? error.message : String(error),
						};
					}
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
					const ownerMessageId = ownerMessageIdForRequest(request);
					const createdAppId = ownerMessageId
						? createdAppTargetsByOwnerRef.current.get(ownerMessageId)
						: undefined;
					const requestSignal =
						requestExecutionLeasesRef.current.get(request)?.controller.signal;
					const readCreatedAppWhenReady = <T,>(
						operation: () => Promise<T>,
						isReady?: (value: T) => boolean,
					) =>
						retryCreatedAppReadiness(operation, {
							appId,
							createdAppId,
							signal: requestSignal,
							deadlineAtMs: requestDeadline(request),
							isReady,
						});
					let boardId = liveSurface?.boardId ?? boardIdArg;
					if (boardId) {
						boardRecoveryScopeByRequestRef.current.set(request.requestId, {
							key: boardEditRecoveryKey(appId, boardId),
						});
					}
					// Explain/readback waits too, otherwise it can observe the pre-commit board while
					// a mutation run for the same board still owns the authoritative snapshot.
					const boardEditAcquireOptions = () => ({
						deadlineAtMs: requestDeadline(request),
						signal:
							requestExecutionLeasesRef.current.get(request)?.controller.signal,
						onInvalidated: () => markRequestExpired(request.requestId),
					});
					// A known board target locks only that board so runs on different boards of the
					// same app can overlap. Without a target the app-scoped key serializes board
					// creation/selection; the board-scoped lock is acquired below once resolved.
					const lockScopedToBoard = Boolean(boardId);
					const releaseBoardEdit = await boardEditCoordinator.acquire(
						boardEditLockKey(appId, boardId),
						boardEditAcquireOptions(),
					);
					let releaseBoardScopedEdit: (() => void) | undefined;
					let pendingDraftingWorkspace:
						| FlowScriptWorkspaceCandidate
						| undefined;
					let draftingWorkspaceTimer: ReturnType<typeof setTimeout> | undefined;
					try {
						assertRequestActive(request, "serialized board snapshot");
						let createdBoard = false;
						// Resolve the target again only after acquiring the board/app lock. Another
						// overlapping run may have created or changed it while this request waited.
						if (!boardId) {
							const boards = await readCreatedAppWhenReady(
								() => backend.boardState.getBoards(appId),
								(candidates) => candidates.length > 0,
							);
							boardId = boards?.[0]?.id ?? "";
						}
						// New apps have no board yet — create one instead of bouncing the task back
						// to the user.
						if (!boardId) {
							assertRequestActive(request, "board creation");
							const boardConversationId = conversationScopeId(request);
							const boardIdempotencyKey =
								argString(args, "idempotency_key") ||
								argString(args, "idempotencyKey");
							const boardCreationIdentity = boardConversationId
								? {
										conversationId: boardConversationId,
										toolName: "flowpilot_board",
										scope: appId,
										instruction,
										...(boardIdempotencyKey
											? { idempotencyKey: boardIdempotencyKey }
											: {}),
									}
								: undefined;
							// Reuse the board this conversation already created for the same request
							// (e.g. a crash/reload retry whose listing has not propagated) instead of
							// minting a duplicate; upsert on the recorded id is idempotent.
							boardId =
								(boardCreationIdentity
									? createdArtifactJournalRef.current.find(
											boardCreationIdentity,
										)?.artifacts.boardId
									: undefined) ?? createId();
							await backend.boardState.upsertBoard(
								appId,
								boardId,
								argString(args, "board_name") || "Main Board",
								instruction.slice(0, 140),
								ILogLevel.Debug,
								IExecutionStage.Dev,
							);
							createdBoard = true;
							if (boardCreationIdentity) {
								createdArtifactJournalRef.current.record(
									boardCreationIdentity,
									{ appId, boardId },
									request.requestId,
								);
							}
						}
						// A create-mode run held only the app-scoped creation lock. Now that it has a
						// concrete board, also take that board's lock (always app key first, board key
						// second) so it cannot overlap a run that targeted the same board explicitly.
						if (!lockScopedToBoard && boardId) {
							releaseBoardScopedEdit = await boardEditCoordinator.acquire(
								boardEditLockKey(appId, boardId),
								boardEditAcquireOptions(),
							);
							assertRequestActive(request, "board-scoped serialization");
						}
						const boardRecoveryKey = boardEditRecoveryKey(appId, boardId);
						boardRecoveryScopeByRequestRef.current.set(request.requestId, {
							key: boardRecoveryKey,
						});
						const zeroProgressOwnerId =
							ownerMessageId ?? parentRequestId(request) ?? request.requestId;
						if (
							!readOnly &&
							!boardZeroProgressRetryRef.current.canStart(
								zeroProgressOwnerId,
								boardRecoveryKey,
							)
						) {
							return {
								status: "zero_progress_retry_exhausted",
								code: "FLOWPILOT_BOARD_ZERO_PROGRESS_RETRY_EXHAUSTED",
								flowscript_status: "no_flowscript",
								message:
									"The board specialist already made the initial attempt and one materially different retry in this assistant turn without retaining any FlowScript source. A third equivalent run was not dispatched. Report the failure honestly instead of rewording the same request again.",
							};
						}
						flowPilotDebugLog(
							"[global-tool-bridge] flowpilot_board: loading board",
							{
								appId,
								boardId,
								createdBoard,
							},
						);
						const [board, catalog, baselineFlowScript] = await Promise.all([
							// Never generate from the captured liveSurface.board: it can be stale after
							// waiting behind another board specialist.
							readCreatedAppWhenReady(() =>
								backend.boardState.getBoard(appId, boardId, undefined, true),
							),
							liveSurface?.catalogNodes?.length
								? Promise.resolve(liveSurface.catalogNodes)
								: backend.boardState.getCatalog(appId),
							readCreatedAppWhenReady(() =>
								backend.boardState.getFlowScript(
									appId,
									boardId,
									undefined,
									true,
								),
							),
						]);
						const baselineFingerprint =
							flowScriptSnapshotFingerprint(baselineFlowScript);
						boardRecoveryScopeByRequestRef.current.set(request.requestId, {
							key: boardRecoveryKey,
							baselineFingerprint,
						});
						const retainedCandidateAtStart = boardRecoveryRef.current.get(
							boardRecoveryKey,
							baselineFingerprint,
						);
						const retainedReferenceAtStart = retainedCandidateAtStart
							? undefined
							: boardRecoveryRef.current.getReference(boardRecoveryKey);

						// Entry nodes already on the board before this run — so we can report which
						// Simple/Generic/Chat entries the copilot ADDED. The outer assistant then
						// attaches a compatible app-level Event setup (cron/chat/form/etc.).
						const preExistingEventNodeIds = new Set(
							Object.values(
								(board?.nodes ?? {}) as Record<
									string,
									{ id: string; name: string }
								>,
							)
								.filter((node) =>
									WORKFLOW_EVENT_ENTRY_NODE_NAMES.has(node.name),
								)
								.map((node) => node.id),
						);

						// Run the board copilot as a sub-agent, using the global chat's selected model.
						const chat = useGlobalChatStore.getState();
						const owningUserPrompt = sourceUserPrompt(request);
						const owningConversationId = conversationScopeId(request);
						const rawSpecialistPrompt = composeDelegatedRawUserPrompt(
							owningUserPrompt,
							instruction,
						);
						const modelId = flowPilotModelIdForProvider(
							normalizeAIProvider(chat.provider),
							chat.selectedModelId,
						);
						flowPilotDebugLog(
							"[global-tool-bridge] flowpilot_board: starting nested copilot_chat",
							{ modelId, boardId },
						);

						// Consume the sub-run's stream for live tool/plan activity and FlowScript
						// previews. External agents also return their last validated workspace in the
						// final nested response so a detached board can apply it safely.
						const nestedRunRequestId = `${request.requestId}:agent`;
						const {
							pushSubRunChunk,
							flushSubRunStream,
							subAcc,
							runIsLive,
							publishSubSteps,
							failProgressSteps,
						} = createSubRunStream({
							requestId: nestedRunRequestId,
							parentRequestId: request.requestId,
							recordDebugEvent: (event) => recordNestedDebug(request, event),
						});
						let workspaceCandidates: FlowScriptWorkspaceCandidate[] =
							retainedCandidateAtStart ? [retainedCandidateAtStart] : [];
						// Latest FlowScript validation tool result observed on the nested stream. When
						// the run ends with validation_errors this carries the concrete defect list and
						// the retained draft identity back to the outer agent.
						let lastFlowScriptValidation:
							| NestedFlowScriptValidationEvidence
							| undefined;
						const publishDraftingWorkspace = () => {
							draftingWorkspaceTimer = undefined;
							const candidate = pendingDraftingWorkspace;
							pendingDraftingWorkspace = undefined;
							if (candidate?.source && runIsLive()) {
								useGlobalChatStore.getState().setFlowscriptWorkspace(candidate);
							}
						};
						const scheduleDraftingWorkspace = (
							candidate: FlowScriptWorkspaceCandidate,
						) => {
							pendingDraftingWorkspace = candidate;
							if (draftingWorkspaceTimer) return;
							draftingWorkspaceTimer = setTimeout(
								publishDraftingWorkspace,
								FLOWSCRIPT_DRAFT_PREVIEW_INTERVAL_MS,
							);
						};
						const discardDraftingWorkspace = () => {
							if (draftingWorkspaceTimer) clearTimeout(draftingWorkspaceTimer);
							draftingWorkspaceTimer = undefined;
							pendingDraftingWorkspace = undefined;
						};
						const consumeSubRunEvents = (
							events: ReturnType<typeof pushSubRunChunk>,
						) => {
							let stepsChanged = false;
							for (const event of events) {
								if (event.type === "flowscript_workspace") {
									const candidate = parseFlowScriptWorkspaceCandidate(
										event.raw,
									);
									if (candidate) {
										if (candidate.status === "drafting") {
											// Incomplete source is useful to watch, but it is not a repair
											// candidate and must never become durable board recovery state.
											scheduleDraftingWorkspace(candidate);
											continue;
										}
										discardDraftingWorkspace();
										workspaceCandidates = rememberFlowScriptWorkspaceCandidate(
											workspaceCandidates,
											candidate,
										);
										const recoverable =
											selectBestRecoverableFlowScriptCandidate(
												workspaceCandidates,
											);
										if (recoverable) {
											boardRecoveryRef.current.set(
												boardRecoveryKey,
												recoverable,
												baselineFingerprint,
											);
										}
										// Keep rejected drafts inspectable in the nested process log even
										// when a later candidate becomes the final/applicable workspace.
										if (candidate.status === "validation_errors") {
											const id = `workspace-candidate-${workspaceCandidates.length}`;
											subAcc.stepOrder.push(id);
											subAcc.steps.set(id, {
												id,
												title: "FlowScript candidate",
												description: "Not applied — validation errors",
												status: "failed",
												reasoning: safeFlowScriptPlanReasoning(
													candidate.source,
												),
												timestamp: Date.now(),
											});
											stepsChanged = true;
										}
										// Authoritative submitted/validation/queued snapshots bypass the
										// draft throttle and replace any pending partial source immediately.
										if (runIsLive()) {
											useGlobalChatStore
												.getState()
												.setFlowscriptWorkspace(candidate);
										}
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
								if (event.type === "tool_end") {
									const evidence = extractNestedFlowScriptValidationEvidence(
										event.data,
									);
									if (evidence) lastFlowScriptValidation = evidence;
								}
								if (event.type === "text") continue;
								applyStreamEvent(subAcc, event);
								stepsChanged = true;
							}
							if (stepsChanged) publishSubSteps();
						};
						const onToken = (chunk: string) =>
							consumeSubRunEvents(pushSubRunChunk(chunk));
						let subRunFlushed = false;
						const flushSubRun = () => {
							if (subRunFlushed) return;
							subRunFlushed = true;
							consumeSubRunEvents(flushSubRunStream());
							publishDraftingWorkspace();
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
						let selectedWorkspace: FlowScriptWorkspaceCandidate | undefined;
						let partialWorkingSlice = false;
						let hadReturnedCommands = false;
						let returnedCommandCount = 0;
						let flowIrCommit: FlowIrCommitToken | undefined;
						let staleSnapshotBlocked = false;
						let persistedReadbackFailed = false;
						let persistedReadbackVerified = false;
						let appliedSourceCorrections = 0;
						let canonicalSourceCorrected = false;
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
						const recoveryContinuation = retainedCandidateAtStart
							? retainedFlowScriptRecoveryInstruction(
									retainedCandidateAtStart.source,
								)
							: retainedReferenceAtStart
								? retainedFlowScriptReferenceInstruction(
										retainedReferenceAtStart.source,
									)
								: "";
						const boardInstruction =
							(readOnly
								? `${instruction}

Answer the user's question about this board clearly and concisely, grounded in its actual nodes and connections. Do NOT modify the board — make no edits and submit no FlowScript.`
								: `${instruction}

Execute the change NOW in this run: draft the complete FlowScript workspace for this request and submit it via your edit tools. Do not stop after analysis and do not merely describe a plan — the run only counts as successful once the complete workspace validates and returns status queued. A partial foundation or a submitted/failed preview is not success.

Create an early retained full-shape FlowScript draft before exhaustive discovery: after one focused declaration batch, submit a draft that preserves the complete requested scope and its end-to-end structure, even when validation diagnostics are still expected. Do not chase every omitted or unmatched declaration before that first write, and perform at most six ancillary database/schema/UI/storage inspection calls before it. This retained diagnostic checkpoint is not success; use its compiler and acceptance diagnostics for narrow follow-up lookups, repair the complete draft, then check and commit it until the workspace is queued.

Completion contract: build complete helper logic first and add the Event entry last. The Event must connect to runnable logic; every helper needs body nodes plus an observable return or side effect; consume accumulators/outputs instead of discarding them; trace execution and data connections end-to-end before submitting. Use eventsSimple() for execution-only/quick-action/scheduled logic, eventsGeneric(payload: Struct, fieldName: string, ...) for typed form/request pins, or eventsChat(...) for chat context. Cron is app Event setup on eventsSimple(), never a catalog node. This board run builds the workflow; the outer assistant attaches its Event interface after success.`) +
							recoveryContinuation;
						recordNestedDebug(
							request,
							nestedAgentRunEvent({
								requestId: nestedRunRequestId,
								parentRequestId: request.requestId,
								toolName: "flowpilot_board",
								stage: "started",
								input: {
									scope: "Board",
									app_id: appId,
									board_id: boardId,
									instruction,
									...(retainedCandidateAtStart
										? {
												retained_candidate: safeFlowScriptPlanReasoning(
													retainedCandidateAtStart.source,
													2_000,
												),
											}
										: {}),
									read_only: readOnly,
									selected_node_ids: liveSurface?.selectedNodeIds ?? [],
								},
								summary: "Delegated board sub-agent started.",
							}),
						);
						let nestedRunSettled = false;
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
								chat.reasoningEffort || undefined,
								undefined /* token */,
								surfaceRunContext,
								undefined /* actionContext */,
								true /* nested: isolate from the pending parent session */,
								readOnly /* explain mode: answer, don't edit */,
								{
									appId,
									boardId,
									parentRequestId: request.requestId,
									conversationId: owningConversationId,
									sourceUserPrompt: owningUserPrompt,
								},
								nestedRunRequestId,
								rawSpecialistPrompt,
								appId,
							);
							flushSubRun();
							flowPilotDebugLog(
								"[global-tool-bridge] flowpilot_board: nested copilot_chat finished",
								{ commands: response.commands?.length ?? 0, readOnly },
							);
							const returnedCommands = response.commands ?? [];
							flowIrCommit = response.flow_ir_commit;
							const hadRetainedCompiledBatch = Boolean(flowIrCommit);
							hadReturnedCommands = returnedCommands.length > 0;
							returnedCommandCount = returnedCommands.length;

							// Read-only explain: nothing is applied. Surface the board (navigating only
							// when it isn't already the live canvas) and relay the copilot's answer.
							if (readOnly) {
								if (flowIrCommit) {
									const dismissed = await dismissFlowIrCommitWithRetry(
										backend.boardState,
										flowIrCommit,
									);
									if (!dismissed) {
										throw new Error(
											"The unexpected compiled workflow returned during a read-only run could not be released.",
										);
									}
									flowIrCommit = undefined;
								}
								assertRequestActive(request, "read-only board navigation");
								if (!liveSurface) {
									useGlobalChatStore
										.getState()
										.setPendingNavigation(`/flow?id=${boardId}&app=${appId}`);
								}
								referenceApp(appId);
								recordNestedDebug(
									request,
									nestedAgentRunEvent({
										requestId: nestedRunRequestId,
										parentRequestId: request.requestId,
										toolName: "flowpilot_board",
										stage: "finished",
										status: "ok",
										output: response,
										summary: "Delegated board explanation finished.",
									}),
								);
								nestedRunSettled = true;
								return {
									status: "ok",
									mode: "explain",
									message: response.message,
									...(createdBoard ? { created_board_id: boardId } : {}),
								};
							}

							// Apply only a workspace that is bound to a successfully queued command batch.
							// `submitted` is a live preview, not validation; treating it as applicable allowed
							// failed/partial drafts to bypass the edit tool's diagnostics on a second reconcile.
							selectedWorkspace = resolveFinalFlowScriptWorkspaceCandidate(
								workspaceCandidates,
								response.flowscript_workspace,
								hadReturnedCommands,
							);
							if (selectedWorkspace) {
								workspaceCandidates = rememberFlowScriptWorkspaceCandidate(
									workspaceCandidates,
									selectedWorkspace,
								);
								const recoverable =
									selectBestRecoverableFlowScriptCandidate(workspaceCandidates);
								if (recoverable) {
									boardRecoveryRef.current.set(
										boardRecoveryKey,
										recoverable,
										baselineFingerprint,
									);
								}
							}
							source = selectedWorkspace?.source;
							workspaceStatus = selectedWorkspace?.status;
							partialWorkingSlice =
								isPartialFlowScriptWorkspace(selectedWorkspace);
							const applicable =
								isFlowScriptWorkspaceApplicable(selectedWorkspace);

							if (flowIrCommit) {
								assertRequestActive(request, "atomic compiled workflow apply");
								const token = flowIrCommit;
								const surfaceNow = useAssistantSurface.getState().boardSurface;
								const applyLive =
									surfaceNow?.appId === appId && surfaceNow.boardId === boardId
										? surfaceNow
										: null;
								const applyRetainedCompiledBatch =
									async (): Promise<IApplyFlowIrCommitResponse> =>
										applyLive
											? await applyLive.applyFlowIrCommit(token)
											: backend.boardState.applyFlowIrCommit
												? await backend.boardState.applyFlowIrCommit(
														appId,
														token,
													)
												: {
														status: "error",
														code: "IR_ATOMIC_APPLY_UNAVAILABLE",
														message:
															"This backend cannot atomically apply retained compiled workflow batches.",
														commands: [],
														board_commands: [],
														diagnostics: [],
													};
								// The native Apply command owns destructive confirmation. Renderer state is
								// intentionally never accepted as authorization for this exact batch.
								const compiledResult: IApplyFlowIrCommitResponse =
									await applyRetainedCompiledBatch();
								recordNestedDebug(
									request,
									agentGenerationReviewDispositionEvent({
										requestId: nestedRunRequestId,
										parentRequestId: request.requestId,
										disposition:
											compiledResult.status === "applied"
												? "applied"
												: compiledResult.status === "stale"
													? "stale"
													: "error",
										draftId: token.draft_id,
										revision: token.revision,
										claimId: token.claim_id,
									}),
								);
								diagnostics = [
									...(compiledResult.status === "applied"
										? []
										: [compiledResult.message]),
									...compiledResult.diagnostics,
								];
								if (compiledResult.status === "applied") {
									appliedCommands = compiledResult.commands.length;
									appliedViaLive = applyLive !== null;
									flowIrCommit = undefined;
									try {
										const persistedFlowScript =
											await backend.boardState.getFlowScript(
												appId,
												boardId,
												undefined,
												true,
											);
										persistedReadbackVerified = flowScriptSnapshotChanged(
											baselineFlowScript,
											persistedFlowScript,
										);
										if (!persistedReadbackVerified) {
											persistedReadbackFailed = true;
											diagnostics = [
												"PERSISTED_FLOWSCRIPT_MISMATCH: Atomic apply reported success but the persisted board snapshot did not advance.",
												...diagnostics,
											];
										}
									} catch (error) {
										persistedReadbackFailed = true;
										diagnostics = [
											`PERSISTED_FLOWSCRIPT_READBACK_FAILED: Atomic apply succeeded, but verification could not reload the board: ${error instanceof Error ? error.message : String(error)}`,
											...diagnostics,
										];
									}
									if (!appliedViaLive && appliedCommands > 0) {
										void queryClient.invalidateQueries({
											predicate: (query) => query.queryKey.includes(appId),
										});
									}
								} else if (compiledResult.status === "stale") {
									staleSnapshotBlocked = true;
								}
							}

							if (
								!hadRetainedCompiledBatch &&
								source &&
								applicable &&
								!staleSnapshotBlocked
							) {
								const flowscript = source;
								const applyOnce = async (allowDeletions: boolean) => {
									assertRequestActive(request, "FlowScript apply");
									const currentFlowScript =
										await backend.boardState.getFlowScript(
											appId,
											boardId,
											undefined,
											true,
										);
									if (
										flowScriptSnapshotChanged(
											baselineFlowScript,
											currentFlowScript,
										)
									) {
										staleSnapshotBlocked = true;
										appliedCommands = 0;
										diagnostics = [
											"STALE_BOARD_SNAPSHOT: The board changed after this specialist started. Nothing from the stale draft was applied; regenerate from the fresh board state.",
										];
										return false;
									}
									// The sub-run can outlast the open board (closed/navigated mid-run):
									// re-resolve the surface at apply time; a stale captured surface would
									// apply through dead closures (lost awareness ping, no user feedback).
									const surfaceNow =
										useAssistantSurface.getState().boardSurface;
									const applyLive =
										surfaceNow?.appId === appId &&
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
										appliedSourceCorrections =
											applyResult?.corrections?.length ?? 0;
										diagnostics = applyResult?.diagnostics ?? [];
									} else {
										assertRequestActive(request, "detached FlowScript apply");
										const applyResult =
											await backend.boardState.applyFlowScript(
												appId,
												boardId,
												flowscript,
												undefined,
												catalog,
												allowDeletions,
											);
										appliedCommands = applyResult.commands?.length ?? 0;
										appliedSourceCorrections =
											applyResult.corrections?.length ?? 0;
										diagnostics = applyResult.diagnostics ?? [];
									}
									blockedDeletion =
										diagnostics[0]?.startsWith(DELETION_DIAGNOSTIC_PREFIX) ??
										false;
									const correctionOnlySucceeded =
										appliedCommands === 0 &&
										appliedSourceCorrections > 0 &&
										diagnostics.length === 0;
									if (
										(appliedCommands > 0 || correctionOnlySucceeded) &&
										!blockedDeletion
									) {
										const persistedFlowScript =
											await backend.boardState.getFlowScript(
												appId,
												boardId,
												undefined,
												true,
											);
										const readback = correctionOnlySucceeded
											? assessFlowScriptCorrectionReadback({
													expected: flowscript,
													actual: persistedFlowScript,
												})
											: assessFlowScriptReadback({
													before: baselineFlowScript,
													expected: flowscript,
													actual: persistedFlowScript,
												});
										if (!readback.ok) {
											persistedReadbackFailed = true;
											diagnostics = [
												`PERSISTED_FLOWSCRIPT_MISMATCH: ${readback.message ?? "The persisted board does not match the validated FlowScript."}`,
												...diagnostics,
											];
										} else {
											persistedReadbackVerified = true;
											if (appliedSourceCorrections > 0 && selectedWorkspace) {
												const canonicalWorkspace = {
													...selectedWorkspace,
													source: persistedFlowScript,
												};
												selectedWorkspace = canonicalWorkspace;
												source = persistedFlowScript;
												workspaceCandidates = [canonicalWorkspace];
												partialWorkingSlice =
													isPartialFlowScriptWorkspace(canonicalWorkspace);
												boardRecoveryRef.current.set(
													boardRecoveryKey,
													canonicalWorkspace,
													flowScriptSnapshotFingerprint(persistedFlowScript),
												);
												canonicalSourceCorrected = true;
											}
										}
									}
									return applyLive !== null;
								};
								appliedViaLive = await applyOnce(false);
								if (blockedDeletion) {
									assertRequestActive(request, "deletion approval");
									// Destructive edits are NEVER auto-applied: ask the user inline and
									// only re-apply with deletions allowed after an explicit approve.
									const diagnostic = diagnostics[0] ?? "";
									const outcome = await openDialog({
										type: "approval",
										request,
										override: {
											destructive: true,
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
									assertRequestActive(request, "board query refresh");
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
							if (!hadRetainedCompiledBatch && workspaceStatus === "queued") {
								recordNestedDebug(
									request,
									agentGenerationReviewDispositionEvent({
										requestId: nestedRunRequestId,
										parentRequestId: request.requestId,
										disposition:
											(appliedCommands > 0 || canonicalSourceCorrected) &&
											!blockedDeletion &&
											!persistedReadbackFailed
												? "applied"
												: staleSnapshotBlocked
													? "stale"
													: "error",
									}),
								);
							}

							// External agents can return an already-validated BoardCommand batch without
							// a FlowScript workspace. A live board owns the safe conversion/execution
							// pipeline. Detached command-only batches cannot be applied by BoardState,
							// whose executeCommands surface accepts lower-level GenericCommands.
							if (
								!hadRetainedCompiledBatch &&
								!source &&
								returnedCommands.length > 0 &&
								!staleSnapshotBlocked
							) {
								const surfaceNow = useAssistantSurface.getState().boardSurface;
								const commandSurface =
									surfaceNow?.appId === appId && surfaceNow.boardId === boardId
										? surfaceNow
										: null;
								if (commandSurface) {
									assertRequestActive(request, "validated command execution");
									await commandSurface.executeCommands(returnedCommands);
									appliedCommands = returnedCommands.length;
									appliedViaLive = true;
								} else {
									appliedViaLive = false;
									diagnostics = [
										`The board copilot returned ${returnedCommands.length} validated command${returnedCommands.length === 1 ? "" : "s"}, but no FlowScript workspace. Command-only results require the target board to remain open and were not applied.`,
									];
								}
							}

							if (flowIrCommit) {
								// A successful native transaction consumes its claim before returning and
								// clears flowIrCommit above. Any token still present here was never applied.
								const dismissed = await dismissFlowIrCommitWithRetry(
									backend.boardState,
									flowIrCommit,
								);
								if (!dismissed) {
									diagnostics = [
										"The compiled workflow was not applied, but its native review reservation could not be released after retries. It will expire automatically.",
										...diagnostics,
									];
									void dismissFlowIrCommitWithRetry(
										backend.boardState,
										flowIrCommit,
									);
								}
								flowIrCommit = undefined;
							}
						} catch (error) {
							if (flowIrCommit) {
								if (isRequestExpired(request)) {
									// Bridge deadline / lost response channel: the exact checked
									// batch stays PENDING on the host so the next run for the same
									// request redelivers its Apply/Dismiss token instead of
									// rebuilding the identical batch. Dismissing here previously
									// destroyed that retained work and forced full rebuild cycles.
									flowPilotDebugLog(
										"[global-tool-bridge] flowpilot_board: keeping retained compiled review for redelivery after request expiry",
										{
											draftId: flowIrCommit.draft_id,
											revision: flowIrCommit.revision,
										},
									);
								} else {
									const tokenToDismiss = flowIrCommit;
									const dismissed = await dismissFlowIrCommitWithRetry(
										backend.boardState,
										tokenToDismiss,
									);
									if (!dismissed) {
										void dismissFlowIrCommitWithRetry(
											backend.boardState,
											tokenToDismiss,
										);
									}
								}
							}
							flushSubRun();
							const retainedCandidate =
								selectBestRecoverableFlowScriptCandidate(workspaceCandidates) ??
								boardRecoveryRef.current.get(
									boardRecoveryKey,
									baselineFingerprint,
								);
							const errorMessage = getErrorMessage(
								error,
								"The queued board change failed without a diagnostic.",
							);
							const interruptedResult = boardEditInterruptionResult({
								status: isRequestExpired(request) ? "timeout" : "error",
								code: isRequestExpired(request)
									? "board_edit_interrupted"
									: "board_subrun_failed",
								message: errorMessage,
								candidate: retainedCandidate,
							});
							if (!readOnly) {
								boardZeroProgressRetryRef.current.recordRunOutcome(
									zeroProgressOwnerId,
									boardRecoveryKey,
									request.requestId,
									Boolean(retainedCandidate),
								);
							}
							if (!nestedRunSettled) {
								recordNestedDebug(
									request,
									nestedAgentRunEvent({
										requestId: nestedRunRequestId,
										parentRequestId: request.requestId,
										toolName: "flowpilot_board",
										stage: "finished",
										status: interruptedResult.status,
										output: interruptedResult,
										error,
										summary: "Delegated board sub-agent failed.",
									}),
								);
							}
							failProgressSteps();
							return interruptedResult;
						}

						const noFlowScript = !source && !hadReturnedCommands;
						if (!readOnly) {
							boardZeroProgressRetryRef.current.recordRunOutcome(
								zeroProgressOwnerId,
								boardRecoveryKey,
								request.requestId,
								!noFlowScript,
							);
						}
						const unvalidatedWorkspace =
							Boolean(source) &&
							workspaceStatus !== "queued" &&
							workspaceStatus !== "no_changes";
						const applyFailed =
							staleSnapshotBlocked ||
							persistedReadbackFailed ||
							(appliedCommands === 0 &&
								diagnostics.length > 0 &&
								!blockedDeletion);
						const resultStatus =
							noFlowScript || unvalidatedWorkspace || applyFailed
								? "error"
								: partialWorkingSlice
									? "partial"
									: "ok";
						// Publish the final workspace too — bits/copilot backends only carry it in the
						// final response, not the stream.
						if (source && runIsLive() && !isRequestExpired(request)) {
							useGlobalChatStore
								.getState()
								.setFlowscriptWorkspace(selectedWorkspace ?? null);
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
												: partialWorkingSlice
													? `${appliedCommands} command${appliedCommands === 1 ? "" : "s"} applied as an incomplete testable slice`
													: canonicalSourceCorrected && appliedCommands === 0
														? "Canonical FlowScript anchors repaired"
														: `${appliedCommands} command${appliedCommands === 1 ? "" : "s"} applied${blockedDeletion ? " (deletions blocked)" : deletionApproved ? " (deletions approved)" : ""}`,
								status:
									workspaceStatus === "validation_errors" || applyFailed
										? "failed"
										: "done",
								reasoning: safeFlowScriptPlanReasoning(source),
								timestamp: Date.now(),
							});
							publishSubSteps();
						}

						// Surface the board only after a verified apply (or an authoritative
						// no-changes result). Scheduling /flow for a submitted/failed workspace makes
						// an empty board look like a successful apply and can overwrite an earlier
						// page-builder destination. Re-resolve the surface here because navigation or
						// mounting may have changed while the nested run was active.
						assertRequestActive(request, "board result publication");
						const surfaceAtPublication =
							useAssistantSurface.getState().boardSurface;
						const targetBoardIsVisible =
							surfaceAtPublication?.appId === appId &&
							surfaceAtPublication.boardId === boardId;
						const verifiedBoardResult =
							!applyFailed &&
							(workspaceStatus === "no_changes" ||
								(canonicalSourceCorrected && persistedReadbackVerified) ||
								(appliedCommands > 0 &&
									(persistedReadbackVerified ||
										(!source && appliedViaLive === true))));
						if (verifiedBoardResult && !targetBoardIsVisible) {
							useGlobalChatStore
								.getState()
								.setPendingNavigation(`/flow?id=${boardId}&app=${appId}`);
						}
						referenceApp(appId);

						// Report all supported entry kinds. Keeping node_type and compatible Event
						// types in the result prevents the outer agent from confusing a sink (cron)
						// with a board node or attaching an incompatible interface.
						let eventNodes: RunnableWorkflowEventEntry[] = [];
						let finalBoardNodeCount: number | undefined;
						let updatedBoard: Awaited<
							ReturnType<typeof backend.boardState.getBoard>
						> | null = null;
						try {
							updatedBoard = await backend.boardState.getBoard(
								appId,
								boardId,
								undefined,
								true,
							);
							if (updatedBoard) {
								// Canonical boards may expose a layer member in both the root node
								// index and the layer-local map. Count identities, not map entries.
								const finalNodeIds = new Set(
									Object.keys(updatedBoard.nodes ?? {}),
								);
								for (const layer of Object.values(updatedBoard.layers ?? {})) {
									for (const nodeId of Object.keys(layer?.nodes ?? {})) {
										finalNodeIds.add(nodeId);
									}
								}
								finalBoardNodeCount = finalNodeIds.size;
							}
						} catch (error) {
							console.error(
								"[global-tool-bridge] flowpilot_board: final board inspection failed",
								error,
							);
						}
						if (
							shouldPromoteFlowScriptWorkspaceEvents(
								selectedWorkspace,
								applyFailed,
								appliedCommands > 0 || workspaceStatus === "no_changes",
							)
						) {
							assertRequestActive(request, "Event promotion");
							if (updatedBoard) {
								eventNodes = collectRunnableWorkflowEventEntries(
									updatedBoard,
									boardId,
									preExistingEventNodeIds,
									(nodeType) => EVENT_CONFIG[nodeType]?.eventTypes ?? [],
								);
							}
						}

						// Prefer the structured diagnostics captured from the nested validation tools;
						// the local apply-path diagnostics are the fallback.
						const validationDiagnostics = lastFlowScriptValidation?.diagnostics
							.length
							? lastFlowScriptValidation.diagnostics
							: diagnostics;
						const result = {
							status: resultStatus,
							message: response.message,
							applied_commands: appliedCommands,
							...(finalBoardNodeCount !== undefined
								? { final_board_node_count: finalBoardNodeCount }
								: {}),
							...(eventNodes.length > 0 ? { event_nodes: eventNodes } : {}),
							...(partialWorkingSlice
								? {
										flowscript_status: "partial",
										completion: "partial_working_slice",
										...(selectedWorkspace?.retained_full_source
											? {
													retained_full_source_summary:
														safeFlowScriptPlanReasoning(
															selectedWorkspace.retained_full_source,
															2_000,
														),
												}
											: {}),
										note: "A valid, independently runnable partial working slice was applied for testing and iterative extension. The requested application is still incomplete, and this slice was not promoted to an app-level Event.",
									}
								: {}),
							...(createdBoard ? { created_board_id: boardId } : {}),
							...(noFlowScript
								? {
										flowscript_status: "no_flowscript",
										note: "IMPORTANT: the board copilot ended WITHOUT submitting a FlowScript — the board was NOT modified and contains no new nodes. Do not tell the user the workflow was built. Retry flowpilot_board at most once, and only with a materially different bounded pre-draft strategy: use one focused declaration batch, no more than six ancillary inspections, then immediately retain a full-shape draft and repair it from diagnostics. If an equivalent zero-progress result already occurred, do not retry by merely rewording or shortening the instruction; stop and tell the user honestly that the edit failed.",
									}
								: {}),
							...(workspaceStatus === "validation_errors"
								? {
										flowscript_status:
											selectedWorkspace?.completion === "regression_blocked"
												? "candidate_regression"
												: "validation_errors",
										...(validationDiagnostics.length > 0
											? {
													diagnostics: validationDiagnostics
														.slice(0, 10)
														.map(compactFlowScriptDiagnostic),
													diagnostics_total: validationDiagnostics.length,
												}
											: {}),
										...(lastFlowScriptValidation?.draftId
											? {
													retained_draft: {
														draft_id: lastFlowScriptValidation.draftId,
														...(lastFlowScriptValidation.revision !== undefined
															? { revision: lastFlowScriptValidation.revision }
															: {}),
													},
												}
											: {}),
										...(selectedWorkspace?.completion === "regression_blocked"
											? {
													retained_flowscript_summary:
														safeFlowScriptPlanReasoning(
															selectedWorkspace.source,
															2_000,
														),
													note: "A smaller queued smoke-test candidate was blocked before apply. The fuller FlowScript remains retained for in-place repair on the next serialized attempt.",
												}
											: {
													note: "The board copilot produced a FlowScript draft with validation errors — nothing was applied. Continue repairing this retained draft instead of replacing it with a test stub.",
												}),
									}
								: {}),
							...(applyFailed && workspaceStatus !== "validation_errors"
								? {
										flowscript_status: staleSnapshotBlocked
											? "stale_snapshot"
											: persistedReadbackFailed
												? "readback_mismatch"
												: "apply_failed",
										diagnostics: diagnostics.slice(0, 5),
										note: staleSnapshotBlocked
											? "The board changed while the specialist was running, so the stale draft was not applied. Retry once; the next run will start from the fresh persisted board."
											: persistedReadbackFailed
												? "Commands were returned, but persisted FlowScript readback did not match the validated workspace. Do not claim success; inspect the diagnostics and retry from the persisted board."
												: hadReturnedCommands
													? `The board copilot returned ${returnedCommandCount} validated command${returnedCommandCount === 1 ? "" : "s"}, but they could not be applied. Report this honestly; do not claim the workflow was built.`
													: "The FlowScript draft could not be applied to the board — report the diagnostics honestly and consider retrying with a clearer instruction.",
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
						if (
							resultStatus === "ok" &&
							!partialWorkingSlice &&
							persistedReadbackVerified
						) {
							boardRecoveryRef.current.delete(boardRecoveryKey);
						} else {
							const recoverable =
								selectBestRecoverableFlowScriptCandidate(workspaceCandidates);
							if (recoverable) {
								boardRecoveryRef.current.set(
									boardRecoveryKey,
									recoverable,
									baselineFingerprint,
								);
							}
						}
						recordNestedDebug(
							request,
							nestedAgentRunEvent({
								requestId: nestedRunRequestId,
								parentRequestId: request.requestId,
								toolName: "flowpilot_board",
								stage: "finished",
								status: resultStatus,
								output: result,
								summary:
									resultStatus === "ok"
										? "Delegated board build finished and was validated."
										: "Delegated board build finished without a valid complete apply.",
							}),
						);
						nestedRunSettled = true;
						return result;
					} finally {
						if (draftingWorkspaceTimer) clearTimeout(draftingWorkspaceTimer);
						draftingWorkspaceTimer = undefined;
						pendingDraftingWorkspace = undefined;
						boardRecoveryScopeByRequestRef.current.delete(request.requestId);
						releaseBoardScopedEdit?.();
						releaseBoardEdit?.();
					}
				}
				case "flowpilot_widget": {
					const instruction = argString(args, "instruction");
					if (!instruction)
						return {
							status: "error",
							message: "flowpilot_widget requires an instruction.",
						};
					// Edit mode targets the OPEN builder surface. When none is open we create a NEW
					// board-scoped page from scratch (mirrors how flowpilot_board bootstraps a board).
					const widgetSurface = useAssistantSurface.getState().widgetSurface;
					const createMode = !widgetSurface;
					const appId =
						widgetSurface?.appId ||
						argString(args, "app_id") ||
						argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: createMode
								? "No widget/page builder is open. To create a NEW page pass app_id (from list_apps/create_app); otherwise ask the user to open a builder first."
								: "The open widget/page builder has no app scope. Reopen it from an app before using FlowPilot.",
						};
					const targetAppId = appId;
					let boardId =
						argString(args, "board_id") || argString(args, "boardId");
					let createdBoard = false;
					const widgetIdempotencyKey =
						argString(args, "idempotency_key") ||
						argString(args, "idempotencyKey");
					const widgetConversationId = conversationScopeId(request);
					const widgetCreationIdentity =
						createMode && widgetConversationId
							? {
									conversationId: widgetConversationId,
									toolName: "flowpilot_widget",
									scope: targetAppId,
									instruction,
									...(widgetIdempotencyKey
										? { idempotencyKey: widgetIdempotencyKey }
										: {}),
								}
							: undefined;
					if (widgetCreationIdentity) {
						const journaledPage = createdArtifactJournalRef.current.find(
							widgetCreationIdentity,
						);
						if (journaledPage?.artifacts.pageId) {
							referenceApp(targetAppId);
							return {
								status: "ok",
								already_created: true,
								app_id: targetAppId,
								...(journaledPage.artifacts.boardId
									? { board_id: journaledPage.artifacts.boardId }
									: {}),
								page: { id: journaledPage.artifacts.pageId },
								widgets: (journaledPage.artifacts.widgetIds ?? []).map(
									(id) => ({ id }),
								),
								note: "A page for this exact request was already created earlier in this conversation; its ids are returned instead of creating a duplicate. Wire or edit that page instead. Only if the user truly wants a second, separate page, call flowpilot_widget again with a distinct `idempotency_key`.",
							};
						}
					}
					if (createMode && !boardId) {
						const boards = await backend.boardState.getBoards(targetAppId);
						boardId = boards?.[0]?.id ?? "";
					}

					// Run the widget copilot as a sub-agent, using the global chat's selected model.
					const chat = useGlobalChatStore.getState();
					const owningUserPrompt = sourceUserPrompt(request);
					const owningConversationId = conversationScopeId(request);
					const rawSpecialistPrompt = composeDelegatedRawUserPrompt(
						owningUserPrompt,
						instruction,
					);
					const modelId = flowPilotModelIdForProvider(
						normalizeAIProvider(chat.provider),
						chat.selectedModelId,
					);

					const nestedRunRequestId = `${request.requestId}:agent`;
					const {
						pushSubRunChunk,
						flushSubRunStream,
						subAcc,
						runIsLive,
						publishSubSteps,
						failProgressSteps,
					} = createSubRunStream({
						requestId: nestedRunRequestId,
						parentRequestId: request.requestId,
						recordDebugEvent: (event) => recordNestedDebug(request, event),
					});
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
					const consumeSubRunEvents = (
						events: ReturnType<typeof pushSubRunChunk>,
					) => {
						let stepsChanged = false;
						for (const event of events) {
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
					const onToken = (chunk: string) =>
						consumeSubRunEvents(pushSubRunChunk(chunk));
					let subRunFlushed = false;
					const flushSubRun = () => {
						if (subRunFlushed) return;
						subRunFlushed = true;
						consumeSubRunEvents(flushSubRunStream());
					};

					let response: Awaited<
						ReturnType<typeof backend.boardState.copilot_chat>
					>;
					recordNestedDebug(
						request,
						nestedAgentRunEvent({
							requestId: nestedRunRequestId,
							parentRequestId: request.requestId,
							toolName: "flowpilot_widget",
							stage: "started",
							input: {
								scope: "Frontend",
								app_id: appId,
								board_id: boardId,
								instruction,
								create_mode: createMode,
								selected_component_ids:
									widgetSurface?.selectedComponentIds ?? [],
								current_components: widgetSurface?.currentComponents ?? [],
							},
							summary: "Delegated UI sub-agent started.",
						}),
					);
					let widgetRunSettled = false;
					const finishWidgetRun = <T extends Record<string, unknown>>(
						result: T,
					) => {
						const status = String(result.status ?? "error");
						recordNestedDebug(
							request,
							nestedAgentRunEvent({
								requestId: nestedRunRequestId,
								parentRequestId: request.requestId,
								toolName: "flowpilot_widget",
								stage: "finished",
								status,
								output: result,
								summary:
									status === "ok"
										? "Delegated UI build finished."
										: "Delegated UI build did not produce an applicable result.",
							}),
						);
						widgetRunSettled = true;
						if (status !== "ok") failProgressSteps();
						return result;
					};
					try {
						response = await backend.boardState.copilot_chat(
							"Frontend",
							null,
							undefined,
							[],
							widgetSurface?.currentComponents ?? [],
							widgetSurface?.selectedComponentIds ?? [],
							instruction,
							[],
							undefined /* images */,
							onToken,
							modelId,
							chat.reasoningEffort || undefined,
							undefined /* token */,
							undefined /* runContext */,
							undefined /* actionContext */,
							true /* nested: isolate from the pending parent session */,
							undefined /* readOnly */,
							{
								appId,
								boardId,
								parentRequestId: request.requestId,
								conversationId: owningConversationId,
								sourceUserPrompt: owningUserPrompt,
							},
							nestedRunRequestId,
							rawSpecialistPrompt,
							appId,
						);
						flushSubRun();
					} catch (error) {
						flushSubRun();
						if (!widgetRunSettled) {
							recordNestedDebug(
								request,
								nestedAgentRunEvent({
									requestId: nestedRunRequestId,
									parentRequestId: request.requestId,
									toolName: "flowpilot_widget",
									stage: "finished",
									status: "error",
									error,
									summary: "Delegated UI sub-agent failed.",
								}),
							);
						}
						failProgressSteps();
						throw error;
					}

					const finalComponents = collectComponents(response.components);
					const components =
						finalComponents.length > 0 ? finalComponents : streamedComponents;
					canvasSettings =
						validateCanvasSettings(response.canvas_settings) ?? canvasSettings;

					if (components.length === 0)
						return finishWidgetRun({
							status: "error",
							message: response.message,
							component_count: 0,
							note: "IMPORTANT: the widget copilot ended WITHOUT generating any UI components — nothing was changed. Do not tell the user the UI was built; retry once with a clearer instruction or tell the user honestly that nothing was generated.",
						});

					// Close the run with a summary step, like the board case's FlowScript step.
					subAcc.stepOrder.push("components");
					subAcc.steps.set("components", {
						id: "components",
						title: "UI components",
						description: `${components.length} component${components.length === 1 ? "" : "s"} ${createMode ? "generated" : "ready for review"}`,
						status: "done",
						timestamp: Date.now(),
					});
					publishSubSteps();

					if (createMode) {
						// A page is board-scoped: reuse the app's board or create one (like
						// flowpilot_board) so the page's logic can be wired next.
						if (!boardId) {
							boardId = createId();
							await backend.boardState.upsertBoard(
								targetAppId,
								boardId,
								argString(args, "board_name") || "Main Board",
								instruction.slice(0, 140),
								ILogLevel.Debug,
								IExecutionStage.Dev,
							);
							createdBoard = true;
						}

						// Persist each reusable widget the copilot embedded inline, and point the
						// page's instances at the saved widget via widgetRefs (keyed by instance id).
						const inlineWidgets = collectInlineWidgets(components);
						const widgetRefs: Record<string, unknown> = {};
						const realIdByCopilotId = new Map<string, string>();
						const widgetByRealId = new Map<string, unknown>();
						// Concrete ids/names/action-ids so the orchestrator can reference these
						// widgets when it calls flowpilot_board to wire the logic.
						const createdWidgets: Array<{
							id: string;
							name: string;
							action_ids: string[];
						}> = [];
						try {
							for (const iw of inlineWidgets) {
								let realId = realIdByCopilotId.get(iw.copilotWidgetId);
								if (!realId) {
									realId = createId();
									const widgetName =
										typeof iw.inlineDef.name === "string"
											? iw.inlineDef.name
											: "Widget";
									const widget = await backend.widgetState.createWidget(
										targetAppId,
										realId,
										widgetName,
									);
									widget.components = ensureRootId(
										collectComponents(iw.inlineDef.components),
									);
									widget.rootComponentId = "root";
									if (Array.isArray(iw.inlineDef.exposedProps))
										(widget as { exposedProps?: unknown }).exposedProps =
											iw.inlineDef.exposedProps;
									if (Array.isArray(iw.inlineDef.actions))
										(widget as { actions?: unknown }).actions =
											iw.inlineDef.actions;
									await backend.widgetState.updateWidget(targetAppId, widget);
									realIdByCopilotId.set(iw.copilotWidgetId, realId);
									widgetByRealId.set(realId, widget);
									const actionIds = Array.isArray(iw.inlineDef.actions)
										? (iw.inlineDef.actions as Array<Record<string, unknown>>)
												.map((action) =>
													typeof action?.id === "string" ? action.id : "",
												)
												.filter(Boolean)
										: [];
									createdWidgets.push({
										id: realId,
										name: widgetName,
										action_ids: actionIds,
									});
								}
								// Point the instance at the saved widget and drop the redundant inline def.
								iw.component.widgetId = realId;
								iw.component.instanceId = iw.instanceId;
								iw.component.appId = targetAppId;
								iw.component.inlineWidgetDef = undefined;
								widgetRefs[iw.instanceId] = widgetByRealId.get(realId);
							}
						} catch (error) {
							return finishWidgetRun({
								status: "error",
								message: `Failed to create the page's widgets: ${error instanceof Error ? error.message : String(error)}`,
							});
						}

						const pageId = createId();
						const pageName =
							argString(args, "page_name") ||
							argString(args, "name") ||
							"New Page";
						const route = slugifyRoute(argString(args, "route") || pageName);
						try {
							const page = await backend.pageState.createPage(
								targetAppId,
								pageId,
								pageName,
								route,
								boardId,
							);
							page.components = ensureRootId(components);
							if (canvasSettings) page.canvasSettings = canvasSettings;
							if (Object.keys(widgetRefs).length > 0)
								(page as { widgetRefs?: unknown }).widgetRefs = widgetRefs;
							await backend.pageState.updatePage(targetAppId, page);
						} catch (error) {
							return finishWidgetRun({
								status: "error",
								message: `Failed to create the page: ${error instanceof Error ? error.message : String(error)}`,
							});
						}

						referenceApp(targetAppId);
						if (widgetCreationIdentity) {
							createdArtifactJournalRef.current.record(
								widgetCreationIdentity,
								{
									appId: targetAppId,
									boardId,
									pageId,
									...(createdWidgets.length > 0
										? { widgetIds: createdWidgets.map((widget) => widget.id) }
										: {}),
								},
								request.requestId,
							);
						}
						// Defer the navigation: router.push mid-stream tears down the run. The bridge
						// navigates once the agent turn ends.
						useGlobalChatStore
							.getState()
							.setPendingNavigation(
								`/page-builder?id=${pageId}&app=${targetAppId}&board=${boardId}`,
							);
						return finishWidgetRun({
							status: "ok",
							message: response.message,
							component_count: components.length,
							app_id: targetAppId,
							board_id: boardId,
							page: { id: pageId, name: pageName, route },
							widgets: createdWidgets,
							...(createdBoard ? { created_board_id: boardId } : {}),
							note: "Created a new page (and any reusable widgets it needs), applied the UI, and scheduled the page builder to open after this agent turn. To wire the logic, call flowpilot_board with this app_id and reference the page (route) and these widgets/action_ids in the instruction.",
						});
					}

					// Edit mode: stage for the user's inline review. The tool never applies this
					// itself; only the review card does, either on a click or via auto mode.
					let staged = false;
					if (runIsLive() && widgetSurface) {
						useGlobalChatStore.getState().setPendingComponents({
							components,
							canvasSettings,
							warnings: warnings.length > 0 ? warnings : undefined,
							surfaceId: widgetSurface.surfaceId,
							appId: widgetSurface.appId,
						});
						staged = true;
					}
					if (widgetSurface?.appId) referenceApp(widgetSurface.appId);
					if (!staged)
						return finishWidgetRun({
							status: "error",
							message: response.message,
							component_count: components.length,
							staged: false,
							note: "IMPORTANT: components were generated but the conversation moved on before they could be staged — they were DISCARDED and there is no review card. Do not tell the user to review anything; offer to regenerate.",
						});
					return finishWidgetRun({
						status: "ok",
						message: response.message,
						component_count: components.length,
						staged: true,
						note: "Components are pending user review in the chat — they are NOT applied yet. Tell the user to review and apply them.",
					});
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

					// Hand the user's attached files to the app chat. The assistant selects which files
					// via `forward_files` (exact names from the FILES ATTACHED THIS TURN manifest):
					// omitted → forward all (safe default so a needed file is never silently dropped);
					// an explicit list forwards only the named files; [] forwards none.
					const currentTurnFiles = (() => {
						const msgs = useGlobalChatStore.getState().messages;
						for (let i = msgs.length - 1; i >= 0; i--) {
							if (msgs[i]?.inner.role === IRole.User)
								return msgs[i].files ?? [];
						}
						return [];
					})();
					const requestedFileNames = Array.isArray(args.forward_files)
						? (args.forward_files as unknown[])
								.filter((value): value is string => typeof value === "string")
								.map((value) => value.trim().toLowerCase())
								.filter((value) => value.length > 0)
						: undefined;
					const attachmentLabels = (file: IAttachment): string[] => {
						const raw =
							typeof file === "string"
								? [file]
								: [file.url, file.name].filter((value): value is string =>
										Boolean(value),
									);
						const withBasenames = raw.flatMap((value) => {
							const basename = value.split("?")[0]?.split("/").pop();
							return basename && basename !== value
								? [value, basename]
								: [value];
						});
						return withBasenames.map((value) => value.toLowerCase());
					};
					const forwardedAttachments =
						requestedFileNames === undefined
							? currentTurnFiles
							: currentTurnFiles.filter((file) =>
									attachmentLabels(file).some((label) =>
										requestedFileNames.includes(label),
									),
								);

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
					// Capture any a2ui UI the app chat pushes (event_type "a2ui"). The app builds this
					// UI for the user, not the model — processChatEvents ignores it, so we fold the
					// pushes into surfaces here and render them as an inline card after the run. The
					// component tree is NEVER returned to the assistant (it only gets text/attachments).
					let pushedSurfaces = new Map<string, Surface>();

					// Surface the app chat's own plan steps as nested "↳" sub-steps in the
					// global chat, the same way flowpilot_board/flowpilot_widget fold their
					// sub-run activity into the owning message. Without this the user only
					// sees the outer call_app_chat step, never the app agent's inner work.
					const { subAcc, publishSubSteps, failProgressSteps } =
						createSubRunStream({
							requestId: `${request.requestId}:app-chat`,
							parentRequestId: request.requestId,
							recordDebugEvent: (event) => recordNestedDebug(request, event),
						});
					const syncSubSteps = () => {
						const steps = responseMessage.plan_steps;
						if (!steps?.length) return;
						for (const step of steps) {
							if (!subAcc.steps.has(step.id)) subAcc.stepOrder.push(step.id);
							subAcc.steps.set(step.id, step);
						}
						publishSubSteps();
					};

					// Widgets the app pushes must keep executing against THEIR board once
					// embedded in the global chat — tag each with the pushing run's
					// context so widget actions route to the original use-case board.
					const widgetOrigin = {
						appId,
						boardId: chatEvent.board_id,
						eventId: chatEvent.id,
					};
					const publishWidgets = () => {
						const widgets = responseMessage.widgets;
						if (!widgets?.length) return;
						useGlobalChatStore
							.getState()
							.addSubWidgets(
								widgets.map((widget) => ({ ...widget, origin: widgetOrigin })),
							);
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
								for (const event of batch) {
									if (event?.event_type === "a2ui" && event.payload) {
										pushedSurfaces = foldA2UIServerMessage(
											pushedSurfaces,
											event.payload as A2UIServerMessage,
										);
									}
								}
								publishWidgets();
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

					// Push the app's UI through to the user as an inline card (display only). Best-effort
					// app name for the header; the surface tree stays entirely on the UI side.
					if (pushedSurfaces.size > 0) {
						const appName = await backend.appState
							.getAppMeta(appId)
							.then((meta) => meta?.name)
							.catch(() => undefined);
						useGlobalChatStore.getState().addInlineAppSurface({
							appId,
							name: appName || chatEvent.name || appId,
							surfaces: Array.from(pushedSurfaces.values()),
						});
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

					const forwardedFileNames = forwardedAttachments.map((file) =>
						typeof file === "string" ? file : (file.name ?? file.url),
					);
					const embeddedWidgetCount = responseMessage.widgets?.length ?? 0;

					referenceApp(appId);
					return {
						status: "ok",
						app_id: appId,
						response: text || "(the app chat returned no text)",
						forwarded_files:
							forwardedFileNames.length > 0 ? forwardedFileNames : undefined,
						attachments:
							attachmentSummaries.length > 0 ? attachmentSummaries : undefined,
						embedded_widgets:
							embeddedWidgetCount > 0 ? embeddedWidgetCount : undefined,
						note:
							embeddedWidgetCount > 0
								? `The app pushed ${embeddedWidgetCount} interactive widget(s) that are already embedded and visible in your reply — do not describe or re-create their content, just reference them.`
								: undefined,
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
			backend.helperState.fileToTemporaryFile,
			backend.helperState.fileToUrl,
			backend.userState,
			backend.pageState,
			backend.widgetState,
			backend.routeState,
			executeRuntimeTool,
			queryClient,
			showConversation,
			addInlineAppChat,
			openDialog,
			recordNestedDebug,
			assertRequestActive,
			isRequestExpired,
			markRequestExpired,
			ownerMessageIdForRequest,
		],
	);

	const execute = useCallback(
		async (request: FrontendToolRequest): Promise<FrontendToolResponse> => {
			try {
				assertRequestActive(request, "approval handling");
				if (request.toolName === "ask_user") {
					const resolution = await openDialog({ type: "ask", request });
					assertRequestActive(request, "question response");
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

				const ownerMessageId = ownerMessageIdForRequest(request);
				const createdAppId = ownerMessageId
					? createdAppTargetsByOwnerRef.current.get(ownerMessageId)
					: undefined;
				const requestedAppId =
					argString(request.arguments, "app_id") ||
					argString(request.arguments, "appId") ||
					undefined;
				if (
					isCreatedAppBuildTargetMismatch({
						createdAppId,
						requestedAppId,
						toolName: request.toolName,
						mode: argString(request.arguments, "mode"),
						operation: argString(request.arguments, "operation"),
					})
				) {
					return {
						requestId: request.requestId,
						approved: true,
						result: {
							status: "error",
							code: "created_app_target_mismatch",
							created_app_id: createdAppId,
							requested_app_id: requestedAppId,
							next_action: `Retry ${request.toolName} with app_id '${createdAppId}'.`,
							message: `This turn created app '${createdAppId}'. Refusing to mutate older app '${requestedAppId}' after a transient failure. Continue the build on the exact app_id returned by create_app.`,
						},
					};
				}

				const approval = request.approval;
				const sessionKey =
					approval?.sessionKey ||
					`${request.toolName}:${approval?.kind ?? "none"}`;
				// Read through getState() rather than a selector so `execute` keeps a stable
				// identity. Auto mode is a frontend waiver only: the approval kind sent by the
				// backend is untouched, so ordered execution of mutating tools still holds.
				const needsApproval =
					!useGlobalChatStore.getState().autoMode &&
					(approval?.kind === "mutating" || approval?.kind === "execute") &&
					!shouldSkipUnavailableCreateTableApproval(
						request.toolName,
						request.arguments,
					);

				if (needsApproval && !approvedKeysRef.current.has(sessionKey)) {
					const outcome = await openDialog({ type: "approval", request });
					assertRequestActive(request, "approval response");
					if (!outcome || !("approved" in outcome) || !outcome.approved) {
						return {
							requestId: request.requestId,
							approved: false,
							error: "User denied the request.",
						};
					}
					if (outcome.remember) approvedKeysRef.current.add(sessionKey);
				}

				assertRequestActive(request, "tool mutation");
				const result = await runTool(request);
				assertRequestActive(request, "tool completion");
				return { requestId: request.requestId, approved: true, result };
			} catch (error) {
				// approved:true + error => the bridge reports status:"error" (not a user denial).
				return {
					requestId: request.requestId,
					approved: true,
					error: getErrorMessage(error, "Frontend tool execution failed."),
				};
			}
		},
		[assertRequestActive, openDialog, ownerMessageIdForRequest, runTool],
	);

	const executeWithDiagnostics = useCallback(
		async (request: FrontendToolRequest): Promise<FrontendToolResponse> => {
			const ownerMessageId =
				ownerMessageIdForRequest(request) ??
				useGlobalChatStore.getState().streamingMessage?.id;
			if (ownerMessageId) {
				rememberRequestOwner(request.requestId, ownerMessageId);
			}
			const startedAt = Date.now();
			recordRequestDebug(request, {
				id: `frontend:${request.requestId}:request`,
				kind: "bridge",
				stage: "request_received",
				status: "progress",
				name: request.toolName,
				started_at_ms: startedAt,
				arguments_preview: agentDebugPreview(request.arguments),
				summary: parentRequestId(request)
					? "Nested frontend tool request received."
					: "Frontend tool request received.",
			});

			const deadline = requestDeadline(request);
			const executionLease = requestExecutionFenceRef.current.begin({
				request,
				requestId: request.requestId,
				parentRequestId: parentRequestId(request),
			});
			requestExecutionLeasesRef.current.set(request, executionLease);
			const execution = execute(request);
			void execution.then(
				(lateResult) => {
					if (requestExecutionFenceRef.current.isInvalidated(executionLease)) {
						recordRequestDebug(request, {
							id: `frontend:${request.requestId}:late-completion`,
							kind: "bridge",
							stage: "late_completion_discarded",
							status: "cancelled",
							name: request.toolName,
							ended_at_ms: Date.now(),
							result_preview: agentDebugPreview(lateResult),
							summary:
								"The expired request finished later; its response was discarded and guarded side effects were blocked.",
						});
					}
					requestExecutionFenceRef.current.settle(executionLease);
				},
				(error) => {
					if (requestExecutionFenceRef.current.isInvalidated(executionLease)) {
						recordRequestDebug(request, {
							id: `frontend:${request.requestId}:late-completion`,
							kind: "bridge",
							stage: "late_completion_failed",
							status: "cancelled",
							name: request.toolName,
							ended_at_ms: Date.now(),
							error: error instanceof Error ? error.message : String(error),
						});
					}
					requestExecutionFenceRef.current.settle(executionLease);
				},
			);
			let timeoutId: ReturnType<typeof setTimeout> | undefined;
			let response: FrontendToolResponse;
			if (deadline !== undefined) {
				const remaining = Math.max(0, deadline - Date.now());
				const timeoutResponse = new Promise<FrontendToolResponse>((resolve) => {
					timeoutId = setTimeout(() => {
						markRequestExpired(request.requestId);
						const reason = `Frontend execution deadline exceeded after ${Date.now() - startedAt} ms; the request was cancelled before the backend bridge timeout.`;
						const recoveryScope = boardRecoveryScopeByRequestRef.current.get(
							request.requestId,
						);
						const retainedCandidate = recoveryScope?.baselineFingerprint
							? boardRecoveryRef.current.get(
									recoveryScope.key,
									recoveryScope.baselineFingerprint,
								)
							: undefined;
						const timeoutResult = boardEditInterruptionResult({
							status: "timeout",
							code: "frontend_execution_deadline",
							message: reason,
							candidate: retainedCandidate,
						});
						if (
							request.toolName === "flowpilot_board" &&
							argString(request.arguments, "mode") !== "explain" &&
							recoveryScope
						) {
							const ownerId =
								ownerMessageIdForRequest(request) ??
								parentRequestId(request) ??
								request.requestId;
							boardZeroProgressRetryRef.current.recordRunOutcome(
								ownerId,
								recoveryScope.key,
								request.requestId,
								Boolean(retainedCandidate),
							);
						}
						cancelRequestDialogs(request.requestId, reason);
						if (
							(request.toolName === "flowpilot_board" ||
								request.toolName === "flowpilot_widget") &&
							backend.boardState.cancelCopilotChat
						) {
							void backend.boardState
								.cancelCopilotChat(`${request.requestId}:agent`)
								.catch((error) =>
									console.warn(
										"[global-tool-bridge] failed to cancel timed-out copilot chat",
										error,
									),
								);
						}
						recordRequestDebug(request, {
							id: `frontend:${request.requestId}:request`,
							kind: "bridge",
							stage: "request_timeout",
							status: "timeout",
							name: request.toolName,
							ended_at_ms: Date.now(),
							error: reason,
						});
						if (request.toolName === "flowpilot_board") {
							recordNestedDebug(
								request,
								nestedAgentRunEvent({
									requestId: `${request.requestId}:agent`,
									parentRequestId: request.requestId,
									toolName: "flowpilot_board",
									stage: "finished",
									status: "timeout",
									output: timeoutResult,
									error: reason,
									summary:
										"Delegated board run reached the frontend deadline; the best candidate was retained.",
								}),
							);
						}
						resolve({
							requestId: request.requestId,
							approved: true,
							result: timeoutResult,
						});
					}, remaining);
				});
				response = await Promise.race([execution, timeoutResponse]);
			} else {
				response = await execution;
			}
			if (timeoutId !== undefined) clearTimeout(timeoutId);

			response = { ...response, requestId: request.requestId };
			boardRecoveryScopeByRequestRef.current.delete(request.requestId);
			const resultRecord =
				response.result && typeof response.result === "object"
					? (response.result as Record<string, unknown>)
					: undefined;
			const resultStatus = String(resultRecord?.status ?? "").toLowerCase();
			const resultTimedOut = ["timeout", "timed_out"].includes(resultStatus);
			const resultFailed = [
				"error",
				"failed",
				"failure",
				"validation_error",
				"validation_errors",
			].includes(resultStatus);
			const resultDenied = resultStatus === "denied";
			const resultCancelled = ["cancelled", "canceled"].includes(resultStatus);
			const resultPartial = resultStatus === "partial";
			const requestTimedOut =
				resultTimedOut ||
				(Boolean(response.error) &&
					requestExecutionFenceRef.current.isInvalidated(executionLease));
			const requestStatus =
				!response.approved || resultDenied
					? "denied"
					: requestTimedOut
						? "timeout"
						: response.error || resultFailed
							? "error"
							: resultCancelled
								? "cancelled"
								: resultPartial
									? "partial"
									: "done";
			const requestStage =
				requestStatus === "denied"
					? "request_denied"
					: requestStatus === "timeout"
						? "request_timeout"
						: requestStatus === "error"
							? "request_failed"
							: requestStatus === "cancelled"
								? "request_cancelled"
								: requestStatus === "partial"
									? "request_partial"
									: "request_completed";
			recordRequestDebug(request, {
				id: `frontend:${request.requestId}:request`,
				kind: "bridge",
				stage: requestStage,
				status: requestStatus,
				name: request.toolName,
				ended_at_ms: Date.now(),
				result_summary:
					!response.approved || resultDenied
						? response.error || "Frontend tool was denied."
						: response.error || resultFailed || requestTimedOut
							? undefined
							: resultCancelled
								? "Frontend tool was cancelled."
								: resultPartial
									? "Frontend tool completed partially."
									: response.approved
										? "Frontend tool completed."
										: "Frontend tool was denied.",
				result_preview: agentDebugPreview(response.result),
				error: response.approved ? response.error : undefined,
			});
			return response;
		},
		[
			backend.boardState,
			cancelRequestDialogs,
			execute,
			markRequestExpired,
			ownerMessageIdForRequest,
			recordNestedDebug,
			recordRequestDebug,
			rememberRequestOwner,
		],
	);

	useEffect(() => {
		executeRef.current = executeWithDiagnostics;
	}, [executeWithDiagnostics]);

	// Expose the executor to the web transport, which receives tool requests inside the chat SSE
	// stream (no Tauri event channel). Desktop also registers it harmlessly; the Tauri listener below
	// is what actually drives tools there.
	useEffect(() => {
		registerGlobalChatToolExecutor((request) => executeRef.current(request));
		return () => registerGlobalChatToolExecutor(null);
	}, []);

	useEffect(() => {
		if (typeof window === "undefined") return;
		let disposed = false;
		let unlisten: Array<() => void> = [];

		void (async () => {
			try {
				const [{ listen }, { invoke }] = await Promise.all([
					import("@tauri-apps/api/event"),
					import("@tauri-apps/api/core"),
				]);
				const [stopRequests, stopCancellation, stopLifecycle] =
					await Promise.all([
						listen<FrontendToolRequest>(
							GLOBAL_FRONTEND_TOOL_EVENT,
							async (event) => {
								const request = event.payload;
								if (!request?.requestId || !request.toolName) {
									const messageId =
										useGlobalChatStore.getState().streamingMessage?.id;
									if (messageId) {
										useGlobalChatStore.getState().recordDebugEvent(messageId, {
											id: `frontend:malformed:${Date.now()}`,
											kind: "bridge",
											stage: "malformed_request",
											status: "error",
											timestamp_ms: Date.now(),
											error:
												"Tauri emitted a frontend tool request without requestId or toolName.",
											arguments_preview: agentDebugPreview(request),
										});
									}
									if (request?.requestId) {
										const response: FrontendToolResponse = {
											requestId: request.requestId,
											approved: true,
											error:
												"Malformed frontend tool request: missing toolName.",
										};
										try {
											await invoke("flowpilot_frontend_tool_result", {
												response,
											});
										} catch (error) {
											console.error(
												"[global-tool-bridge] failed to reject malformed request",
												error,
											);
										}
									}
									return;
								}
								flowPilotDebugLog(
									"[global-tool-bridge] request",
									request.toolName,
									request.requestId,
								);
								const response = await executeRef.current(request);
								flowPilotDebugLog(
									"[global-tool-bridge] responding",
									request.toolName,
									request.requestId,
									{ approved: response.approved, error: response.error },
								);
								try {
									await invoke("flowpilot_frontend_tool_result", { response });
									recordRequestDebug(request, {
										id: `frontend:${request.requestId}:delivery`,
										kind: "bridge",
										stage: "response_delivered",
										status: "done",
										name: request.toolName,
										ended_at_ms: Date.now(),
									});
								} catch (error) {
									recordRequestDebug(request, {
										id: `frontend:${request.requestId}:delivery`,
										kind: "bridge",
										stage: "response_delivery_failed",
										status: "error",
										name: request.toolName,
										ended_at_ms: Date.now(),
										error:
											error instanceof Error ? error.message : String(error),
									});
									console.error(
										"[global-tool-bridge] response delivery failed",
										request.requestId,
										error,
									);
								}
							},
						),
						listen<{
							requestId?: string;
							reason?: string;
							toolName?: string;
							parentRequestId?: string;
						}>(GLOBAL_FRONTEND_TOOL_CANCEL_EVENT, (event) => {
							const requestId = event.payload?.requestId;
							if (!requestId) return;
							const activeRequest =
								requestExecutionFenceRef.current.getLatest(requestId)?.request;
							const cancelledToolName =
								event.payload.toolName ?? activeRequest?.toolName;
							markRequestExpired(requestId);
							const synthetic: FrontendToolRequest = {
								requestId,
								toolName: cancelledToolName ?? "frontend_tool",
								arguments: {},
								parentRequestId: event.payload.parentRequestId,
							};
							recordRequestDebug(synthetic, {
								id: `frontend:${requestId}:cancellation`,
								kind: "bridge",
								stage: "backend_cancelled",
								status: "cancelled",
								name: synthetic.toolName,
								ended_at_ms: Date.now(),
								error:
									event.payload.reason ??
									"Backend cancelled the frontend request.",
							});
							cancelRequestDialogs(
								requestId,
								event.payload.reason ??
									"Backend cancelled the frontend request.",
							);
							// Aborting the renderer-side controller releases the board lease, but it does
							// not stop a Tauri copilot invocation already running underneath it. Cancel the
							// correlated native sub-run as well so it cannot continue issuing tools or
							// mutate after its owning MCP request disappeared.
							if (
								(cancelledToolName === "flowpilot_board" ||
									cancelledToolName === "flowpilot_widget") &&
								backend.boardState.cancelCopilotChat
							) {
								void backend.boardState
									.cancelCopilotChat(`${requestId}:agent`)
									.catch((error) =>
										console.warn(
											"[global-tool-bridge] failed to cancel backend-cancelled copilot chat",
											error,
										),
									);
							}
						}),
						listen<Record<string, unknown>>(
							GLOBAL_FRONTEND_TOOL_LIFECYCLE_EVENT,
							(event) => {
								const payload = event.payload ?? {};
								const requestId =
									typeof payload.requestId === "string"
										? payload.requestId
										: "unknown";
								const synthetic: FrontendToolRequest = {
									requestId,
									toolName:
										typeof payload.toolName === "string"
											? payload.toolName
											: "frontend_tool",
									arguments: {},
									parentRequestId:
										typeof payload.parentRequestId === "string"
											? payload.parentRequestId
											: undefined,
								};
								recordRequestDebug(synthetic, {
									id: `frontend:${requestId}:backend-lifecycle`,
									kind: "bridge",
									stage:
										typeof payload.phase === "string"
											? `backend_${payload.phase}`
											: "backend_lifecycle",
									status:
										typeof payload.outcome === "string"
											? payload.outcome
											: undefined,
									name: synthetic.toolName,
									ended_at_ms: Date.now(),
									result_preview: agentDebugPreview(payload),
								});
							},
						),
					]);
				const stops = [stopRequests, stopCancellation, stopLifecycle];
				if (disposed) {
					for (const stop of stops) stop();
				} else unlisten = stops;
			} catch (error) {
				// Not running under Tauri (e.g. web build) — the global tool bridge is desktop-only.
				if (
					typeof navigator !== "undefined" &&
					navigator.userAgent.toLowerCase().includes("tauri")
				) {
					console.error("[global-tool-bridge] listener setup failed", error);
				}
			}
		})();

		return () => {
			disposed = true;
			for (const stop of unlisten) stop();
			for (const timer of requestOwnerCleanupTimersRef.current.values()) {
				clearTimeout(timer);
			}
			requestOwnerCleanupTimersRef.current.clear();
			const activeRequests =
				requestExecutionFenceRef.current.activeExecutions();
			const pendingDialogRequestIds = new Set<string>();
			if (resolverRef.current) {
				pendingDialogRequestIds.add(resolverRef.current.request.requestId);
			}
			for (const queued of dialogQueueRef.current) {
				pendingDialogRequestIds.add(queued.dialog.request.requestId);
			}

			for (const { request, controller } of activeRequests) {
				markRequestExpired(request.requestId);
				controller.abort();
				pendingDialogRequestIds.add(request.requestId);
				if (
					(request.toolName === "flowpilot_board" ||
						request.toolName === "flowpilot_widget") &&
					backend.boardState.cancelCopilotChat
				) {
					void backend.boardState
						.cancelCopilotChat(`${request.requestId}:agent`)
						.catch((error) =>
							console.warn(
								"[global-tool-bridge] failed to cancel unmounted copilot chat",
								error,
							),
						);
				}
			}
			// Approval and ask-user prompts are promises owned by this bridge. Resolving every
			// active/queued dialog on unmount lets the in-flight listener return an explicit
			// cancellation instead of stranding the native request until its 10-minute timeout.
			for (const requestId of pendingDialogRequestIds) {
				cancelRequestDialogs(
					requestId,
					"Global tool bridge unmounted before the dialog was answered.",
				);
			}
			setToolPrompt(null);
		};
	}, [
		backend.boardState,
		cancelRequestDialogs,
		markRequestExpired,
		recordRequestDebug,
		setToolPrompt,
	]);

	// The pending prompt is rendered inline by the chat surfaces (InlineToolPrompt) via the store.
	return null;
}
