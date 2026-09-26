"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CloudOffIcon } from "lucide-react";
import { useEffect } from "react";
import type { IOfflineWritesState } from "../../../state/backend-state/offline-writes-state";
import { Badge } from "../../ui/badge";
import { LIVE_ONLY, offlineQueryKeys } from "./offline-access-logic";

/** Marks a Data Studio table that this device reads and writes through its offline copy. */
export function OfflineManagedBadge({
	appId,
	table,
	userScoped,
	state,
}: Readonly<{
	appId: string;
	table: string;
	userScoped?: boolean;
	state: IOfflineWritesState;
}>) {
	const { t } = useTranslation("settings");
	const queryClient = useQueryClient();
	const queryKey = [
		...offlineQueryKeys.all(appId),
		"route",
		table,
		userScoped ?? false,
	];
	const route = useQuery({
		queryKey,
		queryFn: () => state.getTableRoute(appId, table, userScoped),
		meta: LIVE_ONLY,
	});

	useEffect(
		() =>
			state.subscribeTables((event) => {
				if (event.appId === appId) {
					void queryClient.invalidateQueries({
						queryKey: offlineQueryKeys.all(appId),
					});
				}
			}),
		[state, appId, queryClient],
	);

	if (route.data !== "device") return null;
	return (
		<Badge variant="secondary">
			<CloudOffIcon />
			{t(
				"settings:offlineAccess.dataStudioManagedBadge",
				"Offline on this device",
			)}
		</Badge>
	);
}
