"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight, Box, Check, Download, Layers, Star } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useAppCategoryLabel } from "../../../lib/app-category";
import {
	APP_CATEGORY_ORDER,
	CATEGORY_ICONS,
	categoryColor,
} from "../../../lib/category-meta";
import type { IApp, IAppCategory } from "../../../lib/schema/app/app";
import { IAppSearchSort } from "../../../lib/schema/app/app-search-query";
import type { IMetadata } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/hub/bit-search-query";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { BitCard } from "../../ui/bit-card";
import {
	type HomeContentProps,
	numberConfig,
	stringList,
	textConfig,
} from "./config";
import {
	HomeEmpty,
	HomeQueryState,
	homeItemClass,
	useHomeScope,
} from "./shared";

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
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const source = textConfig(widget.config, "source", "library");
	const category = textConfig(widget.config, "category");
	const tag = textConfig(widget.config, "tag");
	const query = textConfig(widget.config, "query");
	const appIds = stringList(widget.config, "appIds");
	const limit = numberConfig(widget.config, "limit", 8);
	const profile = useQuery({
		queryKey: ["home", ...scope, "profile-apps"],
		queryFn: () => backend.userState.getProfile(),
		enabled: source === "favorites",
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
		(source === "favorites" && (profile.isLoading || profile.isError))
	)
		return (
			<HomeQueryState
				loading={state.isLoading || profile.isLoading}
				error={state.isError || profile.isError}
				retry={() => {
					void state.refetch();
					if (source === "favorites") void profile.refetch();
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
	const variant = widget.appearance.variant;
	const owned = new Set((library.data ?? []).map(([app]) => app.id));
	return (
		<div
			className={cn(
				"h-full min-h-0 overflow-auto p-3",
				variant === "carousel"
					? "flex snap-x gap-3"
					: variant === "list"
						? "space-y-2"
						: variant === "icons"
							? "grid auto-rows-max content-start grid-cols-[repeat(auto-fit,minmax(84px,1fr))] gap-3"
							: "grid auto-rows-max content-start grid-cols-[repeat(auto-fit,minmax(min(100%,230px),1fr))] gap-3",
			)}
		>
			{rows.map(([app, metadata], index) => {
				const href = owned.has(app.id)
					? `/use?id=${encodeURIComponent(app.id)}`
					: `/store?id=${encodeURIComponent(app.id)}`;
				const title = metadata?.name ?? app.id;
				const image = metadata?.thumbnail ?? metadata?.icon;
				const editorial = variant === "editorial" || variant === "spotlight";
				return (
					<Link
						key={app.id}
						href={href}
						className={cn(
							homeItemClass,
							"relative overflow-hidden",
							variant === "carousel" &&
								"w-64 shrink-0 snap-start flex-col items-start",
							variant === "icons" &&
								"flex-col border-0 bg-transparent p-2 text-center",
							editorial && "flex-col items-start p-0",
							editorial && index === 0 && "col-span-full",
						)}
					>
						{editorial && image ? (
							<img
								src={image}
								alt=""
								loading="lazy"
								className={cn(
									"h-28 w-full object-cover",
									index === 0 && "h-40",
								)}
							/>
						) : (
							<div
								className={cn(
									"flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-xl bg-primary/10 text-primary",
									variant === "icons" && "size-14",
								)}
							>
								{metadata?.icon ? (
									<img
										src={metadata.icon}
										alt=""
										loading="lazy"
										className="size-full object-cover"
									/>
								) : (
									<Box className="size-5" />
								)}
							</div>
						)}
						<div className={cn("min-w-0 flex-1", editorial && "w-full p-4")}>
							<div className="flex items-center justify-between gap-2">
								<span
									className={cn(
										"line-clamp-2 text-sm font-semibold",
										editorial && index === 0 && "text-xl",
									)}
								>
									{title}
								</span>
								{variant !== "icons" && (
									<ArrowUpRight className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-primary" />
								)}
							</div>
							{variant !== "icons" && (
								<>
									<p className="mt-1 line-clamp-2 text-xs leading-relaxed text-muted-foreground">
										{metadata?.description ||
											app.primary_category ||
											"Open this app"}
									</p>
									<div className="mt-3 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
										{owned.has(app.id) && (
											<span className="flex items-center gap-1 text-emerald-500">
												<Check className="size-3" />
												In your library
											</span>
										)}
										{app.avg_rating != null && app.rating_count > 0 && (
											<span className="flex items-center gap-1">
												<Star className="size-3" />
												{app.avg_rating.toFixed(1)}
											</span>
										)}
										{remote && (
											<span className="flex items-center gap-1">
												<Download className="size-3" />
												{app.download_count.toLocaleString()}
											</span>
										)}
									</div>
								</>
							)}
						</div>
					</Link>
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
	const tiles = widget.appearance.variant === "grid";
	return (
		<div
			className={cn(
				"flex h-full content-start flex-wrap gap-2 overflow-auto p-4",
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
		return <HomeEmpty>No packages match this search.</HomeEmpty>;
	return (
		<div
			className={cn(
				"h-full overflow-auto p-3",
				widget.appearance.variant === "grid"
					? "grid auto-rows-max content-start grid-cols-[repeat(auto-fit,minmax(min(100%,210px),1fr))] gap-3"
					: "space-y-2",
			)}
		>
			{results.data.packages.map((pkg) => (
				<Link
					key={pkg.id}
					href={`/store/packages?id=${encodeURIComponent(pkg.id)}`}
					className={homeItemClass}
				>
					<span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
						<Box className="size-5" />
					</span>
					<div className="min-w-0 flex-1">
						<div className="truncate text-sm font-semibold">
							{pkg.metadata?.name ?? pkg.name}
						</div>
						<p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
							{pkg.description}
						</p>
						<p className="mt-2 text-[11px] text-muted-foreground">
							v{pkg.latestVersion} · {pkg.downloadCount.toLocaleString()}{" "}
							downloads{pkg.verified ? " · Verified" : ""}
						</p>
					</div>
					<ArrowUpRight className="size-4 shrink-0 text-muted-foreground" />
				</Link>
			))}
		</div>
	);
}

export function HomeModels({ widget }: HomeContentProps) {
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
							"ImageGeneration",
							"Stt",
							"Tts",
						].includes(bit.type) &&
						(!query ||
							JSON.stringify(bit.meta)
								.toLowerCase()
								.includes(query.toLowerCase())),
				)
				.slice(0, limit);
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
				No models match this selection. Choose Explore models in widget settings
				to browse available models.
			</HomeEmpty>
		);
	return (
		<div className="grid h-full auto-rows-max content-start grid-cols-[repeat(auto-fit,minmax(min(100%,260px),1fr))] gap-3 overflow-auto p-3">
			{results.data.map((bit) => (
				<BitCard
					key={`${bit.hub}:${bit.id}`}
					bit={bit}
					wide={widget.appearance.variant === "list"}
				/>
			))}
		</div>
	);
}
