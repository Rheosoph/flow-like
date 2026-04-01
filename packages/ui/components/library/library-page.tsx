"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowDownAZ,
	Clock,
	Eye,
	EyeOff,
	FilesIcon,
	LayoutGridIcon,
	Plus,
	Search,
	Sparkles,
	X,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useMiniSearch } from "react-minisearch";
import { useInvoke } from "../../hooks/use-invoke";
import { useIsMobile } from "../../hooks/use-mobile";
import { formatAppCategory } from "../../lib/app-category";
import type { IProfileApp } from "../../lib/schema/profile/profile";
import { useBackend } from "../../state/backend-state";
import { useSpotlightStore } from "../../state/spotlight-state";
import { Button } from "../ui/button";
import { EmptyState } from "../ui/empty-state";
import { Input } from "../ui/input";
import { useMobileHeader } from "../ui/mobile-header";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import {
	FavoritesSection,
	JoinInline,
	JoinInlineExpanded,
	LibrarySkeleton,
	PinnedHero,
	SearchResults,
	Section,
} from "./library-sub-components";
import type { LibraryItem, SortMode } from "./library-types";
import { CATEGORY_COLORS, sortItems } from "./library-types";

export interface LibraryPageProps {
	onAppClick?: (appId: string) => void;
	extraToolbarActions?: React.ReactNode;
	extraMobileActions?: React.ReactNode[];
	renderExtras?: (helpers: {
		refetchApps: () => Promise<void>;
	}) => React.ReactNode;
}

