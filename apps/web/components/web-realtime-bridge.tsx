"use client";

import { inboxKeys } from "@flow-like/flow-like-ui/hooks/use-inbox";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { getRealtimeClient } from "@flow-like/flow-like-ui/lib/realtime/runtime-client";
import type { RealtimeClient } from "@flow-like/flow-like-ui/lib/realtime/types";
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { prepareSessionRequest } from "../lib/session-request";

/** Keeps the account inbox current across routes and across browser tabs. */
export function WebRealtimeBridge({ enabled }: { enabled: boolean }) {
	const auth = useAuth();
	const backend = useBackend();
	const queryClient = useQueryClient();
	const authRef = useRef(auth);
	authRef.current = auth;
	const clientRef = useRef<RealtimeClient | undefined>(undefined);
	const hub = getApiOrigin();
	const issuer = auth.user?.profile.iss;
	const subject = auth.user?.profile.sub;
	const scope = JSON.stringify([hub, issuer, subject]);
	const token = auth.user?.access_token;

	useEffect(() => {
		if (!enabled || !subject || !authRef.current.isAuthenticated) return;
		let stopped = false;
		let client: RealtimeClient | undefined;
		let unsubscribe: (() => void) | undefined;
		let retryTimer: ReturnType<typeof setTimeout> | undefined;
		let failures = 0;
		const sessionUrl = `${hub.replace(/\/$/, "")}/api/v1/realtime/poll`;
		const activity = () =>
			client?.setActivity({
				online: navigator.onLine,
				visible: document.visibilityState !== "hidden",
			});
		const currentAccount = () => {
			const current = authRef.current;
			return (
				!stopped &&
				current.isAuthenticated &&
				current.user?.profile.sub === subject &&
				current.user?.profile.iss === issuer
			);
		};
		const start = async () => {
			const prepared = await prepareSessionRequest(undefined, authRef.current);
			if (!currentAccount()) return;
			if (!prepared.headers.has("Authorization"))
				throw new Error("Realtime session is not ready");
			client = await getRealtimeClient(
				{
					scope,
					url: sessionUrl,
					headers: Object.fromEntries(prepared.headers.entries()),
				},
				{
					onUnauthorized: async (rejectedToken) => {
						if (!currentAccount()) return;
						const session = await prepareSessionRequest(
							undefined,
							authRef.current,
						);
						const renewed = await session.onUnauthorized?.(rejectedToken);
						return currentAccount() ? renewed : undefined;
					},
				},
			);
			if (!currentAccount()) {
				client.close();
				return;
			}
			// Credentials can change while the shared worker starts.
			for (;;) {
				const incomingToken = authRef.current.user?.access_token;
				const latest = await prepareSessionRequest(undefined, authRef.current);
				if (!currentAccount()) {
					client.close();
					return;
				}
				if (incomingToken !== authRef.current.user?.access_token) continue;
				if (!latest.headers.has("Authorization"))
					throw new Error("Realtime session is not ready");
				client.setSession({
					scope,
					url: sessionUrl,
					headers: Object.fromEntries(latest.headers.entries()),
				});
				break;
			}
			clientRef.current = client;
			unsubscribe = client.subscribe(async (update) => {
				if (
					!currentAccount() ||
					update.type !== "snapshot" ||
					!update.snapshot.notifications
				)
					return;
				await Promise.all([
					...[
						backend.userState.getNotifications,
						backend.userState.listNotifications,
						backend.teamState.getInvites,
						backend.registryState.listMyInvitations,
					].map((fn) =>
						queryClient.invalidateQueries(
							{ queryKey: [fn.name] },
							{ throwOnError: true },
						),
					),
					queryClient.invalidateQueries(
						{
							predicate: ({ queryKey }) =>
								queryKey[0] === "home" && queryKey[4] === "notifications",
						},
						{ throwOnError: true },
					),
					queryClient.invalidateQueries(
						{ queryKey: inboxKeys.all },
						{ throwOnError: true },
					),
				]);
			});
			activity();
			client.setSubscriptions({ notifications: true, runs: [] });
		};
		const attempt = () => {
			void start().catch(() => {
				unsubscribe?.();
				client?.close();
				if (clientRef.current === client) clientRef.current = undefined;
				client = undefined;
				unsubscribe = undefined;
				if (!currentAccount()) return;
				// Session recovery owns sign-in UI. Transient startup failures retry here.
				retryTimer = setTimeout(
					attempt,
					Math.min(30_000, 1_000 * 2 ** Math.min(failures++, 5)),
				);
			});
		};
		attempt();
		document.addEventListener("visibilitychange", activity);
		window.addEventListener("online", activity);
		window.addEventListener("offline", activity);
		return () => {
			stopped = true;
			clearTimeout(retryTimer);
			unsubscribe?.();
			client?.close();
			if (clientRef.current === client) clientRef.current = undefined;
			document.removeEventListener("visibilitychange", activity);
			window.removeEventListener("online", activity);
			window.removeEventListener("offline", activity);
		};
	}, [enabled, scope, hub, issuer, subject, backend, queryClient]);

	useEffect(() => {
		if (!token || !enabled || !clientRef.current) return;
		clientRef.current.setSession({
			scope,
			url: `${hub.replace(/\/$/, "")}/api/v1/realtime/poll`,
			headers: { Authorization: `Bearer ${token}` },
		});
	}, [token, enabled, scope, hub]);

	return null;
}
