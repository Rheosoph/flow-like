"use client";

import {
	type IOfflineStatusEvent,
	type OfflineSyncState,
	useBackend,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useRouter } from "next/navigation";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

/** Raises one toast per app when its offline changes stop at a change that needs a decision. */
export function OfflineWritesNotifier() {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const router = useRouter();
	const lastState = useRef(new Map<string, OfflineSyncState>());
	const offlineWrites = backend.offlineWritesState;
	const appState = backend.appState;

	useEffect(() => {
		if (!offlineWrites) return;

		const notify = async (event: IOfflineStatusEvent) => {
			const app = await appState
				.getAppMeta(event.appId)
				.then((meta) => meta.name || event.appId)
				.catch(() => event.appId);
			toast.warning(
				t(
					"settings:offlineAccess.notifierTitle",
					"Offline changes need attention",
				),
				{
					id: `offline-writes-blocked:${event.appId}`,
					description: t("settings:offlineAccess.notifierBody", {
						app,
						count: event.blockedCount,
						defaultValue_one:
							"{{app}} has {{count}} change that could not be synced.",
						defaultValue_other:
							"{{app}} has {{count}} changes that could not be synced.",
					}),
					action: {
						label: t("settings:offlineAccess.notifierAction", "Review"),
						onClick: () =>
							router.push(
								`/library/config/offline?id=${encodeURIComponent(event.appId)}`,
							),
					},
				},
			);
		};

		return offlineWrites.subscribe((event) => {
			const previous = lastState.current.get(event.appId);
			lastState.current.set(event.appId, event.syncState);
			if (event.syncState === "blocked" && previous !== "blocked") {
				void notify(event);
			}
		});
	}, [offlineWrites, appState, router, t]);

	return null;
}
