"use client";

import { createId } from "@paralleldrive/cuid2";
import * as Sentry from "@sentry/nextjs";
import { useLiveQuery } from "dexie-react-hooks";
import {
	ChevronDownIcon,
	HistoryIcon,
	HomeIcon,
	Loader2Icon,
	SquarePenIcon,
} from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
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
import { getCurrentPageContext } from "../../lib/page-context";
import type { IInteractionRequest } from "../../lib/schema/interaction";
import { useSetQueryParams } from "../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { captureWidgetSnapshots } from "../../lib/widget-snapshot";
import { useBackend } from "../../state/backend-state";
import { useExecutionEngine } from "../../state/execution-engine-context";
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
import { fileToAttachment } from "./chat-default/attachment";
import { ChatAppearance } from "./chat-default/appearance";
import { Chat, type IChatRef } from "./chat-default/chat";
import {
	type IAttachment,
	type IMessage,
	chatDb,
} from "./chat-default/chat-db";
import {
	ChatWidgetExecutionProvider,
	type RunWidgetAction,
} from "./chat-default/chat-widget-execution";
import type { ISendMessageFunction } from "./chat-default/chatbox";
import { processChatEvents } from "./chat-default/event-processor";
import { ChatHistory } from "./chat-default/history";
import { submitInteractionResponse } from "./chat-default/respond-interaction";
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

