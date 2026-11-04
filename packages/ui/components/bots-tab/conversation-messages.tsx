"use client";

import { useTranslation } from "@flow-like/locales";
import { MailIcon } from "lucide-react";
import { useMemo } from "react";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { IdentityHoverCard } from "../identity/identity-hover-card";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { CallLine, type ThreadCall } from "./conversation-call";
import { ConversationNotePill } from "./conversation-note";
import { activityTimeKind } from "./crew-model";
import type {
	ConversationBlock,
	ConversationMemberEvent,
	ConversationMessage,
} from "./fold-entries";
import { BotSenderChip, type MessageActions } from "./message-row";
import { SpokenLines } from "./spoken-lines";
import type { ConversationText } from "./use-conversation-text";
import type { CrewDirectory } from "./use-crew-directory";

export interface PendingSend {
	messageId: string;
	text: string;
	mentions?: readonly string[];
	replyTo?: string;
	files?: readonly File[];
	failed: boolean;
}

/** What every row of one conversation needs to render and act on itself. */
/** Which messages you starred, and which are pinned for everyone and by whom. */
export interface ThreadMarks {
	starred: ReadonlySet<string>;
	pinnedBy: ReadonlyMap<string, string>;
}

export interface ThreadContext {
	conversation: IConversation;
	me?: string | null;
	directory: CrewDirectory;
	text: ConversationText;
	actions: MessageActions;
	marks: ThreadMarks;
	readOnly: boolean;
	call?: ThreadCall;
}

export interface ThreadHistory {
	isLoading: boolean;
	loadError: boolean;
	hasOlder: boolean;
	loadingOlder: boolean;
	onLoadOlder: () => void;
	onRetry: () => void;
}

export interface PendingSendActions {
	retry: (send: PendingSend) => void;
	discard: (send: PendingSend) => void;
}

function DayDivider({ time }: Readonly<{ time: number }>) {
	const { t, i18n } = useTranslation("bots");
	const label = useMemo(() => {
		const kind = activityTimeKind(time, new Date());
		if (kind === "today") return t("crew.time.today", "Today");
		if (kind === "yesterday") return t("crew.time.yesterday", "Yesterday");
		return new Intl.DateTimeFormat(i18n.language, {
			weekday: "long",
			day: "numeric",
			month: "long",
		}).format(time);
	}, [time, t, i18n.language]);
	return (
		<div className="flex items-center gap-2.5 text-xs text-muted-foreground">
			<span className="h-px flex-1 bg-border" />
			{label}
			<span className="h-px flex-1 bg-border" />
		</div>
	);
}

function MemberLine({
	event,
	directory,
}: Readonly<{ event: ConversationMemberEvent; directory: CrewDirectory }>) {
	const { t } = useTranslation("bots");
	const subject = (id: string) =>
		directory(id).isMe ? t("crew.you", "You") : directory(id).name;
	const object = (id: string) =>
		directory(id).isMe ? t("crew.youObject", "you") : directory(id).name;
	const self = event.by === event.principalId;
	const name = self ? subject(event.principalId) : object(event.principalId);
	const by = subject(event.by);
	let line: string;
	if (event.op === "join") {
		line = self
			? t("crew.members.joined", "{{name}} joined", { name })
			: t("crew.members.added", "{{by}} added {{name}}", { by, name });
	} else {
		line = self
			? t("crew.members.left", "{{name}} left", { name })
			: t("crew.members.removed", "{{by}} removed {{name}}", { by, name });
	}
	return (
		<p className="px-3 text-center text-xs leading-normal text-muted-foreground">
			{line}
		</p>
	);
}

/** The bot of a direct conversation, whose point of view its notes take. */
function directBotOf(conversation: IConversation): string | undefined {
	if (conversation.kind !== "direct") return undefined;
	return conversation.members.find((member) => member.kind === "bot")
		?.principal_id;
}

