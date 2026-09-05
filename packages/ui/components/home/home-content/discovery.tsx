"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowRight,
	ArrowUpRight,
	Layers,
	RefreshCw,
	Sparkles,
	Star,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { useAssetImage } from "../../../hooks/use-asset-image";
import { useAppCategoryLabel } from "../../../lib/app-category";
import type { IApp, IAppCategory } from "../../../lib/schema/app/app";
import { IAppSearchSort } from "../../../lib/schema/app/app-search-query";
import type { IMetadata } from "../../../lib/schema/bit/bit";
import { IBitTypes } from "../../../lib/schema/hub/bit-search-query";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";
import { FlowPilotBubbleOrb } from "../../global-chat/flowpilot-bubble-orb";
import { AppTypeMark } from "../../ui/app-type-mark";
import { Button } from "../../ui/button";
import { ModelDetailSheet } from "../../ui/model-detail-sheet";
import { ModalityFlow, ProviderGlyph, providerLabel } from "../../ui/model-kit";
import { useHomeLibrary } from "./collections";
import {
	type HomeContentProps,
	numberConfig,
	stringList,
	textConfig,
} from "./config";
import { HomeEmpty, HomeQueryState, useHomeScope } from "./shared";

type AppPair = [IApp, IMetadata | undefined];
const compactNumber = new Intl.NumberFormat("en", {
	notation: "compact",
	maximumFractionDigits: 1,
});
const focusClass =
	"outline-none focus-visible:ring-2 focus-visible:ring-violet-400 focus-visible:ring-offset-2 focus-visible:ring-offset-background";
const eyebrowClass =
	"font-mono text-[10px] font-semibold uppercase tracking-[0.18em]";
const cardHeadingClass = "text-base font-semibold leading-snug tracking-tight";
const updatedDate = new Intl.DateTimeFormat("en", {
	month: "short",
	day: "numeric",
	year: "numeric",
});
const modelTypes = [
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.Embedding,
	IBitTypes.ImageEmbedding,
	IBitTypes.ObjectDetection,
	IBitTypes.VideoGeneration,
	IBitTypes.ImageGeneration,
	IBitTypes.Stt,
	IBitTypes.Tts,
];

