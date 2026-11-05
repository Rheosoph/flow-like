"use client";

import {
	type InfiniteData,
	type QueryClient,
	type QueryKey,
	useInfiniteQuery,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import { personUnreadTotal } from "../components/bots-tab/crew-model";
import { isMissingResourceError } from "../lib/api-error";
import {
	attachmentParts,
	conversationFileId,
	inBatches,
} from "../lib/conversation-files";
import { compareInboxCursors } from "../lib/inbox-cursor";
import {
	hasConversationFeed,
	watchConversation,
} from "../lib/realtime/conversation-feed";
import { useBackend } from "../state/backend-state";
import type {
	IBotRoomLimits,
	IConversation,
	IConversationFileDownload,
	ICreateConversationLinkRequest,
	ICreateConversationRequest,
	IInboxEntriesPage,
	IInboxEntry,
	IInboxFileHandle,
	IInboxMessageBody,
	IInboxState,
	IInvitation,
	IInviteRequest,
	ISendInboxEntryRequest,
	IStarredMessagesPage,
} from "../state/backend-state/inbox-state";

export const inboxKeys = {
	all: ["inbox"] as const,
	conversations: (as?: string) =>
		[...inboxKeys.all, "conversations", as ?? "me"] as const,
	conversation: (conversationId: string, as?: string) =>
		[...inboxKeys.all, "conversation", conversationId, as ?? "me"] as const,
	entries: (conversationId: string, as?: string) =>
		[...inboxKeys.all, "entries", conversationId, as ?? "me"] as const,
	turns: (conversationId: string) =>
		[...inboxKeys.all, "turns", conversationId] as const,
	invitations: () => [...inboxKeys.all, "invitations"] as const,
	invitable: (q: string, conversationId?: string) =>
		[...inboxKeys.all, "invitable", conversationId ?? "", q] as const,
	links: (conversationId: string) =>
		[...inboxKeys.all, "links", conversationId] as const,
};

/**
 * Open conversations receive new entries through the realtime feed when one is registered;
 * `openConversation` polls without it, `openConversationSafety` backs the feed up.
 */
export const INBOX_POLL_MS = {
	conversations: 15_000,
	openConversation: 4_000,
	openConversationSafety: 30_000,
} as const;

/** Matches inbox queries other than the entries of conversations the realtime feed keeps current. */
export function unwatchedInboxPredicate(
	isWatched: (conversationId: string, as?: string) => boolean,
) {
	return ({ queryKey }: { queryKey: QueryKey }) => {
		if (queryKey[0] !== inboxKeys.all[0]) return false;
		if (queryKey[1] !== "entries") return true;
		const as = queryKey[3];
		return !isWatched(
			String(queryKey[2]),
			as === "me" ? undefined : String(as),
		);
	};
}

const PAGE_SIZE = 50;

function requireInboxState(state: IInboxState | undefined): IInboxState {
	if (!state)
		throw new Error("Conversations need a signed-in Flow-Like account");
	return state;
}

/** Client-chosen id that makes a retried send idempotent. */
export function newInboxMessageId(): string {
	return `msg_${crypto.randomUUID().replaceAll("-", "")}`;
}

export { compareInboxCursors };

interface EntryPage extends IInboxEntriesPage {
	/**
	 * First page only: the newest cursor a server read returned. Sends merged locally never move
	 * it, so the tail poll can't skip entries it hasn't fetched. It lives in the cached data so it
	 * only changes when the pages it describes are committed.
	 */
	tail?: string;
}

type EntryPages = InfiniteData<EntryPage, string | undefined>;

/** Puts entries the reader has not seen at the top of the newest page, newest first, once each. */
export function mergeNewestEntries(
	pages: EntryPages | undefined,
	entries: readonly IInboxEntry[],
): EntryPages | undefined {
	if (!pages || entries.length === 0) return pages;
	const known = new Set(
		pages.pages.flatMap((page) => page.entries.map((entry) => entry.id)),
	);
	const fresh = entries.filter((entry) => !known.has(entry.id));
	if (fresh.length === 0) return pages;
	const [first, ...rest] = pages.pages;
	const merged = [...fresh, ...(first?.entries ?? [])].sort((a, b) =>
		compareInboxCursors(b.cursor, a.cursor),
	);
	return {
		...pages,
		pages: [
			{ ...first, entries: merged, next_before: first?.next_before ?? null },
			...rest,
		],
	};
}

function withTail(
	pages: EntryPages | undefined,
	tail: string,
): EntryPages | undefined {
	const first = pages?.pages[0];
	if (!pages || !first || first.tail === tail) return pages;
	return { ...pages, pages: [{ ...first, tail }, ...pages.pages.slice(1)] };
}

/** Drops older pages and reads the newest one again, so a gap behind the tail can't stay. */
function reloadNewestPage(queryClient: QueryClient, key: QueryKey) {
	queryClient.setQueryData<EntryPages>(
		key,
		(pages) =>
			pages && {
				pages: pages.pages.slice(0, 1),
				pageParams: pages.pageParams.slice(0, 1),
			},
	);
	return queryClient.refetchQueries({ queryKey: key, exact: true });
}

/** Adds a tail poll's page to the cache, or reloads when it says entries are missing in between. */
async function addTailPage(
	queryClient: QueryClient,
	key: QueryKey,
	page: IInboxEntriesPage,
): Promise<boolean> {
	const newest = page.entries[0]?.cursor;
	if (!newest) return false;
	if (page.next_before !== null) {
		await reloadNewestPage(queryClient, key);
		return true;
	}
	let fresh = false;
	queryClient.setQueryData<EntryPages>(key, (pages) => {
		const merged = mergeNewestEntries(pages, page.entries);
		fresh = merged !== pages;
		return withTail(merged, newest);
	});
	return fresh;
}

function storeConversation(
	queryClient: QueryClient,
	conversation: IConversation,
) {
	queryClient.setQueryData(
		inboxKeys.conversation(conversation.id),
		conversation,
	);
	queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
}

export function useConversations(
	as?: string,
	options: { poll?: boolean } = {},
) {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxKeys.conversations(as),
		queryFn: () => requireInboxState(state).listConversations(as),
		enabled: Boolean(state),
		refetchInterval:
			options.poll === false ? false : INBOX_POLL_MS.conversations,
		refetchIntervalInBackground: false,
	});
}

