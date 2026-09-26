"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpRight, Code, Shield, Star } from "lucide-react";
import { useAssetImage } from "../../../../hooks/use-asset-image";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { usePackageCapabilities } from "../../../../lib/package-capabilities";
import { IAppVisibility } from "../../../../lib/schema/app/app";
import { cn } from "../../../../lib/utils";
import { getPackageInitials, usePackageGradient } from "../../package-card";
import {
	AppIdentityMark,
	type ExploreAppItem,
	ExploreItemLink,
	type ExplorePackageItem,
	PackageGlyph,
	PackageStatCells,
	itemCover,
	itemDescription,
	itemName,
	useExploreLinks,
} from "../explore-item-card";
import { formatPrice, useExploreLabels } from "../explore-labels";
import { type BentoTileSize, accentColor } from "../explore-model";
import type { ExploreFeature } from "../explore-types";
import { CollectionTile } from "./collection-tile";

interface FeatureCopy {
	eyebrow?: string | null;
	headline: string;
	subline: string;
	artwork?: string;
	accent: string;
}

function featureCopy(feature: ExploreFeature): FeatureCopy {
	const { slide } = feature;
	return {
		eyebrow: feature.eyebrow?.trim() || null,
		headline: slide.headline?.trim() || itemName(slide.item),
		subline: slide.subline?.trim() || itemDescription(slide.item),
		artwork: slide.artworkUrl ?? itemCover(slide.item),
		accent: accentColor(slide.accent, slide.item),
	};
}

/** `size` is the xl shape (one or two grid rows); md and phone widths always use the short layout. */
export function FeatureTile({
	feature,
	size,
}: Readonly<{ feature: ExploreFeature; size: BentoTileSize }>) {
	const item = feature.slide.item;
	const copy = featureCopy(feature);
	if (item.kind === "app") {
		return <AppFeature item={item} copy={copy} />;
	}
	if (item.kind === "package") {
		return <PackageFeature item={item} copy={copy} size={size} />;
	}
	return (
		<CollectionTile
			collection={{
				placementId: item.collection.id,
				title: copy.headline,
				blurb: copy.subline,
				source: "hand",
				items: item.collection.preview,
				apps: item.collection.apps,
				packages: item.collection.packages,
			}}
		/>
	);
}