function useDiscoveryApps(
	config: Record<string, unknown>,
	fallbackSource: string,
	limit: number,
) {
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const source = textConfig(config, "source", fallbackSource);
	const category = textConfig(config, "category");
	const tag = textConfig(config, "tag");
	const query = textConfig(config, "query");
	const appId = textConfig(config, "appId");
	const appIds = appId ? [appId] : [...new Set(stringList(config, "appIds"))];
	const profile = useQuery({
		queryKey: ["home", ...scope, "profile-apps"],
		queryFn: () => backend.userState.getProfile(),
	});
	const results = useQuery({
		queryKey: [
			"home",
			...scope,
			"discovery-apps",
			source,
			category,
			tag,
			query,
			appIds,
			limit,
		],
		enabled: source !== "library",
		queryFn: async (): Promise<AppPair[]> => {
			if (source === "manual") {
				const settled = await Promise.allSettled(
					appIds.slice(0, limit).map(async (id): Promise<AppPair> => {
						const [app, meta] = await Promise.all([
							backend.appState.getApp(id),
							backend.appState.getAppMeta(id).catch(() => undefined),
						]);
						return [app, meta];
					}),
				);
				const rows = settled.flatMap((result) =>
					result.status === "fulfilled" ? [result.value] : [],
				);
				if (settled.length && !rows.length)
					throw new Error("The selected apps are unavailable.");
				return rows;
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
	const visible = new Set((profile.data?.apps ?? []).map((app) => app.app_id));
	const matchesFilters = ([app, meta]: AppPair) =>
		(!category ||
			app.primary_category === category ||
			app.secondary_category === category) &&
		(!tag ||
			meta?.tags.some((value) => value.toLowerCase() === tag.toLowerCase())) &&
		(source === "manual" ||
			!query ||
			`${meta?.name ?? app.id} ${meta?.description ?? ""}`
				.toLowerCase()
				.includes(query.toLowerCase()));
	const localRows = (library.data ?? [])
		.filter(([app, meta]) => visible.has(app.id) && matchesFilters([app, meta]))
		.slice(0, limit);
	const isLibrary = source === "library";
	return {
		source,
		rows: isLibrary
			? localRows
			: source === "manual"
				? (results.data ?? []).filter(matchesFilters)
				: (results.data ?? []),
		localRows,
		owned: new Set((library.data ?? []).map(([app]) => app.id)),
		ownershipLoading: library.isLoading,
		isLoading: isLibrary
			? library.isLoading || profile.isLoading
			: results.isLoading,
		isError: isLibrary ? library.isError || profile.isError : results.isError,
		retry: () => {
			if (!isLibrary) void results.refetch();
			void library.refetch();
			void profile.refetch();
		},
	};
}

function appHref(app: IApp, owned: Set<string>) {
	return `${owned.has(app.id) ? "/use" : "/store"}?id=${encodeURIComponent(app.id)}`;
}

function exploreHref(config: Record<string, unknown>) {
	const source = textConfig(config, "source");
	if (source === "library") return "/library";
	const params = new URLSearchParams();
	const category = textConfig(config, "category");
	const query = textConfig(config, "query");
	if (category) params.set("category", category);
	if (query && source !== "manual") params.set("q", query);
	if (source === "new") params.set("sort", "newest");
	return `/store/explore/apps${params.size ? `?${params}` : ""}`;
}

function hasRating(app: IApp) {
	return (
		Number.isFinite(app.rating_count) &&
		app.rating_count > 0 &&
		typeof app.avg_rating === "number" &&
		Number.isFinite(app.avg_rating) &&
		app.avg_rating >= 0 &&
		app.avg_rating <= 5
	);
}

function AppArtwork({
	app,
	meta,
	compact = false,
}: { app: IApp; meta?: IMetadata; compact?: boolean }) {
	const image = useAssetImage(meta?.thumbnail || meta?.icon);
	return (
		<div
			className={cn(
				"relative isolate flex aspect-[2/1] w-full max-w-[470px] items-center justify-center overflow-hidden rounded-2xl border border-white/15 bg-white/[0.035] shadow-[0_20px_80px_-20px_rgba(0,0,0,0.7)] @[700px]/discovery:aspect-[4/3]",
				compact && "aspect-[4/3] rounded-xl",
			)}
		>
			<div
				aria-hidden="true"
				className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_right,rgba(167,139,250,0.3),transparent_70%)]"
			/>
			{image.canRender ? (
				<img
					ref={image.imgRef}
					src={image.src}
					alt={`${meta?.name ?? "App"} artwork`}
					onLoad={image.onLoad}
					onError={image.onError}
					className="relative size-full object-cover"
				/>
			) : (
				<AppTypeMark
					type={app.app_type}
					size={112}
					src={meta?.icon}
					fallback={(meta?.name ?? app.id).slice(0, 2).toUpperCase()}
					background="linear-gradient(145deg,#8b5cf6,#312e81)"
				/>
			)}
		</div>
	);
}

function DiscoveryLink({
	href,
	children,
	className,
}: { href: string; children: React.ReactNode; className?: string }) {
	return (
		<Link
			href={href}
			className={cn(
				"inline-flex w-fit items-center gap-2 rounded-md text-xs font-semibold",
				focusClass,
				className,
			)}
		>
			{children}
			<ArrowRight aria-hidden="true" className="size-3.5 shrink-0" />
		</Link>
	);
}

export function HomeAppSpotlight({ widget, editing }: HomeContentProps) {
	const apps = useDiscoveryApps(widget.config, "new", 1);
	const compact = textConfig(widget.config, "mode") === "compact";
	const categoryLabel = useAppCategoryLabel();
	const openOverlay = useGlobalChatStore((state) => state.openOverlay);
	const fallback =
		!apps.rows.length && apps.source !== "manual" && !apps.isLoading;
	const pair = apps.rows[0] ?? (fallback ? apps.localRows[0] : undefined);
	const [app, meta] = pair ?? [];
	const timestamp = app?.updated_at.secs_since_epoch;
	const date =
		typeof timestamp === "number" && Number.isFinite(timestamp) && timestamp > 0
			? new Date(timestamp * 1000)
			: undefined;
	const updateLabel =
		date && Number.isFinite(date.getTime())
			? updatedDate.format(date)
			: undefined;
	const heading = app
		? (meta?.name ?? "Discover this app")
		: "Build your next big idea.";
	const description = app
		? meta?.description || "Open this app and see what you can build with it."
		: "A helpful assistant. Apps you can make your own. Everything you need to turn an idea into something that works.";
	const eyebrow = app
		? fallback || apps.source === "library"
			? "FROM YOUR LIBRARY"
			: textConfig(widget.config, "eyebrow", "APP SPOTLIGHT")
		: "YOUR IDEAS, IN MOTION";
	return (
		<section
			data-home-discovery="app-spotlight"
			data-home-discovery-mode={compact ? "compact" : "hero"}
			className="@container/discovery relative isolate overflow-hidden rounded-[20px] border border-violet-400/25 bg-[#10101e] text-[#f6f3ff]"
		>
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_90%_10%,rgba(139,92,246,0.25),transparent_55%),linear-gradient(120deg,rgba(91,33,182,0.14),transparent_70%)]"
			/>
			<div
				className={cn(
					"relative grid items-center",
					compact
						? "min-h-[300px] gap-5 p-5 @[540px]/discovery:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)] @[540px]/discovery:gap-6 @[540px]/discovery:p-6"
						: "min-h-[360px] gap-6 p-[clamp(22px,3.5cqw,44px)] @[700px]/discovery:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)] @[700px]/discovery:gap-12",
				)}
			>
				<div className="min-w-0">
					<p
						className={cn(
							eyebrowClass,
							compact
								? "mb-3 pr-16 text-[#ff8c60] @[540px]/discovery:pr-0"
								: "mb-4 text-[#ff8c60] @[700px]/discovery:mb-5",
						)}
					>
						{eyebrow}
					</p>
					<h2
						className={cn(
							"text-balance break-words font-bold leading-[1.08] tracking-[-0.035em]",
							compact
								? "pr-16 text-[clamp(1.6rem,4.8cqw,2.625rem)] @[540px]/discovery:pr-0"
								: "max-w-[15ch] text-[clamp(2.15rem,5.3cqw,4rem)]",
						)}
					>
						{heading}
					</h2>
					{app && (app.primary_category || updateLabel) && (
						<p
							className={cn(
								"flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] leading-relaxed text-[#b8b1c9]",
								compact ? "mt-2" : "mt-3",
							)}
						>
							{app.primary_category && (
								<span>{categoryLabel(app.primary_category)}</span>
							)}
							{app.primary_category && updateLabel && (
								<span aria-hidden="true">·</span>
							)}
							{updateLabel && (
								<time dateTime={date?.toISOString()}>
									Updated {updateLabel}
								</time>
							)}
						</p>
					)}
					<p
						className={cn(
							"max-w-[48ch] text-pretty leading-relaxed text-[#b8b1c9]",
							compact
								? "mt-3 line-clamp-2 text-[13px]"
								: "mt-4 line-clamp-4 text-[clamp(0.875rem,1.6cqw,1.05rem)] @[700px]/discovery:mt-5",
						)}
					>
						{description}
					</p>
					<div
						className={cn(
							"flex flex-wrap items-center",
							compact ? "mt-4 gap-2" : "mt-5 gap-3 @[700px]/discovery:mt-7",
						)}
					>
						{app ? (
							<Button
								asChild
								size="lg"
								className={cn(
									"rounded-lg bg-[#ff5c25] font-semibold text-[#1b110c] hover:bg-[#ff7547]",
									compact ? "h-9 px-3 text-xs" : "h-11 px-5",
								)}
							>
								<Link
									href={appHref(app, apps.owned)}
									aria-disabled={apps.ownershipLoading || undefined}
									onClick={(event) => {
										if (apps.ownershipLoading) event.preventDefault();
									}}
								>
									{apps.ownershipLoading
										? "Loading app…"
										: apps.owned.has(app.id)
											? "Open app"
											: "Explore app"}
									<ArrowUpRight className="size-4" />
								</Link>
							</Button>
						) : (
							<Button
								size="lg"
								disabled={editing}
								onClick={openOverlay}
								className={cn(
									"rounded-lg bg-[#ff5c25] font-semibold text-[#1b110c] hover:bg-[#ff7547]",
									compact ? "h-9 px-3 text-xs" : "h-11 px-5",
								)}
							>
								Build with FlowPilot
								<ArrowRight className="size-4" />
							</Button>
						)}
						<Link
							href={app ? exploreHref(widget.config) : "/library"}
							className={cn(
								"inline-flex items-center rounded-lg border border-white/20 font-semibold text-[#e2ddeb] transition-colors hover:bg-white/5",
								compact ? "h-9 px-3 text-xs" : "h-11 px-5 text-sm",
								focusClass,
							)}
						>
							{app && apps.source !== "library"
								? compact
									? "Browse apps"
									: "Explore all apps"
								: compact
									? "Library"
									: "Open your library"}
						</Link>
					</div>
					{app && (hasRating(app) || app.download_count > 0) && (
						<dl
							className={cn(
								"flex flex-wrap gap-y-2",
								compact
									? "mt-4 gap-x-4"
									: "mt-5 gap-x-7 @[700px]/discovery:mt-7",
							)}
						>
							{hasRating(app) && (
								<div
									className={cn(
										"flex",
										compact ? "items-center gap-1.5" : "flex-col",
									)}
								>
									<dt
										className={cn(
											compact ? "text-[10px]" : eyebrowClass,
											"text-[#9790a8]",
											!compact && "mt-1",
										)}
									>
										Rating
									</dt>
									<dd
										className={cn(
											"-order-1 flex items-center gap-1.5 font-semibold",
											compact ? "text-sm" : "text-xl",
										)}
									>
										<Star
											aria-hidden="true"
											className="size-4 fill-[#ffca65] text-[#ffca65]"
										/>
										{app.avg_rating?.toFixed(1)}
										<span className="text-xs font-normal text-[#9790a8]">
											({compactNumber.format(app.rating_count)})
										</span>
									</dd>
								</div>
							)}
							{Number.isFinite(app.download_count) &&
								app.download_count > 0 && (
									<div
										className={cn(
											"flex",
											compact ? "items-center gap-1.5" : "flex-col",
										)}
									>
										<dt
											className={cn(
												compact ? "text-[10px]" : eyebrowClass,
												"text-[#9790a8]",
												!compact && "mt-1",
											)}
										>
											Downloads
										</dt>
										<dd
											className={cn(
												"-order-1 font-semibold",
												compact ? "text-sm" : "text-xl",
											)}
										>
											{compactNumber.format(app.download_count)}
										</dd>
									</div>
								)}
						</dl>
					)}
					{!app && apps.isError && (
						<button
							type="button"
							onClick={apps.retry}
							className={cn(
								"mt-5 inline-flex items-center gap-1.5 rounded text-xs text-[#b8b1c9] hover:text-white",
								focusClass,
							)}
						>
							<RefreshCw className="size-3" />
							Reconnect to discover apps
						</button>
					)}
					{!app &&
						apps.source === "manual" &&
						!apps.isLoading &&
						!apps.isError && (
							<p className="mt-5 text-xs text-[#b8b1c9]">
								Choose a featured app in widget settings.
							</p>
						)}
				</div>
				<div
					className={cn(
						"flex min-w-0 items-center justify-center",
						compact
							? "absolute right-5 top-6 size-14 @[540px]/discovery:relative @[540px]/discovery:right-auto @[540px]/discovery:top-auto @[540px]/discovery:size-auto"
							: "relative @[700px]/discovery:pl-2",
					)}
				>
					{app ? (
						compact ? (
							<>
								<span className="@[540px]/discovery:hidden">
									<AppTypeMark
										size={56}
										type={app.app_type}
										src={meta?.icon}
										fallback={(meta?.name ?? app.id).slice(0, 2).toUpperCase()}
										background="linear-gradient(145deg,#8b5cf6,#312e81)"
									/>
								</span>
								<div className="hidden w-full @[540px]/discovery:block">
									<AppArtwork app={app} meta={meta} compact />
								</div>
							</>
						) : (
							<AppArtwork app={app} meta={meta} />
						)
					) : (
						<div
							className={cn(
								"relative flex w-full max-w-[370px] items-center justify-center",
								compact
									? "aspect-square @[540px]/discovery:aspect-[4/3]"
									: "aspect-[2/1] @[700px]/discovery:aspect-[4/3]",
							)}
						>
							<div
								aria-hidden="true"
								className="absolute inset-[8%] rounded-full border border-violet-300/10 bg-violet-400/5 shadow-[0_0_100px_15px_rgba(139,92,246,0.15)]"
							/>
							<div
								aria-hidden="true"
								className="absolute inset-[18%] rounded-full border border-violet-300/15"
							/>
							<FlowPilotBubbleOrb
								className={
									compact
										? "size-14 @[540px]/discovery:size-[clamp(100px,18cqw,150px)]"
										: "size-[clamp(120px,24cqw,220px)]"
								}
								onClick={openOverlay}
								disabled={editing}
							/>
						</div>
					)}
				</div>
			</div>
		</section>
	);
}

