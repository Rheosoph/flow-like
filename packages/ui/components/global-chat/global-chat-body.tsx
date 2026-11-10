"use client";

import { resolveMainChatWidget } from "../interfaces/chat-default/main-widget-state";

import {
	getWorkspaceEvaluation,
	workspaceEvaluationAllowsApp,
} from "../../lib/flowpilot/workspace-evaluation";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { FileCode2Icon, WorkflowIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	IBitTypes,
	IRole,
	isHostedLlmModel,
	selectProfileLlmModels,
	useAssistantSurface,
	useBackend,
	useCopilotSDK,
	useInvoke,
} from "../../index";
import { Button } from "../../index";
import { cn } from "../../lib";
import { getApiOrigin } from "../../lib/api-url";
import { searchAllBitsOfType } from "../../lib/bit/model-listing";
import { DEFAULT_CHAT_AI_DISCLOSURE } from "../../lib/chat-appearance";
import { FLOWPILOT_DEBUG_ENABLED } from "../../lib/flowpilot-debug";
import { requiresFreshToolApproval } from "../../lib/frontend-tool-approval-scope";
import {
	type NativeActionRequest,
	assertNativeActionCurrent,
	nativeChatOutcome,
	registerNativeActionRun,
	runNativeAction,
} from "../../lib/native-action-result";
import { isTauri } from "../../lib/platform";
import type { IEventPayloadChat } from "../../lib/schema/flow/event-payload-chat";
import { captureWidgetSnapshots } from "../../lib/widget-snapshot";
import type { ComposerAttachSource } from "../../state/composer-capture";
import {
	botHasInboxHistory,
	desktopInteractiveArgs,
	ensureBotTarget,
	registerBotInbox,
	sendInteractiveMessage,
	webInteractiveBody,
} from "../../state/global-chat/bot-inbox";
import {
	ASSISTANT_BOT_ID,
	conversationBotId,
	currentConversationId,
	desktopChatBotArgs,
	isBotConversation,
	webChatBotBody,
} from "../../state/global-chat/global-chat-bots";
import {
	type IMessage,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";
import {
	FLOWPILOT_RATING_WITHDRAWN,
	buildFlowPilotFeedbackContext,
	flowPilotRatingForUi,
	submitFlowPilotFeedback,
} from "../../state/global-chat/global-chat-feedback";
import {
	globalChatModelInput,
	persistGlobalChatReaction,
} from "../../state/global-chat/global-chat-message-interaction";
import {
	type PreparingChatReply,
	prepareChatReply,
	useChatPreparationStore,
} from "../../state/global-chat/global-chat-preparation";
import {
	AGENT_MODEL_KEY,
	type GlobalChatRun,
	type InlineAppChat,
	type InlineAppPage,
	type InlineAppSurface,
	type InlineCardAnchor,
	LAST_CONVERSATION_KEY,
	MAX_CONCURRENT_GLOBAL_CHAT_RUNS,
	beginGlobalChatTurnSelection,
	isGlobalChatAtRunCapacity,
	rememberGlobalChatConversation,
	selectAutoMode,
	selectToolPromptConversationId,
	selectToolPromptHere,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import {
	cancelGlobalChatRun,
	clearGlobalChatQueueDrain,
	driveGlobalChatStream,
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	persistGlobalChatSession,
	readActiveRun,
	restoreGlobalChatConversation,
	setActiveRun,
	setGlobalChatQueueDrain,
	steerGlobalChatRun,
	switchGlobalChatBot,
	tauriStart,
} from "../../state/global-chat/global-chat-stream";
import { runGlobalChatTool } from "../../state/global-chat/global-chat-tool-registry";
import { webGlobalChatStart } from "../../state/global-chat/global-chat-web-transport";
import { useFollowBotChat } from "../../state/global-chat/use-follow-bot-chat";
import {
	type VoiceSend,
	registerVoiceTurnSender,
	runVoiceTurn,
} from "../../state/global-chat/voice-turn";
import { BotFileAssets } from "../bots-tab/bot-file-assets";
import { FlowScriptWorkspacePanel } from "../flowpilot/flowscript-workspace-panel";
import {
	flowPilotModelIdForProvider,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../flowpilot/types";
import {
	fileToAttachment,
	fitsInlineUnchanged,
} from "../interfaces/chat-default/attachment";
import type {
	IChatConcurrency,
	IChatGroup,
	IChatRef,
} from "../interfaces/chat-default/chat";
import {
	type IChatMessageInteraction,
	createMessageReference,
} from "../interfaces/chat-default/chat-message-interaction";
import { ChatSurface } from "../interfaces/chat-default/chat-surface";
import type {
	ChatBoxThinking,
	ISendMessageFunction,
} from "../interfaces/chat-default/chatbox";
import type { MessageInlineContent } from "../interfaces/chat-default/message";
import { submitInteractionResponse } from "../interfaces/chat-default/respond-interaction";
import { useStorageAttachSource } from "../storage/use-storage-attach-source";
import {
	agentThinkingOptions,
	chatThinkingLevel,
	chatThinkingOptions,
	isValidReasoningEffort,
	turnThinking,
} from "../thinking/thinking-options";
import { useThinkingLabels } from "../thinking/thinking-picker";
import { BotChatActions, BotSettingsDialog } from "./bot-chat-actions";
import { BotWelcomeMark } from "./bot-chat-parts";
import { BotReplyNote } from "./bot-reply-note";
import { useChatActivityVisible } from "./chat-activity";
import { InlineAppChatCard } from "./inline-app-chat-card";
import { InlineAppPageCard } from "./inline-app-page-card";
import { InlineAppSurfaceCard } from "./inline-app-surface-card";
import { InlineToolPrompt, ToolPromptElsewhere } from "./inline-tool-prompt";
import { resolveModelSelection, selectedModelLabel } from "./model-selection";
import { PendingComponentsCard } from "./pending-components-card";
import { useHydrateAgentSelection } from "./use-agent-persistence";
import {
	type GlobalChatBotView,
	useGlobalChatBot,
} from "./use-global-chat-bot";
import { useGlobalChatRunWidgetAction } from "./use-global-widget-action";
import { useOpenAppId } from "./use-open-app-id";
import { usePageSnapshotSource } from "./use-page-snapshot-source";

// The streaming engine (parse the FlowPilot protocol → message content + plan_steps → store) lives
// in lib/global-chat-stream.ts, OUTSIDE this component, so a turn survives the page↔overlay morph
// and a hard reload (re-attaching to the live Rust run via global_chat_resume). This surface only
// builds the send payload, then renders the store's shared transcript + streaming bubble.

/**
 * Flux's empty chat suggests what it builds. They fill the composer, so they are localized: Flux
 * answers in the language it was asked in.
 */
function useFluxExamples(): readonly string[] {
	const { t } = useTranslation("chat");
	return useMemo(
		() => [
			t("createANewApp", "Create a new app"),
			t("showMeThePackageStore", "Show me the package store"),
			t("whatCanIBuildWithFlowlike", "What can I build with Flow-Like?"),
		],
		[t],
	);
}

/** A bot chat is configured like a chat event that takes files; Flux's adds its suggestions. */
function useBotChatConfig(
	chatBot: GlobalChatBotView,
): Partial<IEventPayloadChat> {
	const aiDisclosure = useChatAiDisclosure(chatBot);
	const fluxExamples = useFluxExamples();
	const flux = chatBot.botId === null;
	return useMemo(
		() => ({
			allow_file_upload: true,
			ai_disclosure: aiDisclosure,
			example_messages: flux ? [...fluxExamples] : undefined,
		}),
		[aiDisclosure, flux, fluxExamples],
	);
}

/** The footer names the bot the chat talks to; a bot that's gone gets the neutral default. */
function useChatAiDisclosure(chatBot: GlobalChatBotView): string {
	const { t } = useTranslation("chat");
	const builtIn = chatBot.botId === null || chatBot.botId === ASSISTANT_BOT_ID;
	if (!chatBot.bot && !builtIn) return DEFAULT_CHAT_AI_DISCLOSURE;
	return t(
		"botAiDisclosure",
		"{{name}} is an AI assistant. Responses may be inaccurate—check important information.",
		{ name: chatBot.name },
	);
}

/** Live replies as one string: streamed tokens rewrite `runs` but leave this unchanged. */
function liveReplyIdsKey(runs: readonly GlobalChatRun[]) {
	let key = "";
	for (const run of runs) if (run.message) key += `${run.message.id}\n`;
	return key;
}

/** Messages the transcript renders: the settled ones plus the live replies of `liveReplyIdsKey`. */
function transcriptIds(messages: readonly IMessage[], liveReplies: string) {
	const ids = new Set(liveReplies.split("\n"));
	ids.delete("");
	for (const message of messages) ids.add(message.id);
	return ids;
}

interface InlineCardSources {
	pages: readonly InlineAppPage[];
	surfaces: readonly InlineAppSurface[];
	chats: readonly InlineAppChat[];
	onClosePage: (id: string) => void;
	onCloseSurface: (id: string) => void;
	onCloseChat: (id: string) => void;
	compact: boolean;
}

interface InlineCardLayout {
	/** Cards rendered inside a transcript message, at offsets into its text. */
	byMessage: Map<string, MessageInlineContent[]>;
	/** Unanchored cards and cards whose reply is not in the transcript. */
	trailing: ReactNode[];
}

/** Cards render inside the reply that added them, at its text offset; the rest trail the transcript. */
function layoutInlineCards(
	sources: InlineCardSources,
	messageIds: ReadonlySet<string>,
) {
	const layout: InlineCardLayout = { byMessage: new Map(), trailing: [] };
	const place = (card: InlineCardAnchor & { id: string }, node: ReactNode) => {
		const { messageId, content_offset: offset } = card;
		if (!messageId || offset === undefined || !messageIds.has(messageId)) {
			layout.trailing.push(node);
			return;
		}
		const entry = { key: card.id, offset, node };
		const entries = layout.byMessage.get(messageId);
		if (entries) entries.push(entry);
		else layout.byMessage.set(messageId, [entry]);
	};
	const { compact } = sources;
	for (const page of sources.pages) {
		place(
			page,
			<InlineAppPageCard
				key={page.id}
				page={page}
				onClose={sources.onClosePage}
				compact={compact}
			/>,
		);
	}
	for (const surface of sources.surfaces) {
		place(
			surface,
			<InlineAppSurfaceCard
				key={surface.id}
				surface={surface}
				onClose={sources.onCloseSurface}
				compact={compact}
			/>,
		);
	}
	for (const chat of sources.chats) {
		place(
			chat,
			<InlineAppChatCard
				key={chat.id}
				chat={chat}
				onClose={sources.onCloseChat}
				compact={compact}
			/>,
		);
	}
	return layout;
}

interface GlobalChatBodyProps {
	variant?: "page" | "overlay";
	/** Every composer edit; a stable reference that must not update the surrounding chat's state. */
	onDraftChange?: (draft: string) => void;
	/**
	 * The bot whose chat a Bots tab dock shows (null = Flux). The dock opens that bot's conversation
	 * itself, so this surface restores nothing on mount and leaves Flux's drafts until the dock is
	 * Flux's. Omitted, the surface shows whichever conversation is active.
	 */
	botId?: string | null;
	/** The bot's pill floats over the top: the transcript starts below it and scrolls under it. */
	pillInset?: boolean;
}

/**
 * The global chat surface — Flux (FlowPilot) and every bot — reused by the Bots tab's dock and the
 * docked overlay. Conversation state lives in the global-chat store so both renderers share one
 * transcript; streaming bubbles use the local <Chat> ref, and committed messages are pushed to the
 * store + persisted.
 */
export function GlobalChatBody({
	variant = "page",
	onDraftChange,
	botId: dockBotId,
	pillInset = false,
}: GlobalChatBodyProps) {
	const { t, i18n } = useTranslation("chat");
	const chatRef = useRef<IChatRef | null>(null);
	// The transcript mounts once the welcome gives way, after the replies it must show started.
	const attachChat = useCallback((chat: IChatRef | null) => {
		chatRef.current = chat;
		chat?.pushCurrentMessageUpdate(
			useGlobalChatStore.getState().streamingMessages,
		);
	}, []);

	const messages = useGlobalChatStore((s) => s.messages);
	const activeConversationId = useGlobalChatStore(
		(s) => s.activeConversationId,
	);
	useFollowBotChat(activeConversationId);
	const chatBot = useGlobalChatBot();
	const chatBotNameRef = useRef(chatBot.name);
	chatBotNameRef.current = chatBot.name;
	const chatConfig = useBotChatConfig(chatBot);
	const noteBotId = chatBot.bot?.id ?? chatBot.botId;
	const botNotes = useMemo<IChatGroup | undefined>(
		() =>
			noteBotId
				? {
						renderMessageHeader: (message) => (
							<BotReplyNote
								message={message}
								botId={noteBotId}
								sessionId={activeConversationId}
							/>
						),
					}
				: undefined,
		[noteBotId, activeConversationId],
	);
	const isStreaming = useGlobalChatStore((s) => s.isStreaming);
	const conversationLoading = useGlobalChatStore((s) => s.conversationLoading);
	const provider = useGlobalChatStore((s) => s.provider);
	const selectedModelId = useGlobalChatStore((s) => s.selectedModelId);
	const reasoningEffort = useGlobalChatStore((s) => s.reasoningEffort);
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const setReasoningEffort = useGlobalChatStore((s) => s.setReasoningEffort);
	// Explicit picks persist across sessions; the "keep a valid model" fallbacks
	// below use the plain setters so a still-loading catalog can never clobber them.
	const selectReasoningEffort = useGlobalChatStore(
		(s) => s.selectReasoningEffort,
	);
	const promptAutoMode = useGlobalChatStore((s) =>
		selectAutoMode(s, selectToolPromptConversationId(s)),
	);
	const enableOverlayAutoOpen = useGlobalChatStore(
		(s) => s.enableOverlayAutoOpen,
	);
	const appendMessage = useGlobalChatStore((s) => s.appendMessage);
	const consumeDraft = useGlobalChatStore((s) => s.consumeDraft);
	const runs = useGlobalChatStore((s) => s.runs);
	const queue = useGlobalChatStore((s) => s.queue);
	const preparations = useChatPreparationStore((s) => s.replies);
	const preparingReplies = useMemo(
		() =>
			Object.values(preparations).filter(
				(reply) =>
					currentConversationId(reply.conversationId) === activeConversationId,
			),
		[preparations, activeConversationId],
	);
	const preparingReplyText = t(
		"preparingBotReply",
		"Got it. I’m getting started.",
	);
	const removeQueuedMessage = useGlobalChatStore((s) => s.removeQueuedMessage);
	const activeRuns = useMemo(
		() =>
			Object.values(runs)
				.filter((run) => run.conversationId === activeConversationId)
				.sort((a, b) => a.startedAt - b.startedAt),
		[runs, activeConversationId],
	);
	const queuedMessages = useMemo(
		() =>
			queue.filter((entry) => entry.conversationId === activeConversationId),
		[queue, activeConversationId],
	);
	const inlineAppChats = useGlobalChatStore((s) => s.inlineAppChats);
	const removeInlineAppChat = useGlobalChatStore((s) => s.removeInlineAppChat);
	const inlineAppPages = useGlobalChatStore((s) => s.inlineAppPages);
	const removeInlineAppPage = useGlobalChatStore((s) => s.removeInlineAppPage);
	const inlineAppSurfaces = useGlobalChatStore((s) => s.inlineAppSurfaces);
	const removeInlineAppSurface = useGlobalChatStore(
		(s) => s.removeInlineAppSurface,
	);
	const toolPrompt = useGlobalChatStore((s) => s.toolPrompt);
	// The card lives in the chat of the run that asks; any other chat points there.
	const toolPromptHere = useGlobalChatStore(selectToolPromptHere);
	const toolPromptChat = useGlobalChatStore(selectToolPromptConversationId);
	const promptSlot = !toolPrompt ? undefined : toolPromptHere ? (
		<InlineToolPrompt key={toolPrompt.id} prompt={toolPrompt} />
	) : (
		toolPromptChat && <ToolPromptElsewhere conversationId={toolPromptChat} />
	);
	const activeInteractions = useGlobalChatStore((s) => s.activeInteractions);
	const setInteractionResponded = useGlobalChatStore(
		(s) => s.setInteractionResponded,
	);
	const flowscriptWorkspace = useGlobalChatStore((s) => s.flowscriptWorkspace);
	const pendingComponents = useGlobalChatStore((s) => s.pendingComponents);
	const handlePageInteraction = useCallback(() => {
		if (variant === "page") enableOverlayAutoOpen();
	}, [enableOverlayAutoOpen, variant]);
	// Turning auto mode on mid-run settles approval cards whose promises the bridge captured
	// before the flip, only when it is the asking run's owner's; queued ones drain as each is
	// answered. `ask` prompts are never auto-answered — auto mode waives permission, not questions.
	useEffect(() => {
		if (
			!promptAutoMode ||
			toolPrompt?.kind !== "approval" ||
			requiresFreshToolApproval(toolPrompt.toolName)
		)
			return;
		toolPrompt.respond({ approved: true, remember: false });
	}, [promptAutoMode, toolPrompt]);
	// Live board surface (open canvas) the assistant can see and edit — shown as a context chip.
	const boardSurface = useAssistantSurface((s) => s.boardSurface);
	const runWidgetAction = useGlobalChatRunWidgetAction();

	// Remember the active conversation, and restore it after a hard reload (e.g. a dev refresh or a
	// bot-triggered navigation that reloads the window) — otherwise the transcript looks "lost" even
	// though every message is persisted in IndexedDB.
	useEffect(() => {
		if (messages.length === 0) return;
		rememberGlobalChatConversation(activeConversationId);
	}, [activeConversationId, messages.length]);

	// A dock opens its bot's conversation itself; only the overlay restores the last one.
	const pinnedToDock = dockBotId !== undefined;
	useEffect(() => {
		if (pinnedToDock) return;
		const state = useGlobalChatStore.getState();
		// A pending hero-bar draft owns this mount: restoring the previous conversation underneath
		// it would land the prompt in the old transcript (racy, depends on IndexedDB latency).
		if (state.messages.length > 0 || state.isStreaming || state.draft) return;
		let lastId: string | null = null;
		try {
			lastId = sessionStorage.getItem(LAST_CONVERSATION_KEY);
		} catch {
			return;
		}
		// No restore pointer, but a run is still in flight: the user started a new chat while the
		// previous turn was generating, then reloaded. Restore THAT conversation instead of leaving
		// the live Rust run orphaned — an empty new chat has nothing to lose, an unfinalized answer
		// does. `readActiveRun` survives the reload; only the run's own teardown clears it.
		if (!lastId) lastId = readActiveRun()?.conversationId ?? null;
		if (!lastId) return;
		// Shared restore path (also used by the history popover): normalizes stale checkpoints and
		// re-attaches any run that was still streaming when the webview reloaded. `skipIfBusy`
		// re-checks after the Dexie read so a conversation/draft that appeared meanwhile wins.
		restoreGlobalChatConversation(lastId, { skipIfBusy: true }).catch(() => {
			// best-effort restore
		});
	}, [pinnedToDock]);

	// Streaming bubbles live in the store so replies keep rendering when the conversation morphs
	// between the Bots tab and the overlay mid-response. Mirror them into this surface's <Chat> ref via a
	// non-reactive subscription — pushCurrentMessageUpdate already throttles via RAF. There may be
	// several at once; <Chat> keys them by message id and renders one live bubble each.
	useEffect(
		() =>
			useGlobalChatStore.subscribe((state, prev) => {
				if (state.streamingMessages !== prev.streamingMessages) {
					chatRef.current?.pushCurrentMessageUpdate(state.streamingMessages);
				}
			}),
		[],
	);

	// Restore the last explicit provider/model/effort. The Bots tab is often the first
	// FlowPilot surface mounted (deep link, mobile bottom nav), so it has to
	// hydrate the shared store itself rather than relying on the hero.
	useHydrateAgentSelection();

	// Auth token + identity: profile (Bits) models may need the bearer token, and the assistant's
	// self-awareness context includes the signed-in user (kept fresh via a ref for the send closure).
	const auth = useAuth();
	const authRef = useRef(auth);
	authRef.current = auth;

	// Embedding models available in the current profile power profile-scoped memory (opt-in).
	const backend = useBackend();
	registerBotInbox(backend.inboxState ?? null);
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	// Mirrored into a ref for the same reason as `auth`: the rating callback must keep a stable
	// identity or MessageComponent's memo re-renders the whole transcript on every profile refetch.
	const settingsProfileRef = useRef(settingsProfile.data);
	useEffect(() => {
		settingsProfileRef.current = settingsProfile.data;
	}, [settingsProfile.data]);
	const nativeIdentity = useRef({
		backend,
		auth,
		profile: settingsProfile.data?.hub_profile,
	});
	nativeIdentity.current = {
		backend,
		auth,
		profile: settingsProfile.data?.hub_profile,
	};
	const getNativeScope = useCallback(() => {
		const identity = nativeIdentity.current;
		if (
			identity.auth.isLoading ||
			(identity.auth.isAuthenticated && !identity.auth.user?.profile.sub)
		)
			return;
		const profile = identity.backend.profile ?? identity.profile;
		if (!profile?.id) return;
		return JSON.stringify([
			getApiOrigin(profile),
			profile.id,
			(identity.auth.isAuthenticated
				? identity.auth.user?.profile.sub
				: undefined) ?? "local",
		]);
	}, []);

	// LLM/VLM bits in the current profile — the selectable models for the "Profile" provider.
	const llmBits = useInvoke(
		searchAllBitsOfType,
		backend.bitState,
		[[IBitTypes.Llm, IBitTypes.Vlm]],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const customBits = useInvoke(
		backend.bitState.listCustomBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const {
		canHostLlamaCPP,
		canHostMLX,
		canUseNativeAgentProviders,
		deviceModels,
	} = backend.capabilities();
	const bitsModels = useMemo(
		() =>
			selectProfileLlmModels(
				llmBits.data,
				customBits.data,
				settingsProfile.data?.hub_profile.bits,
				{
					canHostLlamaCPP,
					canHostMLX,
					canUseNativeAgentProviders,
					deviceModels,
				},
			),
		[
			llmBits.data,
			customBits.data,
			settingsProfile.data?.hub_profile.bits,
			canHostLlamaCPP,
			canHostMLX,
			canUseNativeAgentProviders,
			deviceModels,
		],
	);

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

	// Pick a sensible default model whenever the model list for the active provider
	// changes — but re-apply the user's remembered pick the moment the catalog that
	// offers it loads, so a slow/fallback catalog can't strand them on another model.
	// Uses the plain setter throughout: none of this is a new user choice.
	useEffect(() => {
		let remembered: string | null = null;
		try {
			remembered = localStorage.getItem(AGENT_MODEL_KEY);
		} catch {}
		if (isAgent) {
			const models = copilotSDK.models;
			const nextModelId = resolveModelSelection({
				models,
				selectedModelId,
				rememberedModelId: remembered,
				canReplaceInvalidSelection: copilotSDK.hasLoadedModelCatalog,
			});
			if (nextModelId !== null) setSelectedModelId(nextModelId);
			return;
		}
		if (!llmBits.data) return;
		if (bitsModels.length === 0) {
			if (selectedModelId) setSelectedModelId("");
			return;
		}
		if (
			remembered &&
			remembered !== selectedModelId &&
			bitsModels.some((bit) => bit.id === remembered)
		) {
			setSelectedModelId(remembered);
			return;
		}
		if (!bitsModels.some((bit) => bit.id === selectedModelId)) {
			const hosted = bitsModels.find(isHostedLlmModel);
			setSelectedModelId((hosted ?? bitsModels[0]).id);
		}
	}, [
		isAgent,
		copilotSDK.models,
		copilotSDK.hasLoadedModelCatalog,
		llmBits.data,
		bitsModels,
		selectedModelId,
		setSelectedModelId,
	]);

	const selectedAgentModel = isAgent
		? copilotSDK.models.find((model) => model.id === selectedModelId)
		: undefined;

	// Do not validate against the hook's metadata-free static fallback. Once the
	// backend has returned a real catalog (`[]` or populated), stale persisted
	// values are reset to the default instead of being sent to an incompatible model.
	useEffect(() => {
		if (!reasoningEffort) return;
		if (
			isAgent &&
			selectedAgentModel?.supportedReasoningEfforts === undefined
		) {
			return;
		}
		if (!isValidReasoningEffort(isAgent, reasoningEffort, selectedAgentModel)) {
			setReasoningEffort("");
		}
	}, [isAgent, reasoningEffort, selectedAgentModel, setReasoningEffort]);

	const thinkingLabels = useThinkingLabels();
	const thinkingPicker = useMemo<ChatBoxThinking | undefined>(() => {
		const options = isAgent
			? agentThinkingOptions(selectedAgentModel, thinkingLabels)
			: chatThinkingOptions(thinkingLabels);
		if (options.length === 0) return undefined;
		return {
			value: isAgent ? reasoningEffort : chatThinkingLevel(reasoningEffort),
			options,
			onChange: selectReasoningEffort,
		};
	}, [
		isAgent,
		selectedAgentModel,
		thinkingLabels,
		reasoningEffort,
		selectReasoningEffort,
	]);

	const sendPreparedMessage = useCallback(
		async (
			content: string,
			filesAttached?: File[],
			nativeRequest?: NativeActionRequest,
			interaction?: IChatMessageInteraction,
			previouslyAcceptedReaction = false,
			preparation?: PreparingChatReply,
			voice?: VoiceSend,
		): Promise<IMessage | undefined> => {
			let reactionAccepted = previouslyAcceptedReaction;
			const acceptReaction = async (conversationId: string) => {
				if (reactionAccepted || interaction?.type !== "reaction") return;
				await persistGlobalChatReaction(interaction, conversationId);
				reactionAccepted = true;
			};
			const checkCurrent = () => {
				if (nativeRequest)
					assertNativeActionCurrent(nativeRequest, getNativeScope());
			};
			const authorizeNativeRequest = async () => {
				if (!nativeRequest) return;
				checkCurrent();
				const profile = await backend.userState.getProfile();
				checkCurrent();
				const auth = authRef.current;
				assertNativeActionCurrent(
					nativeRequest,
					JSON.stringify([
						getApiOrigin(profile),
						profile.id,
						(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ??
							"local",
					]),
				);
			};
			checkCurrent();
			const trimmed = content.trim();
			const state = useGlobalChatStore.getState();
			// The send belongs to the chat it was typed in: a switch during the awaits below must not
			// move it, fill it from another chat's limit or name it after another bot.
			const sessionId = state.activeConversationId;
			const botName = chatBotNameRef.current;
			// Concurrency is allowed up to the cap; past it the message is queued rather than
			// dropped, and drains as soon as a turn finishes.
			const atCapacity = () =>
				isGlobalChatAtRunCapacity(
					useGlobalChatStore.getState(),
					sessionId,
					preparation?.runId,
				);
			const enqueue = async () => {
				await acceptReaction(sessionId);
				useGlobalChatStore.getState().enqueueMessage({
					conversationId: sessionId,
					content: trimmed,
					files: filesAttached,
					nativeRequest,
					interaction,
					reactionAccepted,
				});
			};
			if (atCapacity()) {
				if (!voice && (trimmed || (filesAttached?.length ?? 0) > 0))
					await enqueue();
				return;
			}
			const agentSelection = Object.freeze({
				provider: state.provider,
				selectedModelId: state.selectedModelId,
				reasoningEffort: state.reasoningEffort,
			});
			const launchingProfileId = settingsProfileRef.current?.hub_profile.id;
			if (nativeRequest) await authorizeNativeRequest();

			// Any file type is accepted: files become local tmp files (Tauri) or presigned tmp
			// uploads — only URLs travel through IPC and land in IndexedDB, no blobs. A signed-in
			// bot chat uploads an image that would not reach the model unchanged inline, so a
			// provider that fetches links reads the original; FlowPilot's history keeps local
			// files that do not expire. FlowPilot itself only reads images (vision); every
			// attachment is also listed in a manifest so it can hand the relevant files to apps it
			// calls (call_app_chat `forward_files`).
			const allFiles = filesAttached ?? [];
			const uploadsLinks =
				isBotConversation(sessionId) && authRef.current.isAuthenticated;
			const keepLocal = (file: File) =>
				!uploadsLinks || fitsInlineUnchanged(file, allFiles);
			let attachments: Awaited<ReturnType<typeof fileToAttachment>> = [];
			if (allFiles.length > 0) {
				try {
					attachments = await fileToAttachment(
						allFiles,
						backend,
						keepLocal,
						nativeRequest ? checkCurrent : undefined,
					);
					checkCurrent();
				} catch (error) {
					checkCurrent();
					toast.error(
						t(
							"failedToPrepareAttachmentsVal",
							"Failed to prepare attachments: {{val}}",
							{ val: error instanceof Error ? error.message : String(error) },
						),
					);
					if (nativeRequest) throw error;
				}
			}
			// Only image attachments feed the vision model; other files travel as a name/type
			// manifest the assistant reasons over when deciding which files to forward downstream.
			const imageAttachmentUrls = attachments
				.map((attachment) =>
					typeof attachment === "string"
						? { url: attachment, type: undefined as string | undefined }
						: { url: attachment.url, type: attachment.type },
				)
				.filter((attachment) => (attachment.type ?? "").startsWith("image/"))
				.map((attachment) => attachment.url);
			const attachmentManifest = attachments.map((attachment) =>
				typeof attachment === "string"
					? { url: attachment }
					: {
							name: attachment.name,
							type: attachment.type,
							size: attachment.size,
							url: attachment.url,
						},
			);
			if (!trimmed && attachments.length === 0) {
				if (nativeRequest)
					throw new Error("Enter a question or attach a file for FlowPilot.");
				return;
			}
			// Attachment preparation is asynchronous, so re-check capacity: several sends can be
			// preparing uploads at the same time.
			if (atCapacity()) {
				if (!voice) await enqueue();
				return;
			}
			// A real interaction on the full FlowPilot page starts a new visibility cycle. If the
			// user previously dismissed the dock, later agent/navigation activity may show it again.
			if (variant === "page") state.enableOverlayAutoOpen();

			// A bot's one conversation is named after the bot, so its activity reads as the bot's.
			const sessionTitle = isBotConversation(sessionId)
				? botName || trimmed
				: trimmed || "Image message";
			const priorMessages = state.messages;
			const effectiveModelId = flowPilotModelIdForProvider(
				normalizeAIProvider(agentSelection.provider),
				agentSelection.selectedModelId,
			);

			const userMessage = makeGlobalChatMessage(IRole.User, trimmed, sessionId);
			if (voice) userMessage.id = voice.messageId;
			userMessage.files = attachments;
			userMessage.interaction = interaction;
			if (interaction?.type === "reaction") await acceptReaction(sessionId);
			userMessage.thinking = turnThinking(
				isAgentBackendProvider(normalizeAIProvider(agentSelection.provider)),
				agentSelection.reasoningEffort,
			);
			if (!nativeRequest) {
				appendMessage(userMessage);
				// A bot chat lives in the inbox once its target resolves; saved before that, the
				// message would stay on this device only.
				await ensureBotTarget(sessionId);
				checkCurrent();
				void persistGlobalChatMessage(userMessage);
				void persistGlobalChatSession(sessionId, sessionTitle);
			}

			const responseMessage = makeGlobalChatMessage(
				IRole.Assistant,
				"",
				sessionId,
			);
			const turnSelection = beginGlobalChatTurnSelection(
				responseMessage.id,
				agentSelection,
			);

			const { prompt: modelPrompt, history: historyPayload } =
				globalChatModelInput(userMessage, priorMessages);

			// Snapshot the latest assistant message's embedded widgets so the
			// model can see the rendered UI state the user is reacting to.
			// Travels as base64 ChatImages on this turn only (vision-capable
			// providers; external code agents drop them).
			const widgetImages: { data: string; media_type: string }[] = [];
			try {
				const latestWidgets = [...priorMessages]
					.reverse()
					.find(
						(m) => m.inner.role === IRole.Assistant && m.widgets?.length,
					)?.widgets;
				const mainWidget = resolveMainChatWidget(
					priorMessages,
					sessionId,
				)?.widget;
				const widgetIds = [
					...new Set([
						...(latestWidgets ?? []).map((widget) => widget.instance_id),
						...(mainWidget ? [mainWidget.instance_id] : []),
					]),
				];
				if (!voice && widgetIds.length) {
					const snapshots = await captureWidgetSnapshots(widgetIds);
					checkCurrent();
					for (const dataUrl of snapshots) {
						const [header, data] = dataUrl.split(",", 2);
						if (!data) continue;
						widgetImages.push({
							data,
							media_type:
								header?.match(/^data:(.+?);base64$/)?.[1] ?? "image/png",
						});
					}
				}
			} catch (error) {
				checkCurrent();
				console.warn("[GlobalChat] widget snapshot failed:", error);
			}

			if (nativeRequest) await authorizeNativeRequest();
			checkCurrent();
			if (nativeRequest && atCapacity()) {
				await enqueue();
				return;
			}
			if (nativeRequest) {
				appendMessage(userMessage);
				void persistGlobalChatMessage(userMessage);
				void persistGlobalChatSession(sessionId, sessionTitle);
			}
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
			const evaluation = getWorkspaceEvaluation(sessionId);
			const boardContext =
				surface &&
				(!evaluation || workspaceEvaluationAllowsApp(evaluation, surface.appId))
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

			// Forward the open Data Studio page (if any) so the assistant resolves "this data" to the
			// right app/overlay instead of asking which app.
			const dataStudio = useAssistantSurface.getState().dataStudioSurface;
			const dataStudioContext =
				dataStudio &&
				(!evaluation ||
					workspaceEvaluationAllowsApp(evaluation, dataStudio.appId))
					? {
							app_id: dataStudio.appId,
							app_name: dataStudio.appName || undefined,
							overlay_id: dataStudio.overlayId || undefined,
							overlay_name: dataStudio.overlayName || undefined,
							selected_table: dataStudio.selectedTable || undefined,
							view: dataStudio.view || undefined,
							forecast_id: dataStudio.forecastId || undefined,
							user_scoped: dataStudio.userScoped || undefined,
							overlay_names:
								dataStudio.overlayNames && dataStudio.overlayNames.length > 0
									? dataStudio.overlayNames
									: undefined,
						}
					: undefined;

			const interactive = await sendInteractiveMessage(
				sessionId,
				{ id: userMessage.id, text: trimmed },
				responseMessage.id,
				voice?.transcript,
			);
			checkCurrent();
			// Without its stored transcript the host could not tell the run is a live call; the
			// call posts the utterance itself and a host answers it.
			if (voice && !interactive) return;

			// The stream is driven OUTSIDE this component (global-chat-stream.ts) so it keeps
			// rendering + finalizing even if this surface unmounts mid-response (the page↔overlay
			// morph) and survives a hard reload via the Rust run registry (global_chat_resume).
			preparation?.finish();
			setActiveRun(sessionId, responseMessage.id, turnSelection);
			if (nativeRequest)
				registerNativeActionRun(nativeRequest, responseMessage.id);
			voice?.onRun(responseMessage.id);
			await driveGlobalChatStream({
				responseMessage,
				agentSelection: turnSelection,
				label: trimmed,
				sourceAttachments: [...attachments],
				// Stamped onto the persisted message so a rating months later still knows how this
				// turn ran — none of it is recoverable once the run record is dropped.
				runContext: {
					effective_model_id: effectiveModelId,
					auto_mode: selectAutoMode(useGlobalChatStore.getState(), sessionId),
					memory_enabled: Boolean(state.embeddingModelId),
					surface: variant,
					mode: boardContext ? "board" : "global",
					board_app_id: boardContext?.app_id,
					board_id: boardContext?.board_id,
					user_message_id: userMessage.id,
					attachment_count: attachments.length,
				},
				inputPreview: {
					prompt: trimmed,
					attachments: allFiles.map((file) => ({
						name: file.name,
						type: file.type,
						size: file.size,
					})),
				},
				// Desktop drives the run over a Tauri Channel (resumable via the Rust registry); the
				// browser drives the same run over HTTP+SSE, with tool requests routed to the mounted
				// tool bridge via the registry. Both feed the shared parser identically.
				start: async (onChunk) => {
					await backend.prepareFlowpilotRun?.();
					// Lazy transport imports must not open a race after the final identity check.
					checkCurrent();
					// The turn's own conversation names its bot, whatever is on screen now, and under
					// the id it has now: a bot that moved to the account mid-upload answers as itself.
					const turnConversationId =
						useGlobalChatStore.getState().runs[responseMessage.id]
							?.conversationId ?? currentConversationId(sessionId);
					return (
						isTauri()
							? tauriStart(
									"global_chat",
									{
										scope: "Frontend",
										userPrompt: modelPrompt,
										attachmentUrls:
											imageAttachmentUrls.length > 0
												? imageAttachmentUrls
												: undefined,
										attachmentsManifest:
											attachmentManifest.length > 0
												? attachmentManifest
												: undefined,
										history: historyPayload,
										currentImages:
											widgetImages.length > 0 ? widgetImages : undefined,
										modelId: effectiveModelId,
										reasoningEffort: turnSelection.reasoningEffort || undefined,
										embeddingModelId: state.embeddingModelId || undefined,
										token: authUser?.access_token ?? undefined,
										userContext: userContext ?? undefined,
										boardContext,
										dataStudioContext,
										runId: responseMessage.id,
										autoApprove: selectAutoMode(
											useGlobalChatStore.getState(),
											turnConversationId,
										),
										conversationId: turnConversationId,
										...desktopChatBotArgs(turnConversationId),
										...desktopInteractiveArgs(interactive),
									},
									checkCurrent,
								)
							: webGlobalChatStart({
									baseUrl: getApiOrigin(),
									token: authUser?.access_token ?? undefined,
									profileId: launchingProfileId,
									// The server mints its own run id; the transport needs ours to tag tool
									// requests and to register this run's cancel/steer control.
									clientRunId: responseMessage.id,
									onToolRequest: runGlobalChatTool,
									onLifecycle: FLOWPILOT_DEBUG_ENABLED
										? (event) => {
												useGlobalChatStore
													.getState()
													.recordDebugEvent(responseMessage.id, {
														...event,
														id: `${responseMessage.id}:${event.id}`,
													});
											}
										: undefined,
									body: {
										scope: "Frontend",
										user_prompt: modelPrompt,
										history: historyPayload,
										current_images:
											widgetImages.length > 0 ? widgetImages : undefined,
										model_id: effectiveModelId,
										embedding_model_id: state.embeddingModelId || undefined,
										user_context: userContext ?? undefined,
										board_context: boardContext,
										data_studio_context: dataStudioContext,
										// Signed tmp-upload URLs (from fileToAttachment) for image vision only; the
										// server fetches them.
										attachment_urls:
											imageAttachmentUrls.length > 0
												? imageAttachmentUrls
												: undefined,
										// Every attachment (name/type/size) so the assistant knows what files it
										// can hand to apps it calls, even non-image files it cannot itself read.
										attachments_manifest:
											attachmentManifest.length > 0
												? attachmentManifest
												: undefined,
										...webChatBotBody(turnConversationId),
										...webInteractiveBody(interactive),
									},
								})
					)(onChunk);
				},
			});
			return responseMessage;
		},
		[appendMessage, backend, variant, getNativeScope, t],
	);
	useEffect(() => {
		if (!dockBotId) return;
		const canTake = () => {
			const state = useGlobalChatStore.getState();
			return (
				conversationBotId(state.activeConversationId) === dockBotId &&
				botHasInboxHistory(dockBotId) &&
				!isGlobalChatAtRunCapacity(state, state.activeConversationId)
			);
		};
		return registerVoiceTurnSender(dockBotId, (request) =>
			runVoiceTurn(request, canTake, (voice) =>
				sendPreparedMessage(
					request.text,
					undefined,
					undefined,
					undefined,
					false,
					undefined,
					voice,
				),
			),
		);
	}, [dockBotId, sendPreparedMessage]);
	const sendMessage = useCallback(
		async (
			content: string,
			files?: File[],
			nativeRequest?: NativeActionRequest,
			interaction?: IChatMessageInteraction,
			reactionAccepted = false,
		) => {
			if (!content.trim() && !files?.length) return;
			const state = useGlobalChatStore.getState();
			const preparation = isGlobalChatAtRunCapacity(state)
				? undefined
				: prepareChatReply(state.activeConversationId);
			try {
				return await sendPreparedMessage(
					content,
					files,
					nativeRequest,
					interaction,
					reactionAccepted,
					preparation,
				);
			} finally {
				preparation?.finish();
			}
		},
		[sendPreparedMessage],
	);
	const handleSendMessage: ISendMessageFunction = useCallback(
		async (content, files, _tools, _audio, interaction) => {
			await sendMessage(content, files, undefined, interaction);
		},
		[sendMessage],
	);
	const handleMessageReaction = useCallback(
		async (
			message: IMessage,
			emoji: string,
			operation: "add" | "remove",
			replacesEmoji?: string,
		) => {
			if (
				message.sessionId !== useGlobalChatStore.getState().activeConversationId
			) {
				throw new Error("The message is no longer in the open conversation.");
			}
			await sendMessage(emoji, undefined, undefined, {
				type: "reaction",
				target: createMessageReference(message),
				emoji,
				operation,
				...(operation === "add" && replacesEmoji ? { replacesEmoji } : {}),
			});
		},
		[sendMessage],
	);
	const handleNativeDraft = useCallback(
		async (draft: {
			prompt: string;
			files?: File[];
			interaction?: IChatMessageInteraction;
			reactionAccepted?: boolean;
			nativeRequest?: NativeActionRequest;
			nativeScope?: string;
		}) => {
			const request =
				draft.nativeRequest ??
				(draft.nativeScope
					? { id: createId(), scope: draft.nativeScope }
					: undefined);
			if (!request) {
				await sendMessage(
					draft.prompt,
					draft.files,
					undefined,
					draft.interaction,
					draft.reactionAccepted,
				);
				return;
			}
			await runNativeAction(request, {
				getCurrentScope: getNativeScope,
				run: async () => {
					const message = await sendMessage(
						draft.prompt,
						draft.files,
						request,
						draft.interaction,
						draft.reactionAccepted,
					);
					return message ? nativeChatOutcome(message) : undefined;
				},
			});
		},
		[sendMessage, getNativeScope],
	);

	// Rate one assistant turn. The local write is authoritative and happens first: a thumb the user
	// pressed must stay pressed whether or not the network agrees, and message.tsx toasts a failure
	// if this rejects — so a successful local save must never rethrow a failed upload.
	//
	// Stable by construction (refs, not props) because MessageComponent memo-compares this callback
	// by identity, and a new one re-renders every message in the transcript.
	const handleMessageUpdate = useCallback(
		async (messageId: string, updates: Partial<IMessage>) => {
			const store = useGlobalChatStore.getState();
			const existing =
				(await globalChatDb.messages.get(messageId).catch(() => undefined)) ??
				store.messages.find((message) => message.id === messageId);
			if (!existing) return;

			// An explicit `ratingSettings: undefined` means "clear it" — spreading alone would leave a
			// present-but-undefined key, which Dexie then persists.
			const merged: IMessage = { ...existing, ...updates };
			const { ratingSettings: _cleared, ...withoutSettings } = merged;
			const next: IMessage =
				Object.prototype.hasOwnProperty.call(updates, "ratingSettings") &&
				updates.ratingSettings === undefined
					? withoutSettings
					: merged;

			// Store first so the thumb fills immediately — this transcript renders from Zustand, not
			// from a live Dexie query, so a database write alone would not repaint anything.
			store.commitMessage(next);
			await persistGlobalChatMessage(next);

			const ratingChanged = Object.prototype.hasOwnProperty.call(
				updates,
				"rating",
			);
			const settingsChanged = Object.prototype.hasOwnProperty.call(
				updates,
				"ratingSettings",
			);
			if (!ratingChanged && !settingsChanged) return;

			// FlowPilot runs signed-out on desktop against local providers. There is no account to
			// attribute a rating to there, so the local row is the terminal state.
			const profile = settingsProfileRef.current?.hub_profile;
			if (!profile?.hub || !authRef.current?.isAuthenticated) return;

			const rating = flowPilotRatingForUi(next.rating);
			try {
				await submitFlowPilotFeedback(backend.apiState, profile, {
					feedback_id: next.id,
					rating,
					comment: next.ratingSettings?.comment?.trim() ?? "",
					context: buildFlowPilotFeedbackContext(
						next,
						useGlobalChatStore.getState().messages,
						{
							includeTranscript: Boolean(
								next.ratingSettings?.includeChatHistory,
							),
							canContact: Boolean(next.ratingSettings?.canContact),
						},
					),
				});
			} catch (error) {
				// A hub that predates this route, an offline desktop, or a dropped connection all end
				// here. The rating is already stored locally, so this is a sync gap, not a user error
				// — but say so, because message.tsx is about to toast an unqualified success.
				console.warn(
					"[FlowPilot] Failed to upload message feedback:",
					rating === FLOWPILOT_RATING_WITHDRAWN ? "withdrawal" : "rating",
					error,
				);
				if (rating !== FLOWPILOT_RATING_WITHDRAWN) {
					toast.warning(
						i18next.t(
							"ratingSavedOnThisDeviceOnly",
							"Rating saved on this device — it could not be sent to the server.",
						),
					);
				}
			}
		},
		[backend.apiState],
	);

	// Answer an app-chat dialog raised during a call_app_chat run. Replying on the interaction's
	// channel unblocks the app's workflow while the outer tool call is still awaiting.
	const handleRespondToInteraction = useCallback(
		async (interactionId: string, value: unknown) => {
			const interaction = useGlobalChatStore
				.getState()
				.activeInteractions.find((i) => i.id === interactionId);
			if (!interaction) return;
			try {
				const accepted = await submitInteractionResponse(interaction, value);
				setInteractionResponded(
					interactionId,
					accepted ? accepted.value : value,
				);
			} catch (error) {
				toast.error(
					t("failedToSubmitResponseVal", "Failed to submit response: {{val}}", {
						val: error instanceof Error ? error.message : String(error),
					}),
				);
			}
		},
		[setInteractionResponded, t],
	);

	// Queue drain. Installed as a module-level hook (not just an effect) because the run that frees
	// capacity may finish long after the surface that started it unmounted — the finishing run
	// calls this directly. The effect below covers the other direction: capacity that frees up
	// while this surface is mounted, e.g. after a cancel.
	const sendRef = useRef(sendMessage);
	const nativeSendRef = useRef(handleNativeDraft);
	nativeSendRef.current = handleNativeDraft;
	useEffect(() => {
		sendRef.current = sendMessage;
	}, [sendMessage]);
	const drainQueue = useCallback((conversationId: string) => {
		const state = useGlobalChatStore.getState();
		if (currentConversationId(conversationId) !== state.activeConversationId)
			return;
		if (isGlobalChatAtRunCapacity(state)) return;
		const next = state.takeNextQueuedMessage(conversationId);
		if (!next) return;
		if (next.nativeRequest)
			void nativeSendRef.current({
				prompt: next.content,
				files: next.files,
				nativeRequest: next.nativeRequest,
				interaction: next.interaction,
				reactionAccepted: next.reactionAccepted,
			});
		else {
			void sendRef.current(
				next.content,
				next.files,
				undefined,
				next.interaction,
				next.reactionAccepted,
			);
		}
	}, []);
	useEffect(() => {
		setGlobalChatQueueDrain(drainQueue);
		return () => clearGlobalChatQueueDrain(drainQueue);
	}, [drainQueue]);
	useEffect(() => {
		if (queuedMessages.length === 0) return;
		if (
			activeRuns.length + preparingReplies.length >=
			MAX_CONCURRENT_GLOBAL_CHAT_RUNS
		)
			return;
		drainQueue(activeConversationId);
	}, [
		queuedMessages,
		activeRuns.length,
		preparingReplies.length,
		activeConversationId,
		drainQueue,
	]);

	/** Stop one live turn. The partial reply is kept and committed. */
	const handleStopRun = useCallback((runId: string) => {
		void cancelGlobalChatRun(runId);
	}, []);

	/**
	 * Send the composer's text into a turn that is already running instead of starting a new one.
	 * Targets the most recently started run — that is the one the user is watching.
	 */
	const handleSteer = useCallback(
		async (content: string) => {
			const target = activeRuns
				.filter((run) => run.status === "streaming")
				.at(-1);
			if (!target) return false;
			const delivered = await steerGlobalChatRun(target.runId, content);
			if (!delivered) {
				toast.error(
					t(
						"flowpilotCouldNotTakeThatMidrunItWasNotSentTryAgainOrWaitForTheTurnToFinish",
						"FlowPilot could not take that mid-run. It was not sent — try again or wait for the turn to finish.",
					),
				);
			}
			return delivered;
		},
		[activeRuns, t],
	);

	// Auto-send a pending draft exactly once — handed off from the landing hero bar or attached to
	// a surface's requestOpenAssistant(prompt). Subscribing to the draft (instead of running only on
	// mount) lets it fire in BOTH variants, including when the overlay body is already mounted;
	// consumeDraft() clears the store atomically so a concurrently mounted page/overlay pair cannot
	// double-send. A live turn no longer blocks it — only the concurrency cap does, and the
	// run-count dependency re-fires the effect when capacity frees up.
	//
	// Readiness gate: right after a reload the draft would otherwise fire before auth and the model
	// list settle — modelId undefined makes the backend pick an arbitrary "best" profile model
	// (which can stall for minutes hosting a local model) and the auth token would be missing for
	// hosted ones. The deps re-fire the effect the moment a model is selected.
	const chatConcurrency = useMemo<IChatConcurrency>(
		() => ({
			runs: activeRuns,
			queued: queuedMessages,
			atCapacity:
				activeRuns.length + preparingReplies.length >=
				MAX_CONCURRENT_GLOBAL_CHAT_RUNS,
			onStop: handleStopRun,
			onSteer: handleSteer,
			onRemoveQueued: removeQueuedMessage,
		}),
		[
			activeRuns,
			preparingReplies,
			queuedMessages,
			handleStopRun,
			handleSteer,
			removeQueuedMessage,
		],
	);

	const pendingDraft = useGlobalChatStore((s) => s.draft);
	const nativeDraftScope = getNativeScope();
	const [nativeDeadlineTick, setNativeDeadlineTick] = useState(0);
	useEffect(() => {
		const deadline = Date.parse(
			pendingDraft?.nativeRequest?.responseDeadline ?? "",
		);
		if (!Number.isFinite(deadline)) return;
		const timer = setTimeout(
			() => setNativeDeadlineTick((tick) => tick + 1),
			Math.max(0, Math.min(2_147_483_647, deadline - Date.now())),
		);
		return () => clearTimeout(timer);
	}, [pendingDraft]);
	const draftReady =
		!auth.isLoading &&
		(isAgent
			? copilotSDK.models.length > 0 && Boolean(selectedModelId)
			: Boolean(selectedModelId) ||
				(llmBits.data !== undefined && bitsModels.length === 0));
	// biome-ignore lint/correctness/useExhaustiveDependencies: send on new drafts / readiness / capacity only, not on every handleSendMessage identity change.
	useEffect(() => {
		if (!pendingDraft) return;
		if (
			pendingDraft.nativeRequest?.responseDeadline &&
			!(Date.parse(pendingDraft.nativeRequest.responseDeadline) > Date.now())
		) {
			consumeDraft(pendingDraft);
			return;
		}
		// A draft goes to the bot it names; FlowPilot entry points (hero bar, board surfaces, native
		// requests) name none, so Flux takes them. The overlay switches to that bot itself, a dock
		// waits until the Bots tab shows it.
		const draftBotId = pendingDraft.botId ?? null;
		if (
			chatBot.botId !== draftBotId ||
			(pinnedToDock && dockBotId !== draftBotId)
		) {
			if (!pinnedToDock) void switchGlobalChatBot(draftBotId);
			return;
		}
		if (
			!draftReady ||
			((pendingDraft.nativeRequest || pendingDraft.nativeScope) &&
				!nativeDraftScope)
		)
			return;
		// A draft only waits for actual capacity now — a live turn no longer blocks it.
		if (isGlobalChatAtRunCapacity(useGlobalChatStore.getState())) return;
		const draft = consumeDraft(pendingDraft);
		if (!draft) return;
		if (draft.modelId) setSelectedModelId(draft.modelId);
		void handleNativeDraft(draft);
	}, [
		pendingDraft,
		nativeDeadlineTick,
		nativeDraftScope,
		draftReady,
		activeRuns.length,
		chatBot.botId,
		pinnedToDock,
		dockBotId,
		consumeDraft,
		setSelectedModelId,
	]);

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
	useChatActivityVisible(
		showWorkspace && !canSideBySide
			? null
			: { kind: "flowpilot", sessionId: activeConversationId },
	);
	// Keyed by the live reply ids so streamed tokens leave the card placement — and every message
	// memo comparing it — untouched until a reply starts or settles.
	const liveReplyIds = liveReplyIdsKey(activeRuns);
	const transcriptMessageIds = useMemo(
		() => transcriptIds(messages, liveReplyIds),
		[messages, liveReplyIds],
	);
	const inlineCards = useMemo(
		() =>
			layoutInlineCards(
				{
					pages: inlineAppPages,
					surfaces: inlineAppSurfaces,
					chats: inlineAppChats,
					onClosePage: removeInlineAppPage,
					onCloseSurface: removeInlineAppSurface,
					onCloseChat: removeInlineAppChat,
					compact,
				},
				transcriptMessageIds,
			),
		[
			compact,
			inlineAppChats,
			inlineAppPages,
			inlineAppSurfaces,
			removeInlineAppChat,
			removeInlineAppPage,
			removeInlineAppSurface,
			transcriptMessageIds,
		],
	);
	const hasInlineArtifacts =
		inlineCards.trailing.length > 0 || pendingComponents !== null;
	const inlineArtifacts = useMemo(
		() =>
			hasInlineArtifacts ? (
				<div className="flex w-full flex-col gap-3 py-1">
					<PendingComponentsCard />
					{inlineCards.trailing}
				</div>
			) : undefined,
		[hasInlineArtifacts, inlineCards],
	);

	const [settingsOpen, setSettingsOpen] = useState(false);
	const openSettings = useCallback(() => setSettingsOpen(true), []);
	const language = i18n.language;
	const defaultModelLabel = useMemo(
		() =>
			selectedModelLabel(
				isAgent
					? copilotSDK.models
					: bitsModels.map((bit) => ({
							id: bit.id,
							name: bit.meta?.[language]?.name ?? bit.meta?.en?.name,
						})),
				selectedModelId,
			),
		[isAgent, copilotSDK.models, bitsModels, language, selectedModelId],
	);
	const composerActions = useMemo(
		() => (
			<BotChatActions
				chatBot={chatBot}
				defaultModelLabel={defaultModelLabel}
				onOpenSettings={openSettings}
			/>
		),
		[chatBot, defaultModelLabel, openSettings],
	);

	const openAppId = useOpenAppId();
	const storageAttach = useStorageAttachSource(openAppId);
	const pageSnapshotSource = usePageSnapshotSource();
	const attachSources = useMemo(
		() =>
			[storageAttach.source, pageSnapshotSource].filter(
				(source): source is ComposerAttachSource => source !== undefined,
			),
		[storageAttach.source, pageSnapshotSource],
	);

	// A queued draft is about to send, so the welcome must not flash up under it.
	const showWelcome =
		messages.length === 0 &&
		activeRuns.length === 0 &&
		preparingReplies.length === 0 &&
		queuedMessages.length === 0 &&
		!hasInlineArtifacts &&
		!promptSlot &&
		activeInteractions.length === 0 &&
		!pendingDraft;
	const welcome = useMemo(
		() =>
			showWelcome
				? {
						title: chatBot.name,
						description: chatBot.description,
						mark: <BotWelcomeMark view={chatBot} />,
					}
				: undefined,
		[showWelcome, chatBot],
	);

	// If a stale desktop selection (an agent backend) carried into the web app, fall back to Bits so
	// the picker and the send path stay on a backend that can actually run here.
	useEffect(() => {
		if (!isTauri() && isAgentBackendProvider(provider)) {
			setProvider("bits");
		}
	}, [provider, setProvider]);

	const chipsShown = Boolean(boardSurface || flowscriptWorkspace);
	const pillOverTranscript = pillInset && !chipsShown;

	return (
		<div
			onPointerDownCapture={handlePageInteraction}
			onKeyDownCapture={handlePageInteraction}
			className="flex flex-col flex-1 min-h-0 w-full h-full"
		>
			{chipsShown && (
				<header
					className={cn(
						"fl-chat-chrome flex shrink-0 items-center gap-1.5 px-3 py-2",
						pillInset && "pt-14",
					)}
					data-chrome-pinned={isStreaming ? "true" : "false"}
				>
					<div className="flex flex-1 min-w-0 items-center gap-1.5 overflow-x-auto no-scrollbar">
						{boardSurface && (
							<div
								className="flex h-9 md:h-7 shrink-0 items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/5 px-2.5 text-xs text-foreground/80"
								title={t(
									"theAssistantCanSeeAndEditThisBoard",
									"The assistant can see and edit this board",
								)}
							>
								<WorkflowIcon className="size-3.5 shrink-0 text-primary" />
								<span className="truncate max-w-32">
									{boardSurface.board?.name || "Board"}
								</span>
								{boardSurface.selectedNodeIds.length > 0 && (
									<span className="shrink-0 text-muted-foreground">
										{t("lengthSelected", "· {{length}} selected", {
											length: boardSurface.selectedNodeIds.length,
										})}
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
										? t(
												"hideTheFlowscriptWorkspace",
												"Hide the FlowScript workspace",
											)
										: t(
												"showTheFlowscriptWorkspace",
												"Show the FlowScript workspace",
											)
								}
								className="h-9 md:h-7 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
							>
								<FileCode2Icon className="size-3.5 shrink-0" />
								{t("flowscript", "FlowScript")}
								{flowscriptWorkspace.status === "validation_errors" && (
									<span
										className="size-1.5 shrink-0 rounded-full bg-red-500"
										aria-hidden
									/>
								)}
							</Button>
						)}
					</div>
				</header>
			)}

			<div
				ref={layoutRef}
				className={`min-h-0 min-w-0 flex-1 overflow-hidden ${
					sideBySideWorkspace ? "flex flex-row" : "flex flex-col"
				}`}
			>
				<div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
					{/* Must be a flex column: the surface sizes itself with flex-1/min-h-0, and
					    without a flex parent its height collapses to content size, breaking the
					    internal scroll area. In a narrow dock the workspace replaces the chat rather
					    than squeezing beside it, so hide (don't unmount) the chat to keep its
					    scroll/stream state alive underneath. */}
					<div
						className={cn(
							"relative flex min-h-0 min-w-0 flex-1 flex-col",
							showWorkspace && !canSideBySide && "hidden",
							pillOverTranscript &&
								"**:data-fl-chat-messages:pt-14 **:data-fl-chat-welcome:pt-14",
						)}
					>
						{/* Embedded-widget actions (ActionHandler's widget_event) route through
						    runWidgetAction to the widget's originating use-case board. */}
						<BotFileAssets botId={chatBot.bot?.id ?? chatBot.botId}>
							<ChatSurface
								chatRef={attachChat}
								loading={conversationLoading}
								welcome={welcome}
								runWidgetAction={runWidgetAction}
								sessionId={activeConversationId}
								messages={messages}
								onSendMessage={handleSendMessage}
								onDraftChange={onDraftChange}
								isStreamActive={isStreaming || preparingReplies.length > 0}
								loadingText={preparingReplyText}
								// Supplying this is what unlocks the composer: sends are never
								// blocked; they start another turn, queue, or steer the running one.
								concurrency={chatConcurrency}
								config={chatConfig}
								thinking={thinkingPicker}
								composerActions={composerActions}
								attachSources={attachSources}
								activeInteractions={activeInteractions}
								onRespondToInteraction={handleRespondToInteraction}
								onMessageUpdate={handleMessageUpdate}
								onMessageReaction={handleMessageReaction}
								showAiDisclosure
								inlineContent={inlineArtifacts}
								messageInlineContent={inlineCards.byMessage}
								inlinePrompt={promptSlot}
								group={botNotes}
							/>
						</BotFileAssets>
					</div>
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

			<BotSettingsDialog
				chatBot={chatBot}
				open={settingsOpen}
				onOpenChange={setSettingsOpen}
			/>
			{storageAttach.dialog}
		</div>
	);
}