/** Unread messages across your person chats; pending requests and bot chats don't count. */
export function usePersonUnreadTotal(): number {
	const { data } = useConversations();
	return useMemo(() => personUnreadTotal(data ?? []), [data]);
}

export function useConversation(conversationId?: string | null, as?: string) {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useQuery({
		queryKey: inboxKeys.conversation(conversationId ?? "", as),
		queryFn: async ({ signal }) => {
			const id = conversationId as string;
			try {
				const conversation = await requireInboxState(state).getConversation(
					id,
					as,
				);
				if (signal.aborted) return conversation;
				const previous = queryClient.getQueryData<IConversation | null>(
					inboxKeys.conversation(id, as),
				);
				if (
					!as &&
					conversation.history_cleared_at &&
					conversation.history_cleared_at !== previous?.history_cleared_at
				) {
					await clearPersonalConversationCache(
						queryClient,
						id,
						false,
						false,
						signal,
					);
				}
				return conversation;
			} catch (error) {
				if (signal.aborted) throw error;
				const localMissing =
					error instanceof Error &&
					error.name === "LocalStoreError" &&
					"code" in error &&
					error.code === "NOT_FOUND";
				if (!isMissingResourceError(error) && !localMissing) throw error;
				if (!as)
					await clearPersonalConversationCache(
						queryClient,
						id,
						false,
						true,
						signal,
					);
				return null;
			}
		},
		enabled: Boolean(state && conversationId),
		initialData: () => {
			const listed = queryClient
				.getQueryData<IConversation[]>(inboxKeys.conversations(as))
				?.find((conversation) => conversation.id === conversationId);
			// A list can already carry a newer boundary while old entry pages remain cached.
			// Let the detail read clear those pages before it commits the boundary.
			return listed?.history_cleared_at ? undefined : listed;
		},
	});
}

/**
 * Pages of one conversation, newest first. While `live`, entries newer than the newest one a
 * server read returned are pushed by the realtime feed (polled on an interval as fallback) and
 * merged into the first page.
 */
