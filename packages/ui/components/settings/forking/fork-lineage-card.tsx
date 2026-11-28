"use client";

import { useTranslation } from "@flow-like/locales";
import { GitForkIcon } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useInvoke } from "../../../hooks";
import { formatAbsoluteDateTime, formatRelativeTime } from "../../../lib/date";
import { useBackend } from "../../../state/backend-state";
import type { IApp } from "../../../types";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { Skeleton } from "../../ui/skeleton";

export interface ForkLineageCardProps {
	localApp: IApp;
}

interface IForkOrigin {
	name: string;
	href: string;
}

/**
 * The original as the viewer can reach it: from their own library first,
 * then from the store. Not `getAppMeta`: the desktop caches whatever it
 * fetches there as local metadata of an app the viewer may not even hold.
 */
function useForkOrigin(sourceId: string) {
	const backend = useBackend();
	const { i18n } = useTranslation();
	const library = useInvoke(backend.appState.getApps, backend.appState, []);
	const own = useMemo(
		() => library.data?.find(([app]) => app.id === sourceId),
		[library.data, sourceId],
	);
	const store = useInvoke(
		backend.appState.searchApps,
		backend.appState,
		[sourceId, undefined, i18n.language],
		library.isFetched && !own,
	);
	const listed = store.data?.find(([app]) => app.id === sourceId);

	const origin = useMemo<IForkOrigin | undefined>(() => {
		if (own)
			return {
				name: own[1]?.name || sourceId,
				href: `/library/config?id=${sourceId}`,
			};
		if (listed)
			return {
				name: listed[1]?.name || sourceId,
				href: `/store?id=${sourceId}`,
			};
		return undefined;
	}, [own, listed, sourceId]);

	const loading = !library.isFetched || (!own && store.isLoading);
	return { origin, loading };
}

function ForkOrigin({ sourceId }: Readonly<{ sourceId: string }>) {
	const { t } = useTranslation("settings");
	const { origin, loading } = useForkOrigin(sourceId);

	if (loading) return <Skeleton className="h-5 w-40" />;
	if (origin)
		return (
			<Link
				href={origin.href}
				className="font-medium text-foreground underline-offset-4 hover:underline"
			>
				{origin.name}
			</Link>
		);
	return (
		<div className="space-y-1">
			<p className="text-muted-foreground">
				{t(
					"theOriginalIsNoLongerAvailableToYou",
					"The original is no longer available to you.",
				)}
			</p>
			<code className="block truncate font-mono text-xs text-muted-foreground">
				{sourceId}
			</code>
		</div>
	);
}

/**
 * Names the app this one was forked from. Renders nothing for apps that are
 * no fork.
 */
export function ForkLineageCard({ localApp }: Readonly<ForkLineageCardProps>) {
	const { t, i18n } = useTranslation("settings");
	const sourceId = localApp.forked_from;
	if (!sourceId) return null;

	const forkedAt = localApp.forked_at;
	const when = formatRelativeTime(forkedAt, "long", "", {
		locale: i18n.language,
	});

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<GitForkIcon className="h-4 w-4" />
					{t("forkedFrom", "Forked from")}
				</CardTitle>
			</CardHeader>
			<CardContent className="space-y-1 text-sm">
				<ForkOrigin sourceId={sourceId} />
				{when && (
					<p
						className="text-xs text-muted-foreground"
						title={formatAbsoluteDateTime(forkedAt)}
					>
						{t("forkedWhen", "Forked {{when}}", { when })}
					</p>
				)}
			</CardContent>
		</Card>
	);
}
