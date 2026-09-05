"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Layers } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { useAppCategoryLabel } from "../../../lib/app-category";
import {
	APP_CATEGORY_ORDER,
	CATEGORY_ICONS,
	categoryColor,
} from "../../../lib/category-meta";
import type { IApp, IAppCategory } from "../../../lib/schema/app/app";
import { IAppSearchSort } from "../../../lib/schema/app/app-search-query";
import type { IBit, IMetadata } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/hub/bit-search-query";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { PackageCard } from "../../store/package-card";
import { AppCard, SpotlightCard } from "../../ui/app-card";
import { AppTypeMark } from "../../ui/app-type-mark";
import { ModelCard } from "../../ui/model-card";
import { ModelDetailSheet } from "../../ui/model-detail-sheet";
import {
	type HomeContentProps,
	homeAppRendering,
	homeModelRendering,
	homePackageRendering,
	numberConfig,
	stringList,
	textConfig,
} from "./config";
import { HomeEmpty, HomeQueryState, useHomeScope } from "./shared";

type AppPair = [IApp, IMetadata | undefined];

export function useHomeLibrary() {
	const backend = useBackend();
	const scope = useHomeScope();
	return useQuery({
		queryKey: ["home", ...scope, "library"],
		queryFn: () => backend.appState.getApps(),
		staleTime: 60_000,
	});
}

