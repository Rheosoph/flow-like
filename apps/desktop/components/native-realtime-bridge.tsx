"use client";

import { useBackend, useBackendReady } from "@flow-like/flow-like-ui";
import { inboxKeys } from "@flow-like/flow-like-ui/hooks/use-inbox";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useAuth } from "react-oidc-context";
import { watchNativeRealtime } from "../lib/native-realtime";
import { TauriBackend } from "./tauri-provider";

const NOTIFICATION_QUERIES = new Set([
	"getNotifications",
	"listNotifications",
	"getInvites",
	"listMyInvitations",
]);

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

	useEffect(() => {
		if (!ready || !signedIn || !profileId || !subject || !issuer || !hasToken)
			return;
		let revision: string | undefined;
		return watchNativeRealtime({
			profileId,
			subject,
			issuer,
			request: { notifications: {}, runs: [], timeout_ms: 20_000 },
			onMessage: (message) => {
				if (message.type !== "update") return;
				const update = message.response.notifications;
				if (!update || update.revision === revision) return;
				return queryClient
					.invalidateQueries(
						{
							predicate: (query) =>
								NOTIFICATION_QUERIES.has(String(query.queryKey[0])) ||
								query.queryKey[0] === inboxKeys.all[0] ||
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
	}, [ready, signedIn, profileId, subject, issuer, hasToken, queryClient]);
	return null;
}