export function HomeAppRanking({ widget }: HomeContentProps) {
	const apps = useDiscoveryApps(
		widget.config,
		"popular",
		Math.min(10, numberConfig(widget.config, "limit", 6)),
	);
	const categoryLabel = useAppCategoryLabel();
	const subtitle =
		apps.source === "popular"
			? "By community ratings"
			: apps.source === "new"
				? "Recently added"
				: apps.source === "manual"
					? "Selected apps"
					: "In your profile";
	return (
		<section
			data-home-discovery="app-ranking"
			className="flex h-full min-w-0 flex-col p-5"
		>
			<header className="mb-4">
				{textConfig(widget.config, "eyebrow") && (
					<p className={cn(eyebrowClass, "mb-2 text-muted-foreground")}>
						{textConfig(widget.config, "eyebrow")}
					</p>
				)}
				<h2 className={cardHeadingClass}>
					{(widget.title !== "Community favorites" ? widget.title : "") ||
						(apps.source === "popular"
							? "Community favorites"
							: "Apps to explore")}
				</h2>
				<p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
			</header>
			{apps.isLoading || apps.isError ? (
				<HomeQueryState
					loading={apps.isLoading}
					error={apps.isError}
					retry={apps.retry}
				/>
			) : !apps.rows.length ? (
				<HomeEmpty icon={<Layers />}>
					{apps.source === "manual"
						? "Choose the apps you want to feature in widget settings."
						: "Your next useful app is waiting to be discovered."}
				</HomeEmpty>
			) : (
				<ol className="divide-y divide-border/50">
					{apps.rows.map(([app, meta], index) => (
						<li key={app.id}>
							<Link
								href={appHref(app, apps.owned)}
								aria-disabled={apps.ownershipLoading || undefined}
								onClick={(event) => {
									if (apps.ownershipLoading) event.preventDefault();
								}}
								className={cn(
									"group flex min-w-0 items-center gap-2.5 rounded-lg py-2.5 transition-colors hover:bg-muted/35",
									focusClass,
								)}
							>
								<span
									aria-hidden="true"
									className="w-4 shrink-0 font-mono text-base font-semibold tabular-nums text-muted-foreground/45"
								>
									{index + 1}
								</span>
								<AppTypeMark
									size={34}
									type={app.app_type}
									src={meta?.icon}
									fallback={(meta?.name ?? app.id).slice(0, 2).toUpperCase()}
								/>
								<span className="min-w-0 flex-1">
									<span className="block truncate text-[13px] font-semibold">
										{meta?.name ?? app.id}
									</span>
									{meta?.description && (
										<span className="mt-0.5 block truncate text-xs leading-relaxed text-muted-foreground">
											{meta.description}
										</span>
									)}
									<span className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
										{app.primary_category && (
											<span>{categoryLabel(app.primary_category)}</span>
										)}
										{hasRating(app) && (
											<span
												aria-label={`${app.avg_rating?.toFixed(1)} out of 5 from ${app.rating_count} ratings`}
												className="inline-flex items-center gap-1 font-medium text-amber-600 dark:text-amber-400"
											>
												<Star
													aria-hidden="true"
													className="size-2.5 fill-current"
												/>
												{app.avg_rating?.toFixed(1)}
												<span className="font-normal text-muted-foreground">
													({compactNumber.format(app.rating_count)})
												</span>
											</span>
										)}
									</span>
								</span>
							</Link>
						</li>
					))}
				</ol>
			)}
			<DiscoveryLink
				href={exploreHref(widget.config)}
				className="mt-auto pt-5 text-muted-foreground hover:text-foreground"
			>
				{apps.source === "library" ? "Open your library" : "Explore apps"}
			</DiscoveryLink>
		</section>
	);
}

