import { createId } from "@paralleldrive/cuid2";
import { create } from "zustand";
import type {
	CanvasSettings,
	Surface,
	SurfaceComponent,
} from "../../components/a2ui/types";
import type { FlowScriptWorkspaceCandidate } from "../../components/flowpilot/flowscript-workspace-candidates";
import type { AIProvider } from "../../components/flowpilot/types";
import type {
	IAttachment,
	IChatUsageStat,
	IChatWidget,
	IPlanStep,
} from "../../components/interfaces/chat-default/chat-db";
import { mergeChatWidgets } from "../../components/interfaces/chat-default/event-processor";
import type { AskUserChoice, AskUserForm } from "../../lib/ask-user";
import { FLOWPILOT_DEBUG_ENABLED } from "../../lib/flowpilot-debug";
import type { IInteractionRequest } from "../../lib/schema/interaction";
import {
	type AgentDebugReportMetadata,
	type IAgentDebugEvent,
	type IAgentDebugReport,
	beginAgentGenerationMetrics,
	clearAgentGenerationMetrics,
	createAgentDebugReport,
	finalizeAgentDebugReport,
	finalizeAgentGenerationMetrics,
	markAgentDebugReportInterrupted,
	recordAgentDebugEvent,
	recordAgentGenerationMetricEvent,
} from "./agent-debug-report";
import type { IMessage } from "./global-chat-db";

/**
 * A checkpoint persisted mid-run carries a debug report frozen at `outcome: "running"`. When a
 * conversation is restored, that run is no longer driving this report: either it died with the
 * old window, or a resumed run finalizes a fresh report over the same message id. Mark it
 * interrupted so a dead run never keeps rendering as live.
 */
export function markRestoredMessageDebugReportStale(
	message: IMessage,
): IMessage {
	const report = message.debug_report;
	if (!report || report.outcome !== "running") return message;
	return { ...message, debug_report: markAgentDebugReportInterrupted(report) };
}

/** Id prefix marking plan steps that belong to a nested sub-agent run (e.g. flowpilot_board). */
export const SUB_STEP_PREFIX = "sub:";

/**
 * Upper bound on assistant turns streaming at once in a single conversation. Sends beyond it are
 * queued instead of rejected. The cap exists because every concurrent turn holds a CLI process
 * (agent backends) or a completion stream (Bits), and the transcript stops being readable past a
 * handful of live bubbles.
 */
export const MAX_CONCURRENT_GLOBAL_CHAT_RUNS = 4;

export type GlobalChatMode = "closed" | "overlay";

/** Session-scoped pointer to the active conversation, so reloads/navigation restore the transcript. */
export const LAST_CONVERSATION_KEY = "flow-like:global-chat:last-conversation";

/** Session-scoped dock visibility, so a hard reload mid-response re-opens the overlay (and thus
 * re-mounts the chat surface that re-attaches to the live stream) instead of hiding it. */
const OVERLAY_MODE_KEY = "flow-like:global-chat:mode";
/** A user dismissal suppresses automatic re-opening for the rest of the current FlowPilot cycle.
 * It is cleared only after the user starts interacting with the full FlowPilot page again. */
const OVERLAY_AUTO_OPEN_DISMISSED_KEY =
	"flow-like:global-chat:auto-open-dismissed";

function persistOverlayMode(mode: GlobalChatMode) {
	try {
		sessionStorage.setItem(OVERLAY_MODE_KEY, mode);
	} catch {
		// persistence is best-effort
	}
}

/** The dock mode persisted before a reload, if any. Consumed once by the overlay on mount. */
export function readPersistedOverlayMode(): GlobalChatMode | null {
	try {
		const raw = sessionStorage.getItem(OVERLAY_MODE_KEY);
		return raw === "overlay" || raw === "closed" ? raw : null;
	} catch {
		return null;
	}
}

function persistOverlayAutoOpenDismissed(dismissed: boolean) {
	try {
		if (dismissed) {
			sessionStorage.setItem(OVERLAY_AUTO_OPEN_DISMISSED_KEY, "true");
		} else {
			sessionStorage.removeItem(OVERLAY_AUTO_OPEN_DISMISSED_KEY);
		}
	} catch {
		// persistence is best-effort
	}
}

function readPersistedOverlayAutoOpenDismissed() {
	try {
		return sessionStorage.getItem(OVERLAY_AUTO_OPEN_DISMISSED_KEY) === "true";
	} catch {
		return false;
	}
}

/** The FlowPilot provider/model/effort the user last *explicitly* chose. Remembered
 * across sessions and hydrated by useHydrateAgentSelection so every surface (hero,
 * /chat, overlay) opens on the same picks. Written only by the select* actions —
 * the plain setters stay non-persisting so a transient "keep a valid model"
 * fallback can never overwrite the remembered choice. */
export const AGENT_PROVIDER_KEY = "flowpilot.hero.provider";
export const AGENT_MODEL_KEY = "flowpilot.hero.model";
export const AGENT_REASONING_KEY = "flowpilot.hero.reasoning-effort";

function persistAgentPref(key: string, value: string) {
	try {
		if (typeof window !== "undefined") localStorage.setItem(key, value);
	} catch {
		// persistence is best-effort
	}
}

export interface InlineAppChat {
	id: string;
	appId: string;
	eventId: string;
	/** Display name of the app / chat event for the card header. */
	name: string;
}