function AppFeature({
	item,
	copy,
}: Readonly<{ item: ExploreAppItem; copy: FeatureCopy }>) {
	const { t } = useTranslation("store");
	const categoryLabel = useAppCategoryLabel();
	const links = useExploreLinks();
	const cover = useAssetImage(copy.artwork);
	const gradient = usePackageGradient(item.app.id);
	const price = item.app.price ?? 0;
	const owned = links.isOwned(item.app.id);
	const rated = item.app.rating_count > 0;
	return (
		<ExploreItemLink
			item={item}
			tile="feature"
			label={t("exploreFeaturedAppNamed", {
				defaultValue: "Featured app: {{name}}",
				name: copy.headline,
			})}
			className="group relative isolate flex h-full min-h-60 flex-col justify-end overflow-hidden rounded-2xl border border-border/50 bg-neutral-950 p-5 text-white transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring @3xl/explore:min-h-0"
		>
			<div aria-hidden="true" className="absolute inset-0 -z-10">
				<div className="absolute inset-0" style={{ background: gradient }} />
				{cover.canRender && (
					<img
						ref={cover.imgRef}
						src={cover.src}
						onLoad={cover.onLoad}
						onError={cover.onError}
						alt=""
						className="absolute inset-0 h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.03]"
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-t from-black/90 via-black/45 to-black/10" />
				<div className="absolute inset-0 bg-linear-to-r from-black/50 to-transparent" />
			</div>
			<span className="absolute left-4.5 top-4 inline-flex h-6 items-center rounded-full border border-white/15 bg-black/50 px-2.5 text-[11.5px] font-semibold backdrop-blur-md">
				{copy.eyebrow ?? t("exploreFeaturedApp", "Featured app")}
			</span>
			<div className="flex min-w-0 flex-col gap-2">
				<div className="flex min-w-0 items-center gap-3">
					<AppIdentityMark item={item} />
					<div className="min-w-0 flex-1">
						<div
							className="truncate text-[11px] font-semibold uppercase leading-3.5 tracking-wider"
							style={{ color: copy.accent }}
						>
							{categoryLabel(item.app.primary_category)}
						</div>
						<h3 className="truncate text-lg font-bold leading-snug text-white">
							{copy.headline}
						</h3>
					</div>
					{item.app.visibility === IAppVisibility.Public && (
						<span
							className={cn(
								"inline-flex h-6.5 shrink-0 items-center rounded-full px-3 text-xs font-semibold",
								price > 0 && !owned
									? "bg-white/90 text-neutral-900"
									: "border border-white/25 bg-white/15 text-white",
								!owned && price <= 0 && "uppercase",
							)}
						>
							{owned
								? t("exploreOpen", "Open")
								: price > 0
									? formatPrice(price)
									: t("exploreGet", "Get")}
						</span>
					)}
				</div>
				{copy.subline && (
					<p className="line-clamp-2 text-sm leading-relaxed text-white/75">
						{copy.subline}
					</p>
				)}
				<span className="flex items-center gap-1.5 text-sm text-white/90">
					{rated ? (
						<>
							<Star
								aria-hidden="true"
								className="size-3.75 fill-yellow-400 text-yellow-400"
							/>
							<span className="font-semibold">
								{(item.app.avg_rating ?? 0).toFixed(1)}
							</span>
							<span className="text-xs text-white/50">
								{`(${item.app.rating_count.toLocaleString()})`}
							</span>
						</>
					) : (
						<span className="text-xs text-white/50">
							{t("exploreNoRatingsYet", "No ratings yet")}
						</span>
					)}
				</span>
			</div>
		</ExploreItemLink>
	);
}

function PackageFeature({
	item,
	copy,
	size,
}: Readonly<{
	item: ExplorePackageItem;
	copy: FeatureCopy;
	size: BentoTileSize;
}>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const pkg = item.package;
	const tall = size === "tall";
	const category = pkg.primaryCategory ?? pkg.secondaryCategory;
	const capabilities = usePackageCapabilities(pkg.capabilities);
	const firstCapability = capabilities[0];
	const version = `v${pkg.latestVersion}`;
	return (
		<ExploreItemLink
			item={item}
			tile="feature"
			label={t("exploreFeaturedPackageNamed", {
				defaultValue: "Featured package: {{name}}",
				name: copy.headline,
			})}
			className="group relative flex h-full min-w-0 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card px-4.5 py-4 transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--border)_0.5px,transparent_0.5px)] bg-size-[7px_7px] opacity-50"
			/>
			{tall && (
				<FeatureThumbnail
					item={item}
					artwork={copy.artwork}
					version={version}
				/>
			)}
			<div className="relative flex items-center justify-between gap-3">
				<span className="inline-flex min-w-0 items-center gap-1.75 truncate font-mono text-[10.5px] uppercase leading-3.5 tracking-widest text-muted-foreground">
					<Code
						aria-hidden="true"
						className="size-3.25 shrink-0"
						style={{ color: copy.accent }}
					/>
					{copy.eyebrow ?? (
						<>
							{t("exploreRailForBuilders", "For builders")}
							<span aria-hidden="true">·</span>
							{t("exploreFeaturedPackage", "Featured package")}
						</>
					)}
				</span>
				<span
					className={cn(
						"shrink-0 rounded-md border border-border bg-muted/60 px-1.5 py-0.75 font-mono text-[10px] leading-none text-muted-foreground",
						tall && "@5xl/explore:hidden",
					)}
				>
					{version}
				</span>
			</div>
			<div className="relative mt-3.5 flex min-w-0 items-center gap-3">
				<PackageGlyph
					pkg={pkg}
					className="size-11 rounded-[11px] border border-border/60"
					textClassName="text-[13px]"
				/>
				<div className="min-w-0 flex-1">
					<div className="flex min-w-0 items-center gap-1.5">
						<h3 className="truncate text-[17px] font-bold leading-5.5 tracking-tight">
							{copy.headline}
						</h3>
						{pkg.verified && (
							<Shield
								aria-label={t("verified", "Verified")}
								className="size-3.75 shrink-0 text-sky-500 dark:text-sky-400"
							/>
						)}
					</div>
					{category && (
						<div className="truncate font-mono text-[10px] uppercase leading-3.5 tracking-wider text-muted-foreground">
							{labels.packageCategory(category)}
						</div>
					)}
				</div>
			</div>
			{copy.subline && (
				<p className="relative mt-2.5 line-clamp-2 text-[13px] leading-5 text-muted-foreground">
					{copy.subline}
				</p>
			)}
			<div className="relative mt-auto flex flex-wrap items-end justify-between gap-x-4 gap-y-2 pt-2.5">
				{pkg.capabilities &&
					(firstCapability ? (
						<span
							title={capabilities.map((entry) => entry.label).join("\n")}
							className={cn(
								"rounded border px-1.5 py-1 font-mono text-[10px] leading-none",
								firstCapability.severity === "elevated"
									? "border-primary/35 bg-primary/10 text-primary"
									: "border-border/60 bg-muted/40 text-muted-foreground",
							)}
						>
							{capabilities.length > 1
								? `${firstCapability.key} +${capabilities.length - 1}`
								: firstCapability.key}
						</span>
					) : (
						<span className="rounded border border-dashed border-border/60 px-1.5 py-1 font-mono text-[10px] leading-none text-muted-foreground">
							{t("noPermissionsRequested", "no permissions requested")}
						</span>
					))}
				<div className="ml-auto flex divide-x divide-border/60">
					<PackageStatCells pkg={pkg} />
				</div>
			</div>
		</ExploreItemLink>
	);
}

function FeatureThumbnail({
	item,
	artwork,
	version,
}: Readonly<{ item: ExplorePackageItem; artwork?: string; version: string }>) {
	const image = useAssetImage(artwork);
	const gradient = usePackageGradient(item.package.id);
	return (
		<div className="relative mb-4 hidden min-h-0 flex-1 overflow-hidden rounded-[10px] border border-border/60 @5xl/explore:block">
			<div
				aria-hidden="true"
				className="absolute inset-0"
				style={{ background: gradient }}
			/>
			{image.canRender ? (
				<img
					ref={image.imgRef}
					src={image.src}
					onLoad={image.onLoad}
					onError={image.onError}
					alt=""
					className="absolute inset-0 h-full w-full object-cover"
				/>
			) : (
				<span
					aria-hidden="true"
					className="absolute inset-0 flex items-center justify-center font-mono text-6xl font-bold tracking-tighter text-white/65"
				>
					{getPackageInitials(itemName(item))}
				</span>
			)}
			<span className="absolute bottom-2 left-2 rounded-md border border-white/20 bg-black/50 px-1.5 py-0.75 font-mono text-[10px] leading-none text-white/90">
				{version}
			</span>
			<span
				aria-hidden="true"
				className="absolute right-2.5 top-2.5 flex size-8 items-center justify-center rounded-full border border-white/25 bg-black/15 text-white transition-transform group-hover:-rotate-12"
			>
				<ArrowUpRight className="size-4" />
			</span>
		</div>
	);
}
