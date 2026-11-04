"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useCallback, useMemo, useRef } from "react";
import type { ConversationActivity } from "../../hooks/use-conversation-activity";
import { IRole } from "../../lib/schema/llm/history";
import { GLOBAL_CHAT_APP_ID } from "../../state/global-chat/global-chat-db";
import { PlainUnresolvedMentionsContext } from "../editor/ui/mention-node-static";
import type { IChatRef } from "../interfaces/chat-default/chat";
import type { IChatGroup } from "../interfaces/chat-default/chat";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { ChatSurface } from "../interfaces/chat-default/chat-surface";
import type { ISendMessageFunction } from "../interfaces/chat-default/chatbox";
import { ChatFeedbackEnabledContext } from "../interfaces/chat-default/message";
import { Button } from "../ui/button";
import { ConversationActivityLine } from "./conversation-activity-line";
import {
	HistoryState,
	MailSentLine,
	type PendingSend,
	type PendingSendActions,
	PendingStatus,
	SpeakerLine,
	type ThreadContext,
	type ThreadHistory,
	ThreadNote,
} from "./conversation-messages";
import type { ConversationBlock, ConversationMessage } from "./fold-entries";
import { type Mentionable, mentionedIn } from "./mentions";
import { MessageMarkLine, MessageMenu } from "./message-marks";
import { roomChatMessage } from "./room-message-body";
import { useRoomAttachments } from "./use-room-attachments";

const noWidgetAction = async () => undefined;
const NO_BOTS: ConversationActivity["bots"] = [];
const NO_TYPING: ConversationActivity["typing"] = [];

/**
 * A room or exchange on the standard chat surface: the same list, message components, composer and
 * hover actions as a bot's own chat. What a group adds rides in `IChatGroup`: a speaker line where
 * the speaker changes, notes between messages, pin and star marks, a star/pin/delete menu and the
 * `@` picker.
 */
