"use client";

import { useTranslation } from "@flow-like/locales";
import { BellIcon, BellOffIcon, UsersIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import {
	compareInboxCursors,
	newInboxMessageId,
	useConversation,
	useInboxEntries,
	useMarkConversationRead,
	useSendInboxEntry,
	useSetConversationMuted,
} from "../../hooks/use-inbox";
import { LocalPrincipalsProvider } from "../../hooks/use-user-lookup";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import type {
	IConversation,
	IInboxDeleteBody,
	IInboxEditBody,
	IInboxEntry,
	IInboxMessageBody,
	IInboxReactionBody,
	ISendInboxEntryRequest,
} from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { ConversationComposer } from "./conversation-composer";
import {
	ConversationMessages,
	type PendingSendActions,
	type ThreadContext,
} from "./conversation-messages";
import {
	awaitingAnswer,
	conversationComposition,
	isSelfPrincipal,
	otherMemberIds,
} from "./crew-model";
import { DockHeader } from "./dock-header";
import {
	type ConversationItem,
	foldInboxEntries,
	groupConversationItems,
} from "./fold-entries";
import { useLocalPrincipals } from "./local-principals";
import { MemberStack } from "./member-stack";
import { mentionablesOf } from "./mentions";
import { DeleteMessageDialog, type MessageActions } from "./message-row";
import {
	discardPendingSend,
	sendPendingMessage,
	usePendingSendList,
	useShowsPendingSends,
} from "./pending-sends";
import type { PickerSource } from "./principal-picker";
import { RoomMembersDialog } from "./room-members-dialog";
import {
	type ConversationText,
	useConversationText,
} from "./use-conversation-text";
import { useCrewDirectory } from "./use-crew-directory";

type PlainBody =
	| IInboxMessageBody
	| IInboxEditBody
	| IInboxDeleteBody
	| IInboxReactionBody;

type SendPlain = (
	kind: ISendInboxEntryRequest["kind"],
	body: PlainBody,
	messageId?: string,
) => Promise<IInboxEntry>;

function useSendPlain(conversationId: string): SendPlain {
	const { mutateAsync } = useSendInboxEntry();
	return useCallback(
		(kind, body, messageId = newInboxMessageId()) =>
			mutateAsync({
				conversationId,
				request: {
					message_id: messageId,
					kind,
					envelope: { v: 1, enc: "none", body },
				},
			}),
		[conversationId, mutateAsync],
	);
}

function subscribeVisibility(onChange: () => void) {
	document.addEventListener("visibilitychange", onChange);
	return () => document.removeEventListener("visibilitychange", onChange);
}

function useDocumentVisible() {
	return useSyncExternalStore(
		subscribeVisibility,
		() => document.visibilityState === "visible",
		() => true,
	);
}

/**
 * Marks what the reader can see as read: on open, and whenever someone else adds something newer
 * than what this view last marked. The reader's own entries never trigger it.
 */
function useMarkReadWhileOpen(
	conversationId: string,
	entries: readonly IInboxEntry[],
	me?: string | null,
) {
	const { mutate } = useMarkConversationRead();
	const visible = useDocumentVisible();
	const marked = useRef<{ conversationId: string; upTo: string } | null>(null);
	const newest = entries[0]?.cursor;
	const newestFromOthers = useMemo(
		() =>
			entries.find((entry) => !isSelfPrincipal(entry.sender_id, me))?.cursor,
		[entries, me],
	);
	useEffect(() => {
		if (!visible || !newest || !newestFromOthers) return;
		const previous = marked.current;
		if (
			previous?.conversationId === conversationId &&
			compareInboxCursors(newestFromOthers, previous.upTo) <= 0
		)
			return;
		const next = { conversationId, upTo: newest };
		marked.current = next;
		mutate(next, {
			onError: () => {
				if (marked.current === next) marked.current = previous;
			},
		});
	}, [conversationId, newest, newestFromOthers, visible, mutate]);
}

const AUTO_LOAD_PAGES = 5;

/** Loads older pages on its own while the loaded ones hold no message, e.g. only reactions. */
function useFillEmptyHistory(
	hasMessages: boolean,
	{
		data,
		hasNextPage,
		isFetching,
		isError,
		fetchNextPage,
	}: Readonly<{
		data?: { pages: readonly unknown[] };
		hasNextPage: boolean;
		isFetching: boolean;
		isError: boolean;
		fetchNextPage: () => Promise<unknown>;
	}>,
) {
	const pageCount = data?.pages.length ?? 0;
	useEffect(() => {
		if (
			hasMessages ||
			!hasNextPage ||
			isFetching ||
			isError ||
			pageCount > AUTO_LOAD_PAGES
		)
			return;
		void fetchNextPage();
	}, [hasMessages, hasNextPage, isFetching, isError, pageCount, fetchNextPage]);
}

/** Messages shown right away while they travel; a failed one waits for retry or discard. */
function usePendingSends(
	conversationId: string,
	sendPlain: SendPlain,
	items: ConversationItem[],
	onFailedUnseen: () => void,
) {
	useShowsPendingSends(conversationId);
	const pending = usePendingSendList(conversationId);

	const send = useCallback(
		(
			text: string,
			mentions: readonly string[] = [],
			messageId = newInboxMessageId(),
		) =>
			sendPendingMessage(
				conversationId,
				{ messageId, text, mentions },
				() =>
					sendPlain(
						"message",
						mentions.length > 0 ? { text, mentions: [...mentions] } : { text },
						messageId,
					),
				onFailedUnseen,
			),
		[conversationId, sendPlain, onFailedUnseen],
	);

	const actions = useMemo<PendingSendActions>(
		() => ({
			retry: (item) => void send(item.text, item.mentions, item.messageId),
			discard: (item) => discardPendingSend(conversationId, item.messageId),
		}),
		[conversationId, send],
	);

	const visible = useMemo(() => {
		const shown = new Set(items.map((item) => item.id));
		return pending.filter((item) => !shown.has(item.messageId));
	}, [items, pending]);

	return { visible, send, actions };
}

/** Title and subtitle of a conversation. */
function useConversationHeading(
	conversation: IConversation,
	text: ConversationText,
) {
	const { t } = useTranslation("bots");
	const { people, bots } = conversationComposition(conversation);
	const peopleText = t("crew.rooms.people", "{{count}} people", {
		count: people,
		defaultValue_one: "{{count}} person",
	});
	const botsText = t("crew.rooms.bots", "{{count}} bots", {
		count: bots,
		defaultValue_one: "{{count}} bot",
	});
	let subtitle = t("crew.directMessage", "Direct message");
	if (conversation.kind === "group") {
		subtitle =
			bots === 0
				? t("crew.dock.groupSubtitlePeople", "Group · {{people}}", {
						people: peopleText,
					})
				: t("crew.dock.groupSubtitle", "Group · {{people}}, {{bots}}", {
						people: peopleText,
						bots: botsText,
					});
	}
	return {
		title: text.title(conversation),
		subtitle: conversation.local
			? t("local.conversationSubtitle", "{{subtitle}} · On this device", {
					subtitle,
				})
			: subtitle,
	};
}

function MuteButton({
	conversationId,
	muted,
}: Readonly<{ conversationId: string; muted: boolean }>) {
	const { t } = useTranslation("bots");
	const { mutate, isPending } = useSetConversationMuted();
	const label = muted
		? t("crew.mute.unmute", "Unmute notifications")
		: t("crew.mute.mute", "Mute notifications");
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="shrink-0 text-muted-foreground"
			disabled={isPending}
			aria-pressed={muted}
			onClick={() => mutate({ conversationId, muted: !muted })}
			aria-label={label}
			title={label}
		>
			{muted ? <BellOffIcon aria-hidden /> : <BellIcon aria-hidden />}
		</Button>
	);
}

