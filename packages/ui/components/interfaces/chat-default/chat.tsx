"use client";

import { i18n as i18next } from "@flow-like/locales";
import { useTheme } from "next-themes";
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
import PuffLoader from "react-spinners/PuffLoader";
import { type IEventPayloadChat, resolveChatColorScheme } from "../../../lib";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import { VoiceMode } from "./VoiceMode";
import { ChatAiDisclosure } from "./ai-disclosure";
import type { IMessage } from "./chat-db";
import { ChatRunControls } from "./chat-run-controls";
import { ChatBox, type ChatBoxRef, type ISendMessageFunction } from "./chatbox";
import { Interaction, InteractionGroup } from "./interaction";
import { MessageComponent } from "./message";
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

function getInteractionCreatedAt(interaction: IInteractionRequest): number {
	return (interaction.expires_at - interaction.ttl_seconds) * 1000;
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
	config?: Partial<IEventPayloadChat>;
	sessionId?: string;
	isStreamActive?: boolean;
	activeInteractions?: IInteractionRequest[];
	onRespondToInteraction?: (interactionId: string, value: any) => void;
	/** Rendered pinned between the message feed and the input — e.g. tool approval prompts. */
	inlinePrompt?: React.ReactNode;
	/**
	 * Live content that belongs in the conversation flow, after the latest message. Global
	 * FlowPilot uses this for app pages and chats so they scroll with the transcript instead of
	 * taking a second, nested viewport above it.
	 */
	inlineContent?: React.ReactNode;
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
			config = {},
			sessionId,
			isStreamActive = false,
			activeInteractions,
			onRespondToInteraction,
			inlinePrompt,
			inlineContent,
			appId,
			boardId,
			eventId,
			showAiDisclosure = false,
			onDraftChange,
		},
		ref,
	) => {
		const { resolvedTheme } = useTheme();
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
		const isSendingRef = useRef(false);
		const [sendingContent, setSendingContent] = useState("");
		const pendingMessagesRef = useRef<IMessage[] | null>(null);
		const rafIdRef = useRef<number | null>(null);
		const [voiceModeOpen, setVoiceModeOpen] = useState(false);

		const voiceConfig = useMemo(() => resolveChatVoiceConfig(config), [config]);
		const voiceEnabled = isVoiceEnabled(voiceConfig);
		const configuredColorScheme = resolveChatColorScheme(config.color_scheme);
		const chatTheme =
			configuredColorScheme === "system"
				? resolvedTheme
				: configuredColorScheme;

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

		// Interactions are rendered separately after the live bubbles to avoid ordering issues
		const interactionItems = useMemo<ChatItem[]>(() => {
			if (!activeInteractions || activeInteractions.length === 0) return [];

			const items: ChatItem[] = [];
			let settledGroup: IInteractionRequest[] = [];

			const flushGroup = () => {
				if (settledGroup.length > 0) {
					items.push({
						type: "interaction-group",
						data: settledGroup,
						timestamp: 0,
					});
					settledGroup = [];
				}
			};

			for (const interaction of activeInteractions) {
				const remaining = Math.max(
					0,
					Math.floor((interaction.expires_at * 1000 - Date.now()) / 1000),
				);
				const isPending = interaction.status === "pending" && remaining > 0;

				if (!isPending) {
					settledGroup.push(interaction);
				} else {
					flushGroup();
					items.push({ type: "interaction", data: interaction, timestamp: 0 });
				}
			}
			flushGroup();

			return items;
		}, [activeInteractions]);

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
		useEffect(() => {
			if (shouldAutoScroll && hasInitiallyScrolled) {
				scrollToBottom();
			}
		}, [
			localMessages,
			currentMessages,
			inlineContent,
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
			) => {
				setShouldAutoScroll(true);
				setIsSending(true);
				setSendingContent(content);

				// Scroll immediately to show the optimistic message
				setTimeout(() => {
					scrollToBottom();
				}, 50);

				try {
					await onSendMessage(content, filesAttached, activeTools, audioFile);
				} finally {
					setIsSending(false);
					setSendingContent("");
				}
				// Scroll after a brief delay to ensure the message is rendered
				setTimeout(() => {
					scrollToBottom();
				}, 50);
			},
			[onSendMessage, scrollToBottom],
		);

		const handleVoiceModeSend = useCallback(
			async (content: string, audioFile?: File) => {
				// keep voice mode open so the orb can react to the spoken answer;
				// VoiceMode closes itself once the answer has been delivered.
				await handleSendMessage(content, undefined, undefined, audioFile);
			},
			[handleSendMessage],
		);

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
				document.removeEventListener("touchstart", onTouchStart, true as any);
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
					setHasInitiallyScrolled(false);
					setShouldAutoScroll(true);
				},
				focusInput: () => {
					chatBox.current?.focusInput?.();
				},
			}),
			[handleSendMessage, scrollToBottom, shouldAutoScroll],
		);

		return (
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
						{chatItems.map((item) => (
							<div
								className="w-full px-1 sm:px-4"
								key={`msg-${item.data.id}`}
								style={{
									maxWidth:
										"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
								}}
							>
								<MessageComponent
									message={item.data as IMessage}
									onMessageUpdate={onMessageUpdate}
									appId={appId}
									boardId={boardId}
									eventId={eventId}
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
											borderLeftColor:
												"var(--fl-chat-ask-rule, var(--primary))",
											borderRadius:
												"0 var(--fl-chat-message-radius, 0.75rem) var(--fl-chat-message-radius, 0.75rem) 0",
											color:
												"var(--fl-chat-user-message-foreground, var(--foreground))",
											maxWidth: "var(--fl-chat-measure, 38rem)",
										}}
									>
										<span className="mb-1.5 block text-[10px] font-semibold uppercase tracking-widest text-muted-foreground/70">
											{i18next.t("asked", "Asked")}
										</span>
										<p className="line-clamp-6 whitespace-pre-wrap text-sm leading-relaxed">
											{sendingContent}
										</p>
									</div>
									<div className="flex items-center gap-2 pl-1">
										<PuffLoader
											size={16}
											color={chatTheme === "dark" ? "white" : "black"}
										/>
										<span className="text-xs text-muted-foreground">
											Processing...
										</span>
									</div>
								</div>
							)}
						{currentMessages.map((liveMessage) => (
							<div
								className="w-full px-4"
								key={`msg-${liveMessage.id}`}
								style={{
									maxWidth:
										"min(var(--fl-chat-content-width, 64rem), var(--fl-chat-wide, 46rem))",
								}}
							>
								<MessageComponent
									loading
									message={liveMessage}
									appId={appId}
									boardId={boardId}
									eventId={eventId}
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
						{defaultActiveTools && (
							<ChatBox
								ref={chatBox}
								availableTools={config?.tools ?? []}
								defaultActiveTools={defaultActiveTools}
								onSendMessage={handleSendMessage}
								onContentChange={onDraftChange}
								fileUpload={config?.allow_file_upload ?? false}
								audioInput={voiceEnabled}
								voiceMode={voiceConfig.mode === "stt" ? "stt" : "record"}
								voiceInvoke={voiceConfig.invoke}
								voiceMaxDuration={voiceConfig.maxDuration}
								// With concurrency the composer never locks: a send starts another
								// turn or queues. Without it, a live stream still blocks as before.
								sendDisabled={concurrency ? false : isSending || isStreamActive}
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
									concurrency && concurrency.runs.length > 0
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
		);
	},
);

export const Chat = memo(ChatInner);
Chat.displayName = "Chat";
