"use client";

import { useTheme } from "next-themes";
import {
	forwardRef,
	memo,
	useCallback,
	useEffect,
	useImperativeHandle,
	useMemo,
	useRef,
	useState,
} from "react";
import PuffLoader from "react-spinners/PuffLoader";
import type { IEventPayloadChat } from "../../../lib";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import type { IMessage } from "./chat-db";
import { ChatBox, type ChatBoxRef, type ISendMessageFunction } from "./chatbox";
import { Interaction, InteractionGroup } from "./interaction";
import { MessageComponent } from "./message";
import { VoiceMode } from "./VoiceMode";

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
	const textContent = content.find((c) => c.type === "text");
	return textContent?.text ?? "";
}

export interface IChatProps {
	messages: IMessage[];
	onSendMessage: ISendMessageFunction;
	onMessageUpdate?: (
		messageId: string,
		updates: Partial<IMessage>,
	) => void | Promise<void>;
	config?: Partial<IEventPayloadChat>;
	sessionId?: string;
	isStreamActive?: boolean;
	activeInteractions?: IInteractionRequest[];
	onRespondToInteraction?: (interactionId: string, value: any) => void;
}

export interface IChatRef {
	pushCurrentMessageUpdate: (message: IMessage) => void;
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
			onMessageUpdate,
			config = {},
			sessionId,
			isStreamActive = false,
			activeInteractions,
			onRespondToInteraction,
		},
		ref,
	) => {
		const { resolvedTheme } = useTheme();
		const messagesEndRef = useRef<HTMLDivElement>(null);
		const scrollContainerRef = useRef<HTMLDivElement>(null);
		const [shouldAutoScroll, setShouldAutoScroll] = useState(true);
		const [currentMessage, setCurrentMessage] = useState<IMessage | null>(null);
		const [localMessages, setLocalMessages] = useState<IMessage[]>(messages);
		const [hasInitiallyScrolled, setHasInitiallyScrolled] = useState(false);
		const chatBox = useRef<ChatBoxRef>(null);
		const isScrollingProgrammatically = useRef(false);
		const [defaultActiveTools, setDefaultActiveTools] = useState<string[]>();
		const [isSending, setIsSending] = useState(false);
		const isSendingRef = useRef(false);
		const [sendingContent, setSendingContent] = useState("");
		const pendingMessageRef = useRef<IMessage | null>(null);
		const rafIdRef = useRef<number | null>(null);
		const [voiceModeOpen, setVoiceModeOpen] = useState(false);

		// Cleanup RAF on unmount
		useEffect(() => {
			return () => {
				if (rafIdRef.current !== null) cancelAnimationFrame(rafIdRef.current);
			};
		}, []);

		const chatItems = useMemo(() => {
			const filtered = currentMessage
				? localMessages.filter((msg) => msg.id !== currentMessage.id)
				: localMessages;
			return filtered
				.map((msg) => ({
					type: "message" as const,
					data: msg,
					timestamp: msg.timestamp,
				}))
				.sort((a, b) => a.timestamp - b.timestamp);
		}, [localMessages, currentMessage]);

		// Interactions are rendered separately after currentMessage to avoid ordering issues
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

		// Reset state when switching sessions (avoids expensive key-based remount)
		useEffect(() => {
			setCurrentMessage(null);
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

			if (lastUserMessage) {
				const availableTools = config?.tools ?? [];
				const lastActiveTools = lastUserMessage.tools ?? [];
				const newActiveTools = lastActiveTools.filter((tool) =>
					availableTools.includes(tool),
				);

				setDefaultActiveTools(newActiveTools);
				return;
			}

			setDefaultActiveTools(config?.default_tools ?? []);
		}, [messages, config?.tools, config?.default_tools]);

		// Initial scroll to bottom when messages first load
		useEffect(() => {
			if (localMessages.length > 0 && !hasInitiallyScrolled) {
				setTimeout(() => {
					scrollToBottom();
					setHasInitiallyScrolled(true);
				}, 100);
			}
		}, [localMessages.length, hasInitiallyScrolled]);

		const scrollToBottom = useCallback(() => {
			if (!messagesEndRef.current) return;
			if (!shouldAutoScroll) return;
			isScrollingProgrammatically.current = true;
			messagesEndRef.current.scrollIntoView({
				behavior: "instant",
				block: "end",
			});
			// Reset the flag after scroll animation completes
			setTimeout(() => {
				isScrollingProgrammatically.current = false;
			}, 500);
		}, [shouldAutoScroll]);

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
			currentMessage,
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
			async (audioFile: File) => {
				setVoiceModeOpen(false);
				await handleSendMessage("", undefined, undefined, audioFile);
			},
			[handleSendMessage],
		);

		// iOS keyboard/open focus handling to reduce layout jump and zoom
		useEffect(() => {
			const onFocusIn = (e: FocusEvent) => {
				const target = e.target as HTMLElement | null;
				if (!target) return;
				// Ensure the input stays visible when keyboard opens
				if (target.tagName === "TEXTAREA" || target.tagName === "INPUT") {
					setTimeout(() => {
						try {
							messagesEndRef.current?.scrollIntoView({
								block: "end",
								behavior: "smooth",
							});
						} catch {}
					}, 100);
				}
			};
			document.addEventListener("focusin", onFocusIn);
			return () => document.removeEventListener("focusin", onFocusIn);
		}, []);

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
				pushCurrentMessageUpdate: (message: IMessage) => {
					// Throttle updates via requestAnimationFrame to avoid per-event re-renders
					pendingMessageRef.current = message;
					if (rafIdRef.current === null) {
						rafIdRef.current = requestAnimationFrame(() => {
							rafIdRef.current = null;
							if (pendingMessageRef.current) {
								setCurrentMessage(pendingMessageRef.current);
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
					pendingMessageRef.current = null;
					setCurrentMessage(null);
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
				className="flex flex-col flex-1 min-h-0 w-full items-center bg-background overflow-hidden"
				style={{
					WebkitOverflowScrolling: "touch",
					touchAction: "manipulation",
				}}
			>
				<div className="flex-1 min-h-0 flex flex-col bg-background w-full overflow-hidden">
					{/* Messages Container */}
					<div
						ref={scrollContainerRef}
						onScroll={handleScroll}
						className="flex-1 overflow-y-auto overscroll-contain p-4 pb-2 space-y-8 flex flex-col items-center flex-grow max-h-full"
						style={{ WebkitOverflowScrolling: "touch" }}
					>
						{chatItems.map((item) => (
							<div
								className="w-full max-w-screen-lg px-1 sm:px-4"
								key={`msg-${item.data.id}`}
							>
								<MessageComponent
									message={item.data as IMessage}
									onMessageUpdate={onMessageUpdate}
								/>
							</div>
						))}
						{isSending &&
							!localMessages.some(
								(m) =>
									m.inner.role === "user" &&
									getMessageTextContent(m) === sendingContent,
							) && (
								<div className="w-full max-w-screen-lg px-4 flex flex-col items-end space-y-1 animate-in fade-in slide-in-from-bottom-2 duration-200">
									<div className="bg-muted dark:bg-muted/30 text-foreground px-4 py-2 rounded-xl rounded-tr-sm max-w-3xl shadow-sm">
										<p className="whitespace-pre-wrap text-sm">
											{sendingContent}
										</p>
									</div>
									<div className="flex items-center gap-2 pr-1">
										<PuffLoader
											size={16}
											color={resolvedTheme === "dark" ? "white" : "black"}
										/>
										<span className="text-xs text-muted-foreground">
											Processing...
										</span>
									</div>
								</div>
							)}
						{currentMessage && (
								<div
									className="w-full max-w-screen-lg px-4"
									key={`msg-${currentMessage.id}`}
								>
									<MessageComponent loading message={currentMessage} />
								</div>
							)}
						{interactionItems.map((item) =>
							item.type === "interaction-group" ? (
								<div
									className="w-full max-w-screen-lg px-4 flex flex-col items-start"
									key={`grp-${(item.data as IInteractionRequest[]).map((i) => i.id).join("-")}`}
								>
									<InteractionGroup
										interactions={item.data as IInteractionRequest[]}
										onRespond={onRespondToInteraction ?? (() => {})}
									/>
								</div>
							) : (
								<div
									className="w-full max-w-screen-lg px-4 flex flex-col items-start"
									key={`int-${(item.data as IInteractionRequest).id}`}
								>
									<Interaction
										interaction={item.data as IInteractionRequest}
										onRespond={onRespondToInteraction ?? (() => {})}
									/>
								</div>
							),
						)}
						<div ref={messagesEndRef} />
					</div>

					{/* ChatBox */}
					<div
						className="bg-background px-2 pb-2 max-w-screen-lg w-full mx-auto"
						style={{
							paddingBottom:
								"calc(0.5rem + var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px)))",
						}}
					>
						{defaultActiveTools && (
							<ChatBox
								ref={chatBox}
								availableTools={config?.tools ?? []}
								defaultActiveTools={defaultActiveTools}
								onSendMessage={handleSendMessage}
								fileUpload={config?.allow_file_upload ?? false}
								audioInput={config?.allow_voice_input ?? true}
								sendDisabled={isSending || isStreamActive}
								onVoiceModeToggle={
									(config?.allow_voice_mode ?? true)
										? () => setVoiceModeOpen(true)
										: undefined
								}
							/>
						)}
					</div>
				</div>

				{/* Voice Mode Overlay */}
				<VoiceMode
					open={voiceModeOpen}
					onClose={() => setVoiceModeOpen(false)}
					onSend={handleVoiceModeSend}
				/>
			</main>
		);
	},
);

export const Chat = memo(ChatInner);
Chat.displayName = "Chat";