export function HomeAppCollectionFeature({ widget }: HomeContentProps) {
	const apps = useDiscoveryApps(
		widget.config,
		"new",
		Math.min(6, numberConfig(widget.config, "limit", 3)),
	);
	const selectionMeaning =
		apps.source === "new"
			? "Newest"
			: apps.source === "popular"
				? "Community ratings"
				: apps.source === "manual"
					? "Selected apps"
					: "This profile";
	return (
		<section
			data-home-discovery="app-collection-feature"
			className="relative flex h-full min-w-0 flex-col overflow-hidden rounded-2xl bg-[#ff5c25] p-5 text-[#251107]"
		>
			<p className={cn(eyebrowClass, "mb-2 text-[#6b260a]")}>
				{textConfig(widget.config, "eyebrow", "COLLECTION")}
			</p>
			<h2 className="max-w-[24ch] text-balance text-[22px] font-semibold leading-tight tracking-tight">
				{textConfig(widget.config, "headline") ||
					widget.title ||
					"Automate the everyday"}
			</h2>
			{textConfig(widget.config, "description") && (
				<p className="mt-2 text-pretty text-xs leading-relaxed text-[#642509]">
					{textConfig(widget.config, "description")}
				</p>
			)}
			<div className="my-4 space-y-2">
				{apps.rows.map(([app, meta]) => (
					<Link
						key={app.id}
						href={appHref(app, apps.owned)}
						aria-disabled={apps.ownershipLoading || undefined}
						onClick={(event) => {
							if (apps.ownershipLoading) event.preventDefault();
						}}
						className={cn(
							"group flex min-w-0 items-center gap-3 rounded-xl bg-black/10 p-2.5 transition-colors hover:bg-black/15",
							focusClass,
						)}
					>
						<AppTypeMark
							size={30}
							type={app.app_type}
							src={meta?.icon}
							fallback={(meta?.name ?? app.id).slice(0, 2).toUpperCase()}
							background="#b8441b"
						/>
						<span className="min-w-0 flex-1">
							<span className="block truncate text-[13px] font-semibold">
								{meta?.name ?? app.id}
							</span>
							{meta?.description && (
								<span className="mt-0.5 block truncate text-[11px] leading-relaxed text-[#642509]">
									{meta.description}
								</span>
							)}
						</span>
						<ArrowUpRight
							aria-hidden="true"
							className="size-3.5 shrink-0 opacity-50 group-hover:opacity-100"
						/>
					</Link>
				))}
				{!apps.rows.length && (
					<div className="rounded-xl bg-black/[0.065] p-4">
						<Layers className="mb-3 size-6 opacity-60" />
						<p className="text-sm font-medium leading-relaxed">
							{apps.isLoading
								? "Finding your next useful app…"
								: apps.source === "manual"
									? "Bring your favorite apps together."
									: "Less busywork. More room for your ideas."}
						</p>
						<p className="mt-2 text-xs leading-relaxed text-[#642509]">
							{apps.isError
								? "Explore your library while the store reconnects."
								: apps.source === "manual"
									? "Choose apps in widget settings to curate this collection."
									: "Explore apps for the things you do every day."}
						</p>
					</div>
				)}
			</div>
			{apps.rows.length > 0 && (
				<p className="mb-3 text-[10px] text-[#642509]">
					{apps.rows.length}{" "}
					{apps.rows.length === 1 ? "app shown" : "apps shown"} ·{" "}
					{selectionMeaning}
				</p>
			)}
			<div className="mt-auto flex flex-wrap items-center gap-4">
				<DiscoveryLink
					href={apps.isError ? "/library" : exploreHref(widget.config)}
					className="text-[#251107] hover:underline"
				>
					{apps.isError || apps.source === "library"
						? "Open your library"
						: apps.source === "manual"
							? "Explore more apps"
							: "Explore the collection"}
				</DiscoveryLink>
				{apps.isError && (
					<button
						type="button"
						onClick={apps.retry}
						className={cn("rounded text-xs font-medium underline", focusClass)}
					>
						Retry
					</button>
				)}
			</div>
		</section>
	);
}