export function HomeAppCollection({ widget }: HomeContentProps) {
	const router = useRouter();
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const source = textConfig(widget.config, "source", "library");
	const category = textConfig(widget.config, "category");
	const tag = textConfig(widget.config, "tag");
	const query = textConfig(widget.config, "query");
	const appIds = stringList(widget.config, "appIds");
	const limit = numberConfig(widget.config, "limit", 8);
	const usesProfile = ["library", "recent", "favorites"].includes(source);
	const profile = useQuery({
		queryKey: ["home", ...scope, "profile-apps"],
		queryFn: () => backend.userState.getProfile(),
		enabled: usesProfile,
	});
	const remote = source === "new" || source === "popular";
	const results = useQuery({
		queryKey: [
			"home",
			...scope,
			"collection",
			source,
			category,
			tag,
			query,
			appIds,
			limit,
		],
		enabled: remote || source === "manual",
		queryFn: async (): Promise<AppPair[]> => {
			if (source === "manual") {
				return Promise.all(
					appIds.slice(0, limit).map(async (id): Promise<AppPair> => {
						const [app, metadata] = await Promise.all([
							backend.appState.getApp(id),
							backend.appState.getAppMeta(id).catch(() => undefined),
						]);
						return [app, metadata];
					}),
				);
			}
			return backend.appState.searchApps(
				undefined,
				query || undefined,
				undefined,
				(category || undefined) as IAppCategory | undefined,
				undefined,
				source === "popular"
					? IAppSearchSort.MostPopular
					: IAppSearchSort.NewestCreated,
				tag || undefined,
				0,
				limit,
			);
		},
		staleTime: 60_000,
	});
	const rows = useMemo(() => {
		let apps = [
			...((remote || source === "manual" ? results.data : library.data) ?? []),
		];
		if (usesProfile) {
			const visible = new Set(
				(profile.data?.apps ?? []).map((app) => app.app_id),
			);
			apps = apps.filter(([app]) => visible.has(app.id));
		}
		if (source === "favorites") {
			const favorites = new Map(
				(profile.data?.apps ?? [])
					.filter((app) => app.favorite)
					.map((app) => [app.app_id, app.favorite_order ?? 0]),
			);
			apps = apps
				.filter(([app]) => favorites.has(app.id))
				.sort(
					([a], [b]) => (favorites.get(a.id) ?? 0) - (favorites.get(b.id) ?? 0),
				);
		}
		if (source === "recent")
			apps.sort(
				([a], [b]) =>
					b.updated_at.secs_since_epoch - a.updated_at.secs_since_epoch,
			);
		if (!remote) {
			if (category)
				apps = apps.filter(
					([app]) =>
						app.primary_category === category ||
						app.secondary_category === category,
				);
			if (tag)
				apps = apps.filter(([, meta]) =>
					meta?.tags.some((value) => value.toLowerCase() === tag.toLowerCase()),
				);
			if (query)
				apps = apps.filter(([app, meta]) =>
					`${meta?.name ?? app.id} ${meta?.description ?? ""}`
						.toLowerCase()
						.includes(query.toLowerCase()),
				);
		}
		return apps.slice(0, limit);
	}, [
		remote,
		usesProfile,
		source,
		results.data,
		library.data,
		profile.data,
		category,
		tag,
		query,
		limit,
	]);
	const state = remote || source === "manual" ? results : library;
	if (
		state.isLoading ||
		state.isError ||
		(usesProfile && (profile.isLoading || profile.isError))
	)
		return (
			<HomeQueryState
				loading={state.isLoading || profile.isLoading}
				error={state.isError || profile.isError}
				retry={() => {
					void state.refetch();
					if (usesProfile) void profile.refetch();
				}}
			/>
		);
	if (!rows.length)
		return (
			<HomeEmpty icon={<Layers className="size-7 opacity-50" />}>
				{source === "manual"
					? "Choose apps for this collection in widget settings."
					: source === "favorites"
						? "Favorite an app in your library to see it here."
						: "No apps match this collection yet."}
			</HomeEmpty>
		);
	const rendering = homeAppRendering(widget.config, widget.appearance.variant);
	const maxColumns =
		Number(widget.config.maxColumns) > 0
			? Math.min(6, numberConfig(widget.config, "maxColumns", 2))
			: 0;
	const owned = new Set((library.data ?? []).map(([app]) => app.id));
	return (
		<div
			data-home-collection-rendering={rendering}
			style={
				maxColumns && ["compact", "standard"].includes(rendering)
					? {
							gridTemplateColumns: `repeat(auto-fit, minmax(min(100%, max(260px, calc((100% - ${(maxColumns - 1) * 16}px) / ${maxColumns}))), 1fr))`,
						}
					: undefined
			}
			className={cn(
				"@container/home-apps min-w-0",
				rendering === "carousel"
					? "flex snap-x snap-proximity gap-4 overflow-x-auto pb-2"
					: rendering === "list" || rendering === "editorial"
						? "space-y-3"
						: rendering === "icons"
							? "grid grid-cols-[repeat(auto-fill,minmax(min(100%,96px),1fr))] gap-x-3 gap-y-4"
							: rendering === "compact"
								? "grid auto-rows-max grid-cols-[repeat(auto-fit,minmax(min(100%,260px),1fr))] gap-4"
								: "grid auto-rows-max grid-cols-[repeat(auto-fill,minmax(min(100%,240px),1fr))] gap-4",
			)}
		>
			{rows.map(([app, metadata]) => {
				const isOwned = owned.has(app.id);
				const href = isOwned
					? `/use?id=${encodeURIComponent(app.id)}`
					: `/store?id=${encodeURIComponent(app.id)}`;
				const title = metadata?.name ?? app.id;
				const updated = new Date(app.updated_at.secs_since_epoch * 1000);
				if (rendering === "editorial")
					return (
						<SpotlightCard
							key={app.id}
							app={app}
							metadata={metadata}
							isOwned={isOwned}
							href={href}
							className="min-w-0 sm:grid-cols-1 @min-[480px]/home-apps:grid-cols-[160px_minmax(0,1fr)] [&>div:first-child]:min-h-40 @min-[480px]/home-apps:[&>div:first-child]:min-h-0"
						/>
					);
				if (rendering === "icons")
					return (
						<Link
							key={app.id}
							href={href}
							className="group flex min-w-0 flex-col items-center gap-2.5 rounded-xl px-2 py-3 text-center transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
						>
							<AppTypeMark
								type={app.app_type}
								size={56}
								src={metadata?.icon ?? undefined}
								fallback={title.slice(0, 2).toUpperCase()}
							/>
							<span className="line-clamp-2 text-xs font-medium leading-snug">
								{title}
							</span>
						</Link>
					);
				return (
					<div
						key={app.id}
						className={cn(
							"min-w-0",
							rendering === "carousel" &&
								"w-[min(100%,264px)] shrink-0 snap-start pt-1",
						)}
					>
						<div className="min-w-0 [&>div]:h-auto">
							<AppCard
								app={app}
								metadata={metadata}
								isOwned={isOwned}
								variant={
									rendering === "compact" || rendering === "list"
										? "small"
										: "extended"
								}
								href={href}
								onClick={() => router.push(href)}
								className="w-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
							/>
						</div>
						{widget.config.showUpdated === true &&
							Number.isFinite(updated.getTime()) && (
								<p
									data-home-app-updated
									className="mt-2 px-1 text-[10px] leading-4 text-muted-foreground"
								>
									Updated{" "}
									{updated.toLocaleDateString(undefined, {
										month: "short",
										day: "numeric",
										year: "numeric",
									})}
								</p>
							)}
					</div>
				);
			})}
		</div>
	);
}

export function HomeCategories({ widget }: HomeContentProps) {
	const label = useAppCategoryLabel();
	const selected = stringList(widget.config, "categories");
	const categories = APP_CATEGORY_ORDER.filter(
		(category) => !selected.length || selected.includes(category),
	);
	const tiles =
		textConfig(widget.config, "rendering", widget.appearance.variant) ===
		"grid";
	return (
		<div
			className={cn(
				"flex min-w-0 content-start flex-wrap gap-2",
				tiles && "grid grid-cols-[repeat(auto-fit,minmax(125px,1fr))]",
			)}
		>
			{categories.map((category) => {
				const Icon = CATEGORY_ICONS[category];
				return (
					<Link
						key={category}
						href={`/store/explore/apps?category=${category}`}
						className={cn(
							"flex items-center gap-2 rounded-full border bg-background/40 px-3 py-2.5 text-xs font-medium transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
							tiles && "flex-col rounded-xl py-4",
						)}
					>
						<Icon
							className={tiles ? "size-6" : "size-4"}
							style={{ color: categoryColor(category) }}
						/>
						{label(category)}
					</Link>
				);
			})}
		</div>
	);
}