async function prepareAttachments(
	filesAttached: File[] | undefined,
	audioFile: File | undefined,
	backend: any,
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
	return { imageAttachments, otherAttachments };
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

function createHistoryMessage(
	content: string,
	imageAttachments: IAttachment[],
) {
	const historyMessage: IHistoryMessage = {
		content: [
			{
				type: IContentType.Text,
				text: content,
			},
		],
		role: IRole.User,
	};

	for (const image of imageAttachments) {
		const url = typeof image === "string" ? image : image.url;
		(historyMessage.content as IContent[]).push({
			type: IContentType.IImageURL,
			image_url: {
				url: url,
			},
		});
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
	};
}

function createPayload(
	userMessage: IMessage,
	lastMessages: IMessage[],
	historyMessage: IHistoryMessage,
	localState: any,
	globalState: any,
	activeTools: string[],
	otherAttachments: IAttachment[],
) {
	return {
		chat_id: userMessage.sessionId,
		messages: [
			...lastMessages.map((msg) => ({
				role: msg.inner.role,
				content:
					typeof msg.inner.content === "string"
						? msg.inner.content
						: msg.inner.content?.map((c) => ({
								type: c.type,
								text: c.text,
								image_url: c.image_url,
							})),
			})),
			historyMessage,
		],
		local_session: localState?.localState ?? {},
		global_session: globalState?.globalState ?? {},
		actions: [],
		tools: activeTools ?? [],
		attachments: otherAttachments,
	};
}

function createResponseMessage(
	sessionId: string,
	appId: string,
	eventName: string,
): IMessage {
	return {
		id: createId(),
		sessionId: sessionId,
		appId,
		files: [],
		inner: {
			role: IRole.Assistant,
			content: "",
		},
		explicit_name: eventName,
		timestamp: Date.now(),
		tools: [],
		actions: [],
	};
}

function cloneResponseMessageForCompletion(
	responseMessage: IMessage,
): IMessage {
	const clonedMessage =
		typeof structuredClone === "function"
			? structuredClone(responseMessage)
			: (JSON.parse(JSON.stringify(responseMessage)) as IMessage);

	clonedMessage.files = [];
	clonedMessage.inner = {
		...clonedMessage.inner,
		content: "",
	};
	clonedMessage.plan_steps = undefined;
	clonedMessage.current_step_id = undefined;
	clonedMessage.usage_stats = undefined;
	clonedMessage.widgets = undefined;

	return clonedMessage;
}

async function handleStreamCompletion(
	responseMessage: IMessage,
	chatRef: RefObject<IChatRef | null>,
	executionEngine: any,
	streamId: string,
	subscriberId: string,
	processedCompletedStreams: MutableRefObject<Set<string>>,
	events: any[],
	intermediateResponse: Response,
	attachments: Map<string, IAttachment>,
	appId: string,
	eventId: string,
	sessionId: string,
	initialLocalState?: any,
	initialGlobalState?: any,
	onInteractions?: (interactions: IInteractionRequest[]) => void,
) {
	if (processedCompletedStreams.current.has(streamId)) {
		return;
	}

	processedCompletedStreams.current.add(streamId);

	try {
		const result = processChatEvents(events, {
			intermediateResponse: Response.default(),
			responseMessage: cloneResponseMessageForCompletion(responseMessage),
			attachments: new Map(),
			tmpLocalState: initialLocalState ?? null,
			tmpGlobalState: initialGlobalState ?? null,
			done: false,
			appId,
			eventId,
			sessionId,
		});

		if (result.interactions?.length && onInteractions) {
			onInteractions(result.interactions);
		}

		if (result.tmpLocalState) {
			await chatDb.localStage.put(result.tmpLocalState);
		}

		if (result.tmpGlobalState) {
			await chatDb.globalState.put(result.tmpGlobalState);
		}

		// Write to Dexie FIRST to ensure the message is persisted before clearing streaming state
		// This prevents the message from briefly disappearing
		await chatDb.messages.put(result.responseMessage);

		// Clear the streaming message AFTER writing to Dexie
		// The useLiveQuery will pick up the new message from DB
		chatRef.current?.clearCurrentMessageUpdate();

		chatRef.current?.scrollToBottom();

		executionEngine.unsubscribeFromEventStream(streamId, subscriberId);
	} catch (error) {
		processedCompletedStreams.current.delete(streamId);
		throw error;
	}
}

/**
 * Creates an incremental save function for chat message streaming.
 * This function saves the current message state to Dexie periodically.
 * The message object is expected to be updated by the subscriber before this is called.
 *
 * Note: The final completion is handled by handleStreamCompletion, so this function
 * only saves intermediate state. The isFinal flag is used only for logging.
 *
 * @param responseMessage - The message object (modified by subscriber)
 * @param localStateRef - Reference to current local state (updated by subscriber)
 * @param globalStateRef - Reference to current global state (updated by subscriber)
 */
function createChatIncrementalSaver(
	responseMessage: IMessage,
	localStateRef: { current: any },
	globalStateRef: { current: any },
): (events: any[], isFinal: boolean) => Promise<void> {
	return async (_events: any[], isFinal: boolean) => {
		// Save the message in its current state (already updated by subscriber)
		await chatDb.messages.put(responseMessage);

		// Save local/global state if present
		if (localStateRef.current) {
			await chatDb.localStage.put(localStateRef.current);
		}
		if (globalStateRef.current) {
			await chatDb.globalState.put(globalStateRef.current);
		}

		// Note: We don't clear streaming state here - that's handled by handleStreamCompletion
		// which also does proper cleanup (unsubscribe, etc.)
		if (isFinal) {
			console.log("[Chat] Incremental save completed (final)");
		}
	};
}

export const ChatInterfaceMemoized = memo(function ChatInterface({
	appId,
	event,
	config = {},
	toolbarRef,
	sidebarRef,
}: Readonly<IUseInterfaceProps>) {
	const router = useRouter();
	const backend = useBackend();
	const executionEngine = useExecutionEngine();
	const searchParams = useSearchParams();
	const pathname = usePathname();
	const sessionIdParameter = searchParams.get("sessionId") ?? "";
	const prefilledMessage = searchParams.get("message");
	const setQueryParams = useSetQueryParams();
	const chatRef = useRef<IChatRef>(null);
	const activeSubscriptions = useRef<string[]>([]);
	const processedCompletedStreams = useRef<Set<string>>(new Set());
	const reconnectSubscribed = useRef<Set<string>>(new Set());
	const pendingSendSessions = useRef<Set<string>>(new Set());
	const [isSendingFromWelcome, setIsSendingFromWelcome] = useState(false);
	const [isStreamActive, setIsStreamActive] = useState(false);
	const [showPrefilledConfirm, setShowPrefilledConfirm] = useState(false);
	const prefilledConsumed = useRef(false);
	const lastNavigateToRef = useRef<string | null>(null);
	const [activeInteractions, setActiveInteractions] = useState<
		IInteractionRequest[]
	>([]);
	const activeInteractionsRef =
		useRef<IInteractionRequest[]>(activeInteractions);
	const interactionsBySession = useRef<Map<string, IInteractionRequest[]>>(
		new Map(),
	);
	useEffect(() => {
		activeInteractionsRef.current = activeInteractions;
	}, [activeInteractions]);

	// Keep interaction cache in sync with current session
	useEffect(() => {
		if (sessionIdParameter) {
			interactionsBySession.current.set(sessionIdParameter, activeInteractions);
		}
	}, [sessionIdParameter, activeInteractions]);

	const addInteractions = useCallback((interactions: IInteractionRequest[]) => {
		setActiveInteractions((prev) => {
			const existingMap = new Map(prev.map((i) => [i.id, i]));
			let changed = false;
			for (const interaction of interactions) {
				const existing = existingMap.get(interaction.id);
				if (!existing) {
					existingMap.set(interaction.id, interaction);
					changed = true;
				} else if (
					existing.status === "pending" &&
					interaction.status !== "pending"
				) {
					existingMap.set(interaction.id, interaction);
					changed = true;
				}
			}
			return changed ? Array.from(existingMap.values()) : prev;
		});
	}, []);

	const handleRespondToInteraction = useCallback(
		async (interactionId: string, value: any) => {
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
				await submitInteractionResponse(interaction, value, backend.profile);

				setActiveInteractions((prev) =>
					prev.map((i) =>
						i.id === interactionId
							? { ...i, status: "responded" as const, response_value: value }
							: i,
					),
				);
			} catch (err) {
				console.error("[Chat] Failed to respond to interaction:", err);
				Sentry.captureException(err, {
					tags: { component: "chat", action: "respond_to_interaction" },
					extra: { interactionId, appId },
				});
				toast.error(`Failed to submit response: ${extractErrorMessage(err)}`);
			}
		},
		[backend.profile],
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
			const navUrl = buildUseNavigationUrl(route, queryParams);
			if (replace) {
				router.replace(navUrl);
			} else {
				router.push(navUrl);
			}
		},
		[buildUseNavigationUrl, router],
	);

	const handleNavigationEvents = useCallback(
		(events: any[]) => {
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

	// Store pending message data for OAuth retry
	const pendingMessageRef = useRef<{
		content: string;
		filesAttached?: File[];
		activeTools?: string[];
		audioFile?: File;
	} | null>(null);

	useEffect(() => {
		if (!sessionIdParameter || sessionIdParameter === "") {
			const newSessionId = createId();
			setQueryParams("sessionId", newSessionId);
		}
	}, [sessionIdParameter, setQueryParams]);

	useEffect(() => {
		if (!sessionIdParameter) {
			setIsStreamActive(false);
			return;
		}

		const update = () => {
			setIsStreamActive(
				pendingSendSessions.current.has(sessionIdParameter) ||
					executionEngine.isStreamActive(sessionIdParameter),
			);
		};

		update();
		return executionEngine.subscribeToGlobalUpdates(update);
	}, [executionEngine, sessionIdParameter]);

	// Cleanup active subscriptions and restore cached interactions on session change
	useEffect(() => {
		const cached = interactionsBySession.current.get(sessionIdParameter) ?? [];
		setActiveInteractions(cached);
		processedCompletedStreams.current.clear();
		return () => {
			interactionsBySession.current.set(
				sessionIdParameter,
				activeInteractionsRef.current,
			);
			activeSubscriptions.current.forEach((subId) => {
				executionEngine.unsubscribeFromEventStream(sessionIdParameter, subId);
			});
			activeSubscriptions.current = [];
		};
	}, [sessionIdParameter, executionEngine]);

	const messagesQuery = useLiveQuery(async () => {
		if (!sessionIdParameter) return [];
		const rawMessages = await chatDb.messages
			.where("sessionId")
			.equals(sessionIdParameter)
			.sortBy("timestamp");
		return deduplicateConsecutiveMessages(rawMessages);
	}, [sessionIdParameter]);

	const messagesLoaded = messagesQuery !== undefined;
	const messages = messagesQuery ?? [];
	const hasMessages = messages.length > 0;

	const messagesRef = useRef<IMessage[]>(messages);
	useEffect(() => {
		messagesRef.current = messages;
	}, [messages]);

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
						Chat History
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
						New Chat
					</div>
				</HoverCardContent>
			</HoverCard>,
		];
	}, [handleSidebarToggle, handleNewChat]);

	const navElements = useMemo(() => {
		const normalizeRoute = (value: string): string => {
			const trimmed = value.trim();
			if (!trimmed) return "";
			return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
		};

		const configuredRoutes = (() => {
			const rawArray = (config as any)?.navigate_to_routes;
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
							Navigate
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
		if (!messagesLoaded) return;

		const streamId = sessionIdParameter;

		// Check if there's a stream (active or completed) for this session
		if (!executionEngine.hasStream(streamId)) return;

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

		// Reuse the last assistant message from Dexie if it exists (e.g. from incremental save)
		// to avoid creating a duplicate when reconnecting to an active stream
		const currentMessages = messagesRef.current;
		const lastMsg = currentMessages[currentMessages.length - 1];
		const responseMessage: IMessage =
			lastMsg?.inner.role === IRole.Assistant
				? { ...lastMsg }
				: {
						id: createId(),
						sessionId: sessionIdParameter,
						appId,
						files: [],
						inner: {
							role: IRole.Assistant,
							content: "",
						},
						explicit_name: event.name,
						timestamp: Date.now(),
						tools: [],
						actions: [],
					};

		let intermediateResponse = Response.default();
		const attachments: Map<string, IAttachment> = new Map();

		// If stream is already complete, save to IndexedDB directly
		// (chatRef may not be mounted yet since Chat only renders when messages exist)
		if (executionEngine.isStreamComplete(streamId)) {
			const accumulatedEvents = executionEngine.getAccumulatedEvents(streamId);
			if (accumulatedEvents.length > 0) {
				handleNavigationEvents(accumulatedEvents);
				void handleStreamCompletion(
					responseMessage,
					chatRef,
					executionEngine,
					streamId,
					subscriberId,
					processedCompletedStreams,
					accumulatedEvents,
					intermediateResponse,
					attachments,
					appId,
					event.id,
					sessionIdParameter,
					null,
					null,
					addInteractions,
				);
			}
			return;
		}

		// For active streams, wait for Chat component to be mounted (messages.length > 0)
		// before subscribing, since we need chatRef to push updates
		if (!hasMessages) return;

		// Mark this subscriber as active before subscribing
		reconnectSubscribed.current.add(subscriberId);

		// For active streams, subscribe to receive events
		executionEngine.subscribeToEventStream(
			streamId,
			subscriberId,
			(events) => {
				handleNavigationEvents(events);

				const result = processChatEvents(events, {
					intermediateResponse,
					responseMessage,
					attachments,
					tmpLocalState: null,
					tmpGlobalState: null,
					done: false,
					appId,
					eventId: event.id,
					sessionId: sessionIdParameter,
				});

				intermediateResponse = result.intermediateResponse;

				if (result.interactions?.length) {
					addInteractions(result.interactions);
				}

				if (result.shouldUpdate) {
					chatRef.current?.pushCurrentMessageUpdate({
						...responseMessage,
					});
					chatRef.current?.scrollToBottom();
				}
			},
			async (events) => {
				handleNavigationEvents(events);
				await handleStreamCompletion(
					responseMessage,
					chatRef,
					executionEngine,
					streamId,
					subscriberId,
					processedCompletedStreams,
					events,
					intermediateResponse,
					attachments,
					appId,
					event.id,
					sessionIdParameter,
					null,
					null,
					addInteractions,
				);
				// Clean up the reconnect subscriber tracking after completion
				reconnectSubscribed.current.delete(subscriberId);
			},
		);

		return () => {
			executionEngine.unsubscribeFromEventStream(streamId, subscriberId);
			reconnectSubscribed.current.delete(subscriberId);
		};
	}, [
		sessionIdParameter,
		appId,
		event.id,
		event.name,
		executionEngine,
		handleNavigationEvents,
		messagesLoaded,
		hasMessages,
		addInteractions,
	]);

	// Internal function to execute the chat (called after OAuth is confirmed)
	const executeChatMessage = useCallback(
		async (
			content: string,
			filesAttached?: File[],
			activeTools?: string[],
			audioFile?: File,
			skipConsentCheck?: boolean,
		) => {
			const streamId = sessionIdParameter;
			if (
				pendingSendSessions.current.has(streamId) ||
				executionEngine.isStreamActive(streamId)
			) {
				toast.error("Please wait for the current response to complete.");
				return;
			}

			pendingSendSessions.current.add(streamId);
			setIsStreamActive(true);

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
						// Store pending message for retry
						pendingMessageRef.current = {
							content,
							filesAttached,
							activeTools,
							audioFile,
						};
						// Emit OAuth required event
						window.dispatchEvent(
							new CustomEvent("flow:oauth-required", {
								detail: {
									missingProviders: oauthResult.missingProviders,
									appId,
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

				const { imageAttachments, otherAttachments } = await prepareAttachments(
					filesAttached,
					audioFile,
					backend,
					isOffline,
				);

				const historyMessage = createHistoryMessage(content, imageAttachments);

				const userMessage = createUserMessage(
					sessionIdParameter,
					appId,
					otherAttachments,
					historyMessage,
					activeTools ?? [],
				);

				await updateSession(sessionIdParameter, appId, content);
				await chatDb.messages.add(userMessage);

				const lastMessages =
					messagesRef.current?.slice(-history_elements) ?? [];

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
						if (latestWidgets?.length) {
							const snapshots = await captureWidgetSnapshots(
								latestWidgets.map((widget) => widget.instance_id),
							);
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
				);

				const responseMessage = createResponseMessage(
					sessionIdParameter,
					appId,
					event.name,
				);

				chatRef.current?.pushCurrentMessageUpdate({ ...responseMessage });
				chatRef.current?.scrollToBottom();

				let intermediateResponse = Response.default();
				let tmpLocalState = localState;
				let tmpGlobalState = globalState;
				let done = false;
				const attachments: Map<string, IAttachment> = new Map();

				// Refs for incremental save to access current state
				const localStateRef = { current: tmpLocalState };
				const globalStateRef = { current: tmpGlobalState };

				const subscriberId = `chat-${responseMessage.id}`;
				activeSubscriptions.current.push(subscriberId);

				// Clear stale completion tracking so this stream's completion is processed
				processedCompletedStreams.current.delete(streamId);
				reconnectSubscribed.current.delete(
					`chat-reconnect-${sessionIdParameter}`,
				);

				// Create incremental save function for robust message persistence
				// This saves the message every N events to prevent data loss
				const incrementalSave = createChatIncrementalSaver(
					responseMessage,
					localStateRef,
					globalStateRef,
				);

				// Start execution first to reset the stream state
				const executionPromise = executionEngine.executeEvent(streamId, {
					appId,
					eventId: event.id,
					payload: {
						id: event.node_id,
						payload: payload,
					},
					streamState: false,
					onExecutionStart: (execution_id: string) => {},
					path: `${pathname}?id=${appId}&eventId=${event.id}&sessionId=${sessionIdParameter}`,
					title: event.name || "Chat",
					interfaceType: "chat",
					skipConsentCheck,
					// Save to Dexie every 10 events and on completion for robustness
					onIncrementalSave: incrementalSave,
					saveIntervalEvents: 10,
				});
				executionEngine.subscribeToEventStream(
					streamId,
					subscriberId,
					(events) => {
						handleNavigationEvents(events);

						const result = processChatEvents(events, {
							intermediateResponse,
							responseMessage,
							attachments,
							tmpLocalState,
							tmpGlobalState,
							done,
							appId,
							eventId: event.id,
							sessionId: sessionIdParameter,
						});

						intermediateResponse = result.intermediateResponse;
						tmpLocalState = result.tmpLocalState;
						tmpGlobalState = result.tmpGlobalState;
						done = result.done;

						// Update refs for incremental save to access
						localStateRef.current = result.tmpLocalState;
						globalStateRef.current = result.tmpGlobalState;

						// Update responseMessage in place for incremental save
						Object.assign(responseMessage, result.responseMessage);

						if (result.interactions?.length) {
							addInteractions(result.interactions);
						}

						if (result.shouldUpdate) {
							chatRef.current?.pushCurrentMessageUpdate({
								...result.responseMessage,
							});
							chatRef.current?.scrollToBottom();
						}
					},
					async (events) => {
						handleNavigationEvents(events);

						try {
							await handleStreamCompletion(
								responseMessage,
								chatRef,
								executionEngine,
								streamId,
								subscriberId,
								processedCompletedStreams,
								events,
								intermediateResponse,
								attachments,
								appId,
								event.id,
								sessionIdParameter,
								tmpLocalState,
								tmpGlobalState,
								addInteractions,
							);
						} finally {
							activeSubscriptions.current = activeSubscriptions.current.filter(
								(id) => id !== subscriberId,
							);
						}
					},
				);

				await executionPromise;
			} finally {
				pendingSendSessions.current.delete(streamId);
				setIsStreamActive(
					pendingSendSessions.current.has(streamId) ||
						executionEngine.isStreamActive(streamId),
				);
			}
		},
		[
			backend,
			executionEngine,
			sessionIdParameter,
			appId,
			event,
			localState,
			globalState,
			handleNavigationEvents,
			pathname,
			addInteractions,
		],
	);

	// Listen for OAuth retry events
	useEffect(() => {
		const handleOAuthRetry = (e: Event) => {
			const retryEvent = e as CustomEvent<{
				appId: string;
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

			console.log("[Chat] Re-sending pending message with skipConsentCheck");

			// Re-execute - consent was just granted so skip the check
			executeChatMessage(
				pending.content,
				pending.filesAttached,
				pending.activeTools,
				pending.audioFile,
				true, // skipConsentCheck
			).catch((err) => {
				console.error("Failed to retry chat message after OAuth:", err);
				Sentry.captureException(err, {
					tags: { component: "chat", action: "oauth_retry_send" },
					extra: { appId, eventId: event.id, sessionId: sessionIdParameter },
				});
				toast.error(`Failed to send message: ${extractErrorMessage(err)}`);
			});
		};

		window.addEventListener("flow:oauth-retry", handleOAuthRetry);
		return () => {
			window.removeEventListener("flow:oauth-retry", handleOAuthRetry);
		};
	}, [appId, event.board_id, executeChatMessage]);

	const handleSendMessage: ISendMessageFunction = useCallback(
		async (
			content,
			filesAttached,
			activeTools?: string[],
			audioFile?: File,
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
				);
			} catch (error) {
				// Active stream errors and OAuth errors are handled separately
				if ((error as any)?.isActiveStreamError) {
					// Already shown a toast in executeChatMessage guard
				} else if (!(error as any)?.isOAuthError) {
					console.error("Error sending message:", error);
					Sentry.captureException(error, {
						tags: { component: "chat", action: "send_message" },
						extra: { appId, eventId: event.id, sessionId: sessionIdParameter },
					});
					toast.error(`Failed to send message: ${extractErrorMessage(error)}`);
				}
			} finally {
				setIsSendingFromWelcome(false);
			}
		},
		[sessionIdParameter, executeChatMessage],
	);

	const onMessageUpdate = useCallback(
		async (messageId: string, updates: Partial<IMessage>) => {
			const existingMessage =
				(await chatDb.messages.get(messageId)) ??
				messagesRef.current.find((message) => message.id === messageId);

			if (!existingMessage) {
				throw new Error(`Message ${messageId} not found`);
			}

			const nextMessage: IMessage = {
				...existingMessage,
				...updates,
			};

			if (
				Object.prototype.hasOwnProperty.call(updates, "ratingSettings") &&
				updates.ratingSettings === undefined
			) {
				delete nextMessage.ratingSettings;
			}

			await chatDb.messages.put(nextMessage);

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
								).map((m) => m.inner)
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
	const runWidgetAction = useCallback<RunWidgetAction>(
		async (actionAppId, actionBoardId, runPayload, onA2UIEvents) => {
			const responseMessage = createResponseMessage(
				sessionIdParameter,
				appId,
				event.name,
			);
			let intermediateResponse = Response.default();
			const attachments = new Map<string, IAttachment>();

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
					onA2UIEvents?.(events);

					const processed = processChatEvents(events, {
						intermediateResponse,
						responseMessage,
						attachments,
						tmpLocalState: null,
						tmpGlobalState: null,
						done: false,
						appId,
						eventId: event.id,
						sessionId: sessionIdParameter,
					});

					intermediateResponse = processed.intermediateResponse;
					Object.assign(responseMessage, processed.responseMessage);

					if (processed.interactions?.length) {
						addInteractions(processed.interactions);
					}

					// The assistant bubble appears only once the action actually
					// streams chat content — a pure widget update shows nothing.
					if (processed.shouldUpdate) {
						chatRef.current?.pushCurrentMessageUpdate({
							...processed.responseMessage,
						});
						chatRef.current?.scrollToBottom();
					}
				},
			);

			// LogLevel::Error = 3 — a node failure inside the run resolves normally
			// (errors are run logs, not exceptions), so surface it explicitly.
			if ((result?.log_level ?? 0) >= 3) {
				toast.error(
					"Widget action failed — check the flow's Runs panel for the failing node.",
				);
			}

			const textContent =
				typeof responseMessage.inner.content === "string"
					? responseMessage.inner.content.trim()
					: (responseMessage.inner.content?.length ?? 0);
			const hasContent = Boolean(
				textContent ||
					responseMessage.files?.length ||
					responseMessage.widgets?.length ||
					responseMessage.plan_steps?.length,
			);

			// Only persist a new assistant message when the action produced chat
			// content; a pure in-place widget update leaves no residue.
			if (hasContent) {
				await chatDb.messages.put(responseMessage);
			}
			chatRef.current?.clearCurrentMessageUpdate();
			if (hasContent) {
				chatRef.current?.scrollToBottom();
			}

			return result;
		},
		[
			backend,
			appId,
			event,
			sessionIdParameter,
			addInteractions,
			handleNavigationEvents,
		],
	);

	return (
		<>
			<ChatAppearance appId={appId} eventId={event.id} config={config}>
				{!messagesLoaded ? (
					<div className="flex h-full flex-col items-center justify-center gap-3">
						<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
						<p className="text-sm text-muted-foreground">
							Loading conversation...
						</p>
					</div>
				) : showWelcome ? (
					<ChatWelcome
						onSendMessage={handleSendMessage}
						event={event}
						config={config}
						isSending={isSendingFromWelcome}
					/>
				) : (
					<ChatWidgetExecutionProvider runWidgetAction={runWidgetAction}>
						<Chat
							ref={chatRef}
							sessionId={sessionIdParameter}
							messages={messages}
							onSendMessage={handleSendMessage}
							onMessageUpdate={onMessageUpdate}
							config={config}
							isStreamActive={isStreamActive}
							activeInteractions={activeInteractions}
							onRespondToInteraction={handleRespondToInteraction}
							appId={appId}
							boardId={event.board_id}
							eventId={event.id}
							showAiDisclosure
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
						<AlertDialogTitle>Send prefilled message?</AlertDialogTitle>
						<AlertDialogDescription>
							This chat was opened with a prefilled message. Please review it
							before sending:
						</AlertDialogDescription>
					</AlertDialogHeader>
					<div className="rounded-md bg-muted p-3 text-sm max-h-48 overflow-y-auto wrap-break-word whitespace-pre-wrap">
						{prefilledMessage}
					</div>
					<AlertDialogFooter>
						<AlertDialogCancel>Cancel</AlertDialogCancel>
						<AlertDialogAction onClick={handlePrefilledConfirm}>
							Send
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
}: Readonly<IUseInterfaceProps>) {
	return (
		<ChatInterfaceMemoized
			appId={appId}
			event={event}
			config={config}
			toolbarRef={toolbarRef}
			sidebarRef={sidebarRef}
		/>
	);
}
