"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, ArrowLeft, Globe, Users } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { ApiResponseError } from "../../lib/api-error";
import { getApiOrigin } from "../../lib/api-url";
import { GlobalPermission } from "../../lib/permission/global-permission";
import { asArray, isRecord } from "../../lib/response-shape";
import type { IProfile } from "../../lib/schema/profile/profile";
import { useBackend, useBackendReady } from "../../state/backend-state";
import { Button } from "../ui/button";
import { createDefaultHomeLayout } from "./catalog";
import { HomeEditor } from "./home-editor";
import { normalizeHomeLayout } from "./home-layout";
import {
	HomeLoading,
	homeDefaultsCacheKey,
	homeDefaultsQueryKey,
} from "./home-page";
import type { IHomeDefault, IHomeDefaults, IHomeLayout } from "./types";

export function AdminHomePage() {
	return (
		<Suspense fallback={<HomeLoading />}>
			<AdminHomePageContent />
		</Suspense>
	);
}

function AdminHomePageContent() {
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
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		ready,
		[origin, viewer, backend.profile?.id, auth?.isAuthenticated],
	);
	const permission = new GlobalPermission(info.data?.permission ?? 0);
	const allowed =
		!info.isError &&
		permission.hasPermission(GlobalPermission.WriteLandingPage);
	const searchParams = useSearchParams();
	const [defaultId, setDefaultId] = useState(
		() => searchParams.get("default") || "main",
	);
	const [editing, setEditing] = useState(false);
	const [conflictKey, setConflictKey] = useState<string | null>(null);
	const editSession = useRef<{
		key: string;
		revision: string | null | undefined;
		conflicted?: boolean;
	} | null>(null);
	const bundled = useMemo(createDefaultHomeLayout, []);
	const templates = useQuery({
		queryKey: ["home-default-templates", origin, viewer, profile.data?.id],
		queryFn: () => {
			const currentProfile = profile.data;
			if (!currentProfile) throw new Error("Profile context is not available");
			return backend.apiState.get<IProfile[]>(currentProfile, "info/profiles");
		},
		enabled: !!profile.data && allowed,
	});
	const profileId = profile.data?.id ?? "";
	const queryKey = homeDefaultsQueryKey(origin, viewer, profileId, defaultId);
	const editorKey = JSON.stringify(queryKey);
	const defaults = useQuery({
		queryKey,
		queryFn: () =>
			backend.userState.getHomeDefaults(
				defaultId === "main" ? undefined : defaultId,
			),
		enabled: ready && !!profile.data && allowed && !editing,
		staleTime: 0,
		retry: 1,
	});
	const selected =
		defaultId === "main" ? defaults.data?.main : defaults.data?.profile;
	const selection = useRef({
		key: editorKey,
		revision: selected?.revision ?? null,
	});
	selection.current = { key: editorKey, revision: selected?.revision ?? null };
	const onEditingChange = useCallback(
		(
			active: boolean,
			session?: { baseRevision: string | null | undefined },
		) => {
			if (active) {
				if (editSession.current?.key !== selection.current.key) {
					editSession.current = {
						key: selection.current.key,
						revision: session
							? session.baseRevision
							: selection.current.revision,
					};
				}
			} else {
				editSession.current = null;
				setConflictKey(null);
			}
			setEditing(active);
		},
		[],
	);
	const inherited =
		defaultId === "main"
			? bundled
			: (normalizeHomeLayout(defaults.data?.main?.layout) ?? bundled);
	const layout = normalizeHomeLayout(selected?.layout) ?? inherited;
	const conflicted =
		conflictKey === editorKey ||
		(editing &&
			editSession.current?.key === editorKey &&
			(editSession.current.conflicted ||
				editSession.current.revision !== (selected?.revision ?? null)));
	const saveBlocked = conflicted
		? "The published default changed. Copy your draft JSON before discarding it, then review the latest layout before applying your changes again."
		: undefined;
	const save = async (value: IHomeLayout | null) => {
		const session = editSession.current;
		if (!allowed || !profileId || !session || session.key !== editorKey) {
			throw new Error("Start editing this default before publishing changes.");
		}
		if (
			session.conflicted ||
			session.revision === undefined ||
			session.revision !== selection.current.revision
		) {
			throw new Error(
				"The published default changed. Copy your draft JSON, then discard it and review the latest layout before publishing.",
			);
		}
		let saved: IHomeDefault | null;
		try {
			saved = await backend.userState.saveHomeDefault(
				defaultId,
				value,
				session.revision,
			);
		} catch (error) {
			if (error instanceof ApiResponseError && error.status === 409) {
				session.conflicted = true;
				if (editSession.current === session) setConflictKey(editorKey);
				void defaults.refetch();
				throw new Error(
					"Another administrator published a newer default. Your draft is still open; review the recovery guidance above the editor.",
				);
			}
			throw error;
		}
		const previous = defaults.data ?? { main: null, profile: null };
		const refreshed: IHomeDefaults =
			defaultId === "main"
				? { ...previous, main: saved }
				: { ...previous, profile: saved };
		queryClient.setQueryData(queryKey, refreshed);
		try {
			localStorage.setItem(
				homeDefaultsCacheKey(origin, viewer, profileId, defaultId),
				JSON.stringify(refreshed),
			);
		} catch {
			/* Persistent cache is optional. */
		}
		await queryClient.invalidateQueries({ queryKey: ["home-defaults"] });
	};
	if (!ready || profile.isLoading || info.isLoading) return <HomeLoading />;
	if (!allowed)
		return (
			<div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
				<AlertCircle className="h-7 w-7 text-muted-foreground" />
				<h1 className="font-semibold">Home publishing permission required</h1>
				<p className="max-w-md text-sm text-muted-foreground">
					An administrator with permission to edit landing pages can publish
					defaults here.
				</p>
				<Button variant="outline" asChild>
					<Link href="/admin">Back to admin</Link>
				</Button>
			</div>
		);
	return (
		<main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
			<div className="flex shrink-0 flex-wrap items-center gap-3 border-b border-border/50 px-5 py-3 text-xs text-muted-foreground">
				<Link
					href="/admin"
					className="flex items-center gap-1.5 hover:text-foreground"
				>
					<ArrowLeft className="h-3.5 w-3.5" />
					Admin
				</Link>
				<span className="hidden sm:inline">
					Publish a shared starting point. Personal layouts stay personal.
				</span>
			</div>
			{conflicted && (
				<div
					role="alert"
					className="shrink-0 border-b border-amber-500/30 bg-amber-500/10 px-5 py-3 text-sm"
				>
					<p className="font-medium">The published default has changed</p>
					<p className="mt-1 text-xs leading-relaxed text-muted-foreground">
						Your draft is still open and has not replaced the published layout.
						Use Edit JSON and Copy to keep your changes. Then choose Cancel to
						discard this draft and Edit default to review the latest layout
						before applying your changes again.
					</p>
				</div>
			)}
			{defaults.isError && !editing ? (
				<div className="flex flex-1 flex-col items-center justify-center gap-3 p-8">
					<p className="text-sm text-muted-foreground">
						The published default could not be loaded.
					</p>
					<Button variant="outline" onClick={() => void defaults.refetch()}>
						Try again
					</Button>
				</div>
			) : defaults.isLoading ? (
				<HomeLoading />
			) : (
				<HomeEditor
					key={editorKey}
					draftKey={JSON.stringify([
						"home-default",
						origin,
						viewer,
						profileId,
						defaultId,
					])}
					draftRevision={selected?.revision ?? null}
					saveBlocked={saveBlocked}
					disabled={!editing && defaults.isFetching}
					admin
					layout={layout}
					defaultLayout={inherited}
					onSave={save}
					onReset={() => save(null)}
					sourceLabel={
						selected
							? "Published default"
							: defaultId === "main"
								? "Bundled fallback"
								: "Following main default"
					}
					onEditingChange={onEditingChange}
					toolbar={
						<label className="flex items-center gap-2">
							{defaultId === "main" ? (
								<Globe className="h-3.5 w-3.5 text-muted-foreground" />
							) : (
								<Users className="h-3.5 w-3.5 text-muted-foreground" />
							)}
							<span className="sr-only">Default home target</span>
							<select
								className="h-8 max-w-60 rounded-md border border-input bg-background px-2 text-xs"
								value={defaultId}
								disabled={editing}
								onChange={(event) => setDefaultId(event.target.value)}
							>
								<option value="main">Main backend default</option>
								{asArray(templates.data)
									.filter(
										(item): item is IProfile & { id: string } =>
											isRecord(item) &&
											typeof item.id === "string" &&
											item.id.length > 0 &&
											item.id !== "main",
									)
									.map((item) => (
										<option key={item.id} value={item.id}>
											{item.name}
										</option>
									))}
							</select>
						</label>
					}
				/>
			)}
		</main>
	);
}