export function HomePackages({ widget }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const query = textConfig(widget.config, "query");
	const category = textConfig(widget.config, "category");
	const limit = numberConfig(widget.config, "limit");
	const sort = textConfig(widget.config, "sort", "downloads");
	const results = useQuery({
		queryKey: ["home", ...scope, "packages", query, category, limit, sort],
		queryFn: () =>
			backend.registryState.searchPackages({
				query,
				category: category || undefined,
				limit,
				sortBy: sort as "downloads",
				sortDesc: true,
			}),
		staleTime: 60_000,
	});
	if (results.isLoading || results.isError)
		return (
			<HomeQueryState
				loading={results.isLoading}
				error={results.isError}
				retry={() => void results.refetch()}
			/>
		);
	if (!results.data?.packages.length)
		return (
			<HomeEmpty
				action={
					<Link
						href="/store/packages"
						className="text-primary underline underline-offset-4"
					>
						Explore all packages
					</Link>
				}
			>
				No packages match this search.
			</HomeEmpty>
		);
	const rendering = homePackageRendering(
		widget.config,
		widget.appearance.variant,
	);
	return (
		<div
			data-home-package-rendering={rendering}
			className="grid min-w-0 auto-rows-max content-start grid-cols-[repeat(auto-fit,minmax(min(100%,260px),1fr))] gap-4"
		>
			{results.data.packages.map((pkg) => (
				<PackageCard key={pkg.id} pkg={pkg} variant={rendering} />
			))}
		</div>
	);
}

export function HomeModels({ widget, editing }: HomeContentProps) {
	const [selectedModel, setSelectedModel] = useState<IBit | null>(null);
	const queryClient = useQueryClient();
	const backend = useBackend();
	const scope = useHomeScope();
	const query = textConfig(widget.config, "query");
	const source = textConfig(widget.config, "source", "profile");
	const limit = numberConfig(widget.config, "limit");
	const results = useQuery({
		queryKey: ["home", ...scope, "models", source, query, limit],
		queryFn: async () => {
			const bits =
				source === "profile"
					? await backend.bitState.getProfileBits()
					: await backend.bitState.searchBits({
							search: query,
							bit_types: [
								IBitTypes.Llm,
								IBitTypes.Vlm,
								IBitTypes.Embedding,
								IBitTypes.ImageEmbedding,
								IBitTypes.ObjectDetection,
								IBitTypes.VideoGeneration,
								IBitTypes.ImageGeneration,
								IBitTypes.Stt,
								IBitTypes.Tts,
							],
							limit,
						});
			return bits
				.filter(
					(bit) =>
						[
							"Llm",
							"Vlm",
							"Embedding",
							"ImageEmbedding",
							"ObjectDetection",
							"VideoGeneration",
							"ImageGeneration",
							"Stt",
							"Tts",
						].includes(bit.type) &&
						(!query ||
							JSON.stringify(bit.meta)
								.toLowerCase()
								.includes(query.toLowerCase())),
				)
				.slice(0, limit)
				.map((bit) => {
					const meta = bit.meta.en ?? Object.values(bit.meta)[0];
					return bit.meta.en || !meta
						? bit
						: { ...bit, meta: { ...bit.meta, en: meta } };
				});
		},
		staleTime: 60_000,
	});
	if (results.isLoading || results.isError)
		return (
			<HomeQueryState
				loading={results.isLoading}
				error={results.isError}
				retry={() => void results.refetch()}
			/>
		);
	if (!results.data?.length)
		return (
			<HomeEmpty>
				{source === "profile"
					? "No models match this profile selection. Choose Explore available models in widget settings to add models."
					: "No models match this search. Try a different search in widget settings."}
			</HomeEmpty>
		);
	const rendering = homeModelRendering(
		widget.config,
		widget.appearance.variant,
	);
	const refreshModels = () =>
		queryClient.invalidateQueries({ queryKey: ["home", ...scope, "models"] });
	return (
		<>
			<div
				data-home-collection-rendering={rendering}
				className={cn(
					"@container/home-models grid min-w-0 auto-rows-max gap-3",
					rendering === "list"
						? "grid-cols-1"
						: "grid-cols-[repeat(auto-fill,minmax(min(100%,260px),1fr))]",
				)}
			>
				{results.data.map((bit) => (
					<ModelCard
						key={`${bit.hub}:${bit.id}`}
						bit={bit}
						variant={rendering === "list" ? "list" : "grid"}
						queryScope={scope}
						onProfileChange={refreshModels}
						onClick={editing ? undefined : setSelectedModel}
					/>
				))}
			</div>
			{!editing && selectedModel && (
				<ModelDetailSheet
					bit={selectedModel}
					queryScope={scope}
					onProfileChange={refreshModels}
					open
					onOpenChange={(open) => !open && setSelectedModel(null)}
					webMode={!backend.capabilities().canExecuteLocally}
				/>
			)}
		</>
	);
}
