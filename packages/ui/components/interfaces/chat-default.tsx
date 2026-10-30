"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { useLiveQuery } from "dexie-react-hooks";
import {
	ChevronDownIcon,
	HistoryIcon,
	HomeIcon,
	Loader2Icon,
	SquarePenIcon,
} from "lucide-react";
import { usePathname } from "next/navigation";
import {
	type MutableRefObject,
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	type IContent,
	IContentType,
	type IHistoryMessage,
	IRole,
	Response,
} from "../../lib";
import { useClientRouter } from "../../lib/client-navigation";
import type { ExecutionEngineProvider } from "../../lib/execution-engine";
import {
	type IChatMessageError,
	buildChatMessageError,
} from "../../lib/flowpilot/chat-error";
import { getCurrentPageContext } from "../../lib/page-context";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import type { IInteractionRequest } from "../../lib/schema/interaction";
import { runtimeChatSessionId } from "../../lib/service-runtime/session-scope";
import {
	useClientSearchParams,
	useSetQueryParams,
} from "../../lib/set-query-params";
import { captureTelemetryError } from "../../lib/telemetry/errors";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { captureWidgetSnapshots } from "../../lib/widget-snapshot";
import { type IBackendState, useBackend } from "../../state/backend-state";
import { registerChatActivitySurface } from "../../state/chat-activity-store";
import { useExecutionEngine } from "../../state/execution-engine-context";
import { getFrontendStateStore } from "../a2ui/frontend-state";
import { buildWorkflowFrontendContext } from "../a2ui/workflow-payload";
import type { ChatThinkingLevel } from "../thinking/thinking-options";
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
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../ui";
import { ChatAppearance } from "./chat-default/appearance";
import { fileToAttachment } from "./chat-default/attachment";
import { Chat, type IChatRef } from "./chat-default/chat";
import {
	completeAppChatTurn,
	createAppChatActivityWriter,
} from "./chat-default/chat-activity";
import {
	type IAttachment,
	type IChatMainWidget,
	type IGlobalState,
	type ILocalChatState,
	type IMessage,
	chatDb,
	decodeMessageRow,
	latestChatDraft,
	promoteChatDrafts,
	putChatMessage,
} from "./chat-default/chat-db";
import {
	type IChatMessageInteraction,
	applyMessageReaction,
	createMessageReference,
	historyMessageForModel,
} from "./chat-default/chat-message-interaction";
import {
	type IChatTurn,
	type IStreamLiveState,
	createChatTurn,
} from "./chat-default/chat-turn";
import {
	ChatWidgetExecutionProvider,
	type RunWidgetAction,
} from "./chat-default/chat-widget-execution";
import type { ISendMessageFunction } from "./chat-default/chatbox";
import { processChatEvents } from "./chat-default/event-processor";
import { ChatHistory } from "./chat-default/history";
import {
	refreshMainChatWidget,
	resolveMainChatWidget,
} from "./chat-default/main-widget-state";
import { submitInteractionResponse } from "./chat-default/respond-interaction";
import {
	addSessionInteractions,
	updateSessionInteractions,
	useSessionInteractions,
} from "./chat-default/session-interactions";
import {
	persistWorkflowReactions,
	putChatDraftsWithReactions,
} from "./chat-default/workflow-reactions";
import { useChatModel } from "./chat-default/use-chat-model";
import { useChatThinking } from "./chat-default/use-chat-thinking";
import { ChatWelcome } from "./chat-default/welcome";
import type { IUseInterfaceProps } from "./interfaces";

function extractErrorMessage(err: unknown): string {
	if (err instanceof Error) return err.message;
	if (typeof err === "string") return err;
	if (err && typeof err === "object") {
		const obj = err as Record<string, unknown>;
		if (typeof obj.message === "string") return obj.message;
		if (typeof obj.error === "string") return obj.error;
		try {
			return JSON.stringify(err);
		} catch {
			return Object.prototype.toString.call(err);
		}
	}
	return String(err);
}

/** A missing consent is no failure: the OAuth flow re-sends the message once it is granted. */
function runFailureCard(
	chatName: string,
	failure: unknown,
): IChatMessageError | undefined {
	if (failure === undefined) return undefined;
	if ((failure as { isOAuthError?: unknown } | null)?.isOAuthError === true)
		return undefined;
	return buildChatMessageError("bits", failure, chatName || "Chat");
}

async function prepareAttachments(
	filesAttached: File[] | undefined,
	audioFile: File | undefined,
	backend: IBackendState,
	isOffline: boolean,
) {
	const imageFiles =
		filesAttached?.filter((file) => file.type.startsWith("image/")) ?? [];
	const otherFiles =
		filesAttached?.filter((file) => !file.type.startsWith("image/")) ?? [];
	const imageAttachments = await fileToAttachment(
		imageFiles ?? [],
		backend,
		isOffline,
	);
	const otherAttachments = await fileToAttachment(
		otherFiles ?? [],
		backend,
		isOffline,
	);
	if (audioFile) {
		otherAttachments.push(
			...(await fileToAttachment([audioFile], backend, isOffline)),
		);
	}
	return {
		imageAttachments,
		otherAttachments,
		historyAttachments: [...imageAttachments, ...otherAttachments],
	};
}

/**
 * Deduplicates messages by ID.
 * When multiple messages share the same ID (e.g. from incremental saves), keeps the one with more content.
 * Preserves legitimate consecutive same-role messages that have different IDs.
 */
function deduplicateConsecutiveMessages(messages: IMessage[]): IMessage[] {
	if (messages.length <= 1) return messages;

	const seen = new Map<string, number>();
	const result: IMessage[] = [];

	for (const message of messages) {
		const existingIdx = seen.get(message.id);
		if (existingIdx !== undefined) {
			const existing = result[existingIdx];
			const existingLen =
				typeof existing.inner.content === "string"
					? existing.inner.content.length
					: JSON.stringify(existing.inner.content).length;
			const currentLen =
				typeof message.inner.content === "string"
					? message.inner.content.length
					: JSON.stringify(message.inner.content).length;
			if (currentLen > existingLen) {
				result[existingIdx] = message;
			}
			continue;
		}
		seen.set(message.id, result.length);
		result.push(message);
	}

	return result;
}

