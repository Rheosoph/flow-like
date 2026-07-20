"use client";

import {
	BotIcon,
	BrainIcon,
	CheckIcon,
	ChevronDownIcon,
	Code2Icon,
	FileCode2Icon,
	GithubIcon,
	LayersIcon,
	Loader2Icon,
	PackageIcon,
	PlusIcon,
	SettingsIcon,
	SparklesIcon,
	Trash2Icon,
	WorkflowIcon,
	ZapIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	IBitTypes,
	IRole,
	useAssistantSurface,
	useBackend,
	useCopilotSDK,
	useInvoke,
} from "../../index";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Popover,
	PopoverContent,
	PopoverTrigger,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../index";
import { getApiOrigin } from "../../lib/api-url";
import { FLOWPILOT_DEBUG_ENABLED } from "../../lib/flowpilot-debug";
import { isTauri } from "../../lib/platform";
import { captureWidgetSnapshots } from "../../lib/widget-snapshot";
import {
	type IMessage,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";
import {
	type MemoryEntry,
	clearGlobalChatMemory,
	deleteGlobalChatMemory,
	globalChatMemoryStatus,
	listGlobalChatMemories,
} from "../../state/global-chat/global-chat-memory";
import {
	AGENT_MODEL_KEY,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import {
	LAST_CONVERSATION_KEY,
	driveGlobalChatStream,
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	persistGlobalChatSession,
	resumeGlobalChatStream,
	setActiveRun,
	tauriStart,
} from "../../state/global-chat/global-chat-stream";
import { runGlobalChatTool } from "../../state/global-chat/global-chat-tool-registry";
import { webGlobalChatStart } from "../../state/global-chat/global-chat-web-transport";
import { FlowScriptWorkspacePanel } from "../flowpilot/flowscript-workspace-panel";
import {
	type AIProvider,
	flowPilotModelIdForProvider,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../flowpilot/types";
import { fileToAttachment } from "../interfaces/chat-default/attachment";
import { Chat, type IChatRef } from "../interfaces/chat-default/chat";
import { ChatWidgetExecutionProvider } from "../interfaces/chat-default/chat-widget-execution";
import type { ISendMessageFunction } from "../interfaces/chat-default/chatbox";
import { submitInteractionResponse } from "../interfaces/chat-default/respond-interaction";
import { GlobalChatHistory } from "./global-chat-history";
import { InlineAppChatCard } from "./inline-app-chat-card";
import { InlineAppPageCard } from "./inline-app-page-card";
import { InlineAppSurfaceCard } from "./inline-app-surface-card";
import { InlineToolPrompt } from "./inline-tool-prompt";
import { PendingComponentsCard } from "./pending-components-card";
import { useHydrateAgentSelection } from "./use-agent-persistence";
import { useGlobalChatRunWidgetAction } from "./use-global-widget-action";

// The streaming engine (parse the FlowPilot protocol → message content + plan_steps → store) lives
// in lib/global-chat-stream.ts, OUTSIDE this component, so a turn survives the page↔overlay morph
// and a hard reload (re-attaching to the live Rust run via global_chat_resume). This surface only
// builds the send payload, then renders the store's shared transcript + streaming bubble.

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
const GLOBAL_CHAT_CONFIG = {
	allow_file_upload: true,
	tools: [] as string[],
};

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
	const reasoningEffort = useGlobalChatStore((s) => s.reasoningEffort);
	const setProvider = useGlobalChatStore((s) => s.setProvider);
	const setSelectedModelId = useGlobalChatStore((s) => s.setSelectedModelId);
	const setReasoningEffort = useGlobalChatStore((s) => s.setReasoningEffort);
	// Explicit picks persist across sessions; the "keep a valid model" fallbacks
	// below use the plain setters so a still-loading catalog can never clobber them.
	const selectProvider = useGlobalChatStore((s) => s.selectProvider);
	const selectModel = useGlobalChatStore((s) => s.selectModel);
	const selectReasoningEffort = useGlobalChatStore(
		(s) => s.selectReasoningEffort,
	);
	const embeddingModelId = useGlobalChatStore((s) => s.embeddingModelId);
	const setEmbeddingModelId = useGlobalChatStore((s) => s.setEmbeddingModelId);
	const autoMode = useGlobalChatStore((s) => s.autoMode);
	const setAutoMode = useGlobalChatStore((s) => s.setAutoMode);
	const appendMessage = useGlobalChatStore((s) => s.appendMessage);
	const setStreaming = useGlobalChatStore((s) => s.setStreaming);
	const consumeDraft = useGlobalChatStore((s) => s.consumeDraft);
	const inlineAppChats = useGlobalChatStore((s) => s.inlineAppChats);
	const removeInlineAppChat = useGlobalChatStore((s) => s.removeInlineAppChat);
	const inlineAppPages = useGlobalChatStore((s) => s.inlineAppPages);
	const removeInlineAppPage = useGlobalChatStore((s) => s.removeInlineAppPage);
	const inlineAppSurfaces = useGlobalChatStore((s) => s.inlineAppSurfaces);
	const removeInlineAppSurface = useGlobalChatStore(
		(s) => s.removeInlineAppSurface,
	);
	const toolPrompt = useGlobalChatStore((s) => s.toolPrompt);
	const activeInteractions = useGlobalChatStore((s) => s.activeInteractions);
	const setInteractionResponded = useGlobalChatStore(
		(s) => s.setInteractionResponded,
	);
	const flowscriptWorkspace = useGlobalChatStore((s) => s.flowscriptWorkspace);
	const pendingComponents = useGlobalChatStore((s) => s.pendingComponents);
	// Turning auto mode on mid-run settles approval cards whose promises the bridge captured
	// before the flip; queued ones drain as each is answered. `ask` prompts are never
	// auto-answered — auto mode waives permission, not questions — and neither are prompts
	// flagged `destructive` (the deletion gate), which always need a real user decision.
	useEffect(() => {
		if (!autoMode || toolPrompt?.kind !== "approval" || toolPrompt.destructive)
			return;
		toolPrompt.respond({ approved: true, remember: false });
	}, [autoMode, toolPrompt]);
	// Live board surface (open canvas) the assistant can see and edit — shown as a context chip.
	const boardSurface = useAssistantSurface((s) => s.boardSurface);
	const runWidgetAction = useGlobalChatRunWidgetAction();

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
					// If a response was still streaming when the webview reloaded, the Rust run kept
					// going into a dead channel — re-attach and continue rendering it live. No-op when
					// nothing is in flight (the run already finished/GC'd → checkpoint stays as-is).
					resumeGlobalChatStream();
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

	// Restore the last explicit provider/model/effort. /chat is often the first
	// FlowPilot surface mounted (deep link, mobile bottom nav), so it has to
	// hydrate the shared store itself rather than relying on the hero.
	useHydrateAgentSelection();

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
	// Read the profile's INSTALLED bits directly rather than intersecting a remote catalog search
	// with profile.bits strings — the latter silently misses embeddings whose stored hub prefix
	// differs from the search hub, which is exactly why the memory picker never appeared.
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const memoryModels = useMemo(
		() =>
			(profileBits.data ?? []).filter(
				(bit) => bit.type === IBitTypes.Embedding,
			),
		[profileBits.data],
	);

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
			if (models.length === 0) return;
			if (
				remembered &&
				remembered !== selectedModelId &&
				models.some((m) => m.id === remembered)
			) {
				setSelectedModelId(remembered);
				return;
			}
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
		if (
			remembered &&
			remembered !== selectedModelId &&
			bitsModels.some((bit) => bit.id === remembered)
		) {
			setSelectedModelId(remembered);
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

	const selectedAgentModel = isAgent
		? copilotSDK.models.find((model) => model.id === selectedModelId)
		: undefined;
	const reasoningEffortOptions =
		selectedAgentModel?.supportedReasoningEfforts ?? [];

	// Do not validate against the hook's metadata-free static fallback. Once the
	// backend has returned a real catalog (`[]` or populated), stale persisted
	// values are reset to Auto instead of being sent to an incompatible model.
	useEffect(() => {
		if (!reasoningEffort) return;
		if (!isAgent) {
			setReasoningEffort("");
			return;
		}
		if (
			!selectedAgentModel ||
			selectedAgentModel.supportedReasoningEfforts === undefined
		) {
			return;
		}
		if (
			!selectedAgentModel.supportedReasoningEfforts.some(
				(option) => option.id === reasoningEffort,
			)
		) {
			setReasoningEffort("");
		}
	}, [isAgent, reasoningEffort, selectedAgentModel, setReasoningEffort]);

	const handleSendMessage: ISendMessageFunction = useCallback(
		async (content, filesAttached) => {
			const trimmed = content.trim();
			const state = useGlobalChatStore.getState();
			if (state.isStreaming) return;

			// Any file type is accepted: files become local tmp files (Tauri) or presigned tmp
			// uploads — only URLs travel through IPC and land in IndexedDB, no blobs. FlowPilot
			// itself only reads images (vision); every attachment is also listed in a manifest so
			// it can hand the relevant files to apps it calls (call_app_chat `forward_files`).
			const allFiles = filesAttached ?? [];
			let attachments: Awaited<ReturnType<typeof fileToAttachment>> = [];
			if (allFiles.length > 0) {
				try {
					attachments = await fileToAttachment(allFiles, backend, true);
				} catch (error) {
					toast.error(
						`Failed to prepare attachments: ${error instanceof Error ? error.message : String(error)}`,
					);
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

			const userMessage = makeGlobalChatMessage(IRole.User, trimmed, sessionId);
			userMessage.files = attachments;
			appendMessage(userMessage);
			void persistGlobalChatMessage(userMessage);
			void persistGlobalChatSession(sessionId, trimmed || "Image message");

			const responseMessage = makeGlobalChatMessage(
				IRole.Assistant,
				"",
				sessionId,
			);
			useGlobalChatStore.getState().setStreamingMessage({ ...responseMessage });
			// Register the run so a reload mid-response can re-attach to the live Rust stream.
			setActiveRun(sessionId, responseMessage.id);

			const historyPayload = priorMessages.map((m) => ({
				role: m.inner.role === IRole.Assistant ? "Assistant" : "User",
				content: typeof m.inner.content === "string" ? m.inner.content : "",
			}));

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
				if (latestWidgets?.length) {
					const snapshots = await captureWidgetSnapshots(
						latestWidgets.map((widget) => widget.instance_id),
					);
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
				console.warn("[GlobalChat] widget snapshot failed:", error);
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
									(sum, layer) => sum + Object.keys(layer?.nodes ?? {}).length,
									0,
								)
							: undefined,
					}
				: undefined;

			// Forward the open Data Studio page (if any) so the assistant defaults data questions to
			// its app/overlay via data_studio_agent without asking which project.
			const dataStudio = useAssistantSurface.getState().dataStudioSurface;
			const dataStudioContext = dataStudio
				? {
						app_id: dataStudio.appId,
						app_name: dataStudio.appName || undefined,
						overlay_id: dataStudio.overlayId || undefined,
						overlay_name: dataStudio.overlayName || undefined,
						selected_table: dataStudio.selectedTable || undefined,
						overlay_names:
							dataStudio.overlayNames && dataStudio.overlayNames.length > 0
								? dataStudio.overlayNames
								: undefined,
					}
				: undefined;

			// The stream is driven OUTSIDE this component (global-chat-stream.ts) so it keeps
			// rendering + finalizing even if this surface unmounts mid-response (the page↔overlay
			// morph) and survives a hard reload via the Rust run registry (global_chat_resume).
			await driveGlobalChatStream({
				responseMessage,
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
				start: isTauri()
					? tauriStart("global_chat", {
							scope: "Frontend",
							userPrompt: trimmed,
							attachmentUrls:
								imageAttachmentUrls.length > 0
									? imageAttachmentUrls
									: undefined,
							attachmentsManifest:
								attachmentManifest.length > 0 ? attachmentManifest : undefined,
							history: historyPayload,
							currentImages: widgetImages.length > 0 ? widgetImages : undefined,
							modelId: effectiveModelId,
							reasoningEffort: state.reasoningEffort || undefined,
							embeddingModelId: state.embeddingModelId || undefined,
							token: authUser?.access_token ?? undefined,
							userContext: userContext ?? undefined,
							boardContext,
							dataStudioContext,
							runId: responseMessage.id,
						})
					: webGlobalChatStart({
							baseUrl: getApiOrigin(),
							token: authUser?.access_token ?? undefined,
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
								user_prompt: trimmed,
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
							},
						}),
			});
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

	// Provider, model, and dynamic reasoning effort share one popover so the toolbar stays compact.
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
	const [memoryManagerOpen, setMemoryManagerOpen] = useState(false);
	const profileId = settingsProfile.data?.hub_profile.id;

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
				const status = await globalChatMemoryStatus(
					profileId,
					auth.user?.access_token,
				);
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
			auth.user?.access_token,
		],
	);

	const confirmEmbeddingChange = useCallback(async () => {
		if (!pendingEmbedding) return;
		try {
			await clearGlobalChatMemory(
				pendingEmbedding.profileId,
				auth.user?.access_token,
			);
		} catch {
			// best-effort delete
		}
		setEmbeddingModelId(pendingEmbedding.modelId);
		setPendingEmbedding(null);
	}, [pendingEmbedding, setEmbeddingModelId, auth.user?.access_token]);

	// Memory recall + the memory tools run on EVERY backend (the Rust loop wires `memory` into the
	// Copilot/Codex/Claude-Code agents and the Bits loop alike), so the picker is shown wherever the
	// profile has an embedding model — not gated to the Bits backend.
	const memoryPicker = useMemo(
		() =>
			memoryModels.length > 0 ? (
				<Select
					value={embeddingModelId || MEMORY_OFF}
					onValueChange={handleEmbeddingChange}
				>
					<SelectTrigger
						className="h-9 md:h-7 data-[size=default]:h-9 md:data-[size=default]:h-7 min-w-0 max-w-36 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
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
		[memoryModels, embeddingModelId, handleEmbeddingChange],
	);
	const defaultReasoningEffortName = reasoningEffortOptions.find(
		(option) => option.id === selectedAgentModel?.defaultReasoningEffort,
	)?.name;
	const autoReasoningEffortName = defaultReasoningEffortName
		? `Auto (${defaultReasoningEffortName} default)`
		: "Auto (provider default)";
	const currentReasoningEffortName = reasoningEffort
		? (reasoningEffortOptions.find((option) => option.id === reasoningEffort)
				?.name ?? reasoningEffort)
		: autoReasoningEffortName;

	const showEmptyState =
		messages.length === 0 &&
		inlineAppChats.length === 0 &&
		inlineAppPages.length === 0 &&
		inlineAppSurfaces.length === 0 &&
		!isStreaming &&
		// A queued draft is about to send — don't flash the empty state under it.
		!pendingDraft;

	// Agent-SDK backends (Copilot / Codex / Claude Code) are local CLIs — desktop only. On web only
	// profile Bits are offered, matching the `/ai/global-chat/backends` capability.
	const availableProviders = useMemo(
		() => PROVIDERS.filter((p) => isTauri() || !isAgentBackendProvider(p.id)),
		[],
	);

	// If a stale desktop selection (an agent backend) carried into the web app, fall back to Bits so
	// the picker and the send path stay on a backend that can actually run here.
	useEffect(() => {
		if (!isTauri() && isAgentBackendProvider(provider)) {
			setProvider("bits");
		}
	}, [provider, setProvider]);

	const currentProvider =
		availableProviders.find(
			(p) => normalizeAIProvider(p.id) === normalizedProvider,
		) ?? availableProviders[0];
	const CurrentProviderIcon = currentProvider.icon;
	const currentModelLabel = modelOptions.find(
		(option) => option.id === selectedModelId,
	)?.label;

	// Provider, model, and model-specific reasoning effort live in one compact picker. A model with
	// configurable reasoning keeps the popover open so the next section can be selected immediately;
	// models without that capability close it as before.
	const providerModelPicker = (
		<Popover open={pickerOpen} onOpenChange={setPickerOpen}>
			<PopoverTrigger asChild>
				<Button
					variant="outline"
					size="sm"
					title={`${currentProvider.label} · ${currentModelLabel ?? "Select a model"}${reasoningEffortOptions.length > 0 ? ` · ${currentReasoningEffortName}` : ""}`}
					className="h-9 md:h-7 shrink-0 gap-1.5 px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
				>
					<CurrentProviderIcon className="size-3.5 shrink-0 text-primary" />
					<span className="max-w-28 truncate">
						{currentModelLabel ?? "Model"}
					</span>
					{reasoningEffortOptions.length > 0 && (
						<>
							<span className="text-border" aria-hidden="true">
								·
							</span>
							<BrainIcon className="size-3.5 shrink-0 text-muted-foreground" />
							<span className="max-w-32 truncate text-muted-foreground">
								{currentReasoningEffortName}
							</span>
						</>
					)}
					<ChevronDownIcon className="size-3 shrink-0 opacity-50" />
				</Button>
			</PopoverTrigger>
			<PopoverContent align="start" className="z-10000 w-72 p-2">
				<p className="px-1 pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
					Provider
				</p>
				<div className="flex gap-0.5 rounded-lg border border-border/40 bg-muted/30 p-0.5">
					{availableProviders.map(({ id, label, icon: Icon }) => {
						const active = normalizeAIProvider(id) === normalizedProvider;
						return (
							<button
								key={id}
								type="button"
								title={label}
								onClick={() => selectProvider(id)}
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
				<div className="max-h-48 space-y-0.5 overflow-y-auto">
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
										selectModel(option.id);
										const nextModel = copilotSDK.models.find(
											(model) => model.id === option.id,
										);
										if (
											!isAgent ||
											(nextModel?.supportedReasoningEfforts?.length ?? 0) === 0
										) {
											setPickerOpen(false);
										}
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
				{reasoningEffortOptions.length > 0 && (
					<>
						<p className="px-1 pb-1.5 pt-2.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
							Reasoning
						</p>
						<div className="grid grid-cols-2 gap-1">
							<button
								type="button"
								onClick={() => {
									selectReasoningEffort("");
									setPickerOpen(false);
								}}
								className={`col-span-2 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40 ${!reasoningEffort ? "bg-primary/10 text-primary" : "hover:bg-muted"}`}
							>
								<BrainIcon className="size-3.5 shrink-0" />
								<span className="flex-1 truncate">
									{autoReasoningEffortName}
								</span>
								{!reasoningEffort && (
									<CheckIcon className="size-3.5 shrink-0" />
								)}
							</button>
							{reasoningEffortOptions.map((option) => {
								const active = option.id === reasoningEffort;
								return (
									<button
										key={option.id}
										type="button"
										title={option.description}
										onClick={() => {
											selectReasoningEffort(option.id);
											setPickerOpen(false);
										}}
										className={`flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40 ${active ? "bg-primary/10 text-primary" : "hover:bg-muted"}`}
									>
										<span className="flex-1 truncate">{option.name}</span>
										{active && <CheckIcon className="size-3.5 shrink-0" />}
									</button>
								);
							})}
						</div>
					</>
				)}
			</PopoverContent>
		</Popover>
	);

	return (
		<div className="flex flex-col flex-1 min-h-0 w-full h-full">
			<header className="flex items-center gap-1.5 px-3 py-2 border-b border-border/50 shrink-0">
				<div className="flex flex-1 min-w-0 items-center gap-1.5 overflow-x-auto no-scrollbar">
					{providerModelPicker}
					{memoryPicker}
					{memoryModels.length > 0 && profileId && (
						<Button
							type="button"
							variant="outline"
							size="icon"
							onClick={() => setMemoryManagerOpen(true)}
							title="Review & manage saved memories"
							className="size-9 md:size-7 shrink-0 outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						>
							<SettingsIcon className="size-3.5 shrink-0 text-muted-foreground" />
						</Button>
					)}
					<Button
						type="button"
						variant={autoMode ? "default" : "outline"}
						size="sm"
						aria-pressed={autoMode}
						onClick={() => setAutoMode(!autoMode)}
						title={
							autoMode
								? "Auto mode on — tools run and changes apply without asking, including destructive ones. Only board-item deletion still asks."
								: "Auto mode off — the assistant asks before acting"
						}
						className="h-9 md:h-7 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
					>
						<ZapIcon className="size-3.5 shrink-0" />
						Auto
					</Button>
					{boardSurface && (
						<div
							className="flex h-9 md:h-7 shrink-0 items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/5 px-2.5 text-xs text-foreground/80"
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
							className="h-9 md:h-7 shrink-0 gap-1.5 px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
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
				</div>
				<GlobalChatHistory />
			</header>

			{(inlineAppChats.length > 0 ||
				inlineAppPages.length > 0 ||
				inlineAppSurfaces.length > 0 ||
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
					{inlineAppSurfaces.map((surface) => (
						<InlineAppSurfaceCard
							key={surface.id}
							surface={surface}
							onClose={removeInlineAppSurface}
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
										className="h-10 md:h-8 gap-1.5 rounded-full border-border/60 bg-background/80 text-xs text-foreground/80 outline-none transition-all hover:border-primary/40 hover:bg-primary/10 hover:text-primary hover:shadow-sm focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0 motion-safe:hover:-translate-y-px"
										onClick={() => void handleSendMessage(prompt)}
									>
										<Icon className="size-3.5" />
										{label}
									</Button>
								))}
							</div>
						</div>
					)}
					{/* Embedded-widget actions (ActionHandler's widget_event) route through
					    runWidgetAction to the widget's originating use-case board. */}
					<ChatWidgetExecutionProvider runWidgetAction={runWidgetAction}>
						<Chat
							ref={chatRef}
							sessionId={activeConversationId}
							messages={messages}
							onSendMessage={handleSendMessage}
							isStreamActive={isStreaming}
							config={GLOBAL_CHAT_CONFIG}
							activeInteractions={activeInteractions}
							onRespondToInteraction={handleRespondToInteraction}
							inlinePrompt={
								toolPrompt ? (
									<InlineToolPrompt key={toolPrompt.id} prompt={toolPrompt} />
								) : undefined
							}
						/>
					</ChatWidgetExecutionProvider>
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

			{profileId && (
				<MemoryManagerDialog
					open={memoryManagerOpen}
					onOpenChange={setMemoryManagerOpen}
					profileId={profileId}
					active={!!embeddingModelId}
				/>
			)}

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

// Human-friendly "how long ago" for a stored memory's epoch-millis timestamp.
function formatMemoryAge(timestamp: number): string {
	if (!timestamp) return "";
	const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
	if (seconds < 60) return "just now";
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60) return `${minutes}m ago`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return `${hours}h ago`;
	const days = Math.floor(hours / 24);
	if (days < 30) return `${days}d ago`;
	return new Date(timestamp).toLocaleDateString();
}

/**
 * Lists the assistant's profile-scoped memories and lets the user delete individual entries or clear
 * them all. Reads on open via `global_chat_list_memories`; deletions hit the backend then update the
 * in-view list optimistically.
 */
function MemoryManagerDialog({
	open,
	onOpenChange,
	profileId,
	active,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	profileId: string;
	active: boolean;
}) {
	const [entries, setEntries] = useState<MemoryEntry[] | null>(null);
	const [loading, setLoading] = useState(false);
	const [busyId, setBusyId] = useState<string | null>(null);
	const [clearing, setClearing] = useState(false);
	const auth = useAuth();

	const load = useCallback(async () => {
		setLoading(true);
		try {
			const rows = await listGlobalChatMemories(
				profileId,
				auth.user?.access_token,
			);
			setEntries(rows);
		} catch {
			toast.error("Couldn't load memories");
			setEntries([]);
		} finally {
			setLoading(false);
		}
	}, [profileId, auth.user?.access_token]);

	useEffect(() => {
		if (open) void load();
		else setEntries(null);
	}, [open, load]);

	const handleDelete = useCallback(
		async (id: string) => {
			setBusyId(id);
			try {
				await deleteGlobalChatMemory(profileId, id, auth.user?.access_token);
				setEntries((prev) => prev?.filter((entry) => entry.id !== id) ?? null);
			} catch {
				toast.error("Couldn't delete memory");
			} finally {
				setBusyId(null);
			}
		},
		[profileId, auth.user?.access_token],
	);

	const handleClearAll = useCallback(async () => {
		setClearing(true);
		try {
			await clearGlobalChatMemory(profileId, auth.user?.access_token);
			setEntries([]);
			toast.success("Cleared all memories");
		} catch {
			toast.error("Couldn't clear memories");
		} finally {
			setClearing(false);
		}
	}, [profileId, auth.user?.access_token]);

	const hasEntries = (entries?.length ?? 0) > 0;

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<BrainIcon className="size-4 text-primary" />
						Saved memories
					</DialogTitle>
					<DialogDescription>
						Facts, preferences, and decisions the assistant remembered for this
						profile. Delete anything it should forget.
					</DialogDescription>
				</DialogHeader>

				{!active && (
					<p className="rounded-md border border-border/50 bg-muted/40 px-2.5 py-2 text-xs text-muted-foreground">
						Memory is off — pick an embedding model in the header to let the
						assistant recall and save memories. You can still review and delete
						saved memories here.
					</p>
				)}

				<ScrollArea className="max-h-[50vh] pr-3">
					{loading ? (
						<div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
							<Loader2Icon className="size-4 animate-spin" />
							Loading…
						</div>
					) : !hasEntries ? (
						<div className="flex flex-col items-center gap-1 py-10 text-center text-sm text-muted-foreground">
							<BrainIcon className="size-6 opacity-40" />
							<p>No memories saved yet.</p>
							<p className="text-xs">
								The assistant stores salient facts as you chat.
							</p>
						</div>
					) : (
						<ul className="flex flex-col gap-2 py-1">
							{entries?.map((entry) => (
								<li
									key={entry.id}
									className="group flex items-start gap-2 rounded-lg border border-border/50 bg-muted/30 p-2.5"
								>
									<div className="min-w-0 flex-1">
										<p className="whitespace-pre-wrap wrap-break-word text-sm text-foreground">
											{entry.content}
										</p>
										<div className="mt-1.5 flex items-center gap-1.5">
											{entry.role && entry.role !== "observation" && (
												<Badge
													variant="secondary"
													className="h-4 px-1.5 text-[10px] font-normal"
												>
													{entry.role}
												</Badge>
											)}
											<span className="text-[11px] text-muted-foreground">
												{formatMemoryAge(entry.timestamp)}
											</span>
										</div>
									</div>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										disabled={busyId === entry.id}
										onClick={() => handleDelete(entry.id)}
										title="Forget this memory"
										className="size-7 shrink-0 text-muted-foreground hover:text-destructive"
									>
										{busyId === entry.id ? (
											<Loader2Icon className="size-3.5 animate-spin" />
										) : (
											<Trash2Icon className="size-3.5" />
										)}
									</Button>
								</li>
							))}
						</ul>
					)}
				</ScrollArea>

				{hasEntries && (
					<DialogFooter className="sm:justify-between">
						<span className="text-xs text-muted-foreground">
							{entries?.length} {entries?.length === 1 ? "memory" : "memories"}
						</span>
						<Button
							type="button"
							variant="outline"
							size="sm"
							disabled={clearing}
							onClick={handleClearAll}
							className="gap-1.5 text-destructive hover:text-destructive"
						>
							{clearing ? (
								<Loader2Icon className="size-3.5 animate-spin" />
							) : (
								<Trash2Icon className="size-3.5" />
							)}
							Clear all
						</Button>
					</DialogFooter>
				)}
			</DialogContent>
		</Dialog>
	);
}