export interface InlineAppPage {
	id: string;
	appId: string;
	/** UI/page event to open. Optional — the use surface falls back to the app's default page. */
	eventId?: string;
	/** Display name of the app / page for the card header. */
	name: string;
}

export interface InlineAppSurface {
	id: string;
	appId: string;
	/** Display name of the app / chat that pushed the UI, for the card header. */
	name: string;
	/** Surfaces the app pushed during a headless `call_app_chat` run, captured for display only. */
	surfaces: Surface[];
}

export interface GlobalChatDraft {
	prompt: string;
	/** Backend-prefixed model id (e.g. "github-copilot:…", "codex:…") or a raw Bits id. */
	modelId?: string;
	/** Raw browser files captured on the landing bar, forwarded to the first /chat send. */
	files?: File[];
}

/** Provider-neutral model configuration captured once when a global assistant turn starts. */
export interface GlobalChatAgentSelection {
	readonly provider: AIProvider;
	/** Raw (un-prefixed) picker model id. */
	readonly selectedModelId: string;
	/** Provider-specific reasoning effort ("" = provider/model default). */
	readonly reasoningEffort: string;
}

/** Immutable owner-tagged selection used by every specialist spawned during one global turn. */
export interface GlobalChatTurnSelection extends GlobalChatAgentSelection {
	readonly runId: string;
}

/** Lifecycle of one assistant turn. `cancelling` is the window between the user pressing stop and
 * the transport actually tearing the run down — the bubble stays rendered but stops accepting new
 * steering. */
export type GlobalChatRunStatus = "streaming" | "cancelling";

/** One user instruction pushed into an already-running turn. */
export interface GlobalChatSteer {
	id: string;
	content: string;
	/** `pending` until the backend confirms the run accepted it; `failed` keeps the text visible. */
	status: "pending" | "delivered" | "failed";
	createdAt: number;
	/** Failure reason, rendered on the steer chip so a dropped instruction is never silent. */
	error?: string;
}

/**
 * One in-flight assistant turn. Everything a run streams — its message, the buffers nested
 * sub-agents publish into, its debug report — hangs off this record rather than a store singleton,
 * so N turns can stream side by side without cross-contaminating each other's steps, widgets,
 * usage, or attachments.
 */
export interface GlobalChatRun {
	runId: string;
	conversationId: string;
	status: GlobalChatRunStatus;
	startedAt: number;
	/** First line of the prompt that started the turn — labels its stop/steer controls. */
	label: string;
	/** Files attached to the owning user turn, snapshotted before concurrent runs can advance chat. */
	sourceAttachments: IAttachment[];
	selection: GlobalChatTurnSelection;
	/** The assistant reply being streamed into. Its id always equals `runId`. */
	message: IMessage | null;
	subPlanSteps: IPlanStep[];
	subAttachments: IAttachment[];
	subUsageStats: IChatUsageStat[];
	subWidgets: IChatWidget[];
	pendingAppRefs: string[];
	debugReport: IAgentDebugReport | null;
	steers: GlobalChatSteer[];
	/** Transport-level teardown (the web SSE AbortController). Desktop cancels via the Rust
	 * registry instead, so this stays undefined there. */
	abort?: () => void;
}

/** A message the user sent while the conversation was at its concurrency cap, or explicitly
 * queued. Drained in order as runs finish. */
export interface GlobalChatQueuedMessage {
	id: string;
	conversationId: string;
	content: string;
	/** Raw browser files, held until the send actually happens (they are uploaded at send time). */
	files?: File[];
	createdAt: number;
}

export type GlobalToolPromptResolution =
	| { approved: boolean; remember: boolean }
	| { answer: unknown }
	| null;

/** One selectable option for an `ask_user` single/multiple-choice question. */
export type GlobalToolAskChoice = AskUserChoice;

/**
 * Parsed `ask_user` payload driving the inline prompt's input controls: one question, or the
 * batched BUILD intake form answered in a single card.
 */
export type GlobalToolAsk = AskUserForm;

/**
 * A pending frontend-tool approval or question from the global assistant, rendered inline in the
 * chat surface (above the input) instead of as a modal. `respond` resolves the bridge's promise.
 */
export interface GlobalToolPrompt {
	id: string;
	kind: "approval" | "ask";
	toolName: string;
	title: string;
	description?: string;
	/**
	 * App the pending action targets (from the tool's `app_id` argument). Lets the approval card
	 * resolve the raw id to the app's name + icon instead of showing the opaque id.
	 */
	appId?: string;
	/** A gate that must never be answered without the user (e.g. the FlowScript deletion
	 * re-apply). Auto mode skips these — waiving permission never extends to deletions. */
	destructive?: boolean;
	/** Present only when `kind === "ask"`: drives freeform vs. single/multiple choice rendering. */
	ask?: GlobalToolAsk;
	respond: (value: GlobalToolPromptResolution) => void;
}

/**
 * Shared state for the global FlowPilot assistant. Lives outside the surface components so the same
 * conversation renders in the full `/chat` view and in the docked bottom-right overlay: when the agent
 * navigates the user away from `/chat`, the chat morphs into the overlay without losing context.
 */
