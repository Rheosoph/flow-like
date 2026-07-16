import { createId } from "@paralleldrive/cuid2";
import { create } from "zustand";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "../../components/a2ui/types";
import type { FlowScriptWorkspaceCandidate } from "../../components/flowpilot/flowscript-workspace-candidates";
import type { AIProvider } from "../../components/flowpilot/types";
import type {
	IAttachment,
	IChatUsageStat,
	IPlanStep,
} from "../../components/interfaces/chat-default/chat-db";
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

export type GlobalChatMode = "closed" | "overlay";

/** Session-scoped dock visibility, so a hard reload mid-response re-opens the overlay (and thus
 * re-mounts the chat surface that re-attaches to the live stream) instead of hiding it. */
const OVERLAY_MODE_KEY = "flow-like:global-chat:mode";

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

export interface GlobalChatDraft {
	prompt: string;
	/** Backend-prefixed model id (e.g. "github-copilot:…", "codex:…") or a raw Bits id. */
	modelId?: string;
	/** Raw browser files captured on the landing bar, forwarded to the first /chat send. */
	files?: File[];
}

export type GlobalToolPromptResolution =
	| { approved: boolean; remember: boolean }
	| { answer: unknown }
	| null;

/** One selectable option for an `ask_user` single/multiple-choice question. */
export interface GlobalToolAskChoice {
	label: string;
	value?: unknown;
	description?: string;
}

