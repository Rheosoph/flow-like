"use client";

import { BOTS_ROUTE } from "@flow-like/flow-like-ui/components/bots-tab/dock-selection";
import { botKeys } from "@flow-like/flow-like-ui/hooks/use-bots";
import { inboxKeys } from "@flow-like/flow-like-ui/hooks/use-inbox";
import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { localBots } from "../lib/local-bots/commands";
import { botsToMove } from "../lib/local-bots/merge";
import { SessionWatch, sessionSnapshot } from "../lib/local-bots/session-watch";
import { TauriBackend } from "./tauri-provider";

const MOVE_TOAST_ID = "local-bots-move";

/**
 * Refreshes bot and inbox queries when this computer's store changes or the
 * signed-in account does, and offers to move local bots right after a sign-in.
 */
export function LocalBotsBridge() {
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();
	const router = useClientRouter();
	const { t } = useTranslation("bots");
	const watch = useRef(new SessionWatch());
	const desktop = backend instanceof TauriBackend ? backend : undefined;

	useEffect(() => {
		const subscriptions = [
			listen("local-bots-changed", () => {
				void queryClient.invalidateQueries({ queryKey: botKeys.all });
			}),
			listen("local-inbox-changed", () => {
				void queryClient.invalidateQueries({ queryKey: inboxKeys.all });
			}),
		];
		return () => {
			for (const subscription of subscriptions) {
				void subscription.then(
					(unlisten) => unlisten(),
					() => undefined,
				);
			}
		};
	}, [queryClient]);

	const offerMove = useCallback(
		async (sub: string) => {
			const bots = botsToMove(await localBots.list());
			const stillSignedIn = desktop && sessionSnapshot(desktop).person === sub;
			if (bots.length === 0 || !stillSignedIn) return;
			toast.info(
				t("bots:move.toast.title", {
					count: bots.length,
					defaultValue_one: "{{count}} bot lives only on this computer",
					defaultValue_other: "{{count}} bots live only on this computer",
				}),
				{
					id: MOVE_TOAST_ID,
					action: {
						label: t("bots:move.toast.action", "Review"),
						onClick: () => router.push(`${BOTS_ROUTE}?move=local`),
					},
				},
			);
		},
		[desktop, router, t],
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: auth changes are the trigger; the session is read from the backend, which AuthInner updated earlier in the same commit.
	useEffect(() => {
		if (!desktop) return;
		const { refetch, signedIn } = watch.current.observe(
			sessionSnapshot(desktop),
		);
		if (refetch) {
			void queryClient.invalidateQueries({ queryKey: botKeys.all });
			void queryClient.invalidateQueries({ queryKey: inboxKeys.all });
		}
		if (signedIn) {
			offerMove(signedIn).catch((error) =>
				console.warn("[LocalBots] Could not offer to move local bots:", error),
			);
		}
	}, [
		desktop,
		auth.isLoading,
		auth.isAuthenticated,
		auth.user,
		queryClient,
		offerMove,
	]);

	return null;
}