export function LibraryPage({
	onAppClick: onAppClickProp,
	extraToolbarActions,
	extraMobileActions,
	renderExtras,
}: LibraryPageProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const router = useRouter();
	const [searchQuery, setSearchQuery] = useState("");
	const [visibilityMode, setVisibilityMode] = useState(false);
	const [sortMode, setSortMode] = useState<SortMode>("recent");
	const isMobile = useIsMobile();

	const handleAppClick = useCallback(
		(appId: string) => {
			if (onAppClickProp) {
				onAppClickProp(appId);
			} else {
				queryClient.invalidateQueries();
				router.push(`/use?id=${appId}`);
			}
		},
		[onAppClickProp, queryClient, router],
	);

	const appHref = useCallback(
		(appId: string) => `/use?id=${appId}`,
		[],
	);

	const handleSettingsClick = useCallback(
		(appId: string) => router.push(`/library/config?id=${appId}`),
		[router],
	);

	const profileAppMap = useMemo(() => {
		const map = new Map<
			string,
			{
				favorite: boolean;
				pinned: boolean;
				favorite_order?: number | null;
			}
		>();
		for (const a of currentProfile.data?.hub_profile.apps ?? []) {
			map.set(a.app_id, {
				favorite: a.favorite,
				pinned: a.pinned,
				favorite_order: a.favorite_order,
			});
		}
		return map;
	}, [currentProfile.data]);

	const activeAppIds = useMemo(
		() =>
			new Set(
				(currentProfile.data?.hub_profile.apps ?? []).map((a) => a.app_id),
			),
		[currentProfile.data],
	);

	const allAvailableItems = useMemo(() => {
		const map = new Map<string, LibraryItem>();
		for (const [app, meta] of apps.data ?? []) {
			if (meta) map.set(app.id, { ...meta, id: app.id, app });
		}
		return Array.from(map.values());
	}, [apps.data]);

	const allItems = useMemo(
		() =>
			currentProfile.data
				? allAvailableItems.filter((item) => activeAppIds.has(item.id))
				: [],
		[allAvailableItems, activeAppIds, currentProfile.data],
	);

	const itemsForDisplay = useMemo(
		() => (visibilityMode ? allAvailableItems : allItems),
		[visibilityMode, allAvailableItems, allItems],
	);

	const handleToggleVisibility = useCallback(
		async (appId: string) => {
			if (!currentProfile.data) return;
			const isCurrentlyActive = activeAppIds.has(appId);
			await backend.userState.updateProfileApp(
				currentProfile.data,
				{ app_id: appId, favorite: false, pinned: false },
				isCurrentlyActive ? "Remove" : "Upsert",
			);
			await currentProfile.refetch();
		},
		[currentProfile, activeAppIds, backend.userState],
	);

	const handleFavoriteReorder = useCallback(
		async (orderedIds: string[]) => {
			if (!currentProfile.data) return;
			for (let i = 0; i < orderedIds.length; i++) {
				const appId = orderedIds[i];
				const existing = profileAppMap.get(appId);
				await backend.userState.updateProfileApp(
					currentProfile.data,
					{
						app_id: appId,
						favorite: true,
						pinned: existing?.pinned ?? false,
						favorite_order: i,
					} as IProfileApp,
					"Upsert",
				);
			}
			await currentProfile.refetch();
		},
		[currentProfile, profileAppMap, backend.userState],
	);

	const pinnedItems = useMemo(
		() =>
			sortItems(
				allItems.filter((item) => profileAppMap.get(item.id)?.pinned),
				sortMode,
			),
		[allItems, profileAppMap, sortMode],
	);

	const favoriteItems = useMemo(() => {
		const favs = allItems.filter(
			(item) => profileAppMap.get(item.id)?.favorite,
		);
		return favs.toSorted((a, b) => {
			const orderA = profileAppMap.get(a.id)?.favorite_order ?? 999;
			const orderB = profileAppMap.get(b.id)?.favorite_order ?? 999;
			if (orderA !== orderB) return orderA - orderB;
			return (a.name ?? "").localeCompare(b.name ?? "");
		});
	}, [allItems, profileAppMap]);

	const recentItems = useMemo(
		() =>
			itemsForDisplay
				.toSorted(
					(a, b) =>
						(b.updated_at?.secs_since_epoch ?? 0) -
						(a.updated_at?.secs_since_epoch ?? 0),
				)
				.slice(0, 10),
		[itemsForDisplay],
	);

	const categorizedItems = useMemo(() => {
		const groups = new Map<string, LibraryItem[]>();
		for (const item of itemsForDisplay) {
			const label = formatAppCategory(item.app.primary_category);
			const existing = groups.get(label) ?? [];
			existing.push(item);
			groups.set(label, existing);
		}
		return Array.from(groups.entries())
			.map(([label, sectionItems]) => ({
				label,
				items: sortItems(sectionItems, sortMode),
			}))
			.toSorted((a, b) => a.label.localeCompare(b.label));
	}, [itemsForDisplay, sortMode]);

	const { addAll, removeAll, clearSearch, search, searchResults } =
		useMiniSearch(itemsForDisplay, {
			fields: [
				"name",
				"description",
				"long_description",
				"tags",
				"category",
				"id",
			],
		});

	useEffect(() => {
		if (itemsForDisplay.length > 0) {
			removeAll();
			addAll(itemsForDisplay);
		}
		return () => {
			removeAll();
			clearSearch();
		};
	}, [itemsForDisplay, removeAll, addAll, clearSearch]);

	const menuActions = useMemo(
		() => [...(extraMobileActions ?? []), <JoinInline key="join-inline" />],
		[extraMobileActions],
	);

	useMobileHeader({
		right: menuActions,
		title: "Library",
	});

	const isLoading = apps.isLoading || currentProfile.isLoading;
	const refetchApps = useCallback(async () => {
		await apps.refetch();
	}, [apps]);

	if (isLoading) {
		return (
			<main className="flex flex-col w-full flex-1 min-h-0">
				<LibrarySkeleton />
			</main>
		);
	}

	if (allItems.length === 0 && allAvailableItems.length === 0) {
		return (
			<main className="flex flex-col w-full flex-1 min-h-0 items-center justify-center px-4">
				<div className="w-full max-w-md space-y-10">
					<div className="text-center space-y-3">
						<h1 className="text-2xl font-semibold tracking-tight">
							Your Library
						</h1>
						<p className="text-sm text-muted-foreground/70">
							Apps you create or join will appear here
						</p>
					</div>

					<EmptyState
						action={[
							{
								label: "Create Your First App",
								onClick: () => {
									useSpotlightStore.getState().open();
									useSpotlightStore.getState().setMode("quick-create");
								},
							},
						]}
						icons={[Sparkles, LayoutGridIcon, FilesIcon]}
						className="w-full border border-dashed border-border/30 rounded-2xl bg-muted/5"
						title="No apps yet"
						description="Create your first app or join one with an invite link."
					/>

					<div className="relative">
						<div className="absolute inset-0 flex items-center">
							<span className="w-full border-t border-border/20" />
						</div>
						<div className="relative flex justify-center text-xs">
							<span className="bg-background px-3 text-muted-foreground/50">
								or join a project
							</span>
						</div>
					</div>

					<JoinInlineExpanded />
				</div>

				{renderExtras?.({ refetchApps })}
			</main>
		);
	}

	const isSearching = searchQuery.length > 0;
	const toggleSort = () =>
		setSortMode((s) => (s === "recent" ? "alpha" : "recent"));

	return (
		<main className="flex flex-col w-full flex-1 min-h-0">
			<div
				className={`pt-5 pb-3 space-y-3 ${isMobile ? "px-4" : "px-4 sm:px-8 pb-4"}`}
			>
				<div className="flex items-center gap-2">
					<div className="relative flex-1 max-w-lg">
						<Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40 pointer-events-none" />
						<Input
							placeholder="Search…"
							value={searchQuery}
							onChange={(e) => {
								search(e.target.value);
								setSearchQuery(e.target.value);
							}}
							className="pl-11 h-10 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50 transition-all text-sm"
						/>
						{searchQuery && (
							<button
								type="button"
								onClick={() => {
									setSearchQuery("");
									clearSearch();
								}}
								className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground transition-colors"
							>
								<X className="h-4 w-4" />
							</button>
						)}
					</div>

					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className={`h-8 w-8 rounded-full ${
										sortMode === "alpha"
											? "text-foreground/80 bg-muted/40"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={toggleSort}
								>
									{sortMode === "alpha" ? (
										<ArrowDownAZ className="h-4 w-4" />
									) : (
										<Clock className="h-4 w-4" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{sortMode === "alpha"
									? "Sorted A\u2013Z \u00B7 click for recent"
									: "Sorted by recent \u00B7 click for A\u2013Z"}
							</TooltipContent>
						</Tooltip>

						<JoinInline />

						{extraToolbarActions}

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									onClick={() => {
										useSpotlightStore.getState().open();
										useSpotlightStore.getState().setMode("quick-create");
									}}
								>
									<Plus className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>Create a new app</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant={visibilityMode ? "secondary" : "ghost"}
									size="icon"
									className={`h-8 w-8 rounded-full ${
										visibilityMode
											? "text-primary bg-primary/10"
											: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
									}`}
									onClick={() => setVisibilityMode((v) => !v)}
								>
									{visibilityMode ? (
										<EyeOff className="h-4 w-4" />
									) : (
										<Eye className="h-4 w-4" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{visibilityMode ? "Exit visibility mode" : "Show / hide apps"}
							</TooltipContent>
						</Tooltip>
					</div>
				</div>

				{visibilityMode && (
					<p className="text-xs text-muted-foreground/40">
						Click any app to toggle it in your library. Faded apps are hidden.
					</p>
				)}
			</div>

			{renderExtras?.({ refetchApps })}

			<div
				className={`flex-1 overflow-auto pb-8 ${isMobile ? "px-4" : "px-4 sm:px-8"}`}
			>
				{isSearching ? (
					<SearchResults
						items={(searchResults as LibraryItem[]) ?? []}
						query={searchQuery}
						onAppClick={handleAppClick}
						onSettingsClick={handleSettingsClick}
						visibilityMode={visibilityMode}
						activeAppIds={activeAppIds}
						onToggleVisibility={handleToggleVisibility}
						appHref={appHref}
						isMobile={isMobile}
					/>
				) : (
					<div className={isMobile ? "space-y-5" : "space-y-10"}>
						{pinnedItems.length > 0 && !visibilityMode && (
							<PinnedHero
								items={pinnedItems}
								onAppClick={handleAppClick}
								onSettingsClick={handleSettingsClick}
								appHref={appHref}
								isMobile={isMobile}
							/>
						)}

						{favoriteItems.length > 0 && !visibilityMode && (
							<FavoritesSection
								items={favoriteItems}
								onAppClick={handleAppClick}
								onSettingsClick={handleSettingsClick}
								onReorder={handleFavoriteReorder}
								appHref={appHref}
								isMobile={isMobile}
							/>
						)}

						{(pinnedItems.length > 0 || favoriteItems.length > 0) &&
							!visibilityMode && <div className="border-t border-border/10" />}

						{recentItems.length > 0 && (
							<Section
								title="Recently updated"
								icon={
									isMobile ? undefined : (
										<Clock className="h-3.5 w-3.5 text-muted-foreground/50" />
									)
								}
								items={recentItems}
								onAppClick={handleAppClick}
								onSettingsClick={handleSettingsClick}
								visibilityMode={visibilityMode}
								activeAppIds={activeAppIds}
								onToggleVisibility={handleToggleVisibility}
								appHref={appHref}
								isMobile={isMobile}
								showSeeAll={isMobile}
							/>
						)}

						{recentItems.length > 0 &&
							categorizedItems.length > 0 &&
							!isMobile && <div className="border-t border-border/10" />}

						{categorizedItems.map(({ label, items }) => (
							<Section
								key={label}
								title={label}
								items={items}
								onAppClick={handleAppClick}
								onSettingsClick={handleSettingsClick}
								visibilityMode={visibilityMode}
								activeAppIds={activeAppIds}
								onToggleVisibility={handleToggleVisibility}
								appHref={appHref}
								categoryColor={isMobile ? undefined : CATEGORY_COLORS[label]}
								isMobile={isMobile}
								showSeeAll={isMobile}
							/>
						))}
					</div>
				)}
			</div>
		</main>
	);
}