export function useInboxEntries(
	conversationId?: string | null,
	as?: string,
	options: { live?: boolean } = {},
) {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	const key = useMemo(
		() => inboxKeys.entries(conversationId ?? "", as),
		[conversationId, as],
	);
	const query = useInfiniteQuery({
		queryKey: key,
		queryFn: async ({ pageParam }): Promise<EntryPage> => {
			const page = await requireInboxState(state).listEntries(
				conversationId as string,
				{ as, before: pageParam, limit: PAGE_SIZE },
			);
			return pageParam === undefined
				? { ...page, tail: page.entries[0]?.cursor }
				: page;
		},
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (page) => page.next_before ?? undefined,
		enabled: Boolean(state && conversationId),
	});

	const live = options.live !== false && query.data !== undefined;
	useEffect(() => {
		if (!live || !state || !conversationId) return;
		let cancelled = false;
		let polling = false;
		const tail = () =>
			queryClient.getQueryData<EntryPages>(key)?.pages[0]?.tail;
		const fetching = () =>
			queryClient.isFetching({ queryKey: key, exact: true }) > 0;
		const poll = async () => {
			const after = tail();
			const page = await state.listEntries(conversationId, {
				as,
				after,
				limit: PAGE_SIZE,
			});
			if (cancelled || fetching() || tail() !== after) return;
			if (await addTailPage(queryClient, key, page)) {
				void queryClient.invalidateQueries({
					queryKey: inboxKeys.conversations(as),
				});
			}
		};
		const tick = async () => {
			if (polling || fetching() || document.visibilityState !== "visible")
				return;
			polling = true;
			try {
				await poll();
			} catch (error) {
				console.warn("Polling conversation entries failed", error);
			} finally {
				polling = false;
			}
		};
		let lastPoll = Date.now();
		const due = () =>
			hasConversationFeed()
				? Date.now() - lastPoll >= INBOX_POLL_MS.openConversationSafety
				: true;
		const timer = window.setInterval(() => {
			if (!due()) return;
			lastPoll = Date.now();
			void tick();
		}, INBOX_POLL_MS.openConversation);
		const unwatch = watchConversation(
			{ conversationId, as, after: tail() },
			(entries) => {
				if (cancelled) return;
				const newest = entries.at(-1)?.cursor;
				if (!newest) return;
				let fresh = false;
				queryClient.setQueryData<EntryPages>(key, (pages) => {
					const merged = mergeNewestEntries(pages, entries);
					fresh = merged !== pages;
					return withTail(merged, newest);
				});
				if (fresh)
					void queryClient.invalidateQueries({
						queryKey: inboxKeys.conversations(as),
					});
			},
		);
		return () => {
			cancelled = true;
			unwatch();
			window.clearInterval(timer);
		};
	}, [live, state, conversationId, as, queryClient, key]);

	return query;
}

export function useCreateConversation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (request: ICreateConversationRequest) =>
			requireInboxState(state).createConversation(request),
		onSuccess: (conversation) => storeConversation(queryClient, conversation),
	});
}

export function useRenameConversation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			title,
		}: {
			conversationId: string;
			title: string;
		}) => requireInboxState(state).renameConversation(conversationId, title),
		onSuccess: (conversation) => storeConversation(queryClient, conversation),
	});
}

export function useSetConversationImage() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			image,
		}: {
			conversationId: string;
			image: string | null;
		}) => requireInboxState(state).setConversationImage(conversationId, image),
		onSuccess: (conversation) => storeConversation(queryClient, conversation),
	});
}

export function useAddConversationMembers() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			members,
		}: {
			conversationId: string;
			members: string[];
		}) => requireInboxState(state).addMembers(conversationId, members),
		onSuccess: (conversation) => storeConversation(queryClient, conversation),
	});
}

export function useRemoveConversationMember() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			principalId,
		}: {
			conversationId: string;
			principalId: string;
		}) => requireInboxState(state).removeMember(conversationId, principalId),
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: [...inboxKeys.all, "conversation", conversationId],
			});
			queryClient.invalidateQueries({
				queryKey: [...inboxKeys.all, "conversations"],
			});
		},
	});
}

