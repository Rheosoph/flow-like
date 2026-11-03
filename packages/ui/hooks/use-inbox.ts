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
import { useBackend } from "../state/backend-state";
import type {
	IConversation,
	ICreateConversationRequest,
	IInboxEntriesPage,
	IInboxEntry,
	IBotRoomLimits,
	IInboxState,
	IInvitation,
	IInviteRequest,
	ISendInboxEntryRequest,
} from "../state/backend-state/inbox-state";

export const inboxKeys = {
	all: ["inbox"] as const,
	conversations: (as?: string) =>
		[...inboxKeys.all, "conversations", as ?? "me"] as const,
	conversation: (conversationId: string, as?: string) =>
		[...inboxKeys.all, "conversation", conversationId, as ?? "me"] as const,
	entries: (conversationId: string, as?: string) =>
		[...inboxKeys.all, "entries", conversationId, as ?? "me"] as const,
	invitations: () => [...inboxKeys.all, "invitations"] as const,
	invitable: (q: string, conversationId?: string) =>
		[...inboxKeys.all, "invitable", conversationId ?? "", q] as const,
};

/** Polling stands in for realtime wakeups until the inbox joins the realtime fingerprint. */
export const INBOX_POLL_MS = {
	conversations: 15_000,
	openConversation: 4_000,
} as const;

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

function cursorParts(cursor: string): readonly [number, string] {
	const split = cursor.indexOf(":");
	const micros = split < 0 ? Number.NaN : Number(cursor.slice(0, split));
	return [
		Number.isFinite(micros) ? micros : Number.NEGATIVE_INFINITY,
		cursor.slice(split + 1),
	];
}

/** Server order of `<createdAt epoch micros>:<entry id>` cursors: micros numerically, then ids by code unit. */
export function compareInboxCursors(a: string, b: string): number {
	const [microsA, idA] = cursorParts(a);
	const [microsB, idB] = cursorParts(b);
	if (microsA !== microsB) return microsA < microsB ? -1 : 1;
	if (idA === idB) return 0;
	return idA < idB ? -1 : 1;
}

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

export function useConversation(conversationId?: string | null, as?: string) {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useQuery({
		queryKey: inboxKeys.conversation(conversationId ?? "", as),
		queryFn: () =>
			requireInboxState(state).getConversation(conversationId as string, as),
		enabled: Boolean(state && conversationId),
		initialData: () =>
			queryClient
				.getQueryData<IConversation[]>(inboxKeys.conversations(as))
				?.find((conversation) => conversation.id === conversationId),
	});
}

/**
 * Pages of one conversation, newest first. While `live`, entries newer than the newest one a
 * server read returned are fetched on an interval and merged into the first page.
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
		const timer = window.setInterval(tick, INBOX_POLL_MS.openConversation);
		return () => {
			cancelled = true;
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
				queryKey: inboxKeys.conversation(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
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
		}: {
			invitation: IInvitation;
			accept: boolean;
			limits?: IBotRoomLimits | null;
		}) => {
			const inbox = requireInboxState(state);
			return accept
				? inbox.acceptInvitation(invitation, limits)
				: inbox.declineInvitation(invitation);
		},
		onSettled: () => {
			queryClient.invalidateQueries({ queryKey: inboxKeys.invitations() });
			queryClient.invalidateQueries({
				queryKey: [...inboxKeys.all, "conversation"],
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
		onSuccess: (_, { conversationId }) => {
			const read = (conversation: IConversation) =>
				conversation.id === conversationId
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