function createHistoryMessage(content: string, attachments: IAttachment[]) {
	const historyMessage: IHistoryMessage = {
		content: [
			{
				type: IContentType.Text,
				text: content,
			},
		],
		role: IRole.User,
	};

	for (const attachment of attachments) {
		const url = typeof attachment === "string" ? attachment : attachment.url;
		const explicitMime =
			typeof attachment === "string" ? undefined : attachment.type;
		const dataMime = url.match(/^data:([^;,]+)/)?.[1];
		const mime = (explicitMime || dataMime || "").toLowerCase();
		const cleanUrl = url.split(/[?#]/, 1)[0].toLowerCase();
		const canInferFromExtension = mime === "";
		const parts = historyMessage.content as IContent[];

		if (
			[
				"image/jpeg",
				"image/jpg",
				"image/png",
				"image/gif",
				"image/webp",
				"image/heic",
				"image/heif",
				"image/svg+xml",
			].includes(mime) ||
			(canInferFromExtension &&
				/\.(jpe?g|png|gif|webp|heic|heif|svg)$/.test(cleanUrl))
		) {
			parts.push({
				type: IContentType.IImageURL,
				image_url: { url, media_type: mime || undefined },
			});
			continue;
		}
		if (
			[
				"audio/wav",
				"audio/x-wav",
				"audio/wave",
				"audio/mp3",
				"audio/mpeg",
				"audio/mpeg3",
				"audio/aiff",
				"audio/x-aiff",
				"audio/aac",
				"audio/ogg",
				"audio/flac",
				"audio/m4a",
				"audio/x-m4a",
				"audio/mp4",
				"audio/pcm16",
				"audio/pcm24",
			].includes(mime) ||
			(canInferFromExtension &&
				/\.(wav|mp3|aiff?|aac|ogg|flac|m4a|pcm16|pcm24)$/.test(cleanUrl))
		) {
			parts.push({
				type: IContentType.AudioURL,
				audio_url: url,
				media_type: mime || undefined,
			});
			continue;
		}
		if (
			[
				"video/avi",
				"video/x-msvideo",
				"video/mp4",
				"video/mpeg",
				"video/mov",
				"video/quicktime",
				"video/webm",
			].includes(mime) ||
			(canInferFromExtension && /\.(avi|mp4|mpe?g|mov|webm)$/.test(cleanUrl))
		) {
			parts.push({
				type: IContentType.VideoURL,
				video_url: url,
				media_type: mime || undefined,
			});
			continue;
		}
		if (
			[
				"application/pdf",
				"text/plain",
				"text/rtf",
				"application/rtf",
				"text/html",
				"text/css",
				"text/markdown",
				"text/md",
				"text/x-markdown",
				"text/csv",
				"text/xml",
				"application/xml",
				"application/x-javascript",
				"application/javascript",
				"text/javascript",
				"text/x-javascript",
				"application/x-python",
				"text/x-python",
			].includes(mime) ||
			(canInferFromExtension &&
				/\.(pdf|txt|rtf|html?|css|md|markdown|csv|xml|m?js|cjs|py)$/.test(
					cleanUrl,
				))
		) {
			parts.push({
				type: IContentType.DocumentURL,
				document_url: url,
				media_type: mime || undefined,
			});
			continue;
		}

		// Unrecognized media types must still reach the model: fall back to the MIME's top-level
		// type so an attachment is never silently dropped from the outgoing message.
		if (mime.startsWith("image/")) {
			parts.push({
				type: IContentType.IImageURL,
				image_url: { url, media_type: mime },
			});
		} else if (mime.startsWith("audio/")) {
			parts.push({
				type: IContentType.AudioURL,
				audio_url: url,
				media_type: mime,
			});
		} else if (mime.startsWith("video/")) {
			parts.push({
				type: IContentType.VideoURL,
				video_url: url,
				media_type: mime,
			});
		} else {
			parts.push({
				type: IContentType.DocumentURL,
				document_url: url,
				media_type: mime || undefined,
			});
		}
	}
	return historyMessage;
}

async function updateSession(
	sessionId: string,
	appId: string,
	content: string,
) {
	const sessionExists = await chatDb.sessions
		.where("id")
		.equals(sessionId)
		.count();

	if (sessionExists <= 0) {
		await chatDb.sessions.add({
			id: sessionId,
			appId,
			summarization: content,
			createdAt: Date.now(),
			updatedAt: Date.now(),
		});
	} else {
		await chatDb.sessions.update(sessionId, {
			updatedAt: Date.now(),
		});
	}
}

function createUserMessage(
	sessionId: string,
	appId: string,
	otherAttachments: IAttachment[],
	historyMessage: IHistoryMessage,
	activeTools: string[],
	thinking: ChatThinkingLevel | undefined,
	interaction?: IChatMessageInteraction,
): IMessage {
	return {
		id: createId(),
		sessionId: sessionId,
		appId,
		files: otherAttachments,
		inner: historyMessage,
		timestamp: Date.now(),
		tools: activeTools ?? [],
		actions: [],
		thinking,
		interaction,
	};
}

function createPayload(
	userMessage: IMessage,
	lastMessages: IMessage[],
	historyMessage: IHistoryMessage,
	localState: ILocalChatState | undefined,
	globalState: IGlobalState | undefined,
	activeTools: string[],
	otherAttachments: IAttachment[],
	thinking: ChatThinkingLevel,
	selectedModel?: string,
) {
	return {
		chat_id: userMessage.sessionId,
		messages: [
			...lastMessages.map((msg) => {
				const history = historyMessageForModel(msg);
				return {
					role: history.role,
					content:
						typeof history.content === "string"
							? history.content
							: history.content?.map((c) => ({
									type: c.type,
									text: c.text,
									image_url: c.image_url,
									audio_url: c.audio_url,
									video_url: c.video_url,
									document_url: c.document_url,
									media_type: c.media_type,
									additional_params: c.additional_params,
								})),
				};
			}),
			historyMessageForModel(userMessage, historyMessage),
		],
		local_session: localState?.localState ?? {},
		global_session: globalState?.globalState ?? {},
		actions: [],
		tools: activeTools ?? [],
		attachments: otherAttachments,
		thinking,
		...(selectedModel ? { selected_model: selectedModel } : {}),
	};
}

const mainWidgetSeeds = new WeakMap<IMessage, IChatMainWidget>();

function seedMainWidget(
	message: IMessage,
	mainWidget: IChatMainWidget | undefined,
) {
	message.main_widget = mainWidget;
	if (mainWidget) mainWidgetSeeds.set(message, mainWidget);
	return message;
}

function createResponseMessage(
	sessionId: string,
	appId: string,
	eventName: string,
	mainWidget?: IChatMainWidget,
	triggerMessageId?: string,
): IMessage {
	return seedMainWidget(
		{
			id: createId(),
			sessionId: sessionId,
			appId,
			trigger_message_id: triggerMessageId,
			files: [],
			inner: {
				role: IRole.Assistant,
				content: "",
			},
			explicit_name: eventName,
			timestamp: Date.now(),
			tools: [],
			actions: [],
		},
		mainWidget,
	);
}

/**
 * A bare copy that keeps the message's identity and slot but none of its streamed state, so a
 * replay of the stream's events rebuilds it from scratch.
 */
function cloneResponseMessageForReplay(responseMessage: IMessage): IMessage {
	const clonedMessage =
		typeof structuredClone === "function"
			? structuredClone(responseMessage)
			: (JSON.parse(JSON.stringify(responseMessage)) as IMessage);

	clonedMessage.id = responseMessage.stream_root_id ?? responseMessage.id;
	clonedMessage.timestamp =
		responseMessage.stream_root_timestamp ?? responseMessage.timestamp;
	clonedMessage.stream_message_id = undefined;
	clonedMessage.files = [];
	clonedMessage.inner = {
		...clonedMessage.inner,
		content: "",
	};
	clonedMessage.plan_steps = undefined;
	clonedMessage.current_step_id = undefined;
	clonedMessage.usage_stats = undefined;
	clonedMessage.widgets = undefined;

	return seedMainWidget(clonedMessage, mainWidgetSeeds.get(responseMessage));
}

interface IStreamCompletionOptions {
	responseMessage: IMessage;
	chatRef: RefObject<IChatRef | null>;
	executionEngine: ExecutionEngineProvider;
	streamId: string;
	subscriberId: string;
	processedCompletedStreams: MutableRefObject<Set<string>>;
	events: IIntercomEvent[];
	appId: string;
	eventId: string;
	sessionId: string;
	/** Absent when nobody was listening while the stream ran. */
	live?: IStreamLiveState;
	/** Why the run failed, rendered on the reply in place of the content it never sent. */
	error?: IChatMessageError;
	onInteractions?: (interactions: IInteractionRequest[]) => void;
}

async function handleStreamCompletion({
	responseMessage,
	chatRef,
	executionEngine,
	streamId,
	subscriberId,
	processedCompletedStreams,
	events,
	appId,
	eventId,
	sessionId,
	live,
	error,
	onInteractions,
}: IStreamCompletionOptions) {
	const persistenceManaged =
		executionEngine.isPersistenceManaged?.(streamId) ?? false;
	// Detach this view before another turn starts using the session.
	executionEngine.unsubscribeFromEventStream(streamId, subscriberId);
	if (processedCompletedStreams.current.has(streamId)) {
		return;
	}

	processedCompletedStreams.current.add(streamId);
	if (persistenceManaged) {
		// The shared runtime publishes completion only after the transcript is durable.
		chatRef.current?.clearCurrentMessageUpdate();
		chatRef.current?.scrollToBottom();
		return;
	}

	try {
		let messages = live?.responseMessages ?? [responseMessage];
		let tmpLocalState = live?.tmpLocalState ?? null;
		let tmpGlobalState = live?.tmpGlobalState ?? null;

		// A subscriber that folded every event already holds the final message: chat_out closed it
		// and its interactions were delivered as they arrived. Only a gap — no listener, or a batch
		// it never processed — justifies rebuilding the turn from all events.
		if (!live || live.processedEvents !== events.length) {
			const result = processChatEvents(events, {
				intermediateResponse: Response.default(),
				responseMessage: cloneResponseMessageForReplay(responseMessage),
				attachments: new Map(),
				tmpLocalState,
				tmpGlobalState,
				done: false,
				appId,
				eventId,
				sessionId,
			});
			messages = result.responseMessages;
			tmpLocalState = result.tmpLocalState ?? null;
			tmpGlobalState = result.tmpGlobalState ?? null;
			if (result.interactions?.length && onInteractions) {
				onInteractions(result.interactions);
			}
		}

		if (error) messages[messages.length - 1].error = error;

		if (tmpLocalState) {
			await chatDb.localStage.put(tmpLocalState);
		}

		if (tmpGlobalState) {
			await chatDb.globalState.put(tmpGlobalState);
		}

		// Persist before clearing the streaming bubble so the message never blinks out in between.
		await completeAppChatTurn(
			messages,
			{ appId, eventId, sessionId },
			{ events },
		);
		await persistWorkflowReactions(events, {
			appId,
			sessionId,
			triggerMessageId: responseMessage.trigger_message_id,
		});

		chatRef.current?.clearCurrentMessageUpdate();
		chatRef.current?.scrollToBottom();
	} catch (error) {
		processedCompletedStreams.current.delete(streamId);
		throw error;
	}
}

function ConversationLoading() {
	const { t } = useTranslation("interfaces");
	return (
		<div className="flex h-full flex-col items-center justify-center gap-3">
			<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
			<p className="text-sm text-muted-foreground">
				{t("loadingConversation", "Loading conversation...")}
			</p>
		</div>
	);
}

/**
 * Periodic checkpoint of the streaming message, written to `drafts` so the session's message query
 * stays quiet until the turn settles. Recovery only: the live bubble renders from React state, and
 * completion moves the final message into `messages`. The engine keeps calling it after the chat
 * that started the run unmounted or switched session, so it folds what that chat no longer sees.
 */
function createChatIncrementalSaver(
	turn: IChatTurn,
): (events: IIntercomEvent[]) => Promise<void> {
	return async (events) => {
		turn.catchUp(events);
		await putChatDraftsWithReactions(turn.responseMessages);
		await persistWorkflowReactions(events, {
			appId: turn.responseMessage.appId,
			sessionId: turn.responseMessage.sessionId,
			triggerMessageId: turn.responseMessage.trigger_message_id,
		});
		const { tmpLocalState, tmpGlobalState } = turn.live();
		if (tmpLocalState) {
			await chatDb.localStage.put(tmpLocalState);
		}
		if (tmpGlobalState) {
			await chatDb.globalState.put(tmpGlobalState);
		}
	};
}

export const ChatInterfaceMemoized = memo(function ChatInterface({
	appId,
	event,
	config = {},
	toolbarRef,
	sidebarRef,
	onNavigate,
}: Readonly<IUseInterfaceProps>) {
	const { t } = useTranslation("interfaces");
	const router = useClientRouter();
	const backend = useBackend();
	const executionEngine = useExecutionEngine();
	const searchParams = useClientSearchParams();
	const pathname = usePathname();
	// A chat cannot read its history without a session, and the router only
	// commits one a tick later. Deriving it on the first render keeps every live
	// query on a single key instead of tearing them all down once it lands.
	const [fallbackSessionId] = useState(() => createId());
	const urlSessionId = searchParams.get("sessionId") ?? "";
	const sessionIdParameter = runtimeChatSessionId(
		appId,
		urlSessionId || fallbackSessionId,
	);
	useEffect(
		() =>
			registerChatActivitySurface({
				kind: "app",
				appId,
				eventId: event.id,
				sessionId: sessionIdParameter,
			}),
		[appId, event.id, sessionIdParameter],
	);
	const prefilledMessage = searchParams.get("message");
	const setQueryParams = useSetQueryParams();
	const chatRef = useRef<IChatRef>(null);
	const sessionIdRef = useRef(sessionIdParameter);
	sessionIdRef.current = sessionIdParameter;
	const activeSubscriptions = useRef<string[]>([]);
	const processedCompletedStreams = useRef<Set<string>>(new Set());
	const reconnectSubscribed = useRef<Set<string>>(new Set());
	const pendingSendSessions = useRef<Set<string>>(new Set());
	const [isSendingFromWelcome, setIsSendingFromWelcome] = useState(false);
	const [isStreamActive, setIsStreamActive] = useState(false);
	const [runtimeReady, setRuntimeReady] = useState(
		() => executionEngine.isRuntimeReady?.() ?? true,
	);
	const [showPrefilledConfirm, setShowPrefilledConfirm] = useState(false);
	const prefilledConsumed = useRef(false);
	const lastNavigateToRef = useRef<string | null>(null);
	const thinking = useChatThinking(appId, event.id, config);
	const thinkingRef = useRef(thinking);
	thinkingRef.current = thinking;
	const model = useChatModel(appId, config, event.execution_mode);
	const modelRef = useRef(model);
	modelRef.current = model;
	const activeInteractions = useSessionInteractions(sessionIdParameter);
	const activeInteractionsRef =
		useRef<IInteractionRequest[]>(activeInteractions);
	useEffect(() => {
		activeInteractionsRef.current = activeInteractions;
	}, [activeInteractions]);

	const handleRespondToInteraction = useCallback(
		async (interactionId: string, value: unknown) => {
			const interaction = activeInteractionsRef.current.find(
				(i) => i.id === interactionId,
			);
			if (!interaction) {
				console.warn(
					"[Chat] Interaction not found for response:",
					interactionId,
				);
				return;
			}

			try {
				const accepted = await submitInteractionResponse(interaction, value);

				updateSessionInteractions(sessionIdParameter, (prev) =>
					prev.map((i) =>
						i.id === interactionId
							? {
									...i,
									status: "responded" as const,
									response_value: accepted ? accepted.value : value,
								}
							: i,
					),
				);
			} catch (err) {
				console.error("[Chat] Failed to respond to interaction:", err);
				captureTelemetryError(err, {
					culprit: "chat.submit_interaction_response",
				});
				toast.error(`Failed to submit response: ${extractErrorMessage(err)}`);
			}
		},
		[sessionIdParameter],
	);

	const buildUseNavigationUrl = useCallback(
		(route: string, queryParams?: Record<string, string>): string => {
			let navUrl = route;

			if (!route) {
				return `/use?id=${appId}&route=/`;
			}

			if (appId && !route.startsWith("/use") && !route.startsWith("http")) {
				const [routePath, routeQueryString] = route.split("?");
				const params = new URLSearchParams();
				params.set("id", appId);
				params.set("route", routePath || "/");
				params.delete("eventId");

				if (routeQueryString) {
					const routeParams = new URLSearchParams(routeQueryString);
					routeParams.forEach((value, key) => {
						params.set(key, value);
					});
				}

				if (queryParams) {
					for (const [key, value] of Object.entries(queryParams)) {
						params.set(key, value);
					}
				}
				return `/use?${params.toString()}`;
			}

			if (queryParams && Object.keys(queryParams).length > 0) {
				const params = new URLSearchParams(queryParams);
				const separator = navUrl.includes("?") ? "&" : "?";
				navUrl = `${navUrl}${separator}${params.toString()}`;
			}

			return navUrl;
		},
		[appId],
	);

	const handleNavigateTo = useCallback(
		(route: string, replace: boolean, queryParams?: Record<string, string>) => {
			if (onNavigate) {
				onNavigate(route, replace, queryParams);
				return;
			}
			const navUrl = buildUseNavigationUrl(route, queryParams);
			if (replace) {
				router.replace(navUrl);
			} else {
				router.push(navUrl);
			}
		},
		[buildUseNavigationUrl, router, onNavigate],
	);

	const handleNavigationEvents = useCallback(
		(events: IIntercomEvent[]) => {
			for (const ev of events) {
				if (ev?.event_type !== "a2ui") continue;
				const message = ev?.payload;
				if (!message || message.type !== "navigateTo") continue;

				const { route, replace, queryParams } = message as {
					route: string;
					replace: boolean;
					queryParams?: Record<string, string>;
				};

				const key = `${route}::${replace ? "r" : "p"}::${JSON.stringify(queryParams ?? {})}`;
				if (lastNavigateToRef.current === key) continue;
				lastNavigateToRef.current = key;

				handleNavigateTo(route, replace, queryParams);
			}
		},
		[handleNavigateTo],
	);
	// A reconnected subscription must not resubscribe (and replay the run) when the router changes.
	const handleNavigationEventsRef = useRef(handleNavigationEvents);
	handleNavigationEventsRef.current = handleNavigationEvents;

	// Store pending message data for OAuth retry
	const pendingMessageRef = useRef<{
		requestId: string;
		appId: string;
		eventId: string;
		sessionId: string;
		content: string;
		filesAttached?: File[];
		activeTools?: string[];
		audioFile?: File;
		interaction?: IChatMessageInteraction;
		selectedModel?: string;
	} | null>(null);

	useEffect(() => {
		if (urlSessionId) return;
		setQueryParams("sessionId", fallbackSessionId, { replace: true });
	}, [urlSessionId, fallbackSessionId, setQueryParams]);

	useEffect(() => {
		if (!sessionIdParameter) {
			setIsStreamActive(false);
			return;
		}

		const update = () => {
			setRuntimeReady(executionEngine.isRuntimeReady?.() ?? true);
			setIsStreamActive(
				pendingSendSessions.current.has(sessionIdParameter) ||
					executionEngine.isStreamActive(sessionIdParameter),
			);
		};

		update();
		return executionEngine.subscribeToGlobalUpdates(update);
	}, [executionEngine, sessionIdParameter]);

	// Cleanup active subscriptions on session change
	useEffect(() => {
		processedCompletedStreams.current.clear();
		return () => {
			for (const subId of activeSubscriptions.current) {
				executionEngine.unsubscribeFromEventStream(sessionIdParameter, subId);
			}
			activeSubscriptions.current = [];
		};
	}, [sessionIdParameter, executionEngine]);

	// liveQuery re-materializes every row on any table write. Handing back the
	// previously materialized object while a message's rev is unchanged keeps
	// settled rows identity-stable and skips decoding them, so the memoized
	// MessageComponent (and the widget content-key stringifies below it) skip
	// untouched messages. Every writer in chat-db bumps rev.
	const materializedMessages = useRef(new Map<string, IMessage>());
	const messagesQuery = useLiveQuery(async () => {
		if (!sessionIdParameter) return [];
		const rows = await chatDb.messages
			.where("sessionId")
			.equals(sessionIdParameter)
			.sortBy("timestamp");
		const cache = materializedMessages.current;
		const materialized = rows.map((row) => {
			const cached = cache.get(row.id);
			if (
				cached &&
				(cached.rev ?? 0) === (row.rev ?? 0) &&
				cached.timestamp === row.timestamp
			) {
				return cached;
			}
			const message = decodeMessageRow(row);
			cache.set(row.id, message);
			return message;
		});
		const stable = deduplicateConsecutiveMessages(materialized);
		if (cache.size > stable.length * 2 + 32) {
			const live = new Set(stable.map((m) => m.id));
			for (const id of cache.keys()) {
				if (!live.has(id)) cache.delete(id);
			}
		}
		return stable;
	}, [sessionIdParameter]);

	const messagesLoaded = messagesQuery !== undefined;
	const messages = messagesQuery ?? [];
	const hasMessages = messages.length > 0;

	const messagesRef = useRef<IMessage[]>(messages);
	useEffect(() => {
		messagesRef.current = messages;
	}, [messages]);
	const liveMainWidgets = useRef(new Map<string, IMessage>());
	const readMainWidget = useCallback(
		(sessionId: string) =>
			resolveMainChatWidget(
				messagesRef.current,
				sessionId,
				liveMainWidgets.current.get(sessionId),
			),
		[],
	);
	const rememberMainWidget = useCallback((responses: IMessage[]) => {
		const last = responses.at(-1);
		if (!last) return;
		const mainWidget = resolveMainChatWidget(
			responses,
			last.sessionId,
			liveMainWidgets.current.get(last.sessionId),
		);
		if (mainWidget)
			liveMainWidgets.current.set(last.sessionId, {
				...last,
				main_widget: mainWidget,
			});
	}, []);

	const localState = useLiveQuery(() => {
		if (!sessionIdParameter) return undefined;
		return chatDb.localStage
			.where("[sessionId+eventId]")
			.equals([sessionIdParameter, event.id])
			.first();
	}, [sessionIdParameter, event.id]);

	const globalState = useLiveQuery(
		() =>
			chatDb.globalState
				.where("[appId+eventId]")
				.equals([appId, event.id])
				.first(),
		[appId, event.id],
	);

	const updateSessionId = useCallback(
		(newSessionId: string) => {
			setQueryParams("sessionId", newSessionId);
		},
		[setQueryParams],
	);

	const handleSidebarToggle = useCallback(() => {
		sidebarRef?.current?.toggleOpen();
	}, [sidebarRef]);

	const handleNewChat = useCallback(() => {
		updateSessionId(createId());
	}, [updateSessionId]);

	const handleSessionChange = useCallback(
		(newSessionId: string) => {
			updateSessionId(newSessionId);
			chatRef.current?.scrollToBottom();
		},
		[updateSessionId],
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `t` only labels the hover cards; a language switch must not rebuild the elements and re-push them into the toolbar.
	const toolbarElements = useMemo(() => {
		return [
			<HoverCard key="chat-history" openDelay={200} closeDelay={100}>
				<HoverCardTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="hover:bg-accent hover:text-accent-foreground transition-colors"
						onClick={handleSidebarToggle}
					>
						<HistoryIcon className="w-4 h-4" />
					</Button>
				</HoverCardTrigger>
				<HoverCardContent
					side="bottom"
					align="center"
					className="w-auto p-2 bg-popover border shadow-lg"
					onClick={() => {
						console.log("Open chat history");
					}}
				>
					<div
						className="flex items-center gap-2 text-sm font-medium"
						style={{
							paddingTop: "var(--fl-safe-top, env(safe-area-inset-top, 0px))",
						}}
					>
						<HistoryIcon className="w-3 h-3" />
						{t("chatHistory", "Chat History")}
					</div>
				</HoverCardContent>
			</HoverCard>,
			<HoverCard key="new-chat" openDelay={200} closeDelay={100}>
				<HoverCardTrigger asChild>
					<Button
						onClick={handleNewChat}
						variant="ghost"
						size="icon"
						className="hover:bg-accent hover:text-accent-foreground transition-colors"
					>
						<SquarePenIcon className="w-4 h-4" />
					</Button>
				</HoverCardTrigger>
				<HoverCardContent
					side="bottom"
					align="center"
					className="w-auto p-2 bg-popover border shadow-lg"
					onClick={handleNewChat}
				>
					<div className="flex items-center gap-2 text-sm font-medium">
						<SquarePenIcon className="w-3 h-3" />
						{t("newChat", "New Chat")}
					</div>
				</HoverCardContent>
			</HoverCard>,
		];
	}, [handleSidebarToggle, handleNewChat]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `t` only labels the Navigate trigger; a language switch must not rebuild the elements and re-push them into the nav bar.
	const navElements = useMemo(() => {
		const normalizeRoute = (value: string): string => {
			const trimmed = value.trim();
			if (!trimmed) return "";
			return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
		};

		const configuredRoutes = (() => {
			const rawArray = config?.navigate_to_routes;
			const raw: string[] = Array.isArray(rawArray) ? rawArray : [];
			const normalized = raw
				.map((r) => normalizeRoute(String(r)))
				.filter((r) => !!r);
			return Array.from(new Set(normalized));
		})();

		if (configuredRoutes.length === 0) return [];

		const getRouteLabel = (path: string): string => {
			if (path === "/") return "Home";
			return path.replace(/^\//, "").replace(/-/g, " ").replace(/\//g, " / ");
		};

		const getRouteIcon = (path: string) => {
			if (path === "/") return <HomeIcon className="h-4 w-4" />;
			return null;
		};

		const elements: React.ReactElement[] = [];

		if (configuredRoutes.length === 1) {
			const route = configuredRoutes[0];
			const icon = getRouteIcon(route);
			elements.push(
				<Button
					key={`navigate-${route}`}
					variant="outline"
					size="sm"
					onClick={() => handleNavigateTo(route, false)}
					className="rounded-full px-4 gap-2 font-medium"
				>
					{icon}
					{getRouteLabel(route)}
				</Button>,
			);
		} else if (configuredRoutes.length === 2) {
			elements.push(
				<div
					key="route-nav"
					className="inline-flex items-center rounded-full bg-muted/50 p-0.5"
				>
					{configuredRoutes.map((route) => {
						const icon = getRouteIcon(route);
						return (
							<button
								key={route}
								type="button"
								onClick={() => handleNavigateTo(route, false)}
								className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-full transition-all text-muted-foreground hover:text-foreground hover:bg-background hover:shadow-sm"
							>
								{icon}
								{getRouteLabel(route)}
							</button>
						);
					})}
				</div>,
			);
		} else if (configuredRoutes.length >= 3) {
			elements.push(
				<DropdownMenu key="navigate-menu">
					<DropdownMenuTrigger asChild>
						<Button
							variant="outline"
							size="sm"
							className="rounded-full px-4 gap-2 font-medium"
						>
							{t("navigate", "Navigate")}
							<ChevronDownIcon className="h-3.5 w-3.5 opacity-60" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="start" className="min-w-40">
						{configuredRoutes.map((route) => {
							const icon = getRouteIcon(route);
							return (
								<DropdownMenuItem
									key={route}
									onSelect={() => handleNavigateTo(route, false)}
									className="gap-2"
								>
									{icon}
									{getRouteLabel(route)}
								</DropdownMenuItem>
							);
						})}
					</DropdownMenuContent>
				</DropdownMenu>,
			);
		}

		return elements;
	}, [config, handleNavigateTo]);

	const sidebarContent = useMemo(
		() => (
			<ChatHistory
				appId={appId}
				sessionId={sessionIdParameter}
				onSessionChange={handleSessionChange}
				sidebarRef={sidebarRef}
			/>
		),
		[sessionIdParameter, appId, handleSessionChange, sidebarRef],
	);

	useEffect(() => {
		toolbarRef?.current?.pushToolbarElements(toolbarElements);
		toolbarRef?.current?.pushNavElements(navElements);
		sidebarRef?.current?.pushSidebar(sidebarContent);
	}, [toolbarElements, navElements, sidebarContent, toolbarRef, sidebarRef]);

	// Reconnect to active stream or process completed stream when component mounts or session changes
	useEffect(() => {
		if (!sessionIdParameter) return;
		// Wait for messages to be loaded from IndexedDB
		if (!messagesLoaded || !runtimeReady) return;

		const streamId = sessionIdParameter;

		// No stream in memory means nothing is writing drafts any more: whatever is left is a turn
		// cut off by a reload or crash, and its partial message is the only record of it.
		if (!executionEngine.hasStream(streamId)) {
			promoteChatDrafts(streamId).catch((error) => {
				console.error("[Chat] Failed to promote drafts:", error);
				captureTelemetryError(error, {
					culprit: "chat.promote_drafts",
				});
			});
			return;
		}

		// Prevent processing the same completed stream multiple times
		if (
			executionEngine.isStreamComplete(streamId) &&
			processedCompletedStreams.current.has(streamId)
		) {
			return;
		}

		const subscriberId = `chat-reconnect-${sessionIdParameter}`;

		// Skip if we already have an active subscription for this stream (from handleSendMessage)
		// This prevents duplicate message creation when the reconnection effect re-runs
		if (activeSubscriptions.current.length > 0) {
			return;
		}

		// Skip if we've already subscribed with this reconnect subscriber
		// This prevents duplicates when the effect re-runs due to messages changes
		if (reconnectSubscribed.current.has(subscriberId)) {
			return;
		}

		// The stream checkpoints under the id its sender chose. Resuming under that id lets completion
		// replace the draft; a fresh id would settle a second copy beside it.
		const resumeResponseMessage = async (): Promise<IMessage> => {
			const sharedSeed = executionEngine.getChatSeed?.(streamId);
			if (sharedSeed) return structuredClone(sharedSeed.responseMessage);
			const draft = await latestChatDraft(sessionIdParameter);
			const mainWidget = resolveMainChatWidget(
				messagesRef.current.filter((message) => message.id !== draft?.id),
				sessionIdParameter,
			);
			return draft
				? seedMainWidget(cloneResponseMessageForReplay(draft), mainWidget)
				: createResponseMessage(
						sessionIdParameter,
						appId,
						event.name,
						mainWidget,
					);
		};

		// Active streams need Chat mounted for live updates. Completed streams also use
		// subscription replay so failures before the first event reach their saved reply.
		if (!hasMessages && !executionEngine.isStreamComplete(streamId)) return;

		if (executionEngine.isStreamActive(streamId)) {
			processedCompletedStreams.current.delete(streamId);
		}
		// Mark this subscriber as active before subscribing
		reconnectSubscribed.current.add(subscriberId);
		let cancelled = false;
		let subscribed = false;

		resumeResponseMessage()
			.then((responseMessage) => {
				if (cancelled) return;
				// Settled and dropped while the draft was read: the draft is the whole turn now.
				if (!executionEngine.hasStream(streamId)) {
					reconnectSubscribed.current.delete(subscriberId);
					return promoteChatDrafts(streamId);
				}

				const turn = createChatTurn({
					responseMessage,
					tmpLocalState: null,
					tmpGlobalState: null,
					appId,
					eventId: event.id,
					sessionId: sessionIdParameter,
				});
				// The first batch replays what earlier listeners already acted on; only later
				// batches are live, so only they may navigate.
				let replaying = true;
				subscribed = true;
				executionEngine.subscribeToEventStream(
					streamId,
					subscriberId,
					(events) => {
						if (!replaying) handleNavigationEventsRef.current(events);
						if (!replaying) {
							refreshMainChatWidget(
								turn.responseMessage,
								readMainWidget(sessionIdParameter),
							);
						}
						const result = turn.fold(events);
						rememberMainWidget(result.responseMessages);

						if (result.interactions?.length) {
							addSessionInteractions(sessionIdParameter, result.interactions);
						}

						if (result.shouldUpdate) {
							chatRef.current?.pushCurrentMessageUpdate(
								result.responseMessages.map((message) => ({ ...message })),
							);
							chatRef.current?.scrollToBottom();
						}
					},
					async (events, failure) => {
						subscribed = false;
						await handleStreamCompletion({
							responseMessage,
							chatRef,
							executionEngine,
							streamId,
							subscriberId,
							processedCompletedStreams,
							events,
							appId,
							eventId: event.id,
							sessionId: sessionIdParameter,
							live: turn.live(),
							error: runFailureCard(event.name, failure),
							onInteractions: (interactions) =>
								addSessionInteractions(sessionIdParameter, interactions),
						});
						// Clean up the reconnect subscriber tracking after completion
						reconnectSubscribed.current.delete(subscriberId);
					},
				);
				replaying = false;
			})
			.catch((error) => {
				console.error("[Chat] Failed to resume stream:", error);
				reconnectSubscribed.current.delete(subscriberId);
			});

		return () => {
			cancelled = true;
			if (subscribed)
				executionEngine.unsubscribeFromEventStream(streamId, subscriberId);
			reconnectSubscribed.current.delete(subscriberId);
		};
	}, [
		sessionIdParameter,
		appId,
		event.id,
		event.name,
		executionEngine,
		messagesLoaded,
		runtimeReady,
		hasMessages,
		isStreamActive,
		readMainWidget,
		rememberMainWidget,
	]);

	// Internal function to execute the chat (called after OAuth is confirmed)
	const executeChatMessage = useCallback(
		async (
			content: string,
			filesAttached?: File[],
			activeTools?: string[],
			audioFile?: File,
			interaction?: IChatMessageInteraction,
			skipConsentCheck?: boolean,
		) => {
			const streamId = sessionIdParameter;
			const pending = pendingMessageRef.current;
			if (
				pending &&
				(pending.appId !== appId ||
					pending.eventId !== event.id ||
					pending.sessionId !== streamId)
			) {
				pendingMessageRef.current = null;
			}
			if (
				!skipConsentCheck &&
				pendingMessageRef.current &&
				(interaction || pendingMessageRef.current.interaction)
			) {
				const error = Object.assign(
					new Error(
						"Complete the current authorization request before sending another message.",
					),
					{ isActiveStreamError: true },
				);
				if (!interaction) toast.error(error.message);
				throw error;
			}
			if (
				pendingSendSessions.current.has(streamId) ||
				executionEngine.isStreamActive(streamId)
			) {
				const error = Object.assign(
					new Error("Please wait for the current response to complete."),
					{ isActiveStreamError: true },
				);
				if (!interaction) toast.error(error.message);
				throw error;
			}

			pendingSendSessions.current.add(streamId);
			setIsStreamActive(true);
			const selectedModel = skipConsentCheck
				? pendingMessageRef.current?.selectedModel
				: modelRef.current.selectedModel;
			let userMessageCommitted = false;

			try {
				const isOffline = await backend.isOffline(appId);
				const history_elements =
					parseUint8ArrayToJson(event.config)?.history_elements ?? 5;

				// Check OAuth BEFORE adding message to DB (skip if consent was just granted)
				console.log(
					"[Chat] Checking OAuth. isOffline:",
					isOffline,
					"skipConsentCheck:",
					skipConsentCheck,
				);
				if (!skipConsentCheck && backend.eventState.checkEventOAuth) {
					const oauthResult = await backend.eventState.checkEventOAuth(
						appId,
						event,
					);
					console.log(
						"[Chat] OAuth check result:",
						oauthResult.missingProviders.length,
						"missing providers",
					);
					if (oauthResult.missingProviders.length > 0) {
						const requestId = createId();
						// Store pending message for retry
						pendingMessageRef.current = {
							requestId,
							appId,
							eventId: event.id,
							sessionId: streamId,
							content,
							filesAttached,
							activeTools,
							audioFile,
							interaction,
							selectedModel,
						};
						// Emit OAuth required event
						window.dispatchEvent(
							new CustomEvent("flow:oauth-required", {
								detail: {
									missingProviders: oauthResult.missingProviders,
									appId,
									eventId: event.id,
									sessionId: streamId,
									requestId,
									boardId: event.board_id,
									nodeId: event.node_id,
									payload: {},
								},
							}),
						);
						return; // Don't add message to DB yet
					}
				}

				// Clear pending message since OAuth is satisfied
				pendingMessageRef.current = null;

				const { otherAttachments, historyAttachments } =
					await prepareAttachments(
						filesAttached,
						audioFile,
						backend,
						isOffline,
					);

				const historyMessage = createHistoryMessage(
					content,
					historyAttachments,
				);

				const userMessage = createUserMessage(
					sessionIdParameter,
					appId,
					otherAttachments,
					historyMessage,
					activeTools ?? [],
					thinkingRef.current.stamp,
					interaction,
				);

				await updateSession(sessionIdParameter, appId, content);
				await chatDb.transaction("rw", chatDb.messages, async () => {
					if (interaction?.type === "reaction") {
						const stored = await chatDb.messages.get(
							interaction.target.messageId,
						);
						const target = stored ? decodeMessageRow(stored) : undefined;
						if (
							!target ||
							target.appId !== appId ||
							target.sessionId !== sessionIdParameter
						) {
							throw new Error(
								"The message is no longer available in this chat.",
							);
						}
						await putChatMessage({
							...target,
							reactions: applyMessageReaction(
								target.reactions,
								interaction.emoji,
								interaction.operation,
								interaction.replacesEmoji,
							),
						});
					}
					await putChatMessage(userMessage);
				});
				userMessageCommitted = true;

				const lastMessages =
					messagesRef.current
						?.filter((message) => message.id !== userMessage.id)
						.slice(-history_elements) ?? [];

				// Let vision-capable models see the rendered UI: snapshot the
				// latest assistant message's embedded widgets and attach them to
				// the outgoing turn only — the persisted user message stays clean.
				let payloadHistoryMessage = historyMessage;
				if (config?.attach_widget_snapshots !== false) {
					try {
						const latestWidgets = [...lastMessages]
							.reverse()
							.find(
								(message) =>
									message.inner.role === IRole.Assistant &&
									message.widgets?.length,
							)?.widgets;
						const mainWidget = resolveMainChatWidget(
							messagesRef.current,
							sessionIdParameter,
						)?.widget;
						const widgetIds = [
							...new Set([
								...(latestWidgets ?? []).map((widget) => widget.instance_id),
								...(mainWidget ? [mainWidget.instance_id] : []),
							]),
						];
						if (widgetIds.length) {
							const snapshots = await captureWidgetSnapshots(widgetIds);
							if (snapshots.length) {
								payloadHistoryMessage = {
									...historyMessage,
									content: [
										...(historyMessage.content as IContent[]),
										...snapshots.map((url) => ({
											type: IContentType.IImageURL,
											image_url: { url },
										})),
									],
								};
							}
						}
					} catch (error) {
						console.warn("[Chat] widget snapshot failed:", error);
					}
				}

				const payload = createPayload(
					userMessage,
					lastMessages,
					payloadHistoryMessage,
					localState,
					globalState,
					activeTools ?? [],
					otherAttachments,
					thinkingRef.current.level,
					selectedModel,
				);

				const responseMessage = createResponseMessage(
					sessionIdParameter,
					appId,
					event.name,
					readMainWidget(sessionIdParameter),
					interaction?.type === "reaction"
						? interaction.target.messageId
						: userMessage.id,
				);

				chatRef.current?.pushCurrentMessageUpdate({ ...responseMessage });
				chatRef.current?.scrollToBottom();

				const turn = createChatTurn({
					responseMessage,
					tmpLocalState: localState,
					tmpGlobalState: globalState,
					appId,
					eventId: event.id,
					sessionId: sessionIdParameter,
				});

				const subscriberId = `chat-${responseMessage.id}`;
				activeSubscriptions.current.push(subscriberId);

				// Clear stale completion tracking so this stream's completion is processed
				processedCompletedStreams.current.delete(streamId);
				reconnectSubscribed.current.delete(
					`chat-reconnect-${sessionIdParameter}`,
				);

				// Create incremental save function for robust message persistence
				// This saves the message every N events to prevent data loss
				const activityWriter = createAppChatActivityWriter(
					turn,
					{ appId, eventId: event.id, sessionId: sessionIdParameter },
					() => executionEngine.getAccumulatedEvents(streamId),
				);
				const saveCheckpoint = createChatIncrementalSaver(turn);
				const incrementalSave = async (events: IIntercomEvent[]) => {
					if (executionEngine.isPersistenceManaged?.(streamId)) return;
					await activityWriter.settled();
					await saveCheckpoint(events);
				};
				// The chat event owns its state; embedded widgets use their surface IDs.
				const frontendContext = await buildWorkflowFrontendContext(
					appId,
					event.id,
				);

				// Start execution first to reset the stream state
				const executionPromise = executionEngine.executeEvent(streamId, {
					appId,
					eventId: event.id,
					payload: {
						id: event.node_id,
						payload: { ...payload, ...frontendContext },
					},
					streamState: false,
					chat: {
						responseMessage: structuredClone(responseMessage),
						tmpLocalState: localState,
						tmpGlobalState: globalState,
					},
					onExecutionStart: (execution_id: string) => {},
					onLiveEvents: (events) => {
						for (const item of events) {
							if (item.event_type === "a2ui") {
								getFrontendStateStore(appId).handleMessage(item.payload);
							}
						}
						if (!executionEngine.isPersistenceManaged?.(streamId))
							activityWriter.onEvents(events);
					},
					path: `${pathname}?id=${appId}&eventId=${event.id}&sessionId=${sessionIdParameter}`,
					title: event.name || "Chat",
					interfaceType: "chat",
					skipConsentCheck,
					// Save to Dexie every 10 events and on completion for robustness
					onIncrementalSave: incrementalSave,
					onUnobservedComplete: async (events, failure) => {
						if (executionEngine.isPersistenceManaged?.(streamId)) return;
						turn.catchUp(events);
						const messages = turn.responseMessages;
						const error = runFailureCard(event.name, failure);
						if (error && messages.length > 0) {
							messages[messages.length - 1].error = error;
						}
						await completeAppChatTurn(
							messages,
							{ appId, eventId: event.id, sessionId: sessionIdParameter },
							{ events },
						);
						await persistWorkflowReactions(events, {
							appId,
							sessionId: sessionIdParameter,
							triggerMessageId: responseMessage.trigger_message_id,
						});
					},
					saveIntervalEvents: 10,
				});
				executionEngine.subscribeToEventStream(
					streamId,
					subscriberId,
					(events) => {
						handleNavigationEvents(events);
						refreshMainChatWidget(
							turn.responseMessage,
							readMainWidget(sessionIdParameter),
						);
						const result = turn.fold(events);
						rememberMainWidget(result.responseMessages);

						if (result.interactions?.length) {
							addSessionInteractions(sessionIdParameter, result.interactions);
						}

						if (result.shouldUpdate) {
							chatRef.current?.pushCurrentMessageUpdate(
								result.responseMessages.map((message) => ({ ...message })),
							);
							chatRef.current?.scrollToBottom();
						}
					},
					async (events, failure) => {
						try {
							await handleStreamCompletion({
								responseMessage,
								chatRef,
								executionEngine,
								streamId,
								subscriberId,
								processedCompletedStreams,
								events,
								appId,
								eventId: event.id,
								sessionId: sessionIdParameter,
								live: turn.live(),
								error: runFailureCard(event.name, failure),
								onInteractions: (interactions) =>
									addSessionInteractions(sessionIdParameter, interactions),
							});
						} finally {
							activeSubscriptions.current = activeSubscriptions.current.filter(
								(id) => id !== subscriberId,
							);
						}
					},
				);

				await executionPromise;
			} catch (error) {
				if (interaction && !userMessageCommitted) {
					throw Object.assign(
						error instanceof Error
							? error
							: new Error(extractErrorMessage(error)),
						{ isUnacceptedChatInteraction: true },
					);
				}
				throw error;
			} finally {
				pendingSendSessions.current.delete(streamId);
				// The user may have switched session while this run streamed; the composer
				// reflects the session on screen, not the one this send belonged to.
				const current = sessionIdRef.current;
				setIsStreamActive(
					pendingSendSessions.current.has(current) ||
						executionEngine.isStreamActive(current),
				);
			}
		},
		[
			backend,
			executionEngine,
			sessionIdParameter,
			appId,
			event,
			config?.attach_widget_snapshots,
			localState,
			globalState,
			handleNavigationEvents,
			readMainWidget,
			rememberMainWidget,
			pathname,
		],
	);

	// Listen for OAuth retry events
	useEffect(() => {
		const handleOAuthRetry = (e: Event) => {
			const retryEvent = e as CustomEvent<{
				appId: string;
				requestId?: string;
				boardId?: string;
				nodeId?: string;
				skipConsentCheck?: boolean;
			}>;

			const { appId: eventAppId, boardId } = retryEvent.detail;
			console.log("[Chat] OAuth retry event received:", {
				eventAppId,
				boardId,
				appId,
				eventBoardId: event.board_id,
			});

			// Only handle if this is for our app (boardId may be undefined from execution engine)
			if (eventAppId !== appId) {
				console.log("[Chat] OAuth retry event not for this app, ignoring");
				return;
			}

			// If boardId is provided, also check it matches
			if (boardId && boardId !== event.board_id) {
				console.log("[Chat] OAuth retry event not for this board, ignoring");
				return;
			}

			const pending = pendingMessageRef.current;
			if (!pending) {
				console.log("[Chat] No pending message to retry");
				return;
			}
			if (retryEvent.detail.requestId !== pending.requestId) return;
			if (
				pending.appId !== appId ||
				pending.eventId !== event.id ||
				pending.sessionId !== sessionIdParameter
			) {
				pendingMessageRef.current = null;
				return;
			}

			console.log("[Chat] Re-sending pending message with skipConsentCheck");

			// Re-execute - consent was just granted so skip the check
			executeChatMessage(
				pending.content,
				pending.filesAttached,
				pending.activeTools,
				pending.audioFile,
				pending.interaction,
				true, // skipConsentCheck
			).catch((err) => {
				console.error("Failed to retry chat message after OAuth:", err);
				captureTelemetryError(err, {
					culprit: "chat.oauth_retry_send",
				});
				toast.error(`Failed to send message: ${extractErrorMessage(err)}`);
			});
		};

		window.addEventListener("flow:oauth-retry", handleOAuthRetry);
		const handleOAuthCancelled = (event: Event) => {
			const detail = (
				event as CustomEvent<{
					requestId?: string;
					appId?: string;
					eventId?: string;
					sessionId?: string;
				}>
			).detail;
			const pending = pendingMessageRef.current;
			if (
				pending &&
				detail?.requestId === pending.requestId &&
				detail.appId === pending.appId &&
				detail.eventId === pending.eventId &&
				detail.sessionId === pending.sessionId
			) {
				pendingMessageRef.current = null;
			}
		};
		window.addEventListener("flow:oauth-cancelled", handleOAuthCancelled);
		return () => {
			window.removeEventListener("flow:oauth-retry", handleOAuthRetry);
			window.removeEventListener("flow:oauth-cancelled", handleOAuthCancelled);
		};
	}, [appId, event.id, event.board_id, sessionIdParameter, executeChatMessage]);

	const handleSendMessage: ISendMessageFunction = useCallback(
		async (
			content,
			filesAttached,
			activeTools?: string[],
			audioFile?: File,
			interaction?: IChatMessageInteraction,
		) => {
			if (!sessionIdParameter || sessionIdParameter === "") {
				toast.error("Session ID is not set. Please start a new chat.");
				return;
			}

			// Show loading state if sending from welcome screen
			const hasFiles = (filesAttached && filesAttached.length > 0) || audioFile;
			if (
				hasFiles &&
				(!messagesRef.current || messagesRef.current.length === 0)
			) {
				setIsSendingFromWelcome(true);
			}

			try {
				await executeChatMessage(
					content,
					filesAttached,
					activeTools,
					audioFile,
					interaction,
				);
			} catch (error) {
				// Active stream errors and OAuth errors are handled separately
				if (
					(error as { isActiveStreamError?: unknown } | null)
						?.isActiveStreamError
				) {
					// Already shown a toast in executeChatMessage guard
				} else if (
					!(error as { isOAuthError?: unknown } | null)?.isOAuthError
				) {
					console.error("Error sending message:", error);
					captureTelemetryError(error, {
						culprit: "chat.send_message",
					});
					toast.error(`Failed to send message: ${extractErrorMessage(error)}`);
				}
				if (
					interaction &&
					((error as { isActiveStreamError?: unknown } | null)
						?.isActiveStreamError ||
						(error as { isUnacceptedChatInteraction?: unknown } | null)
							?.isUnacceptedChatInteraction)
				) {
					throw error;
				}
			} finally {
				setIsSendingFromWelcome(false);
			}
		},
		[sessionIdParameter, executeChatMessage],
	);

	const handleMessageReaction = useCallback(
		async (
			message: IMessage,
			emoji: string,
			operation: "add" | "remove",
			replacesEmoji?: string,
		) => {
			await handleSendMessage(emoji, undefined, undefined, undefined, {
				type: "reaction",
				target: createMessageReference(message),
				emoji,
				operation,
				...(operation === "add" && replacesEmoji ? { replacesEmoji } : {}),
			});
		},
		[handleSendMessage],
	);

	const onMessageUpdate = useCallback(
		async (messageId: string, updates: Partial<IMessage>) => {
			const stored = await chatDb.messages.get(messageId);
			const existingMessage = stored
				? decodeMessageRow(stored)
				: messagesRef.current.find((message) => message.id === messageId);

			if (!existingMessage) {
				throw new Error(`Message ${messageId} not found`);
			}

			const merged: IMessage = {
				...existingMessage,
				...updates,
			};
			const { ratingSettings: _cleared, ...withoutSettings } = merged;
			const nextMessage: IMessage =
				Object.prototype.hasOwnProperty.call(updates, "ratingSettings") &&
				updates.ratingSettings === undefined
					? withoutSettings
					: merged;

			await putChatMessage(nextMessage);

			if (
				updates.rating !== undefined ||
				updates.ratingSettings !== undefined
			) {
				const rating = nextMessage.rating;
				if (rating === undefined || rating === 0) return;

				const feedbackRating = rating > 0 ? 5 : 1;
				await backend.eventState.upsertEventFeedback(
					appId,
					event.id,
					messageId,
					{
						rating: feedbackRating,
						comment: nextMessage.ratingSettings?.comment ?? "",
						localState: {
							pageContext: getCurrentPageContext(pathname, { mode: "path" }),
							eventContext: {
								id: event.id,
								name: event.name,
								route:
									typeof event.route === "string" ? event.route : undefined,
								defaultPageId:
									typeof event.default_page_id === "string"
										? event.default_page_id
										: undefined,
								eventType: event.event_type,
							},
						},
						history: nextMessage.ratingSettings?.includeChatHistory
							? (
									await chatDb.messages
										.where("sessionId")
										.equals(nextMessage.sessionId)
										.toArray()
								).map((row) => decodeMessageRow(row).inner)
							: undefined,
					},
				);
			}
		},
		[
			appId,
			backend.eventState,
			event.default_page_id,
			event.event_type,
			event.id,
			event.name,
			event.route,
			pathname,
		],
	);

	const showWelcome = useMemo(
		() => messagesLoaded && messages.length === 0,
		[messagesLoaded, messages],
	);

	// Show verification dialog when prefilled message is present and no history yet
	useEffect(() => {
		if (prefilledMessage && showWelcome && !prefilledConsumed.current) {
			setShowPrefilledConfirm(true);
		}
	}, [prefilledMessage, showWelcome]);

	const handlePrefilledConfirm = useCallback(() => {
		if (!prefilledMessage) return;
		prefilledConsumed.current = true;
		setShowPrefilledConfirm(false);
		setQueryParams("message", undefined);
		handleSendMessage(prefilledMessage);
	}, [prefilledMessage, handleSendMessage, setQueryParams]);

	const handlePrefilledCancel = useCallback(() => {
		prefilledConsumed.current = true;
		setShowPrefilledConfirm(false);
		setQueryParams("message", undefined);
	}, [setQueryParams]);

	// Runs a widget action triggered from an embedded chat widget. Like the app
	// view, this is a plain BOARD run starting at the bound node (payload.id) —
	// never the chat event, whose node would reject the widget payload. The
	// run executes against the widget's own app/board (for widgets of this
	// chat that equals appId/event.board_id; a widget `origin` may point
	// elsewhere). Its a2ui events (forwarded via onA2UIEvents) update the
	// widget in place; toasts surface via the executeBoard transport; and any
	// chat pushes from the triggered workflow become a new assistant message.
	// biome-ignore lint/correctness/useExhaustiveDependencies: `t` only words the failure toast; a language switch must not hand every embedded widget a new action handler.
	const runWidgetAction = useCallback<RunWidgetAction>(
		async (actionAppId, actionBoardId, runPayload, onA2UIEvents) => {
			const responseMessage = createResponseMessage(
				sessionIdParameter,
				appId,
				event.name,
				readMainWidget(sessionIdParameter),
			);
			const initialMainWidget = responseMessage.main_widget;
			const turn = createChatTurn({
				responseMessage,
				tmpLocalState: null,
				tmpGlobalState: null,
				appId,
				eventId: event.id,
				sessionId: sessionIdParameter,
			});

			const result = await backend.boardState.executeBoard(
				actionAppId,
				actionBoardId,
				runPayload,
				false,
				undefined,
				(events) => {
					if (events.length) {
						console.debug(
							"[ChatWidget] action run events:",
							events.map((e) => e.event_type),
						);
					}
					handleNavigationEvents(events);
					if (!onA2UIEvents) {
						for (const item of events) {
							if (item.event_type === "a2ui") {
								getFrontendStateStore(actionAppId).handleMessage(item.payload);
							}
						}
					}
					onA2UIEvents?.(events);

					refreshMainChatWidget(
						turn.responseMessage,
						readMainWidget(sessionIdParameter),
					);
					const processed = turn.fold(events);
					rememberMainWidget(processed.responseMessages);

					if (processed.interactions?.length) {
						addSessionInteractions(sessionIdParameter, processed.interactions);
					}

					// The assistant bubble appears only once the action actually
					// streams chat content — a pure widget update shows nothing.
					if (processed.shouldUpdate) {
						chatRef.current?.pushCurrentMessageUpdate(
							processed.responseMessages.map((message) => ({ ...message })),
						);
						chatRef.current?.scrollToBottom();
					}
				},
			);

			// LogLevel::Error = 3 — a node failure inside the run resolves normally
			// (errors are run logs, not exceptions), so surface it explicitly.
			if ((result?.log_level ?? 0) >= 3) {
				toast.error(
					t(
						"widgetActionFailedCheckTheFlowsRunsPanelForTheFailingNode",
						"Widget action failed — check the flow's Runs panel for the failing node.",
					),
				);
			}

			const messagesWithContent = turn.responseMessages.filter((message) => {
				const textContent =
					typeof message.inner.content === "string"
						? message.inner.content.trim()
						: (message.inner.content?.length ?? 0);
				return Boolean(
					textContent ||
						message.files?.length ||
						message.widgets?.length ||
						message.main_widget !== initialMainWidget ||
						message.plan_steps?.length,
				);
			});

			// Main widget changes are saved with the turn so they survive reopening the chat.
			for (const message of messagesWithContent) {
				await putChatMessage(message);
			}
			chatRef.current?.clearCurrentMessageUpdate();
			if (messagesWithContent.length > 0) {
				chatRef.current?.scrollToBottom();
			}

			return result;
		},
		[
			backend,
			appId,
			event,
			sessionIdParameter,
			handleNavigationEvents,
			readMainWidget,
			rememberMainWidget,
		],
	);

	return (
		<>
			<ChatAppearance
				appId={appId}
				eventId={event.id}
				config={config}
				fallback={<ConversationLoading />}
			>
				{!messagesLoaded ? (
					<ConversationLoading />
				) : showWelcome ? (
					<ChatWelcome
						onSendMessage={handleSendMessage}
						event={event}
						config={config}
						isSending={isSendingFromWelcome}
						appId={appId}
						thinking={thinking.picker}
						model={model.picker}
					/>
				) : (
					<ChatWidgetExecutionProvider runWidgetAction={runWidgetAction}>
						<Chat
							ref={chatRef}
							sessionId={sessionIdParameter}
							messages={messages}
							onSendMessage={handleSendMessage}
							onMessageUpdate={onMessageUpdate}
							onMessageReaction={handleMessageReaction}
							config={config}
							isStreamActive={isStreamActive}
							activeInteractions={activeInteractions}
							onRespondToInteraction={handleRespondToInteraction}
							appId={appId}
							boardId={event.board_id}
							eventId={event.id}
							showAiDisclosure
							thinking={thinking.picker}
							model={model.picker}
						/>
					</ChatWidgetExecutionProvider>
				)}
			</ChatAppearance>
			<AlertDialog
				open={showPrefilledConfirm}
				onOpenChange={(open) => {
					if (!open) handlePrefilledCancel();
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("sendPrefilledMessage", "Send prefilled message?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"thisChatWasOpenedWithAPrefilledMessagePleaseReviewItBeforeSending",
								"This chat was opened with a prefilled message. Please review it before sending:",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<div className="rounded-md bg-muted p-3 text-sm max-h-48 overflow-y-auto wrap-break-word whitespace-pre-wrap">
						{prefilledMessage}
					</div>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction onClick={handlePrefilledConfirm}>
							{t("send", "Send")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
});

export function ChatInterface({
	appId,
	event,
	config = {},
	toolbarRef,
	sidebarRef,
	onNavigate,
}: Readonly<IUseInterfaceProps>) {
	return (
		<ChatInterfaceMemoized
			appId={appId}
			event={event}
			config={config}
			toolbarRef={toolbarRef}
			sidebarRef={sidebarRef}
			onNavigate={onNavigate}
		/>
	);
}