interface GlobalChatState {
	/** Pending message handed off from the landing bar to the /chat view. */
	draft: GlobalChatDraft | null;
	/** Docked overlay visibility, toggled when the agent navigates or the user opens the dock. */
	mode: GlobalChatMode;
	/** Whether automatic opens are suppressed because the user explicitly dismissed the dock. */
	overlayAutoOpenDismissed: boolean;
	/** Conversation currently shown in both /chat and the overlay. */
	activeConversationId: string;
	/** Committed messages of the active conversation. */
	messages: IMessage[];
	/**
	 * Every in-flight turn in the app, keyed by run id, across ALL conversations — a run keeps
	 * streaming (and finalizing) when the user switches to another chat, and re-appears in place
	 * when they switch back.
	 */
	runs: Record<string, GlobalChatRun>;
	/** Messages waiting for capacity, in send order. Drained as runs finish. */
	queue: GlobalChatQueuedMessage[];
	/** Derived: the active conversation has at least one live run. Maintained on every runs write.
	 * This no longer gates sending — it only drives "is something happening" affordances. */
	isStreaming: boolean;
	/** Derived: live assistant bubbles of the active conversation, oldest run first. */
	streamingMessages: IMessage[];
	/**
	 * A route a tool asked to open, deferred until the agent turn finishes. Navigating mid-stream
	 * tears down the run, so tools (e.g. flowpilot_widget after creating a page) stash the target
	 * here and the bridge navigates once the REQUESTING run (`runId`) ends — scoping to the run
	 * keeps conversation switches from firing it early or unrelated runs from holding it hostage.
	 */
	pendingNavigation: { target: string; runId?: string } | null;
	/** Pending inline approval/question from the global tool bridge (one at a time). */
	toolPrompt: GlobalToolPrompt | null;
	provider: AIProvider;
	/** Raw (un-prefixed) model id selected in the picker. */
	selectedModelId: string;
	/** Provider-specific reasoning effort ("" = use the selected model's default). */
	reasoningEffort: string;
	/** Embedding bit id used for profile-scoped memory ("" = memory off). */
	embeddingModelId: string;
	/** Waive tool-approval prompts and the pending-change review gate. Deliberately not
	 * persisted — it resets on page load, unlike the picker preferences above. Never waives
	 * `ask_user` or destructive-deletion approval. */
	autoMode: boolean;
	/** App chat events the agent surfaced inline in the global chat view. */
	inlineAppChats: InlineAppChat[];
	/** App UI pages the agent embedded inline in the global chat view (artifact-like). */
	inlineAppPages: InlineAppPage[];
	/** UI an app pushed while the agent called its chat headlessly (call_app_chat), shown as cards. */
	inlineAppSurfaces: InlineAppSurface[];
	/**
	 * Interactions (single/multiple choice, form) raised by a nested app-chat run (call_app_chat).
	 * Rendered by the chat body and answered via respond_to_interaction, unblocking the app workflow
	 * while the outer call_app_chat tool call is still in flight. Deliberately conversation-scoped
	 * rather than run-scoped: these are independent cards keyed by their own ids, so interleaving
	 * them across concurrent runs is correct.
	 */
	activeInteractions: IInteractionRequest[];
	/**
	 * FlowScript workspace generated by the latest flowpilot_board run, streamed live so the chat
	 * shows the code as it is written (same panel as the board FlowPilot). There is one workspace
	 * panel, so this stays a single slot — `ownerRunId` records which run filled it, so a finishing
	 * or cancelled run only clears its own workspace.
	 */
	flowscriptWorkspace: FlowScriptWorkspaceCandidate | null;
	flowscriptWorkspaceOwnerRunId: string | null;
	/**
	 * Validated UI components generated by the latest flowpilot_widget run, staged in the chat for
	 * the user to review and apply to the open widget/page builder. Never auto-applied. Single slot
	 * with an owner tag, for the same reason as the FlowScript workspace.
	 */
	pendingComponents: {
		components: SurfaceComponent[];
		canvasSettings?: CanvasSettings;
		warnings?: string[];
		/** Builder the components were generated for — Apply refuses other surfaces. */
		surfaceId?: string;
		appId?: string;
	} | null;
	pendingComponentsOwnerRunId: string | null;