/** Parsed `ask_user` question metadata driving the inline prompt's input controls. */
export interface GlobalToolAsk {
	mode: "freeform" | "single_choice" | "multiple_choice";
	choices: GlobalToolAskChoice[];
	defaultValue?: unknown;
	placeholder?: string;
}

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
	/** Conversation currently shown in both /chat and the overlay. */
	activeConversationId: string;
	/** Committed messages of the active conversation. */
	messages: IMessage[];
	isStreaming: boolean;
	/**
	 * A route a tool asked to open, deferred until the agent turn finishes. Navigating mid-stream
	 * tears down the run, so tools (e.g. flowpilot_widget after creating a page) stash the target
	 * here and the bridge navigates once streaming ends.
	 */
	pendingNavigation: string | null;
	/**
	 * The in-flight assistant reply. Lives in the store (not a surface-local ref) so streaming keeps
	 * rendering when the conversation morphs between the /chat page and the docked overlay mid-response.
	 */
	streamingMessage: IMessage | null;
	/** Pending inline approval/question from the global tool bridge (one at a time). */
	toolPrompt: GlobalToolPrompt | null;
	provider: AIProvider;
	/** Raw (un-prefixed) model id selected in the picker. */
	selectedModelId: string;
	/** Provider-specific reasoning effort ("" = use the selected model's default). */
	reasoningEffort: string;
	/** Embedding bit id used for profile-scoped memory ("" = memory off). */
	embeddingModelId: string;
	/** App chat events the agent surfaced inline in the global chat view. */
	inlineAppChats: InlineAppChat[];
	/** App UI pages the agent embedded inline in the global chat view (artifact-like). */
	inlineAppPages: InlineAppPage[];
	/**
	 * Apps referenced by tools during the current in-flight response. The chat body attaches them
	 * to that assistant message (message.app_refs) so the chips render inline with it.
	 */
	pendingAppRefs: string[];
	/**
	 * Plan steps of a nested sub-agent run (flowpilot_board) during the current response, published
	 * by the tool bridge. Ids carry SUB_STEP_PREFIX; the chat body appends them to the message's
	 * own steps so the user sees the sub-agent working live.
	 */
	subPlanSteps: IPlanStep[];
	/**
	 * Interactions (single/multiple choice, form) raised by a nested app-chat run (call_app_chat)
	 * during the current response. Rendered by the chat body and answered via respond_to_interaction,
	 * unblocking the app workflow while the outer call_app_chat tool call is still in flight.
	 */
	activeInteractions: IInteractionRequest[];
	/**
	 * Attachments produced by a nested app-chat run (call_app_chat), folded into the owning assistant
	 * message's files so generated files/images render in the global chat instead of being dropped.
	 */
	subAttachments: IAttachment[];
	/**
	 * Usage/stats reported by nested runs during the current response: apps called via call_app_chat
	 * (their chat_usage_stat events) and board/widget sub-agents. Folded into the owning message's
	 * usage_stats alongside the agent's own so the <UsageStats> badge covers the whole turn.
	 */
	subUsageStats: IChatUsageStat[];
	/**
	 * FlowScript workspace generated by the latest flowpilot_board run, streamed live so the chat
	 * shows the code as it is written (same panel as the board FlowPilot).
	 */
	flowscriptWorkspace: FlowScriptWorkspaceCandidate | null;
	/** Turn-scoped debug report mirrored into the streaming message and persisted on finalize. */
	debugReport: IAgentDebugReport | null;
	/**
	 * Validated UI components generated by the latest flowpilot_widget run, staged in the chat for
	 * the user to review and apply to the open widget/page builder. Never auto-applied.
	 */
	pendingComponents: {
		components: SurfaceComponent[];
		canvasSettings?: CanvasSettings;
		warnings?: string[];
		/** Builder the components were generated for — Apply refuses other surfaces. */
		surfaceId?: string;
		appId?: string;
	} | null;

	setDraft: (draft: GlobalChatDraft) => void;
	/** Returns the pending draft once and clears it, so it is only auto-sent a single time. */
	consumeDraft: () => GlobalChatDraft | null;
	openOverlay: () => void;
	closeOverlay: () => void;
	/** Defer a route change until the agent turn ends (navigating mid-stream breaks the run). */
	setPendingNavigation: (route: string | null) => void;
	appendMessage: (message: IMessage) => void;
	/** Upsert a message by id — replaces an existing one (e.g. a restored streaming checkpoint that a
	 * resumed run finalizes) or appends it when new. Used to commit finished assistant replies. */
	commitMessage: (message: IMessage) => void;
	setStreaming: (streaming: boolean) => void;
	setStreamingMessage: (message: IMessage | null) => void;
	setToolPrompt: (prompt: GlobalToolPrompt | null) => void;
	setProvider: (provider: AIProvider) => void;
	setSelectedModelId: (modelId: string) => void;
	setReasoningEffort: (effort: string) => void;
	setEmbeddingModelId: (modelId: string) => void;
	addInlineAppChat: (chat: Omit<InlineAppChat, "id">) => void;
	removeInlineAppChat: (id: string) => void;
	addInlineAppPage: (page: Omit<InlineAppPage, "id">) => void;
	removeInlineAppPage: (id: string) => void;
	addPendingAppRef: (appId: string) => void;
	clearPendingAppRefs: () => void;
	/** Replace the nested run's steps and refresh the streaming bubble so they render immediately. */
	setSubPlanSteps: (steps: IPlanStep[]) => void;
	clearSubPlanSteps: () => void;
	/** Merge app-chat interactions (upsert by id, promoting settled status) for inline rendering. */
	addInteractions: (interactions: IInteractionRequest[]) => void;
	/** Mark one interaction as responded so its inline card settles after the user answers. */
	setInteractionResponded: (interactionId: string, value: unknown) => void;
	clearInteractions: () => void;
	/** Append app-chat attachments (deduped by url) to the current response's rendered files. */
	addSubAttachments: (attachments: IAttachment[]) => void;
	clearSubAttachments: () => void;
	/** Append nested-run usage stats (deduped) and merge them into the streaming message live. */
	addSubUsageStats: (stats: IChatUsageStat[]) => void;
	clearSubUsageStats: () => void;
	setFlowscriptWorkspace: (
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

export const useGlobalChatStore = create<GlobalChatState>((set, get) => ({
	draft: null,
	mode: "closed",
	activeConversationId: createId(),
	messages: [],
	isStreaming: false,
	pendingNavigation: null,
	streamingMessage: null,
	toolPrompt: null,
	provider: "bits",
	selectedModelId: "",
	reasoningEffort: "",
	embeddingModelId: "",
	inlineAppChats: [],
	inlineAppPages: [],
	pendingAppRefs: [],
	subPlanSteps: [],
	activeInteractions: [],
	subAttachments: [],
	subUsageStats: [],
	flowscriptWorkspace: null,
	debugReport: null,
	pendingComponents: null,

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
	closeOverlay: () => {
		persistOverlayMode("closed");
		set({ mode: "closed" });
	},
	setPendingNavigation: (pendingNavigation) => set({ pendingNavigation }),
	appendMessage: (message) =>
		set((state) => ({ messages: [...state.messages, message] })),
	commitMessage: (message) =>
		set((state) => {
			const index = state.messages.findIndex((m) => m.id === message.id);
			if (index === -1) return { messages: [...state.messages, message] };
			const messages = [...state.messages];
			messages[index] = message;
			return { messages };
		}),
	setStreaming: (isStreaming) => set({ isStreaming }),
	setStreamingMessage: (streamingMessage) => set({ streamingMessage }),
	setToolPrompt: (toolPrompt) => set({ toolPrompt }),
	setProvider: (provider) => set({ provider }),
	setSelectedModelId: (selectedModelId) => set({ selectedModelId }),
	setReasoningEffort: (reasoningEffort) => set({ reasoningEffort }),
	setEmbeddingModelId: (embeddingModelId) => set({ embeddingModelId }),
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
	addPendingAppRef: (appId) =>
		set((state) =>
			state.pendingAppRefs.includes(appId)
				? state
				: { pendingAppRefs: [...state.pendingAppRefs, appId] },
		),
	clearPendingAppRefs: () => set({ pendingAppRefs: [] }),
	setSubPlanSteps: (subPlanSteps) =>
		set((state) => {
			const message = state.streamingMessage;
			if (!message) return { subPlanSteps };
			// Refresh the streaming bubble immediately — the main channel is silent while the
			// sub-agent runs, so without this the sub-steps would only render on the next token.
			const ownSteps = (message.plan_steps ?? []).filter(
				(step) => !step.id.startsWith(SUB_STEP_PREFIX),
			);
			return {
				subPlanSteps,
				streamingMessage: {
					...message,
					plan_steps: [...ownSteps, ...subPlanSteps],
				},
			};
		}),
	clearSubPlanSteps: () =>
		set((state) =>
			state.subPlanSteps.length > 0 ? { subPlanSteps: [] } : state,
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
	addSubAttachments: (attachments) =>
		set((state) => {
			if (attachments.length === 0) return state;
			const urlOf = (attachment: IAttachment) =>
				typeof attachment === "string" ? attachment : attachment.url;
			const seen = new Set(state.subAttachments.map(urlOf));
			const fresh = attachments.filter((attachment) => {
				const url = urlOf(attachment);
				if (seen.has(url)) return false;
				seen.add(url);
				return true;
			});
			if (fresh.length === 0) return state;
			const subAttachments = [...state.subAttachments, ...fresh];
			const message = state.streamingMessage;
			if (!message) return { subAttachments };
			return {
				subAttachments,
				streamingMessage: { ...message, files: subAttachments },
			};
		}),
	clearSubAttachments: () =>
		set((state) =>
			state.subAttachments.length > 0 ? { subAttachments: [] } : state,
		),
	addSubUsageStats: (stats) =>
		set((state) => {
			if (stats.length === 0) return state;
			const seen = new Set(state.subUsageStats.map((s) => JSON.stringify(s)));
			const fresh = stats.filter((stat) => {
				const signature = JSON.stringify(stat);
				if (seen.has(signature)) return false;
				seen.add(signature);
				return true;
			});
			if (fresh.length === 0) return state;
			const subUsageStats = [...state.subUsageStats, ...fresh];
			const message = state.streamingMessage;
			if (!message) return { subUsageStats };
			// Merge into the message's current stats (which already carry the agent's own),
			// deduped, so nested stats render live while the main channel is quiet.
			const existing = message.usage_stats ?? [];
			const known = new Set(existing.map((s) => JSON.stringify(s)));
			const merged = [...existing];
			for (const stat of fresh) {
				const signature = JSON.stringify(stat);
				if (!known.has(signature)) {
					known.add(signature);
					merged.push(stat);
				}
			}
			return {
				subUsageStats,
				streamingMessage: { ...message, usage_stats: merged },
			};
		}),
	clearSubUsageStats: () =>
		set((state) =>
			state.subUsageStats.length > 0 ? { subUsageStats: [] } : state,
		),
	setFlowscriptWorkspace: (flowscriptWorkspace) => set({ flowscriptWorkspace }),
	beginDebugReport: (messageId, metadata) => {
		beginAgentGenerationMetrics(messageId, metadata?.startedAtMs);
		if (!FLOWPILOT_DEBUG_ENABLED) return;
		set((state) => {
			if (state.debugReport?.message_id === messageId) return state;
			return { debugReport: createAgentDebugReport(messageId, metadata) };
		});
	},
	recordDebugEvent: (messageId, event) => {
		recordAgentGenerationMetricEvent(messageId, event);
		if (!FLOWPILOT_DEBUG_ENABLED) return;
		set((state) => {
			if (state.debugReport?.message_id !== messageId) return state;
			const debugReport = recordAgentDebugEvent(state.debugReport, event);
			const message = state.streamingMessage;
			return {
				debugReport,
				...(message?.id === messageId
					? { streamingMessage: { ...message, debug_report: debugReport } }
					: {}),
			};
		});
	},
	finalizeDebugReport: (messageId, options) => {
		finalizeAgentGenerationMetrics(messageId, options.outcome, {
			publish: !FLOWPILOT_DEBUG_ENABLED,
		});
		if (!FLOWPILOT_DEBUG_ENABLED) return null;
		const report = get().debugReport;
		if (report?.message_id !== messageId) return null;
		const finalized = finalizeAgentDebugReport(report, options);
		set((state) => ({
			debugReport: finalized,
			...(state.streamingMessage?.id === messageId
				? {
						streamingMessage: {
							...state.streamingMessage,
							debug_report: finalized,
						},
					}
				: {}),
		}));
		return finalized;
	},
	clearDebugReport: (messageId) => {
		if (messageId) clearAgentGenerationMetrics(messageId);
		set((state) =>
			!state.debugReport ||
			(messageId && state.debugReport.message_id !== messageId)
				? state
				: { debugReport: null },
		);
	},
	setPendingComponents: (pendingComponents) => set({ pendingComponents }),
	// Start a fresh conversation WITHOUT touching `mode`: clicking "New chat" (or deleting the
	// active chat) from the docked overlay must keep the dock open, not close it.
	newConversation: () =>
		set({
			activeConversationId: createId(),
			messages: [],
			isStreaming: false,
			streamingMessage: null,
			inlineAppChats: [],
			inlineAppPages: [],
			pendingAppRefs: [],
			subPlanSteps: [],
			activeInteractions: [],
			subAttachments: [],
			subUsageStats: [],
			flowscriptWorkspace: null,
			debugReport: null,
			pendingComponents: null,
		}),
	loadConversation: (conversationId, messages) =>
		set({
			activeConversationId: conversationId,
			messages: messages.map(markRestoredMessageDebugReportStale),
			isStreaming: false,
			streamingMessage: null,
			inlineAppChats: [],
			inlineAppPages: [],
			pendingAppRefs: [],
			subPlanSteps: [],
			activeInteractions: [],
			subAttachments: [],
			subUsageStats: [],
			flowscriptWorkspace: null,
			debugReport: null,
			pendingComponents: null,
		}),
}));
