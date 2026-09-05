"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CloudOff, RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { getApiOrigin } from "../../lib/api-url";
import { useBackend, useBackendReady } from "../../state/backend-state";
import {
	useRequestFabBubble,
	useSuppressFabBubble,
} from "../../state/fab-bubble";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { createDefaultHomeLayout } from "./catalog";
import { HomeEditor } from "./home-editor";
import { normalizeHomeLayout, resolveHomeLayout } from "./home-layout";
import type { IHomeDefaults, IHomeLayout } from "./types";

export function homeDefaultsQueryKey(
	origin: string,
	viewer: string,
	profileId: string,
	defaultId?: string | null,
) {
	return [
		"home-defaults",
		origin,
		viewer,
		profileId,
		defaultId ?? "main",
	] as const;
}

export function homeDefaultsCacheKey(
	origin: string,
	viewer: string,
	profileId: string,
	defaultId?: string | null,
) {
	return `flow-like:home-defaults:v1:${homeDefaultsQueryKey(origin, viewer, profileId, defaultId).slice(1).map(encodeURIComponent).join(":")}`;
}

export function readCachedHomeDefaults(key: string): IHomeDefaults | undefined {
	try {
		const raw = localStorage.getItem(key);
		if (!raw) return undefined;
		const value = JSON.parse(raw) as IHomeDefaults;
		if (
			value &&
			[value.main, value.profile].every(
				(item) =>
					item === null ||
					(typeof item?.id === "string" &&
						typeof item.revision === "string" &&
						normalizeHomeLayout(item.layout)),
			)
		)
			return value;
	} catch {
		/* Storage may be unavailable in a private browser session. */
	}
	return undefined;
}

export function HomePage() {
	const backend = useBackend();
	const auth = useAuth();
	const viewer = auth?.user?.profile?.sub
		? `user:${auth.user.profile.sub}`
		: "local";
	const origin = getApiOrigin(backend.profile);
	const ready = useBackendReady();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready,
		[origin, viewer, backend.profile?.id, auth?.isAuthenticated],
	);
	const [editing, setEditing] = useState(false);
	const bundled = useMemo(createDefaultHomeLayout, []);
	const defaultId = profile.data?.home_default_id;
	const profileId = profile.data?.id ?? "";
	const cacheKey = homeDefaultsCacheKey(origin, viewer, profileId, defaultId);
	const defaults = useQuery({
		queryKey: homeDefaultsQueryKey(origin, viewer, profileId, defaultId),
		queryFn: async () => {
			const value = await backend.userState.getHomeDefaults(
				defaultId ?? undefined,
			);
			try {
				localStorage.setItem(cacheKey, JSON.stringify(value));
			} catch {
				/* The page works without a persistent cache. */
			}
			return value;
		},
		enabled: ready && !!profile.data,
		initialData: () => readCachedHomeDefaults(cacheKey),
		initialDataUpdatedAt: 0,
		staleTime: 30_000,
		refetchInterval: editing ? false : 60_000,
		refetchOnWindowFocus: !editing,
		retry: 1,
	});
	const inherited = resolveHomeLayout(null, defaults.data, bundled);
	const resolved = resolveHomeLayout(
		profile.data?.home_layout,
		defaults.data,
		bundled,
	);
	useRequestFabBubble(
		!editing &&
			!resolved.layout.widgets.some((widget) => widget.type === "flowpilot"),
	);
	useSuppressFabBubble(editing);
	const save = async (layout: IHomeLayout | null) => {
		const id = profile.data?.id;
		if (!id) throw new Error("Choose a profile before saving your home.");
		await backend.userState.saveHomeLayout(layout, id);
		await queryClient.invalidateQueries({
			queryKey: [backend.userState.getProfile.name],
		});
		await profile.refetch();
	};
	if (!ready || profile.isLoading) return <HomeLoading />;
	return (
		<main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
			{profile.isError && (
				<div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 bg-muted/40 px-5 py-3 text-xs text-muted-foreground">
					<span className="flex items-center gap-2">
						<CloudOff className="h-4 w-4" />
						Your profile could not be loaded. Showing the default home.
					</span>
					<Button
						size="sm"
						variant="ghost"
						onClick={() => void profile.refetch()}
					>
						<RefreshCw className="h-3 w-3" />
						Retry
					</Button>
				</div>
			)}
			{defaults.isError && !profile.isError && (
				<div className="flex items-center gap-2 px-5 py-2 text-xs text-muted-foreground">
					<CloudOff className="h-3.5 w-3.5" />
					{defaults.data
						? "Showing the last available default. It will refresh when the connection returns."
						: "The published default is unavailable. Using the bundled home."}
				</div>
			)}
			<HomeEditor
				key={JSON.stringify([origin, viewer, profileId])}
				draftKey={JSON.stringify(["home", origin, viewer, profileId])}
				layout={resolved.layout}
				defaultLayout={inherited.layout}
				onSave={save}
				onReset={() => save(null)}
				sourceLabel={
					resolved.source === "personal"
						? "Your layout"
						: resolved.source === "profile"
							? "Profile default"
							: "Default layout"
				}
				disabled={!profile.data?.id}
				onEditingChange={setEditing}
			/>
		</main>
	);
}

export function HomeLoading() {
	return (
		<div
			className="flex min-h-0 flex-1 flex-col gap-5 overflow-hidden p-5 sm:p-8"
			aria-label="Loading home"
		>
			<Skeleton className="h-9 w-44" />
			<Skeleton className="h-36 w-full rounded-2xl" />
			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
				{[0, 1, 2].map((id) => (
					<Skeleton key={id} className="h-48 rounded-2xl" />
				))}
			</div>
		</div>
	);
}