	setDraft: (draft: GlobalChatDraft) => void;
	/** Returns the pending draft once and clears it, so it is only auto-sent a single time. */
	consumeDraft: () => GlobalChatDraft | null;
	/** Explicitly open the dock, regardless of a previous dismissal. */
	openOverlay: () => void;
	/** Open the dock for agent/navigation activity unless the user dismissed it. */
	openOverlayIfAllowed: () => void;
	/** Close the dock and suppress automatic opens until FlowPilot is used on its full page again. */
	dismissOverlay: () => void;
	/** Re-enable automatic opens after renewed interaction on the full FlowPilot page. */
	enableOverlayAutoOpen: () => void;
	/** Close the dock without treating it as a user dismissal (for example on the full chat page). */
	closeOverlay: () => void;
	/** Defer a route change until the agent turn ends (navigating mid-stream breaks the run). */
	setPendingNavigation: (
		navigation: { target: string; runId?: string } | null,
	) => void;
	appendMessage: (message: IMessage) => void;
	/** Upsert a message by id — replaces an existing one (e.g. a restored streaming checkpoint that a
	 * resumed run finalizes) or appends it when new. Used to commit finished assistant replies. */
	commitMessage: (message: IMessage) => void;
	/** Register a new in-flight turn. Idempotent per run id — a resume re-attaches to the record. */
	startRun: (
		run: Pick<
			GlobalChatRun,
			| "runId"
			| "conversationId"
			| "selection"
			| "label"
			| "message"
			| "sourceAttachments"
		>,
	) => void;
	/** Replace a run's streaming bubble. No-op once the run has ended, so a late chunk from a
	 * cancelled transport cannot resurrect a finished bubble. */
	setRunMessage: (runId: string, message: IMessage | null) => void;
	setRunStatus: (runId: string, status: GlobalChatRunStatus) => void;
	/** Attach the transport teardown hook used by `cancelGlobalChatRun` (web SSE only). */
	setRunAbort: (runId: string, abort: (() => void) | undefined) => void;
	/** Drop the run record and any single-slot panel state it owned. */
	endRun: (runId: string) => void;
	/** Record a steering instruction against a live run so it renders while in flight. */
	addRunSteer: (runId: string, steer: GlobalChatSteer) => void;
	setRunSteerStatus: (
		runId: string,
		steerId: string,
		status: GlobalChatSteer["status"],
		error?: string,
	) => void;
	/** Append a message to the send queue. */
	enqueueMessage: (
		entry: Omit<GlobalChatQueuedMessage, "id" | "createdAt">,
	) => string;
	/** Remove one queued message (user deleted it, or it is being sent now). */
	removeQueuedMessage: (id: string) => void;
	/** Edit a queued message's text before it is sent. */
	updateQueuedMessage: (id: string, content: string) => void;
	/** Pop the oldest queued message of a conversation, or undefined when the queue is empty. */
	takeNextQueuedMessage: (
		conversationId: string,
	) => GlobalChatQueuedMessage | undefined;
	clearQueue: (conversationId?: string) => void;
	setToolPrompt: (prompt: GlobalToolPrompt | null) => void;
	setProvider: (provider: AIProvider) => void;
	setSelectedModelId: (modelId: string) => void;
	setReasoningEffort: (effort: string) => void;
	/** Explicit (user-driven) picks — these also persist across sessions. The plain
	 * setters above stay non-persisting for internal "keep a valid model" fallbacks. */
	selectProvider: (provider: AIProvider) => void;
	selectModel: (modelId: string) => void;
	selectReasoningEffort: (effort: string) => void;
	setEmbeddingModelId: (modelId: string) => void;
	setAutoMode: (autoMode: boolean) => void;
	addInlineAppChat: (chat: Omit<InlineAppChat, "id">) => void;
	removeInlineAppChat: (id: string) => void;
	addInlineAppPage: (page: Omit<InlineAppPage, "id">) => void;
	removeInlineAppPage: (id: string) => void;
	addInlineAppSurface: (surface: Omit<InlineAppSurface, "id">) => void;
	removeInlineAppSurface: (id: string) => void;
	addPendingAppRef: (runId: string, appId: string) => void;
	/** Replace the nested run's steps and refresh that run's bubble so they render immediately. */
	setSubPlanSteps: (runId: string, steps: IPlanStep[]) => void;
	/** Merge app-chat interactions (upsert by id, promoting settled status) for inline rendering. */
	addInteractions: (interactions: IInteractionRequest[]) => void;
	/** Mark one interaction as responded so its inline card settles after the user answers. */
	setInteractionResponded: (interactionId: string, value: unknown) => void;
	clearInteractions: () => void;
	/** Append app-chat attachments (deduped by url) to that run's rendered files. */
	addSubAttachments: (runId: string, attachments: IAttachment[]) => void;
	/** Append nested-run usage stats (deduped) and merge them into the run's bubble live. */
	addSubUsageStats: (runId: string, stats: IChatUsageStat[]) => void;
	/** Upsert nested-run widgets (by instance id, longer-updates-wins) into the run's bubble. */
	addSubWidgets: (runId: string, widgets: IChatWidget[]) => void;
	setFlowscriptWorkspace: (
		runId: string | null,
		workspace: FlowScriptWorkspaceCandidate | null,
	) => void;
	beginDebugReport: (
		messageId: string,
		metadata?: AgentDebugReportMetadata,
	) => void;
	recordDebugEvent: (messageId: string, event: IAgentDebugEvent) => void;
	finalizeDebugReport: (
		messageId: string,
		options: Parameters<typeof finalizeAgentDebugReport>[1],
	) => IAgentDebugReport | null;
	clearDebugReport: (messageId?: string) => void;
	setPendingComponents: (
		runId: string | null,
		pending: {
			components: SurfaceComponent[];
			canvasSettings?: CanvasSettings;
			warnings?: string[];
			surfaceId?: string;
			appId?: string;
		} | null,
	) => void;
	/** Start a fresh conversation (new id, cleared transcript). */
	newConversation: () => void;
	/** Resume a persisted conversation from history. */
	loadConversation: (conversationId: string, messages: IMessage[]) => void;
}

const usageSignature = (stat: IChatUsageStat) => JSON.stringify(stat);

/**
 * Re-fold a run's nested-agent buffers into its rendered bubble. Every sub-* write goes through
 * this so nested activity shows up immediately — the run's own channel is silent while a sub-agent
 * works, and without the fold the steps/widgets/usage would only appear on the next token.
 */