export function HomeModelSpotlight({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const queryClient = useQueryClient();
	const [detailOpen, setDetailOpen] = useState(false);
	const source = textConfig(widget.config, "source", "explore");
	const modelId = textConfig(widget.config, "modelId");
	const modelHub = textConfig(widget.config, "modelHub");
	const query = textConfig(widget.config, "query");
	const result = useQuery({
		queryKey: [
			"home",
			...scope,
			"models",
			"spotlight",
			source,
			modelId,
			modelHub,
			query,
		],
		queryFn: async () => {
			const bits = modelId
				? [await backend.bitState.getBit(modelId, modelHub || undefined)]
				: source === "profile"
					? await backend.bitState.getProfileBits()
					: await backend.bitState.searchBits({
							search: query,
							bit_types: modelTypes,
							limit: 1,
						});
			return (
				bits.find(
					(bit) =>
						modelTypes.includes(bit.type as IBitTypes) &&
						(modelId ||
							!query ||
							JSON.stringify(bit.meta)
								.toLowerCase()
								.includes(query.toLowerCase())),
				) ?? null
			);
		},
		staleTime: 60_000,
	});
	const rawBit = result.data;
	const meta =
		rawBit?.meta.en ?? (rawBit ? Object.values(rawBit.meta)[0] : undefined);
	const bit =
		rawBit && !rawBit.meta.en && meta
			? { ...rawBit, meta: { ...rawBit.meta, en: meta } }
			: rawBit;
	const contextLength = bit?.parameters?.context_length;
	const provider = bit?.parameters?.provider?.provider_name;
	const sourceLabel = modelId
		? "Selected model"
		: source === "profile"
			? "In this profile"
			: "Available in the catalog";
	return (
		<section
			data-home-discovery="model-spotlight"
			className="flex h-full min-w-0 flex-col p-5"
		>
			<p className={cn(eyebrowClass, "mb-4 text-muted-foreground")}>
				{textConfig(widget.config, "eyebrow", "MODEL SPOTLIGHT")}
			</p>
			{bit ? (
				<>
					<div className="flex min-w-0 items-center gap-3">
						<ProviderGlyph bit={bit} size={44} />
						<div className="min-w-0">
							<h2 className={cn(cardHeadingClass, "break-words")}>
								{meta?.name ?? bit.id}
							</h2>
							<p className="mt-1 text-[11px] text-muted-foreground">
								{sourceLabel}
							</p>
						</div>
					</div>
					<p className="mt-4 line-clamp-3 text-pretty text-xs leading-relaxed text-muted-foreground">
						{meta?.description ||
							"Add this model to your profile to use it in your apps and flows."}
					</p>
					<dl className="mt-4 divide-y divide-border/50 border-y border-border/50 text-xs">
						{typeof provider === "string" && provider && (
							<div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 py-2.5">
								<dt className="text-muted-foreground">Provider</dt>
								<dd className="min-w-0 break-words font-medium">
									{providerLabel(bit)}
								</dd>
							</div>
						)}
						<div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2 py-2.5">
							<dt className="text-muted-foreground">Input → output</dt>
							<dd>
								<ModalityFlow type={bit.type} />
							</dd>
						</div>
						{typeof contextLength === "number" &&
							Number.isFinite(contextLength) &&
							contextLength > 0 && (
								<div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 py-2.5">
									<dt className="text-muted-foreground">Context window</dt>
									<dd
										title={`${contextLength.toLocaleString()} tokens`}
										className="font-medium tabular-nums"
									>
										{compactNumber.format(contextLength)} tokens
									</dd>
								</div>
							)}
						{typeof bit.license === "string" && bit.license && (
							<div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 py-2.5">
								<dt className="text-muted-foreground">License</dt>
								<dd
									className="min-w-0 truncate font-medium"
									title={bit.license}
								>
									{bit.license}
								</dd>
							</div>
						)}
					</dl>
					<Button
						variant="outline"
						size="sm"
						disabled={editing}
						onClick={() => setDetailOpen(true)}
						className="mt-4 w-fit rounded-lg"
					>
						View model
						<ArrowUpRight className="size-3" />
					</Button>
				</>
			) : (
				<>
					<h2 className={cardHeadingClass}>Find the right model</h2>
					<p className="mt-3 text-xs leading-relaxed text-muted-foreground">
						{result.isLoading
							? "Finding a model for your next idea…"
							: source === "profile"
								? "Choose models for this profile to power your chats, images, and workflows."
								: "Explore models for conversations, images, and more. Choose what works for your app."}
					</p>
					<div
						aria-hidden="true"
						className="my-5 flex size-12 items-center justify-center rounded-xl border border-violet-400/20 bg-violet-400/10 text-violet-400"
					>
						<Sparkles className="size-6" />
					</div>
					<DiscoveryLink
						href="/settings/ai"
						className="mt-auto text-violet-500 dark:text-violet-300"
					>
						Explore models
					</DiscoveryLink>
					{result.isError && (
						<button
							type="button"
							onClick={() => void result.refetch()}
							className={cn(
								"mt-3 w-fit rounded text-xs text-muted-foreground underline",
								focusClass,
							)}
						>
							Reconnect to load models
						</button>
					)}
				</>
			)}
			{!editing && detailOpen && bit && (
				<ModelDetailSheet
					bit={bit}
					queryScope={scope}
					open={detailOpen}
					onOpenChange={setDetailOpen}
					onProfileChange={() =>
						queryClient.invalidateQueries({
							queryKey: ["home", ...scope, "models"],
						})
					}
					webMode={!backend.capabilities().canExecuteLocally}
				/>
			)}
		</section>
	);
}