/** The line above a run of one speaker's messages. */
export function SpeakerLine({
	block,
	thread,
}: Readonly<{
	block: Extract<ConversationBlock, { type: "group" }>;
	thread: ThreadContext;
}>) {
	const { conversation, directory, text } = thread;
	const first = block.messages[0] as ConversationMessage;
	const isBotMember = conversation.members.some(
		(member) => member.principal_id === block.senderId && member.kind === "bot",
	);
	return (
		<div className="mb-1 flex min-w-0 items-center gap-2 px-4">
			<IdentityHoverCard subject={block.senderId} className="shrink-0">
				<IdentityAvatar subject={block.senderId} size={22} />
			</IdentityHoverCard>
			<IdentityHoverCard subject={block.senderId}>
				<span className="truncate text-[13px] font-semibold">
					{directory(block.senderId).name}
				</span>
			</IdentityHoverCard>
			<BotSenderChip
				senderId={block.senderId}
				isBotMember={isBotMember}
				me={thread.me}
				directory={directory}
			/>
			<time
				dateTime={first.createdAt}
				className="text-[11px] text-muted-foreground"
			>
				{text.formats.time.format(first.time)}
			</time>
		</div>
	);
}

/** Above a message the bot sent as an email, so it doesn't read as a chat reply. */
export function MailSentLine({ to }: Readonly<{ to: readonly string[] }>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex min-w-0 items-center gap-1 px-4 text-[11px] text-muted-foreground">
			<MailIcon className="size-3 shrink-0" aria-hidden />
			<span className="truncate">
				{t("crew.messages.sentByEmail", "Sent by email to {{recipients}}", {
					recipients: to.join(", "),
				})}
			</span>
		</div>
	);
}

/** Where an unconfirmed send stands, under the message the chat shows for it. */
export function PendingStatus({
	send,
	actions,
}: Readonly<{ send: PendingSend; actions: PendingSendActions }>) {
	const { t } = useTranslation("bots");
	return send.failed ? (
		<span className="flex items-center gap-1 px-4 text-[11px] text-destructive">
			{t("crew.messages.notSent", "Not sent")}
			<Button
				type="button"
				variant="link"
				size="sm"
				className="h-auto p-0 text-[11px]"
				onClick={() => actions.retry(send)}
			>
				{t("crew.messages.retry", "Retry")}
			</Button>
			<Button
				type="button"
				variant="link"
				size="sm"
				className="h-auto p-0 text-[11px] text-muted-foreground"
				onClick={() => actions.discard(send)}
			>
				{t("crew.messages.discard", "Discard")}
			</Button>
		</span>
	) : (
		<span className="px-4 text-[11px] text-muted-foreground">
			{t("crew.messages.sending", "Sending…")}
		</span>
	);
}

export function HistoryState({
	history,
	empty,
}: Readonly<{ history: ThreadHistory; empty: boolean }>) {
	const { t } = useTranslation("bots");
	if (history.isLoading) {
		return (
			<div className="flex flex-col gap-3">
				<Skeleton className="h-10 w-3/4 rounded-xl" />
				<Skeleton className="ml-auto h-10 w-1/2 rounded-xl" />
				<Skeleton className="h-16 w-2/3 rounded-xl" />
			</div>
		);
	}
	if (history.loadError) {
		return (
			<div className="flex flex-col items-center gap-2 text-center text-sm text-muted-foreground">
				{t("crew.messages.loadFailed", "Couldn't load these messages.")}
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={history.onRetry}
				>
					{t("crew.retry", "Try again")}
				</Button>
			</div>
		);
	}
	if (!empty) return null;
	return (
		<p className="py-10 text-center text-sm text-muted-foreground">
			{t("crew.messages.empty", "No messages yet. Say hello.")}
		</p>
	);
}

/** A non-message block (day, membership, trigger, bot ask) shown between messages. */
export function ThreadNote({
	block,
	thread,
}: Readonly<{ block: ConversationBlock; thread: ThreadContext }>) {
	if (block.type === "day") return <DayDivider time={block.time} />;
	if (block.type === "member") {
		return <MemberLine event={block.event} directory={thread.directory} />;
	}
	if (block.type === "call") {
		return (
			<CallLine
				call={block.call}
				directory={thread.directory}
				thread={thread.call}
			/>
		);
	}
	if (block.type === "spoken") {
		return <SpokenLines block={block} directory={thread.directory} />;
	}
	if (block.type === "note") {
		return (
			<ConversationNotePill
				note={block.note}
				viewerBotId={directBotOf(thread.conversation)}
			/>
		);
	}
	return null;
}