function foldSubBuffers(run: GlobalChatRun): GlobalChatRun {
	const message = run.message;
	if (!message) return run;
	const ownSteps = (message.plan_steps ?? []).filter(
		(step) => !step.id.startsWith(SUB_STEP_PREFIX),
	);
	const usage = [...(message.usage_stats ?? [])];
	const known = new Set(usage.map(usageSignature));
	for (const stat of run.subUsageStats) {
		const signature = usageSignature(stat);
		if (known.has(signature)) continue;
		known.add(signature);
		usage.push(stat);
	}
	return {
		...run,
		message: {
			...message,
			plan_steps: [...ownSteps, ...run.subPlanSteps],
			files: run.subAttachments.length > 0 ? run.subAttachments : message.files,
			widgets: run.subWidgets.length > 0 ? run.subWidgets : message.widgets,
			usage_stats: usage.length > 0 ? usage : undefined,
			app_refs:
				run.pendingAppRefs.length > 0
					? [...run.pendingAppRefs]
					: message.app_refs,
			debug_report: run.debugReport ?? message.debug_report,
		},
	};
}

/** Recompute the two views the UI subscribes to. Called on every write to `runs`. */
function deriveRunViews(
	runs: Record<string, GlobalChatRun>,
	activeConversationId: string,
): Pick<GlobalChatState, "runs" | "isStreaming" | "streamingMessages"> {
	const live = Object.values(runs)
		.filter((run) => run.conversationId === activeConversationId)
		.sort((a, b) => a.startedAt - b.startedAt);
	const streamingMessages: IMessage[] = [];
	for (const run of live) if (run.message) streamingMessages.push(run.message);
	return { runs, isStreaming: live.length > 0, streamingMessages };
}

/** Apply a patch to one run. Unknown run ids and no-op patches leave the store untouched, so a
 * late event from a torn-down transport can never resurrect a finished run. */
function patchRun(
	state: GlobalChatState,
	runId: string,
	patch: (run: GlobalChatRun) => GlobalChatRun,
): Partial<GlobalChatState> | GlobalChatState {
	const existing = state.runs[runId];
	if (!existing) return state;
	const next = patch(existing);
	if (next === existing) return state;
	return deriveRunViews(
		{ ...state.runs, [runId]: next },
		state.activeConversationId,
	);
}

function switchConversation(
	state: GlobalChatState,
	conversationId: string,
	messages: IMessage[],
): Partial<GlobalChatState> {
	return {
		activeConversationId: conversationId,
		messages,
		inlineAppChats: [],
		inlineAppPages: [],
		inlineAppSurfaces: [],
		activeInteractions: [],
		flowscriptWorkspace: null,
		flowscriptWorkspaceOwnerRunId: null,
		pendingComponents: null,
		pendingComponentsOwnerRunId: null,
		// Runs survive the switch; only the derived views are re-scoped to the new conversation.
		...deriveRunViews(state.runs, conversationId),
	};
}

