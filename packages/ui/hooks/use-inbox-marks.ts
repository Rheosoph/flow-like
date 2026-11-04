"use client";

import {
	type QueryClient,
	useInfiniteQuery,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useMemo } from "react";
import { useBackend } from "../state/backend-state";
import {
	type IConversation,
	type IInboxEditBody,
	type IInboxEntry,
	type IInboxEnvelope,
	type IInboxPlainEnvelope,
	type IInboxState,
	type IPinnedMessage,
	type IStarredMessage,
	type IStarredMessagesPage,
	compareConversations,
} from "../state/backend-state/inbox-state";
import { inboxKeys } from "./use-inbox";

/** Under `inbox`, so the realtime wakeup that refreshes conversations refreshes these too. */
export const inboxMarkKeys = {
	pins: (conversationId: string, as?: string) =>
		[...inboxKeys.all, "pins", conversationId, as ?? "me"] as const,
	stars: (conversationId?: string) =>
		[...inboxKeys.all, "stars", conversationId ?? "all"] as const,
};

/** The most stars one conversation's view marks; older ones still list under all stars. */
const CONVERSATION_STARS_LIMIT = 200;
const STARS_PAGE_SIZE = 50;

function requireInboxState(state: IInboxState | undefined): IInboxState {
	if (!state)
		throw new Error("Conversations need a signed-in Flow-Like account");
	return state;
}

function isPlainEnvelope(
	envelope: IInboxEnvelope | undefined,
): envelope is IInboxPlainEnvelope {
	return envelope?.enc === "none" && "body" in envelope;
}

/** The entry as it reads now: the sender's newest edit replaces the original text. */
export function shownEntry(
	entry: IInboxEntry,
	latestEdit: IInboxEntry | null | undefined,
): IInboxEntry {
	const edit = latestEdit?.envelope;
	const original = entry.envelope;
	if (!isPlainEnvelope(edit) || !isPlainEnvelope(original)) return entry;
	const text = (edit.body as Partial<IInboxEditBody> | null)?.text;
	if (typeof text !== "string") return entry;
	return {
		...entry,
		envelope: { ...original, body: { ...(original.body as object), text } },
	};
}

function patchConversations(
	queryClient: QueryClient,
	conversationId: string,
	patch: Partial<IConversation>,
) {
	queryClient.setQueryData<IConversation[]>(
		inboxKeys.conversations(),
		(conversations) =>
			conversations
				?.map((conversation) =>
					conversation.id === conversationId
						? { ...conversation, ...patch }
						: conversation,
				)
				.sort(compareConversations),
	);
	queryClient.setQueryData<IConversation>(
		inboxKeys.conversation(conversationId),
		(conversation) => conversation && { ...conversation, ...patch },
	);
}

/** Pins a conversation to the top of your list, or unpins it; the list reorders at once. */
export function useSetConversationPinned() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: ({
			conversationId,
			pinned,
		}: {
			conversationId: string;
			pinned: boolean;
		}) =>
			requireInboxState(state).setConversationPinned(conversationId, pinned),
		onMutate: async ({ conversationId, pinned }) => {
			await queryClient.cancelQueries({ queryKey: inboxKeys.conversations() });
			const previous = queryClient
				.getQueryData<IConversation[]>(inboxKeys.conversations())
				?.find((conversation) => conversation.id === conversationId)?.pinned_at;
			patchConversations(queryClient, conversationId, {
				pinned_at: pinned ? new Date().toISOString() : null,
			});
			return { previous };
		},
		onError: (_, { conversationId }, context) => {
			patchConversations(queryClient, conversationId, {
				pinned_at: context?.previous ?? null,
			});
		},
		onSuccess: (pin) => {
			patchConversations(queryClient, pin.conversation_id, {
				pinned_at: pin.pinned_at,
			});
		},
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxKeys.conversation(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxKeys.conversations() });
		},
	});
}

/** The messages pinned for everyone in the conversation, newest pin first. */
export function useConversationPins(conversationId: string, as?: string) {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxMarkKeys.pins(conversationId, as),
		queryFn: () => requireInboxState(state).listPins(conversationId, as),
		enabled: Boolean(state && conversationId),
		staleTime: 30_000,
	});
}

export function useSetMessagePinned() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async ({
			conversationId,
			messageId,
			pinned,
		}: {
			conversationId: string;
			messageId: string;
			pinned: boolean;
		}) => {
			const inbox = requireInboxState(state);
			if (pinned) await inbox.pinMessage(conversationId, messageId);
			else await inbox.unpinMessage(conversationId, messageId);
		},
		onMutate: ({ conversationId, messageId, pinned }) => {
			if (pinned) return;
			queryClient.setQueryData<IPinnedMessage[]>(
				inboxMarkKeys.pins(conversationId),
				(pins) => pins?.filter((pin) => pin.message_id !== messageId),
			);
		},
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxMarkKeys.pins(conversationId),
			});
		},
	});
}

/** Your stars in one conversation, newest first, and their message ids. */
export function useConversationStars(conversationId: string) {
	const state = useBackend().inboxState;
	const query = useQuery({
		queryKey: inboxMarkKeys.stars(conversationId),
		queryFn: () =>
			requireInboxState(state).listStars({
				conversationId,
				limit: CONVERSATION_STARS_LIMIT,
			}),
		enabled: Boolean(state && conversationId),
		staleTime: 30_000,
	});
	const stars = query.data?.stars;
	const starredIds = useMemo(
		() => new Set((stars ?? []).map((star) => star.message_id)),
		[stars],
	);
	return { ...query, stars: stars ?? [], starredIds };
}

/** Your stars across every conversation you are in, newest first, a page at a time. */
export function useStarredMessages(enabled = true) {
	const state = useBackend().inboxState;
	return useInfiniteQuery({
		queryKey: inboxMarkKeys.stars(),
		queryFn: ({ pageParam }) =>
			requireInboxState(state).listStars({
				before: pageParam,
				limit: STARS_PAGE_SIZE,
			}),
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (page: IStarredMessagesPage) =>
			page.next_before ?? undefined,
		enabled: Boolean(state) && enabled,
	});
}

export function useSetMessageStarred() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async ({
			conversationId,
			messageId,
			starred,
		}: {
			conversationId: string;
			messageId: string;
			starred: boolean;
		}): Promise<IStarredMessage | null> => {
			const inbox = requireInboxState(state);
			if (starred) return inbox.starMessage(conversationId, messageId);
			await inbox.unstarMessage(conversationId, messageId);
			return null;
		},
		onMutate: ({ conversationId, messageId, starred }) => {
			if (starred) return;
			queryClient.setQueryData<IStarredMessagesPage>(
				inboxMarkKeys.stars(conversationId),
				(page) =>
					page && {
						...page,
						stars: page.stars.filter((star) => star.message_id !== messageId),
					},
			);
		},
		onSuccess: (star, { conversationId }) => {
			if (!star) return;
			queryClient.setQueryData<IStarredMessagesPage>(
				inboxMarkKeys.stars(conversationId),
				(page) =>
					page && {
						...page,
						stars: [
							star,
							...page.stars.filter(
								(known) => known.message_id !== star.message_id,
							),
						],
					},
			);
		},
		onSettled: (_, __, { conversationId }) => {
			queryClient.invalidateQueries({
				queryKey: inboxMarkKeys.stars(conversationId),
			});
			queryClient.invalidateQueries({ queryKey: inboxMarkKeys.stars() });
		},
	});
}