export function ConversationChat({
	blocks,
	pending,
	thread,
	history,
	sends,
	placeholder,
	disabledReason,
	mentionables,
	activity,
	onSend,
}: Readonly<{
	blocks: readonly ConversationBlock[];
	pending: readonly PendingSend[];
	thread: ThreadContext;
	history: ThreadHistory;
	sends: PendingSendActions;
	placeholder: string;
	disabledReason?: string | null;
	mentionables?: readonly Mentionable[];
	activity?: ConversationActivity;
	onSend: (
		text: string,
		mentions: string[],
		replyTo?: string,
		files?: readonly File[],
	) => void;
}>) {
	const { t } = useTranslation("bots");
	const chatRef = useRef<IChatRef>(null);
	const { directory, actions, marks, readOnly } = thread;
	const conversationId = thread.conversation.id;

	const errorTitle = t("crew.messages.errorTitle", "Couldn't answer");
	const sealed = t("crew.messages.encrypted", "Encrypted message");
	const unsupported = t(
		"crew.messages.unsupported",
		"Message this app can't show",
	);
	const editedLabel = t("crew.messages.edited", "(edited)");

	const allMessages = useMemo(
		() =>
			blocks.flatMap((block) => (block.type === "group" ? block.messages : [])),
		[blocks],
	);
	const attachments = useRoomAttachments(conversationId, allMessages);
	const chatConfig = useMemo(
		() => ({ allow_file_upload: !readOnly }),
		[readOnly],
	);

	const { messages, byId, headers, trailing, runStarts, lastMine } =
		useMemo(() => {
			const list: ConversationMessage[] = [];
			const heads = new Map<string, ReactNode[]>();
			const starts = new Set<string>();
			let notes: ReactNode[] = [];
			for (const block of blocks) {
				if (block.type !== "group") {
					notes.push(
						<ThreadNote key={block.key} block={block} thread={thread} />,
					);
					continue;
				}
				block.messages.forEach((message, index) => {
					const lead = [...notes];
					notes = [];
					if (index === 0 && !block.mine) {
						lead.push(
							<SpeakerLine
								key={`s:${block.key}`}
								block={block}
								thread={thread}
							/>,
						);
					}
					if (lead.length > 0) heads.set(message.id, lead);
					if (index === 0 || lead.length > 0) starts.add(message.id);
					list.push(message);
				});
			}
			const known = new Map(list.map((message) => [message.id, message]));
			const you = t("crew.you", "You");
			const chat = list.map((message) =>
				roomChatMessage(message, conversationId, {
					errorTitle,
					editedLabel,
					reactorName: (id) => (directory(id).isMe ? you : directory(id).name),
					replyTarget: message.replyTo ? known.get(message.replyTo) : undefined,
					files: attachments.get(message.id),
					notice: message.content.kind === "sealed" ? sealed : unsupported,
					mentions: message.mentions.map((id) => ({
						id,
						name: directory(id).name,
					})),
				}),
			);
			return {
				messages: chat,
				byId: new Map(list.map((message) => [message.id, message])),
				headers: heads,
				trailing: notes,
				runStarts: starts,
				lastMine: list.at(-1)?.mine,
			};
		}, [
			blocks,
			thread,
			conversationId,
			directory,
			errorTitle,
			editedLabel,
			sealed,
			unsupported,
			attachments,
			t,
		]);

	const pendingMessages = useMemo<IMessage[]>(
		() =>
			pending.map((send, index) => {
				const target = send.replyTo ? byId.get(send.replyTo) : undefined;
				return {
					id: send.messageId,
					appId: GLOBAL_CHAT_APP_ID,
					sessionId: conversationId,
					inner: {
						role: IRole.User,
						content: send.text,
					},
					...(target?.content.kind === "text"
						? {
								interaction: {
									type: "reply" as const,
									target: {
										messageId: target.id,
										role: target.mine ? IRole.User : IRole.Assistant,
										text: target.content.text,
									},
								},
							}
						: {}),
					files: [],
					timestamp: Number.MAX_SAFE_INTEGER - pending.length + index,
				};
			}),
		[pending, conversationId, byId],
	);

	const firstPendingId = pending[0]?.messageId;
	const lastWasOthers = lastMine === false;

	const olderButton = history.hasOlder && (
		<div key="older" className="flex justify-center">
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
	);

	const group = useMemo<IChatGroup>(
		() => ({
			layout: "room",
			runStart: (message) =>
				runStarts.has(message.id) ||
				(message.id === firstPendingId && lastWasOthers),
			renderMessageHeader: (message, index) => {
				const lead = headers.get(message.id);
				const older = index === 0 ? olderButton : null;
				const pinner = marks.pinnedBy.get(message.id);
				const starred = marks.starred.has(message.id);
				const marked = Boolean(pinner) || starred;
				const mailedTo = byId.get(message.id)?.mailedTo;
				if (!lead && !older && !marked && !mailedTo) return null;
				return (
					<div className="mb-1.5 flex flex-col gap-3">
						{older}
						{lead}
						{marked && (
							<MessageMarkLine
								pinnedBy={pinner && thread.text.senderLabel(pinner)}
								starred={starred}
								mine={Boolean(byId.get(message.id)?.mine)}
							/>
						)}
						{mailedTo && <MailSentLine to={mailedTo} />}
					</div>
				);
			},
			renderMessageMenu: readOnly
				? undefined
				: (message) => {
						const source = byId.get(message.id);
						if (!source) return null;
						const starred = marks.starred.has(source.id);
						const pinned = marks.pinnedBy.has(source.id);
						return (
							<MessageMenu
								starred={starred}
								pinned={pinned}
								onStar={() => actions.star(source.id, !starred)}
								onPin={() => actions.pin(source.id, !pinned)}
								onDelete={
									source.mine
										? () => actions.requestDelete(source.id)
										: undefined
								}
							/>
						);
					},
			mentionables,
			placeholder: disabledReason ?? placeholder,
			composerDisabled: Boolean(disabledReason),
			editable: (message) =>
				!readOnly &&
				Boolean(byId.get(message.id)?.mine) &&
				byId.get(message.id)?.content.kind === "text",
		}),
		[
			runStarts,
			firstPendingId,
			lastWasOthers,
			headers,
			olderButton,
			readOnly,
			byId,
			actions,
			marks,
			thread.text,
			mentionables,
			placeholder,
			disabledReason,
		],
	);

	const react = useCallback(
		async (message: IMessage, emoji: string, op: "add" | "remove") => {
			actions.react(message.id, emoji, op);
		},
		[actions],
	);

	const update = useCallback(
		async (messageId: string, updates: Partial<IMessage>) => {
			const content = updates.inner?.content;
			if (typeof content === "string") {
				await actions.edit(messageId, content).catch(() => undefined);
			}
		},
		[actions],
	);

	const stopTyping = activity?.stopTyping;
	const send = useCallback<ISendMessageFunction>(
		async (content, files, _tools, _audio, interaction) => {
			stopTyping?.();
			onSend(
				content,
				mentionedIn(content, mentionables ?? []),
				interaction?.type === "reply"
					? interaction.target.messageId
					: undefined,
				files,
			);
		},
		[mentionables, onSend, stopTyping],
	);

	const empty =
		blocks.length === 0 && pending.length === 0 && !history.hasOlder;
	const inline = (
		<div className="flex flex-col gap-3">
			{messages.length === 0 && olderButton}
			{trailing}
			{pending.map((item) => (
				<PendingStatus key={item.messageId} send={item} actions={sends} />
			))}
			<HistoryState history={{ ...history, isLoading: false }} empty={empty} />
		</div>
	);

	return (
		<PlainUnresolvedMentionsContext.Provider value={true}>
			<ChatFeedbackEnabledContext.Provider value={false}>
				<ChatSurface
					chatRef={chatRef}
					loading={history.isLoading}
					runWidgetAction={noWidgetAction}
					sessionId={conversationId}
					config={chatConfig}
					messages={[...messages, ...pendingMessages]}
					onSendMessage={send}
					onMessageReaction={readOnly ? undefined : react}
					onMessageUpdate={readOnly ? undefined : update}
					inlineContent={inline}
					inlinePrompt={
						<ConversationActivityLine
							bots={activity?.bots ?? NO_BOTS}
							typing={activity?.typing ?? NO_TYPING}
							directory={directory}
						/>
					}
					onDraftChange={activity?.notifyTyping}
					group={group}
				/>
			</ChatFeedbackEnabledContext.Provider>
		</PlainUnresolvedMentionsContext.Provider>
	);
}