async function clearPersonalConversationCache(
	queryClient: QueryClient,
	conversationId: string,
	removeDetail: boolean,
	removeFromList: boolean,
	signal?: AbortSignal,
) {
	if (signal?.aborted) return;
	const personalConversation = ({ queryKey }: { queryKey: QueryKey }) => {
		if (queryKey[0] !== inboxKeys.all[0] || queryKey[2] !== conversationId)
			return false;
		switch (queryKey[1]) {
			case "conversation":
				return removeDetail && queryKey[3] === "me";
			case "entries":
			case "pins":
				return queryKey[3] === "me";
			case "file":
				return queryKey[4] === "me";
			case "stars":
			case "links":
			case "turns":
				return true;
			default:
				return false;
		}
	};
	// Cancel reads started before deletion so their old history cannot refill the cache.
	await Promise.all([
		queryClient.cancelQueries({ predicate: personalConversation }),
		queryClient.cancelQueries({
			queryKey: inboxKeys.conversations(),
			exact: true,
		}),
		queryClient.cancelQueries({
			queryKey: [...inboxKeys.all, "stars", "all"],
			exact: true,
		}),
	]);
	if (signal?.aborted) return;
	queryClient.removeQueries({ predicate: personalConversation });
	if (removeFromList)
		queryClient.setQueryData<IConversation[]>(
			inboxKeys.conversations(),
			(conversations) =>
				conversations?.filter((item) => item.id !== conversationId),
		);
	queryClient.setQueryData<InfiniteData<IStarredMessagesPage>>(
		[...inboxKeys.all, "stars", "all"],
		(data) =>
			data && {
				...data,
				pages: data.pages.map((page) => ({
					...page,
					stars: page.stars.filter(
						(star) => star.conversation_id !== conversationId,
					),
				})),
			},
	);
	void queryClient.invalidateQueries({
		queryKey: inboxKeys.conversations(),
		exact: true,
	});
	void queryClient.invalidateQueries({
		queryKey: [...inboxKeys.all, "stars", "all"],
		exact: true,
	});
	void queryClient.invalidateQueries({ queryKey: inboxKeys.invitations() });
}

/** Deletes your view of a conversation, leaving other participants' cached copies alone. */
export function useDeleteConversation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (conversationId: string) =>
			requireInboxState(state).deleteConversation(conversationId),
		onSuccess: (_, conversationId) =>
			clearPersonalConversationCache(queryClient, conversationId, true, true),
	});
}

export function useInvitations() {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxKeys.invitations(),
		queryFn: () => requireInboxState(state).listInvitations(),
		enabled: Boolean(state),
		refetchInterval: INBOX_POLL_MS.conversations,
		refetchIntervalInBackground: false,
	});
}

/** Who the server lets you bring into a room; `conversationId` marks who is already in it. */
export function useInvitable(
	q: string,
	options: { conversationId?: string; enabled?: boolean } = {},
) {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxKeys.invitable(q, options.conversationId),
		queryFn: () =>
			requireInboxState(state).listInvitable({
				q: q || undefined,
				conversationId: options.conversationId,
			}),
		enabled: Boolean(state) && options.enabled !== false,
		placeholderData: (previous) => previous,
	});
}

export function useInviteToConversation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			request,
		}: {
			conversationId: string;
			request: IInviteRequest;
		}) => requireInboxState(state).invite(conversationId, request),
		onSuccess: ({ conversation }) => {
			storeConversation(queryClient, conversation);
			queryClient.invalidateQueries({
				queryKey: [...inboxKeys.all, "invitable"],
			});
		},
	});
}

export function useAnswerInvitation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			invitation,
			accept,
			limits,
			block,
		}: {
			invitation: IInvitation;
			accept: boolean;
			limits?: IBotRoomLimits | null;
			block?: boolean;
		}) => {
			const inbox = requireInboxState(state);
			if (accept) return inbox.acceptInvitation(invitation, limits);
			return block
				? inbox.blockInvitation(invitation)
				: inbox.declineInvitation(invitation);
		},
		onSettled: () => {
			queryClient.invalidateQueries({ queryKey: [...inboxKeys.all, "blocks"] });
			queryClient.invalidateQueries({ queryKey: inboxKeys.invitations() });
			queryClient.invalidateQueries({
				queryKey: [...inboxKeys.all, "conversation"],
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

export function useSetConversationMuted() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			muted,
		}: {
			conversationId: string;
			muted: boolean;
		}) => requireInboxState(state).setConversationMuted(conversationId, muted),
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxKeys.conversation(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

export function useCancelInvitation() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			principalId,
		}: {
			conversationId: string;
			principalId: string;
		}) =>
			requireInboxState(state).cancelInvitation(conversationId, principalId),
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxKeys.conversation(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

export function useConversationLinks(conversationId: string, enabled = true) {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxKeys.links(conversationId),
		queryFn: () =>
			requireInboxState(state).listConversationLinks(conversationId),
		enabled: Boolean(state) && enabled,
	});
}

