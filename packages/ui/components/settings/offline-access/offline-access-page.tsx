"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CloudOffIcon, RefreshCwIcon, TriangleAlertIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useBackend, useBackendReady } from "../../../state/backend-state";
import type {
	IOfflineAppOverview,
	IOfflineWritesState,
} from "../../../state/backend-state/offline-writes-state";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import {
	LIVE_ONLY,
	type OfflineTranslate,
	offlineQueryKeys,
} from "./offline-access-logic";
import { OfflineQueueCard } from "./offline-queue-card";
import { OfflineStorageCard } from "./offline-storage-card";
import { OfflineSyncBadge } from "./offline-sync-badge";
import { OfflineTablesCard } from "./offline-tables-card";

const EVENT_REFRESH_DELAY_MS = 250;

/** Status events arrive per drained change, so a burst refetches once. */
function useOfflineEvents(
	state: IOfflineWritesState | undefined,
	appId: string,
	onEvent: () => void,
) {
	useEffect(() => {
		if (!state) return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const forApp = (event: { appId: string }) => {
			if (event.appId !== appId || timer !== undefined) return;
			timer = setTimeout(() => {
				timer = undefined;
				onEvent();
			}, EVENT_REFRESH_DELAY_MS);
		};
		const unsubscribers = [
			state.subscribe(forApp),
			state.subscribeTables(forApp),
			state.subscribeMirror(forApp),
		];
		return () => {
			clearTimeout(timer);
			for (const unsubscribe of unsubscribers) unsubscribe();
		};
	}, [state, appId, onEvent]);
}

function unavailableMessage(
	t: OfflineTranslate,
	overview: IOfflineAppOverview,
	localOnly: boolean,
): string {
	if (localOnly) {
		return t(
			"settings:offlineAccess.unavailableOfflineApp",
			"This project only lives on this device. Offline access is for online projects.",
		);
	}
	if (!overview.subject) {
		return t(
			"settings:offlineAccess.unavailableSignedOut",
			"Sign in to manage offline access for this project.",
		);
	}
	return (
		overview.unavailableReason ??
		t(
			"settings:offlineAccess.unavailableOfflineApp",
			"This project only lives on this device. Offline access is for online projects.",
		)
	);
}

function OfflineAccessContent({
	appId,
	state,
}: Readonly<{ appId: string; state: IOfflineWritesState }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [syncing, setSyncing] = useState(false);
	const [syncError, setSyncError] = useState<string>();

	const overview = useQuery({
		queryKey: offlineQueryKeys.overview(appId),
		queryFn: () => state.getOverview(appId),
		meta: LIVE_ONLY,
	});
	const localOnly = useQuery({
		queryKey: ["offline-writes", appId, "local-only"],
		queryFn: async () => (await backend.isLocalOnly?.(appId)) ?? false,
		meta: LIVE_ONLY,
	});

	const refresh = useCallback(() => {
		void queryClient.invalidateQueries({
			queryKey: offlineQueryKeys.all(appId),
		});
	}, [queryClient, appId]);

	useOfflineEvents(state, appId, refresh);

	const syncNow = async () => {
		setSyncing(true);
		setSyncError(undefined);
		try {
			await state.syncNow(appId);
			refresh();
		} catch (cause) {
			setSyncError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setSyncing(false);
		}
	};

	if (overview.isLoading) {
		return (
			<div className="space-y-4">
				<Skeleton className="h-40 w-full" />
				<Skeleton className="h-32 w-full" />
			</div>
		);
	}

	if (!overview.data) {
		return (
			<Alert variant="destructive">
				<TriangleAlertIcon />
				<AlertDescription className="flex flex-wrap items-center gap-3">
					<span>{overview.error?.message}</span>
					<Button size="sm" variant="outline" onClick={() => refresh()}>
						{t("common:retry", "Retry")}
					</Button>
				</AlertDescription>
			</Alert>
		);
	}

	const data = overview.data;
	if (!data.available || localOnly.data) {
		return (
			<Alert>
				<CloudOffIcon />
				<AlertDescription>
					{unavailableMessage(t, data, localOnly.data === true)}
				</AlertDescription>
			</Alert>
		);
	}

	const hubUnsupported = data.hubSupport === false;

	return (
		<div className="space-y-5">
			<div className="flex flex-wrap items-center gap-3">
				<OfflineSyncBadge state={data.queue.syncState} account={data.subject} />
				<Button
					size="sm"
					variant="outline"
					disabled={syncing}
					onClick={() => void syncNow()}
				>
					<RefreshCwIcon
						className={syncing ? "mr-1.5 size-3 animate-spin" : "mr-1.5 size-3"}
					/>
					{t("settings:offlineAccess.syncNow", "Sync now")}
				</Button>
				{syncError && (
					<p role="alert" className="text-sm text-destructive">
						{syncError}
					</p>
				)}
			</div>
			{hubUnsupported && (
				<Alert variant="destructive">
					<TriangleAlertIcon />
					<AlertTitle>
						{t(
							"settings:offlineAccess.hubUnsupported",
							"This hub does not support offline access for desktop apps yet.",
						)}
					</AlertTitle>
				</Alert>
			)}
			{data.otherAccountsPending > 0 && (
				<Alert>
					<CloudOffIcon />
					<AlertDescription>
						{t("settings:offlineAccess.otherAccounts", {
							count: data.otherAccountsPending,
							defaultValue_one:
								"{{count}} change from another account on this device is waiting until that account signs in.",
							defaultValue_other:
								"{{count}} changes from another account on this device are waiting until that account signs in.",
						})}
					</AlertDescription>
				</Alert>
			)}
			<OfflineTablesCard
				appId={appId}
				state={state}
				tables={data.tables}
				enableBlocked={hubUnsupported}
				onChanged={refresh}
			/>
			<OfflineQueueCard
				appId={appId}
				state={state}
				queue={data.queue}
				onChanged={refresh}
			/>
			<OfflineStorageCard
				appId={appId}
				state={state}
				limits={data.limits}
				usage={data.usage}
				onChanged={refresh}
			/>
		</div>
	);
}

export function OfflineAccessPage({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const ready = useBackendReady();
	const state = backend.offlineWritesState;

	return (
		<div className="mx-auto flex w-full max-w-4xl flex-col gap-5 p-4 pb-8">
			<div>
				<h1 className="text-2xl font-bold tracking-tight">
					{t("settings:offlineAccess.title", "Offline access")}
				</h1>
				<p className="max-w-prose text-sm text-muted-foreground">
					{t(
						"settings:offlineAccess.description",
						"Choose which tables flows on this device can read and change while the hub is unreachable. Their data is downloaded as flows use it, or all at once if you choose. New files are always kept and uploaded later.",
					)}
				</p>
			</div>
			{ready && state && <OfflineAccessContent appId={appId} state={state} />}
		</div>
	);
}
