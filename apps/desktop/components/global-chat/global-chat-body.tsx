"use client";

import {
	IBitTypes,
	IRole,
	useAssistantSurface,
	useBackend,
	useCopilotSDK,
	useInvoke,
} from "@flow-like/flow-like-ui";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Button,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@flow-like/flow-like-ui";
import { createCopilotStreamParser } from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
import { FlowScriptWorkspacePanel } from "@flow-like/flow-like-ui/components/flowpilot/flowscript-workspace-panel";
import {
	type AIProvider,
	flowPilotModelIdForProvider,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "@flow-like/flow-like-ui/components/flowpilot/types";
import { fileToAttachment } from "@flow-like/flow-like-ui/components/interfaces/chat-default/attachment";
import {
	Chat,
	type IChatRef,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat";
import type { ISendMessageFunction } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox";
import { submitInteractionResponse } from "@flow-like/flow-like-ui/components/interfaces/chat-default/respond-interaction";
import { createId } from "@paralleldrive/cuid2";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	BotIcon,
	BrainIcon,
	CheckIcon,
	ChevronDownIcon,
	Code2Icon,
	FileCode2Icon,
	GithubIcon,
	LayersIcon,
	PackageIcon,
	PlusIcon,
	SparklesIcon,
	WorkflowIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
	globalChatDb,
} from "../../lib/global-chat-db";
import { useGlobalChatStore } from "../../lib/global-chat-store";
import {
	applyStreamEvent,
	createStreamAccumulator,
	mergeUsageStats,
	orderedSteps,
} from "./copilot-stream-steps";
import { GlobalChatHistory } from "./global-chat-history";
import { InlineAppChatCard } from "./inline-app-chat-card";
import { InlineAppPageCard } from "./inline-app-page-card";
import { InlineToolPrompt } from "./inline-tool-prompt";
import { PendingComponentsCard } from "./pending-components-card";

// global_chat streams raw assistant text interleaved with the FlowPilot XML control protocol; the
// shared parser (createCopilotStreamParser) turns chunks into typed events, which we accumulate
// (copilot-stream-steps.ts) into the message's content + plan_steps so the presentational <Chat>
// renders tool activity and reasoning exactly like the board copilot and the simple chat.

const PROVIDERS: Array<{
	id: AIProvider;
	label: string;
	icon: typeof GithubIcon;
}> = [
	{ id: "bits", label: "Profile", icon: LayersIcon },
	{ id: "github-copilot", label: "Copilot", icon: GithubIcon },
	{ id: "codex", label: "Codex", icon: Code2Icon },
	{ id: "claude-code", label: "Claude Code", icon: BotIcon },
];

// Quick-start prompts on the empty chat; the `prompt` is what actually gets sent.
const EMPTY_SUGGESTIONS: Array<{
	label: string;
	icon: typeof SparklesIcon;
	prompt: string;
}> = [
	{ label: "Create an app", icon: PlusIcon, prompt: "Create a new app" },
	{
		label: "Browse the store",
		icon: PackageIcon,
		prompt: "Show me the package store",
	},
	{
		label: "What can I build?",
		icon: SparklesIcon,
		prompt: "What can I build with Flow-Like?",
	},
];

// Radix Select disallows an empty value, so "memory off" uses a sentinel mapped back to "".
const MEMORY_OFF = "__off__";

/** Session-scoped pointer to the active conversation, so reloads/navigation restore the transcript. */
const LAST_CONVERSATION_KEY = "flow-like:global-chat:last-conversation";

function makeMessage(
	role: IRole,
	content: string,
	sessionId: string,
): IMessage {
	return {
		id: createId(),
		appId: GLOBAL_CHAT_APP_ID,
		sessionId,
		inner: { role, content },
		files: [],
		tools: [],
		actions: [],
		timestamp: Date.now(),
	};
}

async function persist(message: IMessage) {
	try {
		await globalChatDb.messages.put(message);
	} catch {
		// history persistence is best-effort in v1
	}
}

/** Create/update the conversation's session row so it shows up in the history list. */
async function persistSession(sessionId: string, title: string) {
	try {
		const existing = await globalChatDb.sessions.get(sessionId);
		const now = Date.now();
		await globalChatDb.sessions.put({
			id: sessionId,
			appId: GLOBAL_CHAT_APP_ID,
			summarization: existing?.summarization || title.slice(0, 80),
			createdAt: existing?.createdAt ?? now,
			updatedAt: now,
		});
	} catch {
		// history persistence is best-effort in v1
	}
}

interface GlobalChatBodyProps {
	variant?: "page" | "overlay";
}

/**
 * The global FlowPilot assistant surface — reused by the full `/chat` page and the docked overlay.
 * Conversation state lives in the global-chat store so both renderers share one transcript; streaming
 * bubbles use the local <Chat> ref, and committed messages are pushed to the store + persisted.
 */
export function GlobalChatBody({ variant = "page" }: GlobalChatBodyProps) {
	const chatRef = useRef<IChatRef>(null);

	const messages = useGlobalChatStore((s) => s.messages);
	const activeConversationId = useGlobalChatStore(
		(s) => s.activeConversationId,
	);
	const isStreaming = useGlobalChatStore((s) => s.isStreaming);
	const provider = useGlobalChatStore((s) => s.provider);
	const selectedModelId = useGlobalChatStore((s) => s.selectedModelId);
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const embeddingModelId = useGlobalChatStore((s) => s.embeddingModelId);
	const setEmbeddingModelId = useGlobalChatStore((s) => s.setEmbeddingModelId);
	const appendMessage = useGlobalChatStore((s) => s.appendMessage);
	const setStreaming = useGlobalChatStore((s) => s.setStreaming);
	const consumeDraft = useGlobalChatStore((s) => s.consumeDraft);
	const inlineAppChats = useGlobalChatStore((s) => s.inlineAppChats);
	const removeInlineAppChat = useGlobalChatStore((s) => s.removeInlineAppChat);
	const inlineAppPages = useGlobalChatStore((s) => s.inlineAppPages);
	const removeInlineAppPage = useGlobalChatStore((s) => s.removeInlineAppPage);
	const toolPrompt = useGlobalChatStore((s) => s.toolPrompt);
	const activeInteractions = useGlobalChatStore((s) => s.activeInteractions);
	const setInteractionResponded = useGlobalChatStore(
		(s) => s.setInteractionResponded,
	);
	const flowscriptWorkspace = useGlobalChatStore((s) => s.flowscriptWorkspace);
	const pendingComponents = useGlobalChatStore((s) => s.pendingComponents);
	// Live board surface (open canvas) the assistant can see and edit — shown as a context chip.
	const boardSurface = useAssistantSurface((s) => s.boardSurface);

	// Remember the active conversation, and restore it after a hard reload (e.g. a dev refresh or a
	// bot-triggered navigation that reloads the window) — otherwise the transcript looks "lost" even
	// though every message is persisted in IndexedDB.
	useEffect(() => {
		if (messages.length === 0) return;
		try {
			sessionStorage.setItem(LAST_CONVERSATION_KEY, activeConversationId);
		} catch {
			// storage unavailable — restore is best-effort
		}
	}, [activeConversationId, messages.length]);

	useEffect(() => {
		const state = useGlobalChatStore.getState();
		if (state.messages.length > 0 || state.isStreaming) return;
		let lastId: string | null = null;
		try {
			lastId = sessionStorage.getItem(LAST_CONVERSATION_KEY);
		} catch {
			return;
		}
		if (!lastId) return;
		void (async () => {
			try {
				const restored = await globalChatDb.messages
					.where("sessionId")
					.equals(lastId)
					.sortBy("timestamp");
				const current = useGlobalChatStore.getState();
				if (
					restored.length > 0 &&
					current.messages.length === 0 &&
					!current.isStreaming
				) {
					// Mid-stream checkpoints may carry unsettled steps — settle them so the
					// restored message doesn't render an eternal spinner.
					const normalized = restored.map((message) => ({
						...message,
						current_step_id: undefined,
						tools: [],
						plan_steps: message.plan_steps?.map((step) =>
							step.status === "progress" || step.status === "planned"
								? { ...step, status: "done" as const }
								: step,
						),
					}));
					current.loadConversation(lastId, normalized);
				}
			} catch {
				// best-effort restore
			}
		})();
	}, []);

	// The streaming bubble lives in the store so the reply keeps rendering when the conversation
	// morphs between /chat and the overlay mid-response. Mirror it into this surface's <Chat> ref
	// via a non-reactive subscription — pushCurrentMessageUpdate already throttles via RAF.
	useEffect(() => {
		const push = (message: IMessage | null) => {
			if (message) chatRef.current?.pushCurrentMessageUpdate(message);
			else chatRef.current?.clearCurrentMessageUpdate();
		};
		push(useGlobalChatStore.getState().streamingMessage);
		return useGlobalChatStore.subscribe((state, prev) => {
			if (state.streamingMessage !== prev.streamingMessage) {
				push(state.streamingMessage);
			}
		});
	}, []);

	// Auth token + identity: profile (Bits) models may need the bearer token, and the assistant's
	// self-awareness context includes the signed-in user (kept fresh via a ref for the send closure).
	const auth = useAuth();
	const authRef = useRef(auth);
	useEffect(() => {
		authRef.current = auth;
	}, [auth]);

	// Embedding models available in the current profile power profile-scoped memory (opt-in).
	const backend = useBackend();
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	const embeddingBits = useInvoke(
		backend.bitState.searchBits,
		backend.bitState,
		[{ bit_types: [IBitTypes.Embedding] }],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const memoryModels = useMemo(() => {
		const profileBits = settingsProfile.data?.hub_profile.bits;
		if (!embeddingBits.data || !profileBits) return [];
		const ids = new Set(profileBits);
		return embeddingBits.data.filter((bit) => ids.has(`${bit.hub}:${bit.id}`));
	}, [embeddingBits.data, settingsProfile.data?.hub_profile.bits]);

	// LLM/VLM bits in the current profile — the selectable models for the "Profile" provider.
	const llmBits = useInvoke(
		backend.bitState.searchBits,
		backend.bitState,
		[{ bit_types: [IBitTypes.Llm, IBitTypes.Vlm] }],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const bitsModels = useMemo(() => {
		const profileBits = settingsProfile.data?.hub_profile.bits;
		if (!llmBits.data || !profileBits) return [];
		const ids = new Set(profileBits);
		return llmBits.data.filter((bit) => ids.has(`${bit.hub}:${bit.id}`));
	}, [llmBits.data, settingsProfile.data?.hub_profile.bits]);

	const normalizedProvider = normalizeAIProvider(provider);
	const isAgent = isAgentBackendProvider(normalizedProvider);
	const activeAgentBackend = isAgentBackendProvider(normalizedProvider)
		? normalizedProvider
		: "github-copilot";
	const copilotSDK = useCopilotSDK(activeAgentBackend);

	// Start the selected external backend so its models/auth become available.
	useEffect(() => {
		if (isAgent && !copilotSDK.isRunning && !copilotSDK.isConnecting) {
			void copilotSDK.start().catch(() => undefined);
		}
	}, [
		isAgent,
		copilotSDK.isRunning,
		copilotSDK.isConnecting,
		copilotSDK.start,
	]);

	// Pick a sensible default model whenever the model list for the active provider changes.
	useEffect(() => {
		if (isAgent) {
			const models = copilotSDK.models;
			if (models.length === 0) return;
			if (!selectedModelId || !models.some((m) => m.id === selectedModelId)) {
				setSelectedModelId(models[0].id);
			}
			return;
		}
		if (!llmBits.data) return;
		if (bitsModels.length === 0) {
			if (selectedModelId) setSelectedModelId("");
			return;
		}
		if (!bitsModels.some((bit) => bit.id === selectedModelId)) {
			const hosted = bitsModels.find(
				(bit) => bit.parameters?.provider?.provider_name === "Hosted",
			);
			setSelectedModelId((hosted ?? bitsModels[0]).id);
		}
	}, [
		isAgent,
		copilotSDK.models,
		llmBits.data,
		bitsModels,
		selectedModelId,
		setSelectedModelId,
	]);

	const handleSendMessage: ISendMessageFunction = useCallback(
		async (content, filesAttached) => {
			const trimmed = content.trim();
			const state = useGlobalChatStore.getState();
			if (state.isStreaming) return;

			// Same attachment handling as the simple chat: files become local tmp files (Tauri) or
			// presigned tmp uploads — only URLs travel through IPC and land in IndexedDB, no blobs.
			const imageFiles = (filesAttached ?? []).filter((file) =>
				file.type.startsWith("image/"),
			);
			const skipped = (filesAttached?.length ?? 0) - imageFiles.length;
			if (skipped > 0) {
				toast.warning(
					"Only image attachments are supported in the global chat right now.",
				);
			}
			let attachments: Awaited<ReturnType<typeof fileToAttachment>> = [];
			if (imageFiles.length > 0) {
				try {
					attachments = await fileToAttachment(imageFiles, backend, true);
				} catch (error) {
					toast.error(
						`Failed to prepare attachments: ${error instanceof Error ? error.message : String(error)}`,
					);
				}
			}
			if (!trimmed && attachments.length === 0) return;
			setStreaming(true);
			useGlobalChatStore.getState().clearPendingAppRefs();
			useGlobalChatStore.getState().clearSubPlanSteps();
			useGlobalChatStore.getState().clearInteractions();
			useGlobalChatStore.getState().clearSubAttachments();
			useGlobalChatStore.getState().clearSubUsageStats();

			const sessionId = state.activeConversationId;
			const priorMessages = state.messages;
			const effectiveModelId = flowPilotModelIdForProvider(
				normalizeAIProvider(state.provider),
				state.selectedModelId,
			);

			const userMessage = makeMessage(IRole.User, trimmed, sessionId);
			userMessage.files = attachments;
			appendMessage(userMessage);
			void persist(userMessage);
			void persistSession(sessionId, trimmed || "Image message");

			const responseMessage = makeMessage(IRole.Assistant, "", sessionId);
			useGlobalChatStore.getState().setStreamingMessage({ ...responseMessage });

			const historyPayload = priorMessages.map((m) => ({
				role: m.inner.role === IRole.Assistant ? "Assistant" : "User",
				content: typeof m.inner.content === "string" ? m.inner.content : "",
			}));

			const parser = createCopilotStreamParser();
			const acc = createStreamAccumulator();
			// Checkpoint the in-flight reply to IndexedDB (throttled) so a hard reload mid-response
			// restores the partial transcript instead of losing the whole turn.
			let lastCheckpoint = 0;
			const syncMessage = () => {
				const state = useGlobalChatStore.getState();
				responseMessage.inner.content = acc.content;
				// Nested sub-agent activity (flowpilot_board) is published by the tool bridge into
				// subPlanSteps — render it inline after this response's own steps.
				responseMessage.plan_steps = [
					...orderedSteps(acc),
					...state.subPlanSteps,
				];
				responseMessage.current_step_id = acc.currentStepId;
				responseMessage.tools = acc.currentStepId ? ["working"] : [];
				responseMessage.app_refs =
					state.pendingAppRefs.length > 0
						? [...state.pendingAppRefs]
						: undefined;
				// Attachments produced by nested app-chat runs (call_app_chat) are published into
				// subAttachments by the tool bridge — render them on this response's message.
				responseMessage.files = state.subAttachments;
				// The agent's own token usage (streamed usage_stat frames) plus stats reported by
				// called apps / sub-agents (subUsageStats) — rendered by <UsageStats>.
				const combinedUsage = mergeUsageStats(
					acc.usageStats,
					state.subUsageStats,
				);
				responseMessage.usage_stats =
					combinedUsage.length > 0 ? combinedUsage : undefined;
				state.setStreamingMessage({ ...responseMessage });
				const now = Date.now();
				if (now - lastCheckpoint > 1_000) {
					lastCheckpoint = now;
					void persist({ ...responseMessage });
				}
			};

			const channel = new Channel<string>();
			channel.onmessage = (chunk) => {
				for (const event of parser.push(chunk)) applyStreamEvent(acc, event);
				syncMessage();
			};

			try {
				const authUser = authRef.current?.user;
				const userContext =
					authUser?.profile?.name ??
					authUser?.profile?.preferred_username ??
					authUser?.profile?.email;
				// Forward the open board (if any) so the assistant knows what "this workflow /
				// these nodes" refers to and routes board questions to flowpilot_board without
				// asking which app/board. Read imperatively at send time — the live surface is the
				// same one the flowpilot_board tool later resolves.
				const surface = useAssistantSurface.getState().boardSurface;
				const boardContext = surface
					? {
							app_id: surface.appId,
							board_id: surface.boardId,
							board_name: surface.board?.name || undefined,
							current_layer: surface.currentLayer || undefined,
							selected_node_ids: surface.selectedNodeIds,
							node_count: surface.board
								? Object.keys(surface.board.nodes ?? {}).length +
									Object.values(surface.board.layers ?? {}).reduce(
										(sum, layer) =>
											sum + Object.keys(layer?.nodes ?? {}).length,
										0,
									)
								: undefined,
						}
					: undefined;
				await invoke("global_chat", {
					scope: "Frontend",
					userPrompt: trimmed,
					attachmentUrls:
						attachments.length > 0
							? attachments.map((attachment) =>
									typeof attachment === "string" ? attachment : attachment.url,
								)
							: undefined,
					history: historyPayload,
					modelId: effectiveModelId,
					embeddingModelId: state.embeddingModelId || undefined,
					token: authUser?.access_token ?? undefined,
					userContext: userContext ?? undefined,
					boardContext,
					channel,
				});
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				// Surface mid-stream failures even when partial content already arrived —
				// a silent stop reads as a crash. The failed step keeps the panel open.
				acc.stepOrder.push("stream-error");
				acc.steps.set("stream-error", {
					id: "stream-error",
					title: "Stream failed",
					description: message.slice(0, 300),
					status: "failed",
					timestamp: Date.now(),
				});
				if (!acc.content) {
					acc.content = `Something went wrong: ${message}`;
				}
			} finally {
				// Emit any held-back partial-tag fragment so replies ending in '<...' are not lost.
				for (const event of parser.flush()) applyStreamEvent(acc, event);
				for (const id of acc.stepOrder) {
					const step = acc.steps.get(id);
					if (step?.status === "progress") {
						acc.steps.set(id, { ...step, status: "done" });
					}
				}
				const finalState = useGlobalChatStore.getState();
				responseMessage.inner.content = acc.content;
				responseMessage.plan_steps = [
					...orderedSteps(acc),
					...finalState.subPlanSteps.map((step) =>
						step.status === "progress"
							? { ...step, status: "done" as const }
							: step,
					),
				];
				responseMessage.current_step_id = undefined;
				responseMessage.tools = [];
				responseMessage.app_refs =
					finalState.pendingAppRefs.length > 0
						? [...finalState.pendingAppRefs]
						: undefined;
				// Bake app-chat attachments into the persisted message before clearing the transient
				// sub-run buffer. Interactions stay in the store so answered dialogs remain visible.
				responseMessage.files = finalState.subAttachments;
				// Bake the turn's usage stats (agent's own + called apps/sub-agents) into the message.
				const finalUsage = mergeUsageStats(
					acc.usageStats,
					finalState.subUsageStats,
				);
				responseMessage.usage_stats =
					finalUsage.length > 0 ? finalUsage : undefined;
				const finalized = { ...responseMessage };
				appendMessage(finalized);
				void persist(finalized);
				finalState.clearPendingAppRefs();
				finalState.clearSubPlanSteps();
				finalState.clearSubAttachments();
				finalState.clearSubUsageStats();
				finalState.setStreamingMessage(null);
				setStreaming(false);
			}
		},
		[appendMessage, setStreaming, backend],
	);

	// Answer an app-chat dialog raised during a call_app_chat run. Responding unblocks the app's
	// workflow (respond_to_interaction / hub API) while the outer tool call is still awaiting.
	const handleRespondToInteraction = useCallback(
		async (interactionId: string, value: unknown) => {
			const interaction = useGlobalChatStore
				.getState()
				.activeInteractions.find((i) => i.id === interactionId);
			if (!interaction) return;
			try {
				await submitInteractionResponse(interaction, value, backend.profile);
				setInteractionResponded(interactionId, value);
			} catch (error) {
				toast.error(
					`Failed to submit response: ${error instanceof Error ? error.message : String(error)}`,
				);
			}
		},
		[backend.profile, setInteractionResponded],
	);

	// Auto-send a pending draft exactly once — handed off from the landing hero bar or attached to
	// a surface's requestOpenAssistant(prompt). Subscribing to the draft (instead of running only on
	// mount) lets it fire in BOTH variants, including when the overlay body is already mounted;
	// consumeDraft() clears the store atomically so a concurrently mounted page/overlay pair cannot
	// double-send. While a response streams the draft stays queued (handleSendMessage would drop it
	// silently) — the isStreaming dependency re-fires the effect when the stream ends.
	//
	// Readiness gate: right after a reload the draft would otherwise fire before auth and the model
	// list settle — modelId undefined makes the backend pick an arbitrary "best" profile model
	// (which can stall for minutes hosting a local model) and the auth token would be missing for
	// hosted ones. The deps re-fire the effect the moment a model is selected.
	const pendingDraft = useGlobalChatStore((s) => s.draft);
	const draftReady =
		!auth.isLoading &&
		(isAgent
			? copilotSDK.models.length > 0 && Boolean(selectedModelId)
			: Boolean(selectedModelId) ||
				(llmBits.data !== undefined && bitsModels.length === 0));
	// biome-ignore lint/correctness/useExhaustiveDependencies: send on new drafts / readiness / stream-end only, not on every handleSendMessage identity change.
	useEffect(() => {
		if (!pendingDraft || !draftReady) return;
		if (useGlobalChatStore.getState().isStreaming) return;
		const draft = consumeDraft();
		if (!draft) return;
		if (draft.modelId) setSelectedModelId(draft.modelId);
		void handleSendMessage(draft.prompt, draft.files);
	}, [pendingDraft, draftReady, isStreaming, consumeDraft, setSelectedModelId]);

	const compact = variant === "overlay";

	// FlowScript workspace layout. The panel can either sit side-by-side with the chat (wide surfaces)
	// or replace it entirely (narrow docks) — decided by the ACTUAL measured width of the layout row,
	// not a viewport media query, so the docked overlay never overflows into horizontal scroll.
	const layoutRef = useRef<HTMLDivElement>(null);
	const [layoutWidth, setLayoutWidth] = useState(0);
	useEffect(() => {
		const el = layoutRef.current;
		if (!el || typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver((entries) => {
			setLayoutWidth(entries[0].contentRect.width);
		});
		observer.observe(el);
		return () => observer.disconnect();
	}, []);

	// The workspace shows by DEFAULT whenever one exists (visibility must not depend on an effect
	// firing — that regressed the panel to hidden). The toolbar chip hides it; a brand-new workspace
	// (null -> present) re-reveals it even if the user had hidden the previous one.
	const hasFlowscript = Boolean(flowscriptWorkspace);
	const [flowscriptHidden, setFlowscriptHidden] = useState(false);
	const hadFlowscriptRef = useRef(false);
	useEffect(() => {
		if (hasFlowscript && !hadFlowscriptRef.current) setFlowscriptHidden(false);
		hadFlowscriptRef.current = hasFlowscript;
	}, [hasFlowscript]);

	// Below this the chat + a ~420px code panel can't coexist without cramping, so the panel replaces
	// the chat instead of squeezing beside it.
	const canSideBySide = layoutWidth >= 768;
	const showWorkspace = hasFlowscript && !flowscriptHidden;
	const sideBySideWorkspace = showWorkspace && canSideBySide;

	// Provider + model live in ONE combined popover selector (see providerModelPicker below) to keep
	// the toolbar compact: the trigger shows the active provider icon + model name, and the popover
	// hosts the provider switch and that provider's model list.
	const modelOptions = useMemo(
		() =>
			isAgent
				? copilotSDK.models.map((model) => ({
						id: model.id,
						label: model.name || model.id,
					}))
				: bitsModels.map((bit) => ({
						id: bit.id,
						label: bit.meta?.en?.name ?? bit.id,
					})),
		[isAgent, copilotSDK.models, bitsModels],
	);
	const [pickerOpen, setPickerOpen] = useState(false);

	const [pendingEmbedding, setPendingEmbedding] = useState<{
		modelId: string;
		profileId: string;
		count: number;
	} | null>(null);

	// Switching the embedding model makes existing (differently-embedded) memories unreadable, so
	// warn + delete on confirm. Only prompts when the profile already has memories from another model.
	const handleEmbeddingChange = useCallback(
		async (value: string) => {
			const newModelId = value === MEMORY_OFF ? "" : value;
			if (newModelId === embeddingModelId) return;
			if (!newModelId) {
				setEmbeddingModelId("");
				return;
			}
			const profileId = settingsProfile.data?.hub_profile.id;
			if (!profileId) {
				setEmbeddingModelId(newModelId);
				return;
			}
			try {
				const status = await invoke<{
					count: number;
					embedding_model_id: string | null;
				}>("global_chat_memory_status", { profileId });
				if (status.count > 0 && status.embedding_model_id !== newModelId) {
					setPendingEmbedding({
						modelId: newModelId,
						profileId,
						count: status.count,
					});
					return;
				}
			} catch {
				// memory status unavailable — switch without prompting
			}
			setEmbeddingModelId(newModelId);
		},
		[
			embeddingModelId,
			setEmbeddingModelId,
			settingsProfile.data?.hub_profile.id,
		],
	);

	const confirmEmbeddingChange = useCallback(async () => {
		if (!pendingEmbedding) return;
		try {
			await invoke("global_chat_clear_memory", {
				profileId: pendingEmbedding.profileId,
			});
		} catch {
			// best-effort delete
		}
		setEmbeddingModelId(pendingEmbedding.modelId);
		setPendingEmbedding(null);
	}, [pendingEmbedding, setEmbeddingModelId]);

	// Memory is only wired into the profile (Bits) agent loop today — hide the picker for the
	// external agent backends so the UI never implies memory that silently no-ops.
	const memoryPicker = useMemo(
		() =>
			!isAgent && memoryModels.length > 0 ? (
				<Select
					value={embeddingModelId || MEMORY_OFF}
					onValueChange={handleEmbeddingChange}
				>
					<SelectTrigger
						className="h-7 data-[size=default]:h-7 min-w-0 max-w-36 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						title="Profile memory embedding model"
					>
						<BrainIcon className="size-3.5 mr-1 text-muted-foreground shrink-0" />
						<SelectValue placeholder="Memory: off" />
					</SelectTrigger>
					<SelectContent className="z-10000">
						<SelectItem value={MEMORY_OFF} className="text-xs">
							Memory: off
						</SelectItem>
						{memoryModels.map((bit) => (
							<SelectItem key={bit.id} value={bit.id} className="text-xs">
								{bit.meta?.en?.name ?? bit.id}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : null,
		[isAgent, memoryModels, embeddingModelId, handleEmbeddingChange],
	);

	const showEmptyState =
		messages.length === 0 &&
		inlineAppChats.length === 0 &&
		inlineAppPages.length === 0 &&
		!isStreaming &&
		// A queued draft is about to send — don't flash the empty state under it.
		!pendingDraft;

	const currentProvider =
		PROVIDERS.find((p) => normalizeAIProvider(p.id) === normalizedProvider) ??
		PROVIDERS[0];
	const CurrentProviderIcon = currentProvider.icon;
	const currentModelLabel = modelOptions.find(
		(option) => option.id === selectedModelId,
	)?.label;

	// Combined provider + model selector — one compact trigger instead of a provider pill row plus a
	// separate model dropdown. The popover switches provider (which reloads its model list) and picks
	// the model; picking a model closes it, switching provider keeps it open.
	const providerModelPicker = (
		<Popover open={pickerOpen} onOpenChange={setPickerOpen}>
			<PopoverTrigger asChild>
				<Button
					variant="outline"
					size="sm"
					title={`${currentProvider.label} · ${currentModelLabel ?? "Select a model"}`}
					className="h-7 shrink-0 gap-1.5 px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
				>
					<CurrentProviderIcon className="size-3.5 shrink-0 text-primary" />
					<span className="max-w-32 truncate">
						{currentModelLabel ?? "Model"}
					</span>
					<ChevronDownIcon className="size-3 shrink-0 opacity-50" />
				</Button>
			</PopoverTrigger>
			<PopoverContent align="start" className="z-10000 w-64 p-2">
				<p className="px-1 pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
					Provider
				</p>
				<div className="flex gap-0.5 rounded-lg border border-border/40 bg-muted/30 p-0.5">
					{PROVIDERS.map(({ id, label, icon: Icon }) => {
						const active = normalizeAIProvider(id) === normalizedProvider;
						return (
							<button
								key={id}
								type="button"
								title={label}
								onClick={() => setProvider(id)}
								className={`flex h-7 flex-1 items-center justify-center rounded-md outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40 ${active ? "bg-linear-to-br from-primary to-purple-600 text-primary-foreground shadow-sm" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}
							>
								<Icon className="size-4" />
							</button>
						);
					})}
				</div>
				<p className="px-1 pb-1.5 pt-2.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
					Model
				</p>
				<div className="max-h-56 space-y-0.5 overflow-y-auto">
					{modelOptions.length === 0 ? (
						<p className="px-2 py-4 text-center text-xs text-muted-foreground">
							{isAgent ? "Starting backend…" : "No models available"}
						</p>
					) : (
						modelOptions.map((option) => {
							const active = option.id === selectedModelId;
							return (
								<button
									key={option.id}
									type="button"
									onClick={() => {
										setSelectedModelId(option.id);
										setPickerOpen(false);
									}}
									className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40 ${active ? "bg-primary/10 text-primary" : "hover:bg-muted"}`}
								>
									<span className="flex-1 truncate">{option.label}</span>
									{active && <CheckIcon className="size-3.5 shrink-0" />}
								</button>
							);
						})
					)}
				</div>
			</PopoverContent>
		</Popover>
	);

	return (
		<div className="flex flex-col flex-1 min-h-0 w-full h-full">
			<header className="flex items-center gap-1.5 px-3 py-2 border-b border-border/50 shrink-0 overflow-x-auto">
				{providerModelPicker}
				{memoryPicker}
				{boardSurface && (
					<div
						className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/5 px-2.5 text-xs text-foreground/80"
						title="The assistant can see and edit this board"
					>
						<WorkflowIcon className="size-3.5 shrink-0 text-primary" />
						<span className="truncate max-w-32">
							{boardSurface.board?.name || "Board"}
						</span>
						{boardSurface.selectedNodeIds.length > 0 && (
							<span className="shrink-0 text-muted-foreground">
								· {boardSurface.selectedNodeIds.length} selected
							</span>
						)}
					</div>
				)}
				{flowscriptWorkspace && (
					<Button
						type="button"
						variant={showWorkspace ? "default" : "outline"}
						size="sm"
						aria-pressed={showWorkspace}
						onClick={() => setFlowscriptHidden((hidden) => !hidden)}
						title={
							showWorkspace
								? "Hide the FlowScript workspace"
								: "Show the FlowScript workspace"
						}
						className="h-7 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					>
						<FileCode2Icon className="size-3.5 shrink-0" />
						FlowScript
						{flowscriptWorkspace.status === "validation_errors" && (
							<span
								className="size-1.5 shrink-0 rounded-full bg-red-500"
								aria-hidden
							/>
						)}
					</Button>
				)}
				<GlobalChatHistory />
			</header>

			{(inlineAppChats.length > 0 ||
				inlineAppPages.length > 0 ||
				pendingComponents !== null) && (
				<div className="shrink-0 max-h-[60vh] overflow-y-auto pt-2">
					<PendingComponentsCard />
					{inlineAppPages.map((page) => (
						<InlineAppPageCard
							key={page.id}
							page={page}
							onClose={removeInlineAppPage}
							compact={compact}
						/>
					))}
					{inlineAppChats.map((chat) => (
						<InlineAppChatCard
							key={chat.id}
							chat={chat}
							onClose={removeInlineAppChat}
							compact={compact}
						/>
					))}
				</div>
			)}

			<div
				ref={layoutRef}
				className={`flex min-h-0 flex-1 ${sideBySideWorkspace ? "flex-row" : "flex-col"}`}
			>
				{/* Must be a flex column: <Chat>'s root sizes itself with flex-1/min-h-0, and without a
				    flex parent its height collapses to content size, breaking the internal scroll area.
				    In a narrow dock the workspace replaces the chat rather than squeezing beside it, so
				    hide (don't unmount) the chat to keep its scroll/stream state alive underneath. */}
				<div
					className={`relative flex flex-col flex-1 min-h-0 ${
						showWorkspace && !canSideBySide ? "hidden" : ""
					}`}
				>
					{showEmptyState && (
						<div className="pointer-events-none absolute inset-x-0 top-0 bottom-28 z-10 flex flex-col items-center justify-center gap-5 px-6 text-center">
							<span className="flex size-16 items-center justify-center rounded-[1.25rem] bg-linear-to-br from-primary/25 via-primary/10 to-purple-600/20 text-primary shadow-xl shadow-primary/25 ring-1 ring-primary/15">
								<SparklesIcon className="size-8" />
							</span>
							<div className="space-y-1.5">
								<h2 className="text-xl font-semibold tracking-tight">
									Chat with FlowPilot
								</h2>
								<p className="mx-auto max-w-xs text-sm text-muted-foreground">
									Ask anything — or let it create apps, open the store, and talk
									to your apps for you.
								</p>
							</div>
							<div className="pointer-events-auto flex flex-wrap items-center justify-center gap-2">
								{EMPTY_SUGGESTIONS.map(({ label, icon: Icon, prompt }) => (
									<Button
										key={label}
										variant="outline"
										size="sm"
										className="h-8 gap-1.5 rounded-full border-border/60 bg-background/80 text-xs text-foreground/80 outline-none transition-all hover:border-primary/40 hover:bg-primary/10 hover:text-primary hover:shadow-sm focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0 motion-safe:hover:-translate-y-px"
										onClick={() => void handleSendMessage(prompt)}
									>
										<Icon className="size-3.5" />
										{label}
									</Button>
								))}
							</div>
						</div>
					)}
					<Chat
						ref={chatRef}
						sessionId={activeConversationId}
						messages={messages}
						onSendMessage={handleSendMessage}
						isStreamActive={isStreaming}
						config={{ allow_file_upload: true, tools: [] }}
						activeInteractions={activeInteractions}
						onRespondToInteraction={handleRespondToInteraction}
						inlinePrompt={
							toolPrompt ? (
								<InlineToolPrompt key={toolPrompt.id} prompt={toolPrompt} />
							) : undefined
						}
					/>
				</div>
				{showWorkspace && flowscriptWorkspace && (
					<FlowScriptWorkspacePanel
						source={flowscriptWorkspace.source}
						status={flowscriptWorkspace.status}
						fill={!canSideBySide}
						onClose={() => setFlowscriptHidden(true)}
					/>
				)}
			</div>

			<AlertDialog
				open={pendingEmbedding !== null}
				onOpenChange={(open) => !open && setPendingEmbedding(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Change memory embedding model?</AlertDialogTitle>
						<AlertDialogDescription>
							This profile has {pendingEmbedding?.count ?? 0} saved{" "}
							{pendingEmbedding?.count === 1 ? "memory" : "memories"} embedded
							with a different model. They can&apos;t be read by the new model,
							so switching will permanently delete them. Continue?
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>Keep current model</AlertDialogCancel>
						<AlertDialogAction onClick={confirmEmbeddingChange}>
							Delete &amp; switch
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}
