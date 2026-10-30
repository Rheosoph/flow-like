"use client";

import { i18n as i18next } from "@flow-like/locales";
import {
	forwardRef,
	memo,
	useCallback,
	useEffect,
	useImperativeHandle,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { IEventPayloadChat } from "../../../lib";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import { VoiceMode } from "./VoiceMode";
import { ChatAiDisclosure } from "./ai-disclosure";
import type { IMessage } from "./chat-db";
import { ChatFollowUpProvider } from "./chat-follow-up";
import {
	type IChatMessageInteraction,
	type IChatMessageReference,
	createMessageReference,
} from "./chat-message-interaction";
import { ChatRunControls } from "./chat-run-controls";
import {
	ChatBox,
	type ChatBoxRef,
	type ChatBoxThinking,
	type ISendMessageFunction,
} from "./chatbox";
import { Interaction, InteractionGroup } from "./interaction";
import { MainWidgetPane } from "./main-widget-pane";
import { resolveMainChatWidget } from "./main-widget-state";
import { MessageComponent, type MessageInlineContent } from "./message";
import { MessageReference } from "./message-reference";
import { MessageShell } from "./message-shell";
import {
	type ChatSend,
	type PendingAsk,
	askRuleColor,
	isAwaitingFirstEvent,
	resolvePendingAsks,
} from "./pending-ask";
import { PendingAskStatus } from "./pending-ask-status";
import { useAnswerPlayback } from "./use-answer-playback";
import { isVoiceEnabled, resolveChatVoiceConfig } from "./voice-config";

type ChatItem =
	| { type: "message"; data: IMessage; timestamp: number }
	| { type: "interaction"; data: IInteractionRequest; timestamp: number }
	| {
			type: "interaction-group";
			data: IInteractionRequest[];
			timestamp: number;
	  };

/**
 * Settled rows this close to the end mount at once so the initial
 * scroll-to-bottom lands on real content; everything above is a placeholder
 * until it scrolls near.
 */
const IMMEDIATE_MESSAGE_TAIL = 8;

function getInteractionCreatedAt(interaction: IInteractionRequest): number {
	return (interaction.expires_at - interaction.ttl_seconds) * 1000;
}

function findAnchorMessage(
	sortedMessages: readonly IMessage[],
	createdAt: number,
): IMessage | undefined {
	let anchor: IMessage | undefined;
	for (const message of sortedMessages) {
		if (message.timestamp > createdAt) break;
		anchor = message;
	}
	return anchor;
}

const noopRespond = () => {};

function SettledInteractions({
	interactions,
	onRespond,
}: Readonly<{
	interactions: IInteractionRequest[] | undefined;
	onRespond?: (interactionId: string, value: unknown) => void;
}>) {
	if (!interactions?.length) return null;
	return (
		<div className="mt-2 flex w-full flex-col items-start">
			<InteractionGroup
				interactions={interactions}
				onRespond={onRespond ?? noopRespond}
			/>
		</div>
	);
}

function getMessageTextContent(message: IMessage): string {
	const content = message.inner.content;
	if (typeof content === "string") return content;
	return content
		.filter((c) => c.type === "text" && typeof c.text === "string")
		.map((c) => c.text)
		.join("");
}

function sameStringArray(
	left: readonly string[] | undefined,
	right: readonly string[],
): boolean {
	return (
		left === right ||
		(left !== undefined &&
			left.length === right.length &&
			left.every((value, index) => value === right[index]))
	);
}

/** One turn currently generating, as the composer needs to see it. Structurally satisfied by the
 * global chat's `GlobalChatRun` — kept local so this generic surface stays decoupled from it. */
export interface IChatActiveRun {
	runId: string;
	label: string;
	status: "streaming" | "cancelling";
	steers: readonly {
		id: string;
		content: string;
		status: "pending" | "delivered" | "failed";
		error?: string;
	}[];
}

export interface IChatQueuedMessage {
	id: string;
	content: string;
}

/**
 * Concurrency controls. When supplied, the composer NEVER blocks: a send starts another turn, is
 * queued at capacity, or is steered into a running turn. Omitted (app chats, event chats) keeps
 * the classic one-reply-at-a-time behaviour driven by `isStreamActive`.
 */
export interface IChatConcurrency {
	runs: readonly IChatActiveRun[];
	queued: readonly IChatQueuedMessage[];
	/** At the cap, a send is queued instead of starting a turn — the composer says so. */
	atCapacity: boolean;
	onStop: (runId: string) => void;
	/** Push the composer's text into the running turn. Resolves false when it was not accepted. */
	onSteer: (content: string) => Promise<boolean>;
	onRemoveQueued: (id: string) => void;
}

export interface IChatProps {
	messages: IMessage[];
	onSendMessage: ISendMessageFunction;
	concurrency?: IChatConcurrency;
	onMessageUpdate?: (
		messageId: string,
		updates: Partial<IMessage>,
	) => void | Promise<void>;
	onMessageReaction?: (
		message: IMessage,
		emoji: string,
		operation: "add" | "remove",
	) => Promise<void>;
	config?: Partial<IEventPayloadChat>;
	sessionId?: string;
	isStreamActive?: boolean;
	activeInteractions?: IInteractionRequest[];
	onRespondToInteraction?: (interactionId: string, value: unknown) => void;
	/** Rendered pinned between the message feed and the input — e.g. tool approval prompts. */
	inlinePrompt?: React.ReactNode;
	/**
	 * Live content that belongs in the conversation flow, after the latest message. Global
	 * FlowPilot uses this for app pages and chats so they scroll with the transcript instead of
	 * taking a second, nested viewport above it.
	 */
	inlineContent?: React.ReactNode;
	/**
	 * Live content that belongs inside a specific message, keyed by message id and placed at an
	 * offset into its text like a pushed widget. Each message's list is compared by identity, so
	 * keep the map and its lists stable while the content is unchanged.
	 */
	messageInlineContent?: ReadonlyMap<string, readonly MessageInlineContent[]>;
	/** App id owning the chat — needed to render + trigger embedded widgets. */
	appId?: string;
	/** Board id of the chat event — target for widget action workflows. */
	boardId?: string;
	/** Chat event id — forwarded to embedded widget surfaces. */
	eventId?: string;
	/** The event Chat UI keeps its AI transparency disclosure below the composer. */
	showAiDisclosure?: boolean;
	/**
	 * Called on every composer edit with the current draft. Fires on each keystroke, so pass a
	 * stable reference and keep the handler free of React state updates.
	 */
	onDraftChange?: (content: string) => void;
	/** Thinking level picker in the composer; omitted when the chat does not offer one. */
	thinking?: ChatBoxThinking;
}

export interface IChatRef {
	/** Replace the set of live (still generating) bubbles. Several may stream at once. */
	pushCurrentMessageUpdate: (messages: IMessage | IMessage[]) => void;
	clearCurrentMessageUpdate: () => void;
	pushMessage: (message: IMessage) => void;
	sendMessage: ISendMessageFunction;
	scrollToBottom: () => void;
	clearMessages: () => void;
	focusInput: () => void;
}

const ChatInner = forwardRef<IChatRef, IChatProps>(
	(
		{
			messages,
			onSendMessage,
			concurrency,
			onMessageUpdate,
			onMessageReaction,
			config = {},
			sessionId,
			isStreamActive = false,
			activeInteractions,
			onRespondToInteraction,
			inlinePrompt,
			inlineContent,
			messageInlineContent,
			appId,
			boardId,
			eventId,
			showAiDisclosure = false,
			onDraftChange,
			thinking,
		},
		ref,
	) => {
		const scrollContainerRef = useRef<HTMLDivElement>(null);
		const [shouldAutoScroll, setShouldAutoScroll] = useState(true);
		// Several turns can generate at once (global chat); app chats keep exactly one.
		const [currentMessages, setCurrentMessages] = useState<IMessage[]>([]);
		const [localMessages, setLocalMessages] = useState<IMessage[]>(messages);
		const [hasInitiallyScrolled, setHasInitiallyScrolled] = useState(false);
		const chatBox = useRef<ChatBoxRef>(null);
		const isScrollingProgrammatically = useRef(false);
		const [defaultActiveTools, setDefaultActiveTools] = useState<string[]>();
		const [isSending, setIsSending] = useState(false);
		const [replyTarget, setReplyTarget] =
			useState<IChatMessageReference | null>(null);
		const currentSession = useRef(sessionId);
		currentSession.current = sessionId;
		const interactionDisabled = !concurrency && (isSending || isStreamActive);
		const isSendingRef = useRef(false);
		const [sendingContent, setSendingContent] = useState("");
		const [lastSend, setLastSend] = useState<ChatSend | null>(null);
		const pendingMessagesRef = useRef<IMessage[] | null>(null);
		const rafIdRef = useRef<number | null>(null);
		const [voiceModeOpen, setVoiceModeOpen] = useState(false);

		const voiceConfig = useMemo(() => resolveChatVoiceConfig(config), [config]);
		const voiceEnabled = isVoiceEnabled(voiceConfig);
		const widgetSnapshots = config.attach_widget_snapshots !== false;

		const latestAudioUrl = useMemo(() => {
			let assistant: IMessage | null = null;
			const liveAssistant = [...currentMessages]
				.reverse()
				.find((message) => message.inner.role === "assistant");
			if (liveAssistant) {
				assistant = liveAssistant;
			} else {
				for (let i = localMessages.length - 1; i >= 0; i--) {
					if (localMessages[i]?.inner.role === "assistant") {
						assistant = localMessages[i];
						break;
					}
				}
			}
			if (!assistant) return null;
			for (const file of assistant.files ?? []) {
				if (
					file &&
					typeof file === "object" &&
					file.url &&
					file.type?.includes("audio")
				) {
					return file.url;
				}
			}
			return null;
		}, [currentMessages, localMessages]);

		const playback = useAnswerPlayback(
			voiceConfig.playback === "audio" || voiceConfig.playback === "both",
			latestAudioUrl,
		);

		// Cleanup RAF on unmount. The id MUST be cleared, not just cancelled:
		// `pushCurrentMessageUpdate` only schedules a frame while it is null, and refs
		// survive React's mount → cleanup → remount cycle. Leaving a stale id latches
		// the gate shut, so every later push lands in `pendingMessagesRef` and is never
		// flushed — the global chat pushes on mount, right inside that window, which
		// killed its live bubbles while app chats (first push long after mount) were fine.
		useEffect(() => {
			return () => {
				if (rafIdRef.current !== null) {
					cancelAnimationFrame(rafIdRef.current);
					rafIdRef.current = null;
				}
			};
		}, []);

		const mainWidget = useMemo(() => {
			const liveIds = new Set(currentMessages.map((message) => message.id));
			const candidates = [
				...localMessages.filter((message) => !liveIds.has(message.id)),
				...currentMessages,
			].sort((a, b) => a.timestamp - b.timestamp);
			return resolveMainChatWidget(candidates, sessionId ?? "");
		}, [localMessages, currentMessages, sessionId]);

		const chatItems = useMemo(() => {
			const liveIds = new Set(currentMessages.map((message) => message.id));
			const filtered =
				liveIds.size > 0
					? localMessages.filter((msg) => !liveIds.has(msg.id))
					: localMessages;
			return filtered
				.map((msg) => ({
					type: "message" as const,
					data: msg,
					timestamp: msg.timestamp,
				}))
				.sort((a, b) => a.timestamp - b.timestamp);
		}, [localMessages, currentMessages]);

		// Pending interactions stay at the bottom where the user answers them; settled ones fold
		// into a compact group under the message that was current when they were asked.
		const { interactionItems, settledByMessageId } = useMemo(() => {
			const items: ChatItem[] = [];
			const anchored = new Map<string, IInteractionRequest[]>();
			if (!activeInteractions || activeInteractions.length === 0) {
				return { interactionItems: items, settledByMessageId: anchored };
			}

			const anchors = [
				...chatItems.map((item) => item.data),
				...currentMessages,
			].sort((a, b) => a.timestamp - b.timestamp);
			const unanchored: IInteractionRequest[] = [];

			for (const interaction of activeInteractions) {
				const isPending =
					interaction.status === "pending" &&
					interaction.expires_at * 1000 > Date.now();
				if (isPending) {
					items.push({ type: "interaction", data: interaction, timestamp: 0 });
					continue;
				}
				const anchor = findAnchorMessage(
					anchors,
					getInteractionCreatedAt(interaction),
				);
				if (!anchor) {
					unanchored.push(interaction);
					continue;
				}
				const group = anchored.get(anchor.id);
				if (group) group.push(interaction);
				else anchored.set(anchor.id, [interaction]);
			}

			if (unanchored.length > 0) {
				items.unshift({
					type: "interaction-group",
					data: unanchored,
					timestamp: 0,
				});
			}

			return { interactionItems: items, settledByMessageId: anchored };
		}, [activeInteractions, chatItems, currentMessages]);

		// Until an answer shows its first block, the question carries the wait and the empty
		// answer stays out of the transcript.
		const pendingAsks = useMemo(() => {
			const interactionShown = interactionItems.length > 0;
			const hasHostContent = (messageId: string) =>
				interactionShown ||
				settledByMessageId.has(messageId) ||
				Boolean(messageInlineContent?.get(messageId)?.length);
			return resolvePendingAsks(
				chatItems.map((item) => item.data),
				currentMessages,
				lastSend,
				hasHostContent,
			);
		}, [
			chatItems,
			currentMessages,
			lastSend,
			interactionItems,
			settledByMessageId,
			messageInlineContent,
		]);
		const visibleLiveMessages = useMemo(
			() =>
				currentMessages.filter(
					(message) =>
						!pendingAsks.hidden.has(message.id) &&
						(!message.main_widget ||
							!isAwaitingFirstEvent(
								message,
								settledByMessageId.has(message.id) ||
									Boolean(messageInlineContent?.get(message.id)?.length),
							)),
				),
			[currentMessages, pendingAsks, messageInlineContent, settledByMessageId],
		);
		const optimisticPending: PendingAsk | undefined = lastSend
			? { phase: "sending", since: lastSend.startedAt }
			: undefined;

		useEffect(() => {
			isSendingRef.current = isSending;
		}, [isSending]);

		// Reset state when switching sessions (avoids expensive key-based remount). Live bubbles are
		// FILTERED by session rather than cleared: the store-side mirror may have already pushed the
		// new session's bubbles (or a stale RAF may still fire afterwards) — filtering makes every
		// interleaving converge on "only this session's bubbles" instead of racing a blind clear.
		useEffect(() => {
			const belongsToSession = (message: IMessage) =>
				!message.sessionId || message.sessionId === sessionId;
			if (pendingMessagesRef.current) {
				pendingMessagesRef.current =
					pendingMessagesRef.current.filter(belongsToSession);
			}
			setCurrentMessages((prev) => prev.filter(belongsToSession));
			setLocalMessages([]);
			setShouldAutoScroll(true);
			setHasInitiallyScrolled(false);
			setIsSending(false);
			setSendingContent("");
			setLastSend(null);
			setReplyTarget(null);
		}, [sessionId]);

		// Sync external messages with local state (no useTransition to avoid flash gap)
		useEffect(() => {
			setLocalMessages(messages);

			// Clear optimistic sending state when the user message appears in DB
			if (isSendingRef.current) {
				const lastMessage = messages[messages.length - 1];
				if (lastMessage?.inner.role === "user") {
					setIsSending(false);
					setSendingContent("");
				}
			}
		}, [messages]);

		// Update active tools based on last user message and available tools
		useEffect(() => {
			const lastUserMessage = messages
				.slice()
				.reverse()
				.find((msg) => msg.inner.role === "user");

			let nextActiveTools: string[];
			if (lastUserMessage) {
				const availableTools = config?.tools ?? [];
				const lastActiveTools = lastUserMessage.tools ?? [];
				nextActiveTools = lastActiveTools.filter((tool) =>
					availableTools.includes(tool),
				);
			} else {
				nextActiveTools = config?.default_tools ?? [];
			}

			// `config` is often assembled by a parent render. Avoid setting a freshly-created but
			// semantically identical array on every pass, which otherwise causes an update-depth
			// loop when `config.tools`/`default_tools` are inline arrays.
			setDefaultActiveTools((current) =>
				sameStringArray(current, nextActiveTools) ? current : nextActiveTools,
			);
		}, [messages, config?.tools, config?.default_tools]);

		const scrollMessagesToEnd = useCallback(() => {
			const container = scrollContainerRef.current;
			if (!container) return;
			container.scrollTop = container.scrollHeight;
		}, []);

		// Pin the transcript to the bottom BEFORE the first paint — a deferred scroll flashes the
		// oldest messages at the top on every mount and session switch. The follow-up passes catch
		// late layout (async editor mount, images) without ever painting the top first; they are
		// deliberately not cleaned up (one-shot, and the scroll callback no-ops once unmounted).
		useLayoutEffect(() => {
			if (localMessages.length === 0 || hasInitiallyScrolled) return;
			scrollMessagesToEnd();
			requestAnimationFrame(scrollMessagesToEnd);
			setTimeout(scrollMessagesToEnd, 120);
			setHasInitiallyScrolled(true);
		}, [localMessages.length, hasInitiallyScrolled, scrollMessagesToEnd]);

		const scrollToBottom = useCallback(() => {
			if (!shouldAutoScroll) return;
			isScrollingProgrammatically.current = true;
			scrollMessagesToEnd();
			// Reset the flag after scroll animation completes
			setTimeout(() => {
				isScrollingProgrammatically.current = false;
			}, 500);
		}, [scrollMessagesToEnd, shouldAutoScroll]);

		const isAtBottom = useCallback(() => {
			if (!scrollContainerRef.current) return false;
			const { scrollTop, scrollHeight, clientHeight } =
				scrollContainerRef.current;
			const threshold = 100; // Larger threshold for better detection
			return Math.abs(scrollHeight - scrollTop - clientHeight) < threshold;
		}, []);

		const handleScroll = useCallback(() => {
			const atBottom = isAtBottom();
			if (isScrollingProgrammatically.current) {
				if (!atBottom) {
					setShouldAutoScroll(false);
				}
				return;
			}

			setShouldAutoScroll(atBottom);
		}, [isAtBottom]);

		// Auto-scroll when new messages arrive or current message updates, but only if should auto-scroll
		// biome-ignore lint/correctness/useExhaustiveDependencies: the content deps are the re-run triggers; the body only reads the scroll flags.
		useEffect(() => {
			if (shouldAutoScroll && hasInitiallyScrolled) {
				scrollToBottom();
			}
		}, [
			localMessages,
			currentMessages,
			inlineContent,
			messageInlineContent,
			shouldAutoScroll,
			hasInitiallyScrolled,
			scrollToBottom,
		]);

		// When user sends a message, always scroll to bottom and enable auto-scroll
		const handleSendMessage = useCallback(
			async (
				content: string,
				filesAttached?: File[],
				activeTools?: string[],
				audioFile?: File,
				interaction?: IChatMessageInteraction,
			) => {
				const startedAt = Date.now();
				setShouldAutoScroll(true);
				setIsSending(true);
				setSendingContent(content);
				setLastSend({ content, startedAt, inFlight: true });

				// Scroll immediately to show the optimistic message
				setTimeout(() => {
					scrollToBottom();
				}, 50);

				try {
					await onSendMessage(
						content,
						filesAttached,
						activeTools,
						audioFile,
						interaction,
					);
				} finally {
					setIsSending(false);
					setSendingContent("");
					setLastSend((send) =>
						send?.startedAt === startedAt ? { ...send, inFlight: false } : send,
					);
				}
				// Scroll after a brief delay to ensure the message is rendered
				setTimeout(() => {
					scrollToBottom();
				}, 50);
			},
			[onSendMessage, scrollToBottom],
		);

		const handleReply = useCallback(
			(message: IMessage, selectedText?: string) => {
				if (interactionDisabled || message.inner.role !== "assistant") return;
				setReplyTarget(createMessageReference(message, selectedText));
				chatBox.current?.focusInput?.();
			},
			[interactionDisabled],
		);

		const handleComposerSend: ISendMessageFunction = useCallback(
			async (content, filesAttached, activeTools, audioFile) => {
				const target = replyTarget;
				const sendSession = sessionId;
				setReplyTarget(null);
				try {
					await handleSendMessage(
						content,
						filesAttached,
						activeTools,
						audioFile,
						target ? { type: "reply", target } : undefined,
					);
				} catch (error) {
					if (currentSession.current === sendSession) {
						setReplyTarget((current) => current ?? target);
					}
					throw error;
				}
			},
			[handleSendMessage, replyTarget, sessionId],
		);

		const handleVoiceModeSend = useCallback(
			async (content: string, audioFile?: File) => {
				// keep voice mode open so the orb can react to the spoken answer;
				// VoiceMode closes itself once the answer has been delivered.
				await handleComposerSend(content, undefined, undefined, audioFile);
			},
			[handleComposerSend],
		);

		// A reply's `send_message` takes the composer's send path: it starts a turn or queues
		// like a typed message, and is unavailable whenever Send is disabled.
		const handleSendMessageRef = useRef(handleSendMessage);
		handleSendMessageRef.current = handleSendMessage;
		const sendFollowUp = useCallback((content: string) => {
			void handleSendMessageRef.current(
				content,
				undefined,
				chatBox.current?.getActiveTools(),
			);
		}, []);
		const followUpBlocked = !concurrency && (isSending || isStreamActive);

		// Keep the transcript pinned without asking scrollIntoView to pan the
		// document/visual viewport while iOS is opening its keyboard.
		useEffect(() => {
			let focusTimer = 0;
			const onFocusIn = (e: FocusEvent) => {
				const target = e.target as HTMLElement | null;
				if (!target?.closest("[data-fl-chat-composer]")) return;
				window.clearTimeout(focusTimer);
				focusTimer = window.setTimeout(scrollMessagesToEnd, 100);
			};
			document.addEventListener("focusin", onFocusIn);
			return () => {
				window.clearTimeout(focusTimer);
				document.removeEventListener("focusin", onFocusIn);
			};
		}, [scrollMessagesToEnd]);

		// Dismiss keyboard when tapping outside inputs on iOS
		useEffect(() => {
			const onTouchStart = (e: TouchEvent) => {
				const active = document.activeElement as HTMLElement | null;
				if (!active) return;
				const tag = active.tagName;
				if (tag === "INPUT" || tag === "TEXTAREA") {
					const target = e.target as Node | null;
					if (target && active && !active.contains(target)) {
						setTimeout(() => {
							try {
								active.blur();
							} catch {}
						}, 50);
					}
				}
			};
			document.addEventListener("touchstart", onTouchStart, {
				passive: true,
				capture: true,
			} as AddEventListenerOptions);
			return () =>
				document.removeEventListener("touchstart", onTouchStart, true);
		}, []);

		// Expose methods via ref
		useImperativeHandle(
			ref,
			() => ({
				pushCurrentMessageUpdate: (message: IMessage | IMessage[]) => {
					// Throttle updates via requestAnimationFrame to avoid per-event re-renders
					pendingMessagesRef.current = Array.isArray(message)
						? message
						: [message];
					if (rafIdRef.current === null) {
						rafIdRef.current = requestAnimationFrame(() => {
							rafIdRef.current = null;
							if (pendingMessagesRef.current) {
								setCurrentMessages(pendingMessagesRef.current);
							}
						});
					}
				},
				clearCurrentMessageUpdate: () => {
					// Cancel pending RAF and clear immediately
					if (rafIdRef.current !== null) {
						cancelAnimationFrame(rafIdRef.current);
						rafIdRef.current = null;
					}
					pendingMessagesRef.current = null;
					setCurrentMessages([]);
				},
				pushMessage: (message: IMessage) => {
					setLocalMessages((prev) => [...prev, message]);
				},
				sendMessage: handleSendMessage,
				scrollToBottom,
				clearMessages: () => {
					setLocalMessages([]);
					setReplyTarget(null);
					setHasInitiallyScrolled(false);
					setShouldAutoScroll(true);
				},
				focusInput: () => {
					chatBox.current?.focusInput?.();
				},
			}),
			[handleSendMessage, scrollToBottom],
		);

		return (
			<ChatFollowUpProvider send={followUpBlocked ? null : sendFollowUp}>
				<MainWidgetPane
					mainWidget={mainWidget}
					sessionId={sessionId ?? ""}
					appId={appId}
					boardId={boardId}
					eventId={eventId}
					snapshots={widgetSnapshots}
				>
					<main
						className="fl-chat-surface flex min-h-0 w-full flex-1 flex-col items-center overflow-hidden bg-transparent"
						data-fl-chat-surface
						style={{
							backgroundColor:
								"var(--fl-chat-surface-background, var(--background))",
							WebkitOverflowScrolling: "touch",
							touchAction: "manipulation",
						}}
					>
						<div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden bg-transparent">
							{/* Messages Container */}
							<div
								ref={scrollContainerRef}
								onScroll={handleScroll}
								className="flex-1 overflow-y-auto overscroll-contain p-4 pb-2 space-y-8 flex flex-col items-center grow max-h-full"
								data-fl-chat-messages
								style={{ WebkitOverflowScrolling: "touch" }}
							>
								{chatItems
									.filter(
										(item) =>
											!item.data.main_widget ||
											!isAwaitingFirstEvent(
												item.data,
												settledByMessageId.has(item.data.id) ||
													Boolean(
														messageInlineContent?.get(item.data.id)?.length,
													),
											),
									)
									.map((item, index) => (
										<div
											className="w-full px-1 sm:px-4"
											key={`msg-${item.data.id}`}
											style={{
												maxWidth:
													"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
											}}
										>
											<MessageShell
												immediate={
													index >= chatItems.length - IMMEDIATE_MESSAGE_TAIL
												}
												root={scrollContainerRef}
											>
												<MessageComponent
													message={item.data as IMessage}
													onMessageUpdate={onMessageUpdate}
													onReply={handleReply}
													onReaction={onMessageReaction}
													interactionDisabled={interactionDisabled}
													appId={appId}
													boardId={boardId}
													eventId={eventId}
													widgetSnapshots={widgetSnapshots}
													anchoredContent={messageInlineContent?.get(
														item.data.id,
													)}
													pending={pendingAsks.byAsk.get(item.data.id)}
												/>
											</MessageShell>
											<SettledInteractions
												interactions={settledByMessageId.get(item.data.id)}
												onRespond={onRespondToInteraction}
											/>
										</div>
									))}
								{isSending &&
									!localMessages.some(
										(m) =>
											m.inner.role === "user" &&
											getMessageTextContent(m) === sendingContent,
									) && (
										<div
											className="flex w-full animate-in flex-col items-start space-y-1 px-4 fade-in slide-in-from-bottom-2 duration-200"
											style={{
												maxWidth:
													"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
											}}
										>
											<div
												className="w-full border-l-2 py-2 pr-4 pl-3.5"
												data-fl-chat-message="user"
												style={{
													backgroundColor:
														"var(--fl-chat-ask-background, transparent)",
													borderLeftColor: askRuleColor(optimisticPending),
													borderRadius:
														"0 var(--fl-chat-message-radius, 0.75rem) var(--fl-chat-message-radius, 0.75rem) 0",
													color:
														"var(--fl-chat-user-message-foreground, var(--foreground))",
													maxWidth: "var(--fl-chat-measure, 38rem)",
												}}
											>
												<span className="mb-1.5 flex items-baseline gap-2 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground/70">
													{i18next.t("asked", "Asked")}
													{optimisticPending && (
														<PendingAskStatus pending={optimisticPending} />
													)}
												</span>
												<p className="line-clamp-6 whitespace-pre-wrap text-sm leading-relaxed">
													{sendingContent}
												</p>
											</div>
										</div>
									)}
								{visibleLiveMessages.map((liveMessage) => (
									<div
										className="w-full px-4 animate-in fade-in slide-in-from-top-1 duration-300"
										key={`msg-${liveMessage.id}`}
										style={{
											maxWidth:
												"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
										}}
									>
										<MessageComponent
											loading={
												!liveMessage.stream_root_id ||
												liveMessage.id === currentMessages.at(-1)?.id
											}
											message={liveMessage}
											appId={appId}
											boardId={boardId}
											eventId={eventId}
											widgetSnapshots={widgetSnapshots}
											anchoredContent={messageInlineContent?.get(
												liveMessage.id,
											)}
										/>
										<SettledInteractions
											interactions={settledByMessageId.get(liveMessage.id)}
											onRespond={onRespondToInteraction}
										/>
									</div>
								))}
								{inlineContent && (
									<div
										className="w-full px-1 sm:px-4"
										data-fl-chat-inline-content
										style={{
											maxWidth: "var(--fl-chat-content-width, 64rem)",
										}}
									>
										{inlineContent}
									</div>
								)}
								{interactionItems.map((item) =>
									item.type === "interaction-group" ? (
										<div
											className="flex w-full flex-col items-start px-4"
											key={`grp-${(item.data as IInteractionRequest[]).map((i) => i.id).join("-")}`}
											style={{
												maxWidth:
													"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
											}}
										>
											<InteractionGroup
												interactions={item.data as IInteractionRequest[]}
												onRespond={onRespondToInteraction ?? (() => {})}
											/>
										</div>
									) : (
										<div
											className="flex w-full flex-col items-start px-4"
											key={`int-${(item.data as IInteractionRequest).id}`}
											style={{
												maxWidth:
													"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
											}}
										>
											<Interaction
												interaction={item.data as IInteractionRequest}
												onRespond={onRespondToInteraction ?? (() => {})}
											/>
										</div>
									),
								)}
								<div aria-hidden="true" />
							</div>

							{inlinePrompt && (
								<div
									className="mx-auto w-full shrink-0 px-2 pt-1"
									style={{
										maxWidth:
											"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
									}}
								>
									{inlinePrompt}
								</div>
							)}

							{/* ChatBox */}
							<div
								className="mx-auto w-full space-y-2 px-3"
								data-fl-chat-composer-dock
								style={{
									maxWidth:
										"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
									paddingBottom:
										"calc(var(--fl-chat-pad-bottom, 0.75rem) + var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px)))",
								}}
							>
								{concurrency && <ChatRunControls concurrency={concurrency} />}
								{replyTarget && (
									<MessageReference
										reference={replyTarget}
										onCancel={() => setReplyTarget(null)}
									/>
								)}
								{defaultActiveTools && (
									<ChatBox
										ref={chatBox}
										availableTools={config?.tools ?? []}
										defaultActiveTools={defaultActiveTools}
										onSendMessage={handleComposerSend}
										onContentChange={onDraftChange}
										thinking={thinking}
										fileUpload={config?.allow_file_upload ?? false}
										audioInput={voiceEnabled}
										voiceMode={voiceConfig.mode === "stt" ? "stt" : "record"}
										voiceInvoke={voiceConfig.invoke}
										voiceMaxDuration={voiceConfig.maxDuration}
										// With concurrency the composer never locks: a send starts another
										// turn or queues. Without it, a live stream still blocks as before.
										sendDisabled={
											concurrency ? false : isSending || isStreamActive
										}
										sendHint={
											concurrency?.atCapacity
												? i18next.t("queueThisMessage", "Queue this message")
												: concurrency && concurrency.runs.length > 0
													? i18next.t(
															"startAnotherResponse",
															"Start another response",
														)
													: undefined
										}
										onSteer={
											!replyTarget && concurrency && concurrency.runs.length > 0
												? concurrency.onSteer
												: undefined
										}
										onInterrupt={playback.stop}
										onVoiceModeToggle={
											voiceEnabled && voiceConfig.invoke === "auto"
												? () => {
														playback.stop();
														setVoiceModeOpen(true);
													}
												: undefined
										}
									/>
								)}
								{showAiDisclosure && (
									<ChatAiDisclosure text={config.ai_disclosure} />
								)}
							</div>
						</div>

						{/* Voice Mode Overlay */}
						<VoiceMode
							open={voiceModeOpen}
							onClose={() => setVoiceModeOpen(false)}
							onSend={handleVoiceModeSend}
							voice={voiceConfig}
							busy={isStreamActive || playback.isPlaying}
							speaking={playback.isPlaying}
							speakingAnalyser={playback.analyser}
							onInterrupt={() => playback.stop()}
						/>
					</main>
				</MainWidgetPane>
			</ChatFollowUpProvider>
		);
	},
);

export const Chat = memo(ChatInner);
Chat.displayName = "Chat";