function ConversationView({
	conversation,
	me,
	bots,
	onClose,
}: Readonly<{
	conversation: IConversation;
	me?: string | null;
	bots?: readonly IBot[];
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const entriesQuery = useInboxEntries(conversation.id);
	const pages = entriesQuery.data?.pages;
	const entries = useMemo(
		() => pages?.flatMap((page) => page.entries) ?? [],
		[pages],
	);
	const items = useMemo(() => foldInboxEntries(entries, me), [entries, me]);
	const blocks = useMemo(() => groupConversationItems(items), [items]);
	useFillEmptyHistory(
		items.some((item) => item.type === "message"),
		entriesQuery,
	);
	const principalIds = useMemo(
		() => [
			...conversation.members.map((member) => member.principal_id),
			...(conversation.pending ?? []).map((member) => member.principal_id),
			...entries.map((entry) => entry.sender_id),
			...items.flatMap((item) =>
				item.type === "member" ? [item.principalId, item.by] : [],
			),
		],
		[conversation.members, conversation.pending, entries, items],
	);
	const directory = useCrewDirectory(principalIds, { me, bots });
	const text = useConversationText(directory, me);
	const heading = useConversationHeading(conversation, text);
	const [membersOpen, setMembersOpen] = useState(false);
	const pickerSource = useMemo<PickerSource>(
		() => ({
			me,
			bots: bots ?? [],
			recentPeopleIds: [],
			directory,
			people: true,
		}),
		[bots, directory, me],
	);
	const manageable = conversation.kind === "group" && !conversation.local;
	const personChat =
		!conversation.local &&
		conversation.members.filter((member) => member.kind === "user").length > 1;
	useMarkReadWhileOpen(conversation.id, entries, me);

	const mentionables = useMemo(
		() => mentionablesOf(conversation, directory, me),
		[conversation, directory, me],
	);
	const requestHint = awaitingAnswer(conversation)
		? t(
				"crew.requests.pendingHint",
				"Pending: they haven't accepted your message yet",
			)
		: null;

	const sendPlain = useSendPlain(conversation.id);
	const notifyUnseenFailure = useCallback(
		() =>
			toast.error(t("crew.messages.notSent", "Not sent"), {
				description: heading.title,
			}),
		[t, heading.title],
	);
	const pending = usePendingSends(
		conversation.id,
		sendPlain,
		items,
		notifyUnseenFailure,
	);
	const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

	const failed = useCallback(
		(error: unknown) => {
			console.warn("Conversation update failed", error);
			toast.error(
				t("crew.messages.actionFailed", "That didn't go through. Try again."),
			);
		},
		[t],
	);
	const actions = useMemo<MessageActions>(
		() => ({
			react: (target, emoji, op) => {
				sendPlain("reaction", { target, emoji, op }).catch(failed);
			},
			edit: (target, body) =>
				sendPlain("edit", { target, text: body }).catch((error: unknown) => {
					failed(error);
					throw error;
				}),
			requestDelete: setDeleteTarget,
		}),
		[failed, sendPlain],
	);
	const confirmDelete = useCallback(() => {
		if (deleteTarget) {
			sendPlain("delete", { target: deleteTarget }).catch(failed);
		}
		setDeleteTarget(null);
	}, [deleteTarget, failed, sendPlain]);

	const encrypted = conversation.encryption === "e2e";
	const thread: ThreadContext = {
		conversation,
		me,
		directory,
		text,
		actions,
		readOnly: encrypted,
	};

	return (
		<>
			<DockHeader
				avatar={
					<MemberStack
						principalIds={otherMemberIds(conversation, me)}
						max={3}
						size={34}
					/>
				}
				title={heading.title}
				subtitle={heading.subtitle}
				encrypted={encrypted}
				actions={
					<>
						{personChat && (
							<MuteButton
								conversationId={conversation.id}
								muted={Boolean(conversation.muted)}
							/>
						)}
						{manageable && (
							<Button
								type="button"
								variant="ghost"
								size="icon"
								className="shrink-0 text-muted-foreground"
								onClick={() => setMembersOpen(true)}
								aria-label={t("roomMembers.open", "Members and invitations")}
								title={t("roomMembers.open", "Members and invitations")}
							>
								<UsersIcon aria-hidden />
							</Button>
						)}
					</>
				}
				onClose={onClose}
			/>
			{manageable && (
				<RoomMembersDialog
					open={membersOpen}
					onOpenChange={setMembersOpen}
					conversation={conversation}
					directory={directory}
					source={pickerSource}
					me={me}
				/>
			)}
			<ConversationMessages
				blocks={blocks}
				pending={pending.visible}
				thread={thread}
				history={{
					isLoading: entriesQuery.isPending,
					loadError: entriesQuery.isError && !pages,
					hasOlder: Boolean(entriesQuery.hasNextPage),
					loadingOlder: entriesQuery.isFetchingNextPage,
					onLoadOlder: () => void entriesQuery.fetchNextPage(),
					onRetry: () => void entriesQuery.refetch(),
				}}
				sends={pending.actions}
			/>
			<ConversationComposer
				recipient={heading.title}
				disabledReason={
					encrypted
						? t(
								"crew.composer.encrypted",
								"Writing in end-to-end encrypted chats isn't available in this app yet.",
							)
						: null
				}
				mentionables={mentionables}
				hint={requestHint}
				onSend={(body, mentions) => void pending.send(body, mentions)}
			/>
			<DeleteMessageDialog
				open={deleteTarget !== null}
				onOpenChange={(open) => {
					if (!open) setDeleteTarget(null);
				}}
				onConfirm={confirmDelete}
			/>
		</>
	);
}

function ConversationUnavailable({
	failed,
	onRetry,
	onClose,
}: Readonly<{ failed: boolean; onRetry: () => void; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			<DockHeader
				title={
					failed
						? t("crew.dock.unavailableTitle", "Conversation unavailable")
						: t("crew.dock.loading", "Loading…")
				}
				onClose={onClose}
			/>
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-sm text-muted-foreground">
				{failed ? (
					<>
						<p>
							{t(
								"crew.dock.conversationUnavailable",
								"This conversation isn't available. You may have left it, or it no longer exists.",
							)}
						</p>
						<Button type="button" variant="outline" size="sm" onClick={onRetry}>
							{t("crew.retry", "Try again")}
						</Button>
					</>
				) : (
					<div className="flex w-full flex-col gap-3">
						<Skeleton className="h-10 w-3/4 rounded-xl" />
						<Skeleton className="ml-auto h-10 w-1/2 rounded-xl" />
					</div>
				)}
			</div>
		</>
	);
}

const everyoneButYou = (principalId: string) => principalId !== LOCAL_OWNER_ID;

/** In a chat on this device everyone but you is a local bot, even one that is gone. */
function LocalConversationPrincipals({
	bots,
	children,
}: Readonly<{ bots?: readonly IBot[]; children: ReactNode }>) {
	const resolve = useLocalPrincipals(bots, everyoneButYou);
	return (
		<LocalPrincipalsProvider value={resolve}>
			{children}
		</LocalPrincipalsProvider>
	);
}

export function ConversationPanel({
	conversationId,
	me,
	bots,
	onClose,
}: Readonly<{
	conversationId: string;
	me?: string | null;
	bots?: readonly IBot[];
	onClose: () => void;
}>) {
	const query = useConversation(conversationId);
	if (!query.data) {
		return (
			<ConversationUnavailable
				failed={query.isError}
				onRetry={() => void query.refetch()}
				onClose={onClose}
			/>
		);
	}
	const view = (
		<ConversationView
			key={query.data.id}
			conversation={query.data}
			me={me}
			bots={bots}
			onClose={onClose}
		/>
	);
	return query.data.local ? (
		<LocalConversationPrincipals bots={bots}>
			{view}
		</LocalConversationPrincipals>
	) : (
		view
	);
}
