"use client";

import {
	IBitTypes,
	IRole,
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
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@flow-like/flow-like-ui";
import {
	type CopilotStreamEvent,
	createCopilotStreamParser,
} from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
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
import type {
	IPlanStep,
	PlanStepStatus,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import type { ISendMessageFunction } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chatbox";
import { createId } from "@paralleldrive/cuid2";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	BotIcon,
	BrainIcon,
	Code2Icon,
	GithubIcon,
	LayersIcon,
	SparklesIcon,
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
import { GlobalChatHistory } from "./global-chat-history";
import { InlineAppChatCard } from "./inline-app-chat-card";

// global_chat streams raw assistant text interleaved with the FlowPilot XML control protocol; the
// shared parser (createCopilotStreamParser) turns chunks into typed events, which we accumulate into
// the message's content + plan_steps so the presentational <Chat> renders tool activity and reasoning
// exactly like the board copilot and the simple chat.

function mapPlanStepStatus(status: unknown): PlanStepStatus {
	const value = String(status ?? "").toLowerCase();
	if (["done", "completed", "complete", "success"].includes(value))
		return "done";
	if (["failed", "error", "cancelled"].includes(value)) return "failed";
	if (["planned", "pending", "todo", "queued"].includes(value))
		return "planned";
	return "progress";
}

function readPlanStep(data: unknown): Omit<IPlanStep, "timestamp"> | null {
	if (!data || typeof data !== "object") return null;
	const source = (data as { PlanStep?: unknown }).PlanStep ?? data;
	if (!source || typeof source !== "object") return null;
	const record = source as Record<string, unknown>;
	const id = String(record.id ?? record.step_id ?? "");
	if (!id) return null;
	const title = String(
		record.title ?? record.tool_name ?? record.message ?? "Step",
	);
	const description =
		typeof record.description === "string"
			? record.description
			: typeof record.message === "string"
				? record.message
				: undefined;
	return {
		id,
		title,
		description,
		status: mapPlanStepStatus(record.status),
		reasoning:
			typeof record.reasoning === "string" ? record.reasoning : undefined,
	};
}

interface StreamAccumulator {
	content: string;
	stepOrder: string[];
	steps: Map<string, IPlanStep>;
	currentStepId?: string;
}

function toolFieldId(data: unknown, fallback: string): string {
	const record = (data ?? {}) as Record<string, unknown>;
	return String(
		record.tool_call_id ?? record.toolCallId ?? record.id ?? fallback,
	);
}

function toolFieldName(data: unknown): string {
	const record = (data ?? {}) as Record<string, unknown>;
	return String(record.tool_name ?? record.toolName ?? record.name ?? "tool");
}

function applyStreamEvent(acc: StreamAccumulator, event: CopilotStreamEvent) {
	const upsertStep = (step: IPlanStep) => {
		if (!acc.steps.has(step.id)) acc.stepOrder.push(step.id);
		acc.steps.set(step.id, step);
	};

	switch (event.type) {
		case "text":
			if (event.text) acc.content += event.text;
			break;
		case "plan_step": {
			const step = readPlanStep(event.data);
			if (!step) break;
			const existing = acc.steps.get(step.id);
			upsertStep({
				...step,
				description: step.description ?? existing?.description,
				reasoning: step.reasoning ?? existing?.reasoning,
				timestamp: existing?.timestamp ?? Date.now(),
			});
			acc.currentStepId =
				step.status === "progress" || step.status === "planned"
					? step.id
					: undefined;
			break;
		}
		case "tool_start":
		case "tool_call": {
			const id =
				event.type === "tool_call"
					? `call-${acc.stepOrder.length}`
					: toolFieldId(event.data, `tool-${acc.stepOrder.length}`);
			const name =
				event.type === "tool_call"
					? (event.raw ?? "tool")
					: toolFieldName(event.data);
			upsertStep({
				id,
				title: `Using ${name}`,
				status: "progress",
				timestamp: acc.steps.get(id)?.timestamp ?? Date.now(),
			});
			acc.currentStepId = id;
			break;
		}
		case "tool_end": {
			const id = toolFieldId(event.data, "");
			const existing = id ? acc.steps.get(id) : undefined;
			const record = (event.data ?? {}) as Record<string, unknown>;
			if (existing) {
				acc.steps.set(id, {
					...existing,
					status: record.status === "error" ? "failed" : "done",
				});
			}
			acc.currentStepId = undefined;
			break;
		}
		case "tool_result": {
			if (acc.currentStepId) {
				const existing = acc.steps.get(acc.currentStepId);
				if (existing)
					acc.steps.set(acc.currentStepId, { ...existing, status: "done" });
			}
			acc.currentStepId = undefined;
			break;
		}
		default:
			break;
	}
}

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

// Radix Select disallows an empty value, so "memory off" uses a sentinel mapped back to "".
const MEMORY_OFF = "__off__";

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
		if (!isAgent) {
			setSelectedModelId("");
			return;
		}
		const models = copilotSDK.models;
		if (models.length === 0) return;
		if (!selectedModelId || !models.some((m) => m.id === selectedModelId)) {
			setSelectedModelId(models[0].id);
		}
	}, [isAgent, copilotSDK.models, selectedModelId, setSelectedModelId]);

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
			chatRef.current?.pushCurrentMessageUpdate({ ...responseMessage });

			const historyPayload = priorMessages.map((m) => ({
				role: m.inner.role === IRole.Assistant ? "Assistant" : "User",
				content: typeof m.inner.content === "string" ? m.inner.content : "",
			}));

			const parser = createCopilotStreamParser();
			const acc: StreamAccumulator = {
				content: "",
				stepOrder: [],
				steps: new Map(),
			};
			const syncMessage = () => {
				responseMessage.inner.content = acc.content;
				responseMessage.plan_steps = acc.stepOrder
					.map((id) => acc.steps.get(id))
					.filter((step): step is IPlanStep => step !== undefined);
				responseMessage.current_step_id = acc.currentStepId;
				responseMessage.tools = acc.currentStepId ? ["working"] : [];
				chatRef.current?.pushCurrentMessageUpdate({ ...responseMessage });
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
					channel,
				});
			} catch (error) {
				acc.content =
					acc.content ||
					`Something went wrong: ${error instanceof Error ? error.message : String(error)}`;
			} finally {
				// Emit any held-back partial-tag fragment so replies ending in '<...' are not lost.
				for (const event of parser.flush()) applyStreamEvent(acc, event);
				for (const id of acc.stepOrder) {
					const step = acc.steps.get(id);
					if (step?.status === "progress") {
						acc.steps.set(id, { ...step, status: "done" });
					}
				}
				responseMessage.inner.content = acc.content;
				responseMessage.plan_steps = acc.stepOrder
					.map((id) => acc.steps.get(id))
					.filter((step): step is IPlanStep => step !== undefined);
				responseMessage.current_step_id = undefined;
				responseMessage.tools = [];
				const finalized = { ...responseMessage };
				appendMessage(finalized);
				void persist(finalized);
				chatRef.current?.clearCurrentMessageUpdate();
				setStreaming(false);
			}
		},
		[appendMessage, setStreaming, backend],
	);

	// Auto-send the draft handed off from the landing hero bar (once).
	const draftSentRef = useRef(false);
	// biome-ignore lint/correctness/useExhaustiveDependencies: run once on the initial draft, not on every handleSendMessage identity change.
	useEffect(() => {
		if (draftSentRef.current) return;
		const draft = consumeDraft();
		if (!draft) return;
		draftSentRef.current = true;
		if (draft.modelId) setSelectedModelId(draft.modelId);
		void handleSendMessage(draft.prompt, draft.files);
	}, [consumeDraft, setSelectedModelId]);

	const compact = variant === "overlay";

	const modelPicker = useMemo(
		() =>
			isAgent && copilotSDK.models.length > 0 ? (
				<Select value={selectedModelId} onValueChange={setSelectedModelId}>
					<SelectTrigger className="h-8 min-w-0 max-w-40 ml-1">
						<SelectValue placeholder="Model" />
					</SelectTrigger>
					<SelectContent className="z-[10000]">
						{copilotSDK.models.map((model) => (
							<SelectItem key={model.id} value={model.id} className="text-xs">
								{model.name || model.id}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : null,
		[isAgent, copilotSDK.models, selectedModelId, setSelectedModelId],
	);

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
						className="h-8 min-w-0 max-w-36 ml-1"
						title="Profile memory embedding model"
					>
						<BrainIcon className="size-3.5 mr-1 text-muted-foreground shrink-0" />
						<SelectValue placeholder="Memory: off" />
					</SelectTrigger>
					<SelectContent className="z-[10000]">
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
		messages.length === 0 && inlineAppChats.length === 0 && !isStreaming;

	return (
		<div className="flex flex-col flex-1 min-h-0 w-full h-full">
			<header className="flex items-center gap-1.5 px-3 py-2 border-b border-border/50 shrink-0 overflow-x-auto">
				<div className="flex items-center gap-0.5 rounded-xl border border-border/40 bg-background/40 p-1 shrink-0">
					{PROVIDERS.map(({ id, label, icon: Icon }) => {
						const active = normalizeAIProvider(id) === normalizedProvider;
						return (
							<Button
								key={id}
								variant="ghost"
								size="sm"
								className={`h-7 gap-1.5 rounded-lg px-2.5 text-xs ${active ? "bg-primary text-primary-foreground shadow-sm hover:bg-primary hover:text-primary-foreground" : "text-muted-foreground"}`}
								onClick={() => setProvider(id)}
							>
								<Icon className="w-3.5 h-3.5" />
								{compact ? null : label}
							</Button>
						);
					})}
				</div>
				{modelPicker}
				{memoryPicker}
				<GlobalChatHistory />
			</header>

			{inlineAppChats.length > 0 && (
				<div className="shrink-0 max-h-[60vh] overflow-y-auto pt-2">
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

			<div className="relative flex-1 min-h-0">
				{showEmptyState && (
					<div className="pointer-events-none absolute inset-x-0 top-0 bottom-24 z-10 flex flex-col items-center justify-center gap-4 px-6 text-center">
						<span className="flex items-center justify-center size-14 rounded-2xl bg-primary/10 text-primary">
							<SparklesIcon className="size-7" />
						</span>
						<div className="space-y-1">
							<h2 className="text-lg font-semibold tracking-tight">
								Chat with FlowPilot
							</h2>
							<p className="text-sm text-muted-foreground max-w-sm">
								Ask anything — or let it create apps, open the store, and talk
								to your apps for you.
							</p>
						</div>
						{!compact && (
							<div className="pointer-events-auto flex flex-wrap items-center justify-center gap-2 max-w-md">
								{[
									"Create a new app",
									"What can I build with Flow-Like?",
									"Show me the package store",
								].map((suggestion) => (
									<Button
										key={suggestion}
										variant="outline"
										size="sm"
										className="h-8 rounded-full text-xs text-foreground/80 border-border/60 bg-background/60 backdrop-blur-sm hover:bg-primary/10 hover:text-primary hover:border-primary/40 transition-colors"
										onClick={() => void handleSendMessage(suggestion)}
									>
										{suggestion}
									</Button>
								))}
							</div>
						)}
					</div>
				)}
				<Chat
					ref={chatRef}
					sessionId={activeConversationId}
					messages={messages}
					onSendMessage={handleSendMessage}
					isStreamActive={isStreaming}
					config={{ allow_file_upload: true, tools: [] }}
				/>
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
