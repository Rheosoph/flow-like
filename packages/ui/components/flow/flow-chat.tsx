"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowDownIcon,
	MessageCircleIcon,
	SendHorizontalIcon,
	XIcon,
} from "lucide-react";
import {
	type ChangeEvent,
	type KeyboardEvent,
	type ReactNode,
	type UIEvent,
	memo,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import type { ChatMessage } from "../../hooks/use-realtime-chat";
import { PING_EMOJI } from "../../lib/realtime/presence-signals";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import {
	CHAT_AT_BOTTOM_PX,
	CHAT_COUNTER_FROM,
	CHAT_MAX_LENGTH,
	type ChatGroup,
	type ChatTimelineItem,
	type DaySeparator,
	type TypingLabelParts,
	appendDraft,
	buildChatTimeline,
	formatChatDateTime,
	formatChatTime,
	insertAtCaret,
	nodeReferenceLabel,
	parseChatSegments,
	typingLabelParts,
} from "./flow-chat-model";

const COMPOSER_MAX_ROWS = 4;
const LAST_SEEN_PREFIX = "flow-chat:last-seen:";

/** Last-seen timestamps for boards without a storage key, and a warm cache for those with one. */
const sessionLastSeen = new Map<string, number>();
/** Drafts already placed into the composer; a reopened chat must not re-insert a stale one. */
const appliedDraftTokens = new Map<string, number>();

function readLastSeen(storageKey: string | undefined): number | undefined {
	const key = storageKey ?? "";
	const cached = sessionLastSeen.get(key);
	if (cached !== undefined || !storageKey || typeof window === "undefined")
		return cached;
	try {
		const raw = window.localStorage.getItem(LAST_SEEN_PREFIX + storageKey);
		const value = raw === null ? Number.NaN : Number(raw);
		return Number.isFinite(value) ? value : undefined;
	} catch {
		return undefined;
	}
}

function writeLastSeen(storageKey: string | undefined, ts: number): void {
	sessionLastSeen.set(storageKey ?? "", ts);
	if (!storageKey || typeof window === "undefined") return;
	try {
		window.localStorage.setItem(LAST_SEEN_PREFIX + storageKey, String(ts));
	} catch {}
}

interface ChatIdentity {
	color: string;
	name: string;
	avatarUrl?: string;
}

function identityOf(
	sub: string,
	peerUsers: Map<string, PeerUserInfo>,
): ChatIdentity {
	const info = peerUsers.get(sub);
	return {
		color: info?.color ?? colorFromSub(sub),
		name: info?.truncatedName ?? truncateName(sub.slice(-8)),
		avatarUrl: info?.avatarUrl,
	};
}

export interface FlowChatProps {
	messages: ChatMessage[];
	onSendMessage: (text: string) => void;
	onClose: () => void;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
	/** Text to place in the composer (e.g. "[[node:ID]] "), applied whenever `draftToken` changes. */
	draft?: string;
	draftToken?: number;
	/** Subs currently typing (the parent derives this); never includes the local user. */
	typingSubs?: readonly string[];
	/** Called on each keystroke so the hook can publish the typing heartbeat. */
	onTyping?: () => void;
	/** Called when the composer loses focus or the chat closes, so the heartbeat clears at once. */
	onStopTyping?: () => void;
	onlineCount?: number;
	/** Persists the last-seen timestamp (the "New" divider) per board. */
	storageKey?: string;
}

/**
 * The board's team chat: one shared room per board, left-aligned like a
 * channel, with node references as chips that jump to the node on the canvas.
 */
export const FlowChat = memo(function FlowChat({
	messages,
	onSendMessage,
	onClose,
	peerUsers,
	sub,
	resolveNodeName,
	onFocusNode,
	draft,
	draftToken,
	typingSubs,
	onTyping,
	onStopTyping,
	onlineCount,
	storageKey,
}: Readonly<FlowChatProps>) {
	const { t, i18n } = useTranslation("flow");
	const title = t("boardChat", "Board Chat");

	// The divider position is fixed at open time; what the user sees while the
	// chat is open is persisted as seen, so the next open starts after it.
	const [dividerLastSeen] = useState(() => readLastSeen(storageKey));
	// Only what the reader scrolled to counts as seen: a message that arrived
	// while they were reading older history keeps the "New" divider next time.
	const latestSeenRef = useRef<number | undefined>(undefined);
	const markSeenToBottom = useCallback(() => {
		const latest = messages[messages.length - 1]?.timestamp;
		if (latest === undefined) return;
		latestSeenRef.current = latest;
		writeLastSeen(storageKey, latest);
	}, [messages, storageKey]);

	const timeline = useMemo(
		() =>
			buildChatTimeline(messages, {
				now: Date.now(),
				locale: i18n.language,
				lastSeenTimestamp: dividerLastSeen,
				sub,
			}),
		[messages, dividerLastSeen, sub, i18n.language],
	);

	const listRef = useRef<HTMLDivElement>(null);
	const atBottomRef = useRef(true);
	const seenCountRef = useRef(messages.length);
	const [pendingNew, setPendingNew] = useState(0);

	const scrollToBottom = useCallback(() => {
		const list = listRef.current;
		if (list) list.scrollTop = list.scrollHeight;
		atBottomRef.current = true;
		setPendingNew(0);
		markSeenToBottom();
	}, [markSeenToBottom]);

	useLayoutEffect(() => {
		scrollToBottom();
	}, [scrollToBottom]);

	useLayoutEffect(() => {
		const added = messages.length - seenCountRef.current;
		seenCountRef.current = messages.length;
		if (added <= 0) return;
		const ownLast = messages[messages.length - 1]?.sub === sub;
		if (atBottomRef.current || ownLast) scrollToBottom();
		else setPendingNew((count) => count + added);
	}, [messages, sub, scrollToBottom]);

	const handleScroll = useCallback(
		(event: UIEvent<HTMLDivElement>) => {
			const list = event.currentTarget;
			const atBottom =
				list.scrollHeight - list.scrollTop - list.clientHeight <=
				CHAT_AT_BOTTOM_PX;
			atBottomRef.current = atBottom;
			if (atBottom) {
				setPendingNew(0);
				markSeenToBottom();
			}
		},
		[markSeenToBottom],
	);

	const [input, setInput] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const pendingCaretRef = useRef<number | undefined>(undefined);

	// biome-ignore lint/correctness/useExhaustiveDependencies: re-measure and place the caret after every draft change
	useLayoutEffect(() => {
		const textarea = textareaRef.current;
		if (!textarea) return;
		const styles = getComputedStyle(textarea);
		const line = Number.parseFloat(styles.lineHeight) || 16;
		const padding =
			(Number.parseFloat(styles.paddingTop) || 0) +
			(Number.parseFloat(styles.paddingBottom) || 0);
		const max = line * COMPOSER_MAX_ROWS + padding;
		textarea.style.height = "0px";
		textarea.style.height = `${Math.min(textarea.scrollHeight, max)}px`;
		textarea.style.overflowY = textarea.scrollHeight > max ? "auto" : "hidden";
		const caret = pendingCaretRef.current;
		if (caret !== undefined) {
			pendingCaretRef.current = undefined;
			textarea.focus();
			textarea.setSelectionRange(caret, caret);
		}
	}, [input]);

	useEffect(() => {
		textareaRef.current?.focus();
	}, []);

	// Only the unmount matters here; an inline callback from the parent must not
	// re-run the cleanup on every render and erase a heartbeat it just published.
	const stopTypingRef = useRef(onStopTyping);
	stopTypingRef.current = onStopTyping;
	useEffect(() => () => stopTypingRef.current?.(), []);

	useEffect(() => {
		if (draftToken === undefined || !draft) return;
		const key = storageKey ?? "";
		if (appliedDraftTokens.get(key) === draftToken) return;
		appliedDraftTokens.set(key, draftToken);
		setInput((current) => {
			const next = appendDraft(current, draft);
			pendingCaretRef.current = next.length;
			return next;
		});
	}, [draft, draftToken, storageKey]);

	const handleChange = useCallback(
		(event: ChangeEvent<HTMLTextAreaElement>) => {
			setInput(event.target.value.slice(0, CHAT_MAX_LENGTH));
			onTyping?.();
		},
		[onTyping],
	);

	const canSend = input.trim().length > 0;
	const handleSend = useCallback(() => {
		const text = input.trim();
		if (!text) return;
		onSendMessage(text);
		setInput("");
		pendingCaretRef.current = 0;
		scrollToBottom();
	}, [input, onSendMessage, scrollToBottom]);

	const handleKeyDown = useCallback(
		(event: KeyboardEvent<HTMLTextAreaElement>) => {
			if (event.key === "Enter" && !event.shiftKey) {
				if (event.nativeEvent.isComposing) return;
				event.preventDefault();
				event.stopPropagation();
				handleSend();
			}
		},
		[handleSend],
	);
	const handleSectionKeyDown = useCallback(
		(event: KeyboardEvent<HTMLElement>) => {
			if (event.key !== "Escape" || event.nativeEvent.isComposing) return;
			event.stopPropagation();
			onClose();
		},
		[onClose],
	);

	const insertEmoji = useCallback(
		(emoji: string) => {
			const textarea = textareaRef.current;
			const start = textarea?.selectionStart ?? input.length;
			const end = textarea?.selectionEnd ?? start;
			const result = insertAtCaret(input, emoji, start, end);
			if (result.text === input) return;
			setInput(result.text);
			pendingCaretRef.current = result.caret;
			onTyping?.();
		},
		[input, onTyping],
	);

	const typingParts = useMemo(
		() =>
			typingLabelParts(
				(typingSubs ?? []).filter((typingSub) => typingSub !== sub),
				(typingSub) => identityOf(typingSub, peerUsers).name,
			),
		[typingSubs, sub, peerUsers],
	);

	const sendLabel = t("sendMessage", "Send message");

	return (
		<section
			aria-label={title}
			onKeyDown={handleSectionKeyDown}
			className="flex h-[26rem] max-h-[70vh] w-[min(22rem,calc(100vw-1.5rem))] flex-col overflow-hidden rounded-md border bg-background text-foreground shadow-lg"
		>
			<header className="flex h-9 shrink-0 items-center gap-1.5 border-b pl-3 pr-1.5">
				<MessageCircleIcon
					className="size-3.5 shrink-0 text-muted-foreground"
					aria-hidden="true"
				/>
				<span className="truncate text-xs font-medium">{title}</span>
				{onlineCount !== undefined && (
					<span className="text-[11px] tabular-nums text-muted-foreground">
						{t("countOnline", {
							defaultValue_one: "{{count}} online",
							defaultValue_other: "{{count}} online",
							count: onlineCount,
						})}
					</span>
				)}
				<span className="flex-1" />
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-6 rounded-sm"
					onClick={onClose}
					aria-label={t("close", "Close")}
					title={t("close", "Close")}
				>
					<XIcon className="size-3.5" aria-hidden="true" />
				</Button>
			</header>

			<div className="relative min-h-0 flex-1">
				<div
					ref={listRef}
					onScroll={handleScroll}
					role="log"
					aria-live="polite"
					aria-label={title}
					className="h-full overflow-y-auto overscroll-contain px-3 py-2"
				>
					{messages.length === 0 ? (
						<div className="flex h-full flex-col items-center justify-center gap-1.5 text-center text-[11px] text-muted-foreground">
							<MessageCircleIcon
								className="size-6 text-muted-foreground/50"
								aria-hidden="true"
							/>
							{t("noMessagesYetSayHi", "No messages yet. Say hi!")}
						</div>
					) : (
						timeline.map((item) => (
							<TimelineItem
								key={item.key}
								item={item}
								peerUsers={peerUsers}
								sub={sub}
								resolveNodeName={resolveNodeName}
								onFocusNode={onFocusNode}
							/>
						))
					)}
				</div>
				{pendingNew > 0 && (
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={scrollToBottom}
						title={t("scrollToLatest", "Scroll to latest")}
						className="absolute bottom-2 left-1/2 h-6 -translate-x-1/2 gap-1 rounded-full px-2.5 text-[11px] shadow-lg"
					>
						<ArrowDownIcon className="size-3" aria-hidden="true" />
						{t("countNew", {
							defaultValue_one: "{{count}} new",
							defaultValue_other: "{{count}} new",
							count: pendingNew,
						})}
					</Button>
				)}
			</div>

			<TypingIndicator parts={typingParts} />

			<div className="shrink-0 border-t px-2 pb-2 pt-1.5">
				<div className="flex items-end gap-1 rounded-md border bg-muted/40 py-1 pl-2.5 pr-1 focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30">
					<textarea
						ref={textareaRef}
						rows={1}
						value={input}
						onChange={handleChange}
						onKeyDown={handleKeyDown}
						onBlur={onStopTyping}
						placeholder={t("typeAMessage", "Type a message...")}
						aria-label={t("typeAMessage", "Type a message...")}
						maxLength={CHAT_MAX_LENGTH}
						className="min-h-4 flex-1 resize-none bg-transparent py-0.5 text-xs leading-4 outline-none placeholder:text-muted-foreground/70"
					/>
					<Button
						type="button"
						size="icon"
						className="size-6 shrink-0 rounded-sm"
						disabled={!canSend}
						onClick={handleSend}
						aria-label={sendLabel}
						title={sendLabel}
					>
						<SendHorizontalIcon className="size-3.5" aria-hidden="true" />
					</Button>
				</div>
				<div className="mt-1 flex items-center gap-1">
					<div
						aria-label={t("quickReactions", "Quick reactions")}
						className="flex items-center gap-0.5"
					>
						{PING_EMOJI.map((emoji) => (
							<button
								key={emoji}
								type="button"
								onMouseDown={(event) => event.preventDefault()}
								onClick={() => insertEmoji(emoji)}
								aria-label={t("insertEmoji", "Insert {{emoji}}", { emoji })}
								className="rounded-sm px-1 py-0.5 text-[13px] leading-none outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50"
							>
								{emoji}
							</button>
						))}
					</div>
					<span className="flex-1" />
					{input.length >= CHAT_COUNTER_FROM && (
						<span
							aria-live="polite"
							className={cn(
								"text-[10px] tabular-nums",
								input.length >= CHAT_MAX_LENGTH
									? "text-destructive"
									: "text-muted-foreground",
							)}
						>
							{input.length}/{CHAT_MAX_LENGTH}
						</span>
					)}
				</div>
			</div>
		</section>
	);
});

const TimelineItem = memo(function TimelineItem({
	item,
	peerUsers,
	sub,
	resolveNodeName,
	onFocusNode,
}: Readonly<{
	item: ChatTimelineItem;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const { t, i18n } = useTranslation("flow");
	if (item.type === "day") {
		return <DayDivider separator={item.separator} />;
	}
	if (item.type === "unread") {
		const label = t("new", "New");
		return (
			<div className="my-1.5 flex items-center gap-2 text-[10px] font-semibold text-primary">
				<span aria-hidden="true" className="h-px flex-1 bg-primary/40" />
				<span>{label}</span>
			</div>
		);
	}
	return (
		<MessageGroup
			group={item.group}
			identity={identityOf(item.group.sub, peerUsers)}
			isSelf={item.group.sub === sub}
			resolveNodeName={resolveNodeName}
			onFocusNode={onFocusNode}
		/>
	);
});

const DayDivider = memo(function DayDivider({
	separator,
}: Readonly<{ separator: DaySeparator }>) {
	const { t } = useTranslation("flow");
	const label =
		separator.kind === "today"
			? t("today", "Today")
			: separator.kind === "yesterday"
				? t("yesterday", "Yesterday")
				: separator.label;
	return (
		<div className="my-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
			<span aria-hidden="true" className="h-px flex-1 bg-border" />
			<span>{label}</span>
			<span aria-hidden="true" className="h-px flex-1 bg-border" />
		</div>
	);
});

const MessageGroup = memo(function MessageGroup({
	group,
	identity,
	isSelf,
	resolveNodeName,
	onFocusNode,
}: Readonly<{
	group: ChatGroup;
	identity: ChatIdentity;
	isSelf: boolean;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const [first, ...rest] = group.messages;
	const name = isSelf ? t("you", "You") : identity.name;
	return (
		<div className="mt-2 first:mt-0">
			<div className="flex gap-1.5">
				<span className="flex w-8 shrink-0 items-start">
					<Avatar
						className="size-6 rounded-md"
						style={{ boxShadow: `0 0 0 1px ${identity.color}` }}
					>
						{identity.avatarUrl && (
							<AvatarImage
								src={identity.avatarUrl}
								alt=""
								className="object-cover"
							/>
						)}
						<AvatarFallback
							className="rounded-md text-[9px] font-semibold text-white"
							style={{ background: identity.color }}
						>
							{identity.name.charAt(0).toUpperCase()}
						</AvatarFallback>
					</Avatar>
				</span>
				<div className="min-w-0 flex-1">
					<div className="flex items-baseline gap-1.5 leading-4">
						<span
							className="truncate text-xs font-medium"
							style={{
								color: `color-mix(in oklch, ${identity.color} 55%, var(--foreground))`,
							}}
						>
							{name}
						</span>
						<time
							dateTime={new Date(first.timestamp).toISOString()}
							title={formatChatDateTime(first.timestamp)}
							className="shrink-0 text-[10px] tabular-nums text-muted-foreground"
						>
							{formatChatTime(first.timestamp)}
						</time>
					</div>
					<MessageBubble
						message={first}
						isSelf={isSelf}
						resolveNodeName={resolveNodeName}
						onFocusNode={onFocusNode}
					/>
				</div>
			</div>
			{rest.map((message) => (
				<div key={message.id} className="group/msg flex gap-1.5">
					<span
						aria-hidden="true"
						className="flex w-8 shrink-0 items-start justify-end whitespace-nowrap pt-1.5 text-[9px] leading-none tabular-nums text-muted-foreground opacity-0 group-hover/msg:opacity-100"
					>
						{formatChatTime(message.timestamp)}
					</span>
					<div className="min-w-0 flex-1">
						<MessageBubble
							message={message}
							isSelf={isSelf}
							resolveNodeName={resolveNodeName}
							onFocusNode={onFocusNode}
						/>
					</div>
				</div>
			))}
		</div>
	);
});

const MessageBubble = memo(function MessageBubble({
	message,
	isSelf,
	resolveNodeName,
	onFocusNode,
}: Readonly<{
	message: ChatMessage;
	isSelf: boolean;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
}>) {
	return (
		<div
			title={formatChatDateTime(message.timestamp)}
			className={cn(
				"mt-0.5 w-fit max-w-full whitespace-pre-wrap break-words rounded-md px-2 py-1 text-xs leading-4",
				isSelf ? "bg-primary/10" : "bg-muted",
			)}
		>
			<MessageText
				text={message.text}
				resolveNodeName={resolveNodeName}
				onFocusNode={onFocusNode}
			/>
		</div>
	);
});

const MessageText = memo(function MessageText({
	text,
	resolveNodeName,
	onFocusNode,
}: Readonly<{
	text: string;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const segments = useMemo(() => parseChatSegments(text), [text]);
	const rendered: ReactNode[] = [];
	let offset = 0;
	for (const segment of segments) {
		const key = `${segment.kind}@${offset}`;
		offset += segment.value.length;
		if (segment.kind === "link") {
			rendered.push(
				<a
					key={key}
					href={segment.value}
					target="_blank"
					rel="noopener noreferrer"
					className="break-all text-primary underline underline-offset-2"
				>
					{segment.value}
				</a>,
			);
		} else if (segment.kind === "node") {
			rendered.push(
				<NodeChip
					key={key}
					nodeId={segment.value}
					resolveNodeName={resolveNodeName}
					onFocusNode={onFocusNode}
				/>,
			);
		} else {
			rendered.push(segment.value);
		}
	}
	return <>{rendered}</>;
});

const NodeChip = memo(function NodeChip({
	nodeId,
	resolveNodeName,
	onFocusNode,
}: Readonly<{
	nodeId: string;
	resolveNodeName?: (nodeId: string) => string | undefined;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const label = `@${nodeReferenceLabel(nodeId, resolveNodeName)}`;
	const chipClass =
		"inline-flex max-w-full items-center truncate rounded-sm bg-primary/10 px-1 align-baseline text-[11px] font-medium text-primary";
	if (!onFocusNode) return <span className={chipClass}>{label}</span>;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={() => onFocusNode(nodeId)}
					aria-label={t("focusNodeNamed", "Focus {{name}}", { name: label })}
					className={cn(chipClass, "hover:bg-primary/20")}
				>
					{label}
				</button>
			</TooltipTrigger>
			<TooltipContent side="top">{t("focusNode", "Focus node")}</TooltipContent>
		</Tooltip>
	);
});

const TypingIndicator = memo(function TypingIndicator({
	parts,
}: Readonly<{ parts: TypingLabelParts }>) {
	const { t } = useTranslation("flow");
	const label =
		parts.kind === "one"
			? t("isTyping", "{{name}} is typing…", { name: parts.name })
			: parts.kind === "two"
				? t("twoAreTyping", "{{first}} and {{second}} are typing…", {
						first: parts.first,
						second: parts.second,
					})
				: parts.kind === "many"
					? t("countPeopleTyping", {
							defaultValue_one: "{{count}} person is typing…",
							defaultValue_other: "{{count}} people are typing…",
							count: parts.count,
						})
					: undefined;
	return (
		<div
			aria-live="polite"
			className="flex h-4 shrink-0 items-center gap-1.5 px-3 text-[10px] text-muted-foreground"
		>
			{label && (
				<>
					<span className="flex items-center gap-0.5" aria-hidden="true">
						<span className="size-1 rounded-full bg-muted-foreground animate-pulse motion-reduce:animate-none" />
						<span className="size-1 rounded-full bg-muted-foreground animate-pulse [animation-delay:150ms] motion-reduce:animate-none" />
						<span className="size-1 rounded-full bg-muted-foreground animate-pulse [animation-delay:300ms] motion-reduce:animate-none" />
					</span>
					<span className="truncate">{label}</span>
				</>
			)}
		</div>
	);
});
