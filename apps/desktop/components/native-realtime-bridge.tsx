"use client";

import { useBackend, useBackendReady } from "@flow-like/flow-like-ui";
import { IncomingCallProvider } from "@flow-like/flow-like-ui/components/incoming-call-provider";
import { unwatchedInboxPredicate } from "@flow-like/flow-like-ui/hooks/use-inbox";
import { deliverRealtimeReply } from "@flow-like/flow-like-ui/lib/realtime/reply-streams";
import type { RealtimeConversationSubscription } from "@flow-like/flow-like-ui/lib/realtime/types";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { pokeBotHost } from "../lib/bot-host-poke";
import { watchBotHostReplies } from "../lib/bot-host-replies";
import {
	desktopConversationFeed,
	onWatchedConversationsChange,
} from "../lib/desktop-conversation-feed";
import { localRegistries } from "../lib/local-bots/registry";
import { watchNativeRealtime } from "../lib/native-realtime";
import { TauriBackend } from "./tauri-provider";

const NOTIFICATION_QUERIES = new Set([
	"getNotifications",
	"listNotifications",
	"getInvites",
	"listMyInvitations",
]);

const WATCH_DEBOUNCE_MS = 100;

interface AccountConversations {
	readonly subscriptions: RealtimeConversationSubscription[];
	readonly localIds: ReadonlyMap<string, string>;
}

/** Watched conversations the account serves; ones on this computer are read from the local store. */
async function accountConversations(
	backend: object,
	subject: string,
): Promise<AccountConversations> {
	const registry = localRegistries(backend).conversations;
	const targets = desktopConversationFeed.targets();
	const local = await Promise.all(
		targets.map((target) =>
			registry.isLocal(target.conversationId, subject).catch(() => true),
		),
	);
	const remote = targets.filter((_, index) => !local[index]);
	return {
		subscriptions: remote.map((target) => ({
			conversation_id: registry.accountId(target.conversationId, subject),
			as: target.as,
			after: target.after,
		})),
		localIds: new Map(
			remote.map((target) => [
				registry.accountId(target.conversationId, subject),
				target.conversationId,
			]),
		),
	};
}

export function NativeRealtimeBridge() {
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const queryClient = useQueryClient();
	const profileId =
		backend instanceof TauriBackend ? backend.profile?.id : undefined;
	const subject = auth.user?.profile.sub;
	const issuer = auth.user?.profile.iss ?? auth.settings.authority;
	// The native owner refreshes its own token; a refresh here must not retire its connection.
	const hasToken = Boolean(auth.user?.access_token);
	const signedIn = auth.isAuthenticated && !auth.isLoading;
	const localIds = useRef<ReadonlyMap<string, string>>(new Map());

	// A host on this computer answered the call: its reply joins the replies the server pushes.
	useEffect(
		() =>
			watchBotHostReplies((reply) =>
				deliverRealtimeReply(
					reply,
					localIds.current.get(reply.conversation_id) ?? reply.conversation_id,
				),
			),
		[],
	);

	useEffect(() => {
		if (!ready || !signedIn || !profileId || !subject || !issuer || !hasToken)
			return;
		let revision: string | undefined;
		let closed = false;
		let stopWatching: (() => void) | undefined;
		let requested = "[]";
		let timer: ReturnType<typeof setTimeout> | undefined;
		const unwatched = unwatchedInboxPredicate((id, as) =>
			desktopConversationFeed.isWatched(id, as),
		);
		const localId = (accountId: string) =>
			localIds.current.get(accountId) ?? accountId;
		const open = (conversations: RealtimeConversationSubscription[]) => {
			stopWatching?.();
			stopWatching = watchNativeRealtime({
				profileId,
				subject,
				issuer,
				request: {
					notifications: {},
					runs: [],
					...(conversations.length ? { conversations } : {}),
					timeout_ms: 20_000,
				},
				onMessage: (message) => {
					if (message.type === "reply") {
						deliverRealtimeReply(
							message.reply,
							localId(message.reply.conversation_id),
						);
						return;
					}
					if (message.type !== "update") return;
					for (const item of message.response.conversations ?? [])
						desktopConversationFeed.deliver(
							localId(item.conversation_id),
							item.entries,
							{ as: item.as },
						);
					const update = message.response.notifications;
					if (!update || update.revision === revision) return;
					pokeBotHost();
					return queryClient
						.invalidateQueries(
							{
								predicate: (query) =>
									NOTIFICATION_QUERIES.has(String(query.queryKey[0])) ||
									unwatched(query) ||
									(query.queryKey[0] === "home" &&
										query.queryKey[4] === "notifications"),
							},
							{ throwOnError: true },
						)
						.then(() => {
							revision = update.revision;
						});
				},
			});
		};
		const resubscribe = async () => {
			const account = await accountConversations(backend, subject);
			const signature = JSON.stringify(
				account.subscriptions.map(({ conversation_id, as }) => [
					conversation_id,
					as,
				]),
			);
			if (closed) return;
			localIds.current = account.localIds;
			if (signature === requested) return;
			requested = signature;
			open(account.subscriptions);
		};
		const stopListening = onWatchedConversationsChange(() => {
			clearTimeout(timer);
			timer = setTimeout(() => void resubscribe(), WATCH_DEBOUNCE_MS);
		});
		open([]);
		void resubscribe();
		return () => {
			closed = true;
			localIds.current = new Map();
			clearTimeout(timer);
			stopListening();
			stopWatching?.();
		};
	}, [
		ready,
		signedIn,
		profileId,
		subject,
		issuer,
		hasToken,
		queryClient,
		backend,
	]);
	return (
		<IncomingCallProvider
			enabled={ready && signedIn && Boolean(profileId && subject && hasToken)}
			scope={JSON.stringify([profileId, issuer, subject])}
			userId={subject ?? ""}
		/>
	);
}