export const useGlobalChatStore = create<GlobalChatState>((set, get) => ({
	draft: null,
	mode: "closed",
	overlayAutoOpenDismissed: false,
	activeConversationId: createId(),
	messages: [],
	runs: {},
	queue: [],
	isStreaming: false,
	streamingMessages: [],
	pendingNavigation: null,
	toolPrompt: null,
	provider: "bits",
	selectedModelId: "",
	reasoningEffort: "",
	embeddingModelId: "",
	autoMode: false,
	inlineAppChats: [],
	inlineAppPages: [],
	inlineAppSurfaces: [],
	activeInteractions: [],
	flowscriptWorkspace: null,
	flowscriptWorkspaceOwnerRunId: null,
	pendingComponents: null,
	pendingComponentsOwnerRunId: null,

	setDraft: (draft) => set({ draft }),
	consumeDraft: () => {
		const { draft } = get();
		if (draft) set({ draft: null });
		return draft;
	},
	openOverlay: () => {
		persistOverlayMode("overlay");
		set({ mode: "overlay" });
	},
	openOverlayIfAllowed: () => {
		if (
			get().overlayAutoOpenDismissed ||
			readPersistedOverlayAutoOpenDismissed()
		) {
			set({ overlayAutoOpenDismissed: true });
			return;
		}
		persistOverlayMode("overlay");
		set({ mode: "overlay" });
	},
	dismissOverlay: () => {
		persistOverlayMode("closed");
		persistOverlayAutoOpenDismissed(true);
		set({ mode: "closed", overlayAutoOpenDismissed: true });
	},
	enableOverlayAutoOpen: () => {
		if (
			!get().overlayAutoOpenDismissed &&
			!readPersistedOverlayAutoOpenDismissed()
		) {
			return;
		}
		persistOverlayAutoOpenDismissed(false);
		set({ overlayAutoOpenDismissed: false });
	},
	closeOverlay: () => {
		persistOverlayMode("closed");
		set({ mode: "closed" });
	},
	setPendingNavigation: (pendingNavigation) => set({ pendingNavigation }),
	// Both writers are conversation-guarded: a run that finishes after the user switched chats must
	// not splice its reply into the transcript now on screen. It is still persisted to IndexedDB by
	// the stream engine, so switching back shows it.
	appendMessage: (message) =>
		set((state) =>
			message.sessionId && message.sessionId !== state.activeConversationId
				? state
				: { messages: [...state.messages, message] },
		),
	commitMessage: (message) =>
		set((state) => {
			if (
				message.sessionId &&
				message.sessionId !== state.activeConversationId
			) {
				return state;
			}
			const index = state.messages.findIndex((m) => m.id === message.id);
			if (index === -1) return { messages: [...state.messages, message] };
			const messages = [...state.messages];
			messages[index] = message;
			return { messages };
		}),
	startRun: ({
		runId,
		conversationId,
		selection,
		label,
		message,
		sourceAttachments,
	}) =>
		set((state) => {
			const existing = state.runs[runId];
			if (existing) {
				// Re-entrant for resumes: keep the buffers the previous attachment accumulated.
				return deriveRunViews(
					{ ...state.runs, [runId]: { ...existing, status: "streaming" } },
					state.activeConversationId,
				);
			}
			return deriveRunViews(
				{
					...state.runs,
					[runId]: {
						runId,
						conversationId,
						selection,
						label,
						sourceAttachments,
						message,
						status: "streaming",
						startedAt: Date.now(),
						subPlanSteps: [],
						subAttachments: [],
						subUsageStats: [],
						subWidgets: [],
						pendingAppRefs: [],
						debugReport: null,
						steers: [],
					},
				},
				state.activeConversationId,
			);
		}),
	setRunMessage: (runId, message) =>
		set((state) =>
			patchRun(state, runId, (run) => foldSubBuffers({ ...run, message })),
		),
	setRunStatus: (runId, status) =>
		set((state) =>
			patchRun(state, runId, (run) =>
				run.status === status ? run : { ...run, status },
			),
		),
	setRunAbort: (runId, abort) =>
		set((state) => patchRun(state, runId, (run) => ({ ...run, abort }))),
	endRun: (runId) =>
		set((state) => {
			if (!state.runs[runId]) return state;
			const runs = { ...state.runs };
			delete runs[runId];
			return {
				...deriveRunViews(runs, state.activeConversationId),
				// Release the single-slot panels this run owned; another run's stay untouched.
				...(state.flowscriptWorkspaceOwnerRunId === runId
					? { flowscriptWorkspace: null, flowscriptWorkspaceOwnerRunId: null }
					: {}),
			};
		}),
	addRunSteer: (runId, steer) =>
		set((state) =>
			patchRun(state, runId, (run) => ({
				...run,
				steers: [...run.steers, steer],
			})),
		),
	setRunSteerStatus: (runId, steerId, status, error) =>
		set((state) =>
			patchRun(state, runId, (run) => ({
				...run,
				steers: run.steers.map((steer) =>
					steer.id === steerId ? { ...steer, status, error } : steer,
				),
			})),
		),
	enqueueMessage: (entry) => {
		const id = createId();
		set((state) => ({
			queue: [...state.queue, { ...entry, id, createdAt: Date.now() }],
		}));
		return id;
	},
	removeQueuedMessage: (id) =>
		set((state) => {
			const queue = state.queue.filter((entry) => entry.id !== id);
			return queue.length === state.queue.length ? state : { queue };
		}),
	updateQueuedMessage: (id, content) =>
		set((state) => ({
			queue: state.queue.map((entry) =>
				entry.id === id ? { ...entry, content } : entry,
			),
		})),
	takeNextQueuedMessage: (conversationId) => {
		const next = get().queue.find(
			(entry) => entry.conversationId === conversationId,
		);
		if (!next) return undefined;
		set((state) => ({
			queue: state.queue.filter((entry) => entry.id !== next.id),
		}));
		return next;
	},
	clearQueue: (conversationId) =>
		set((state) => ({
			queue: conversationId
				? state.queue.filter((entry) => entry.conversationId !== conversationId)
				: [],
		})),
	setToolPrompt: (toolPrompt) => set({ toolPrompt }),
	setProvider: (provider) => set({ provider }),
	setSelectedModelId: (selectedModelId) => set({ selectedModelId }),
	setReasoningEffort: (reasoningEffort) => set({ reasoningEffort }),
	selectProvider: (provider) => {
		persistAgentPref(AGENT_PROVIDER_KEY, provider);
		set({ provider });
	},
	selectModel: (selectedModelId) => {
		persistAgentPref(AGENT_MODEL_KEY, selectedModelId);
		set({ selectedModelId });
	},
	selectReasoningEffort: (reasoningEffort) => {
		persistAgentPref(AGENT_REASONING_KEY, reasoningEffort);
		set({ reasoningEffort });
	},
	setEmbeddingModelId: (embeddingModelId) => set({ embeddingModelId }),
	setAutoMode: (autoMode) => set({ autoMode }),
	addInlineAppChat: (chat) =>
		set((state) => {
			// One card per (app, event) — surfacing the same chat twice just keeps the existing card.
			if (
				state.inlineAppChats.some(
					(existing) =>
						existing.appId === chat.appId && existing.eventId === chat.eventId,
				)
			) {
				return state;
			}
			return {
				inlineAppChats: [...state.inlineAppChats, { ...chat, id: createId() }],
			};
		}),
	removeInlineAppChat: (id) =>
		set((state) => ({
			inlineAppChats: state.inlineAppChats.filter((chat) => chat.id !== id),
		})),
	addInlineAppPage: (page) =>
		set((state) => {
			// One card per (app, event) — embedding the same page twice keeps the existing card.
			if (
				state.inlineAppPages.some(
					(existing) =>
						existing.appId === page.appId && existing.eventId === page.eventId,
				)
			) {
				return state;
			}
			return {
				inlineAppPages: [...state.inlineAppPages, { ...page, id: createId() }],
			};
		}),
	removeInlineAppPage: (id) =>
		set((state) => ({
			inlineAppPages: state.inlineAppPages.filter((page) => page.id !== id),
		})),
	addInlineAppSurface: (surface) =>
		set((state) => {
			if (surface.surfaces.length === 0) return state;
			return {
				inlineAppSurfaces: [
					...state.inlineAppSurfaces,
					{ ...surface, id: createId() },
				],
			};
		}),
	removeInlineAppSurface: (id) =>
		set((state) => ({
			inlineAppSurfaces: state.inlineAppSurfaces.filter(
				(surface) => surface.id !== id,
			),
		})),
	addPendingAppRef: (runId, appId) =>
		set((state) =>
			patchRun(state, runId, (run) =>
				run.pendingAppRefs.includes(appId)
					? run
					: foldSubBuffers({
							...run,
							pendingAppRefs: [...run.pendingAppRefs, appId],
						}),
			),
		),
	setSubPlanSteps: (runId, subPlanSteps) =>
		set((state) =>
			patchRun(state, runId, (run) => {
				// Anchor each sub-step where the parent's text stood when it first appeared, so it
				// renders inline next to the tool call it belongs to. Re-publishes rebuild the step
				// objects from scratch — keep the first-seen anchor or the steps would drift.
				const priorAnchors = new Map(
					run.subPlanSteps.map((step) => [step.id, step.content_offset]),
				);
				const content = run.message?.inner.content;
				const anchor = typeof content === "string" ? content.length : 0;
				const anchored = subPlanSteps.map((step) =>
					step.content_offset !== undefined
						? step
						: {
								...step,
								content_offset: priorAnchors.get(step.id) ?? anchor,
							},
				);
				return foldSubBuffers({ ...run, subPlanSteps: anchored });
			}),
		),
	addInteractions: (interactions) =>
		set((state) => {
			const byId = new Map(state.activeInteractions.map((i) => [i.id, i]));
			let changed = false;
			for (const interaction of interactions) {
				const existing = byId.get(interaction.id);
				// Add new interactions; let a non-pending update supersede a pending one, but never
				// let a late "pending" echo overwrite a response the user already submitted.
				if (!existing) {
					byId.set(interaction.id, interaction);
					changed = true;
				} else if (
					existing.status === "pending" &&
					interaction.status !== "pending"
				) {
					byId.set(interaction.id, interaction);
					changed = true;
				}
			}
			return changed
				? { activeInteractions: Array.from(byId.values()) }
				: state;
		}),
	setInteractionResponded: (interactionId, value) =>
		set((state) => ({
			activeInteractions: state.activeInteractions.map((interaction) =>
				interaction.id === interactionId
					? {
							...interaction,
							status: "responded" as const,
							response_value: value,
						}
					: interaction,
			),
		})),
	clearInteractions: () =>
		set((state) =>
			state.activeInteractions.length > 0 ? { activeInteractions: [] } : state,
		),
	addSubAttachments: (runId, attachments) =>
		set((state) =>
			patchRun(state, runId, (run) => {
				if (attachments.length === 0) return run;
				const urlOf = (attachment: IAttachment) =>
					typeof attachment === "string" ? attachment : attachment.url;
				const seen = new Set(run.subAttachments.map(urlOf));
				const fresh = attachments.filter((attachment) => {
					const url = urlOf(attachment);
					if (seen.has(url)) return false;
					seen.add(url);
					return true;
				});
				if (fresh.length === 0) return run;
				return foldSubBuffers({
					...run,
					subAttachments: [...run.subAttachments, ...fresh],
				});
			}),
		),
	addSubUsageStats: (runId, stats) =>
		set((state) =>
			patchRun(state, runId, (run) => {
				if (stats.length === 0) return run;
				const seen = new Set(run.subUsageStats.map(usageSignature));
				const fresh = stats.filter((stat) => {
					const signature = usageSignature(stat);
					if (seen.has(signature)) return false;
					seen.add(signature);
					return true;
				});
				if (fresh.length === 0) return run;
				return foldSubBuffers({
					...run,
					subUsageStats: [...run.subUsageStats, ...fresh],
				});
			}),
		),
	addSubWidgets: (runId, widgets) =>
		set((state) =>
			patchRun(state, runId, (run) =>
				widgets.length === 0
					? run
					: foldSubBuffers({
							...run,
							subWidgets: mergeChatWidgets(run.subWidgets, widgets),
						}),
			),
		),
	setFlowscriptWorkspace: (runId, flowscriptWorkspace) =>
		set((state) => {
			// A run may only clear the workspace it owns — otherwise a finishing board sub-agent
			// would wipe a concurrent run's freshly generated code out of the shared panel.
			if (
				!flowscriptWorkspace &&
				runId &&
				state.flowscriptWorkspaceOwnerRunId &&
				state.flowscriptWorkspaceOwnerRunId !== runId
			) {
				return state;
			}
			return {
				flowscriptWorkspace,
				flowscriptWorkspaceOwnerRunId: flowscriptWorkspace ? runId : null,
			};
		}),
	// The debug report is addressed by message id, which IS the run id — so these route straight
	// into the owning run's record and never touch a sibling run's report.
	beginDebugReport: (messageId, metadata) => {
		beginAgentGenerationMetrics(messageId, metadata?.startedAtMs);
		if (!FLOWPILOT_DEBUG_ENABLED) return;
		set((state) =>
			patchRun(state, messageId, (run) =>
				run.debugReport
					? run
					: foldSubBuffers({
							...run,
							debugReport: createAgentDebugReport(messageId, metadata),
						}),
			),
		);
	},
	recordDebugEvent: (messageId, event) => {
		recordAgentGenerationMetricEvent(messageId, event);
		if (!FLOWPILOT_DEBUG_ENABLED) return;
		set((state) =>
			patchRun(state, messageId, (run) =>
				run.debugReport
					? foldSubBuffers({
							...run,
							debugReport: recordAgentDebugEvent(run.debugReport, event),
						})
					: run,
			),
		);
	},
	finalizeDebugReport: (messageId, options) => {
		finalizeAgentGenerationMetrics(messageId, options.outcome, {
			publish: !FLOWPILOT_DEBUG_ENABLED,
			failure: { code: options.terminalCode, message: options.summary },
		});
		if (!FLOWPILOT_DEBUG_ENABLED) return null;
		const report = get().runs[messageId]?.debugReport;
		if (!report) return null;
		const finalized = finalizeAgentDebugReport(report, options);
		set((state) =>
			patchRun(state, messageId, (run) =>
				foldSubBuffers({ ...run, debugReport: finalized }),
			),
		);
		return finalized;
	},
	clearDebugReport: (messageId) => {
		if (messageId) clearAgentGenerationMetrics(messageId);
		if (!messageId) return;
		set((state) =>
			patchRun(state, messageId, (run) =>
				run.debugReport ? { ...run, debugReport: null } : run,
			),
		);
	},
	setPendingComponents: (runId, pendingComponents) =>
		set((state) => {
			if (
				!pendingComponents &&
				runId &&
				state.pendingComponentsOwnerRunId &&
				state.pendingComponentsOwnerRunId !== runId
			) {
				return state;
			}
			return {
				pendingComponents,
				pendingComponentsOwnerRunId: pendingComponents ? runId : null,
			};
		}),
	// Start a fresh conversation WITHOUT touching `mode`: clicking "New chat" (or deleting the
	// active chat) from the docked overlay must keep the dock open, not close it. Runs of OTHER
	// conversations are deliberately preserved — switching chats no longer kills what is in flight.
	newConversation: () => {
		// Drop the reload-restore pointer, or the next surface mount silently swaps the fresh
		// conversation back to the previous one.
		try {
			sessionStorage.removeItem(LAST_CONVERSATION_KEY);
		} catch {
			// storage unavailable — restore is best-effort anyway
		}
		set((state) => switchConversation(state, createId(), []));
	},
	loadConversation: (conversationId, messages) =>
		set((state) =>
			switchConversation(
				state,
				conversationId,
				messages.map(markRestoredMessageDebugReportStale),
			),
		),
}));