export function useCreateConversationLink() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			request,
		}: {
			conversationId: string;
			request: ICreateConversationLinkRequest;
		}) =>
			requireInboxState(state).createConversationLink(conversationId, request),
		onSuccess: (_, { conversationId }) =>
			queryClient.invalidateQueries({
				queryKey: inboxKeys.links(conversationId),
			}),
	});
}

export function useRevokeConversationLink() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			linkId,
		}: {
			conversationId: string;
			linkId: string;
		}) =>
			requireInboxState(state).revokeConversationLink(conversationId, linkId),
		onSettled: (_, __, { conversationId }) =>
			queryClient.invalidateQueries({
				queryKey: inboxKeys.links(conversationId),
			}),
	});
}

export function useSetMemberLimits() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			botId,
			limits,
		}: {
			conversationId: string;
			botId: string;
			limits: IBotRoomLimits | null;
		}) =>
			requireInboxState(state).setMemberLimits(conversationId, botId, limits),
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxKeys.conversation(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

export function useSendInboxEntry() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			request,
		}: {
			conversationId: string;
			request: ISendInboxEntryRequest;
		}) => requireInboxState(state).sendEntry(conversationId, request),
		onSuccess: (entry) => {
			queryClient.setQueryData<EntryPages>(
				inboxKeys.entries(entry.conversation_id),
				(pages) => mergeNewestEntries(pages, [entry]),
			);
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

export interface SendMessageWithFilesInput {
	conversationId: string;
	/** The message; `parts` it already carries stay and the uploads follow them. */
	body: IInboxMessageBody;
	/** The files `ChatBox` hands to `onSendMessage`. */
	files?: readonly File[];
	/** A client-chosen id makes a retried send idempotent. */
	messageId?: string;
}

/**
 * Uploads the files into the conversation, then sends one message carrying them as attachments.
 * Nothing is sent when an upload fails, and the files already uploaded lapse after a day.
 */
export function useSendMessageWithFiles() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async ({
			conversationId,
			body,
			files = [],
			messageId = newInboxMessageId(),
		}: SendMessageWithFilesInput) => {
			const inbox = requireInboxState(state);
			const uploaded = await inBatches(files, (file) =>
				inbox.uploadConversationFile(conversationId, file),
			);
			const existing = body.parts ?? [];
			const parts = [...existing, ...attachmentParts(uploaded, existing)];
			return inbox.sendEntry(conversationId, {
				message_id: messageId,
				kind: "message",
				envelope: {
					v: 1,
					enc: "none",
					body: parts.length > 0 ? { ...body, parts } : body,
				},
			});
		},
		onSuccess: (entry) => {
			queryClient.setQueryData<EntryPages>(
				inboxKeys.entries(entry.conversation_id),
				(pages) => mergeNewestEntries(pages, [entry]),
			);
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

/** A fresh link to an attachment of a conversation, for rendering or downloading it. */
export function useConversationFileUrl(
	conversationId: string | undefined,
	handle: Pick<IInboxFileHandle, "path" | "mime"> | undefined,
	as?: string,
) {
	const state = useBackend().inboxState;
	return useQuery<IConversationFileDownload>({
		queryKey: [
			...inboxKeys.all,
			"file",
			conversationId ?? "",
			handle ? conversationFileId(handle.path) : "",
			as ?? "me",
		],
		queryFn: () =>
			requireInboxState(state).conversationFileDownload(
				conversationId as string,
				handle as Pick<IInboxFileHandle, "path" | "mime">,
				as,
			),
		enabled: Boolean(state && conversationId && handle),
		staleTime: 4 * 60_000,
	});
}

export function useMarkConversationRead() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			upTo,
		}: {
			conversationId: string;
			upTo?: string;
		}) => requireInboxState(state).markRead(conversationId, upTo),
		onSuccess: (_, { conversationId, upTo }) => {
			// A message received during the request stays unread until its own cursor is shown.
			const read = (conversation: IConversation) =>
				conversation.id === conversationId &&
				(!upTo ||
					!conversation.last_entry ||
					compareInboxCursors(conversation.last_entry.cursor, upTo) <= 0)
					? { ...conversation, unread_count: 0 }
					: conversation;
			queryClient.setQueryData<IConversation[]>(
				inboxKeys.conversations(),
				(conversations) => conversations?.map(read),
			);
			queryClient.setQueryData<IConversation>(
				inboxKeys.conversation(conversationId),
				(conversation) => conversation && read(conversation),
			);
		},
	});
}
