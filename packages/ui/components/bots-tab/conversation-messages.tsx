"use client";

import { useTranslation } from "@flow-like/locales";
import { AlarmClockIcon, MailIcon, WebhookIcon, ZapIcon } from "lucide-react";
import { useMemo } from "react";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { activityTimeKind } from "./crew-model";
import type {
	ConversationBlock,
	ConversationMemberEvent,
	ConversationMessage,
	ConversationTrigger,
} from "./fold-entries";
import { BotSenderChip, type MessageActions, MessageRow } from "./message-row";
import type { ConversationText } from "./use-conversation-text";
import type { CrewDirectory } from "./use-crew-directory";

export interface PendingSend {
	messageId: string;
	text: string;
	failed: boolean;
}

/** What every row of one conversation needs to render and act on itself. */
export interface ThreadContext {
	conversation: IConversation;
	me?: string | null;
	directory: CrewDirectory;
	text: ConversationText;
	actions: MessageActions;
	readOnly: boolean;
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

const TRIGGER_ICONS = {
	schedule: AlarmClockIcon,
	webhook: WebhookIcon,
	email: MailIcon,
	event: ZapIcon,
} as const;

/** A compact card for work that woke the bot, e.g. "Scheduled: Morning brief". */
export function TriggerCard({
	trigger,
}: Readonly<{ trigger: ConversationTrigger }>) {
	const { t } = useTranslation("bots");
	const Icon = TRIGGER_ICONS[trigger.source];
	const labels = {
		schedule: t("crew.trigger.schedule", "Scheduled: {{title}}", {
			title: trigger.title,
		}),
		webhook: t("crew.trigger.webhook", "Webhook: {{title}}", {
			title: trigger.title,
		}),
		email: t("crew.trigger.email", "Email: {{title}}", {
			title: trigger.title,
		}),
		event: t("crew.trigger.event", "Event: {{title}}", {
			title: trigger.title,
		}),
	};
	return (
		<div
			data-slot="trigger-card"
			className="mx-auto flex max-w-[85%] items-center gap-1.5 rounded-full border bg-muted/40 px-3 py-1 text-xs text-muted-foreground"
		>
			<Icon className="size-3.5 shrink-0" aria-hidden />
			<span className="truncate">{labels[trigger.source]}</span>
		</div>
	);
}

function MessageGroup({
	block,
	thread,
}: Readonly<{
	block: Extract<ConversationBlock, { type: "group" }>;
	thread: ThreadContext;
}>) {
	const { conversation, directory, text } = thread;
	const first = block.messages[0] as ConversationMessage;
	const last = block.messages[block.messages.length - 1] as ConversationMessage;
	const rows = block.messages.map((message) => (
		<MessageRow
			key={message.id}
			message={message}
			directory={directory}
			actions={thread.actions}
			readOnly={thread.readOnly}
		/>
	));
	if (block.mine) {
		return (
			<div className="flex flex-col items-end gap-1">
				{rows}
				<span className="text-[11px] text-muted-foreground">
					{text.formats.time.format(last.time)}
				</span>
			</div>
		);
	}
	const isBotMember = conversation.members.some(
		(member) => member.principal_id === block.senderId && member.kind === "bot",
	);
	return (
		<div className="flex items-start gap-2.5">
			<IdentityAvatar subject={block.senderId} size={30} />
			<div className="flex min-w-0 flex-1 flex-col gap-0.5">
				<div className="flex flex-wrap items-center gap-1.5">
					<span className="text-[13px] font-semibold">
						{directory(block.senderId).name}
					</span>
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
				{rows}
			</div>
		</div>
	);
}

function PendingBubble({
	send,
	actions,
}: Readonly<{ send: PendingSend; actions: PendingSendActions }>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-col items-end gap-1">
			<div className="max-w-[85%] rounded-[14px] rounded-br-sm bg-secondary px-3 py-2 text-sm leading-relaxed text-secondary-foreground opacity-70">
				<span className="wrap-break-word whitespace-pre-wrap">{send.text}</span>
			</div>
			{send.failed ? (
				<span className="flex items-center gap-1 text-[11px] text-destructive">
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
				<span className="text-[11px] text-muted-foreground">
					{t("crew.messages.sending", "Sending…")}
				</span>
			)}
		</div>
	);
}

function HistoryState({
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

function ThreadBlock({
	block,
	thread,
}: Readonly<{ block: ConversationBlock; thread: ThreadContext }>) {
	if (block.type === "day") return <DayDivider time={block.time} />;
	if (block.type === "member") {
		return <MemberLine event={block.event} directory={thread.directory} />;
	}
	if (block.type === "trigger") return <TriggerCard trigger={block.trigger} />;
	return <MessageGroup block={block} thread={thread} />;
}

/** The scrolling message log. Column-reverse keeps it pinned to the newest message. */
export function ConversationMessages({
	blocks,
	pending,
	thread,
	history,
	sends,
}: Readonly<{
	blocks: readonly ConversationBlock[];
	pending: readonly PendingSend[];
	thread: ThreadContext;
	history: ThreadHistory;
	sends: PendingSendActions;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex min-h-0 flex-1 flex-col-reverse overflow-y-auto overscroll-contain">
			<div
				role="log"
				aria-label={t("crew.messages.label", "Messages")}
				className="flex flex-col gap-4 px-4.5 pt-4 pb-5"
			>
				{history.hasOlder && (
					<div className="flex justify-center">
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={history.loadingOlder}
							onClick={history.onLoadOlder}
						>
							{history.loadingOlder
								? t("crew.messages.loadingOlder", "Loading…")
								: t("crew.messages.loadOlder", "Load older messages")}
						</Button>
					</div>
				)}
				<HistoryState
					history={history}
					empty={
						blocks.length === 0 && pending.length === 0 && !history.hasOlder
					}
				/>
				{blocks.map((block) => (
					<ThreadBlock key={block.key} block={block} thread={thread} />
				))}
				{pending.map((send) => (
					<PendingBubble key={send.messageId} send={send} actions={sends} />
				))}
			</div>
		</div>
	);
}