function freezeTurnSelection(
	runId: string,
	selection: GlobalChatAgentSelection,
): GlobalChatTurnSelection {
	return Object.freeze({
		runId,
		provider: selection.provider,
		selectedModelId: selection.selectedModelId,
		reasoningEffort: selection.reasoningEffort,
	});
}

/**
 * Pin one run's execution selection. Idempotent per run id: once the run record exists, its stored
 * selection wins, so nested specialists of that run can never be moved to another model — while a
 * *different* concurrent run is free to pin its own. Before the run exists this just freezes a
 * snapshot for the caller to hand to `startRun`.
 */
export function beginGlobalChatTurnSelection(
	runId: string,
	selection?: GlobalChatAgentSelection,
): GlobalChatTurnSelection {
	const state = useGlobalChatStore.getState();
	const pinned = state.runs[runId]?.selection;
	if (pinned) return pinned;
	return freezeTurnSelection(
		runId,
		selection ?? {
			provider: state.provider,
			selectedModelId: state.selectedModelId,
			reasoningEffort: state.reasoningEffort,
		},
	);
}

/**
 * The immutable selection of a specific run. Falls back to the live picker only when the caller has
 * no run in hand — with concurrent runs, callers that omit `runId` are accepting whatever the user
 * currently has selected, not "the" active turn.
 */
export function getGlobalChatTurnSelection(
	runId?: string,
): GlobalChatAgentSelection {
	const state = useGlobalChatStore.getState();
	const pinned = runId ? state.runs[runId]?.selection : undefined;
	return (
		pinned ??
		Object.freeze({
			provider: state.provider,
			selectedModelId: state.selectedModelId,
			reasoningEffort: state.reasoningEffort,
		})
	);
}

/** Live runs of a conversation (defaults to the active one), oldest first. */
export function selectGlobalChatRuns(
	state: GlobalChatState,
	conversationId?: string,
): GlobalChatRun[] {
	const target = conversationId ?? state.activeConversationId;
	return Object.values(state.runs)
		.filter((run) => run.conversationId === target)
		.sort((a, b) => a.startedAt - b.startedAt);
}

/** Queued messages of a conversation (defaults to the active one), in send order. */
export function selectGlobalChatQueue(
	state: GlobalChatState,
	conversationId?: string,
): GlobalChatQueuedMessage[] {
	const target = conversationId ?? state.activeConversationId;
	return state.queue.filter((entry) => entry.conversationId === target);
}

/** True when the conversation is at its concurrency cap and further sends must queue. */
export function isGlobalChatAtRunCapacity(
	state: GlobalChatState,
	conversationId?: string,
): boolean {
	return (
		selectGlobalChatRuns(state, conversationId).length >=
		MAX_CONCURRENT_GLOBAL_CHAT_RUNS
	);
}
