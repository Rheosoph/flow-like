"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight, Layers } from "lucide-react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import {
	type MouseEvent,
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
} from "react";
import { useAssetImage } from "../../../hooks/use-asset-image";
import type { PackageSummary } from "../../../lib/schema/wasm";
import { cn } from "../../../lib/utils";
import type { IEventMapping } from "../../interfaces/interfaces";
import { AppCard } from "../../ui/app-card";
import { AppTypeMark } from "../../ui/app-type-mark";
import {
	PackageCard,
	PackageCardStat,
	type PackageCardVariant,
	PackageRatingStar,
	formatCompact,
	getPackageInitials,
	usePackageGradient,
} from "../package-card";
import { packageStoreHref } from "../package-navigation";
import { exploreSearchHref } from "./explore-href";
import { formatPrice, useExploreLabels } from "./explore-labels";
import type {
	ExploreCollectionSummary,
	ExploreResolvedItem,
} from "./explore-types";
import { useExploreAppHref } from "./use-explore-app-href";

export type ExploreAppItem = Extract<ExploreResolvedItem, { kind: "app" }>;
export type ExplorePackageItem = Extract<
	ExploreResolvedItem,
	{ kind: "package" }
>;

interface ExploreLinks {
	/** Current path + query, handed to the package store so its Back returns here. */
	from: string;
	appHref: (appId: string) => string;
	openApp: (appId: string) => void;
	isOwned: (appId: string) => boolean;
	packageHref: (packageId: string) => string;
}

const ExploreLinksContext = createContext<ExploreLinks | null>(null);

export function ExploreLinksProvider({
	eventConfig,
	children,
}: Readonly<{ eventConfig?: IEventMapping; children: ReactNode }>) {
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const { appHref, openApp, isOwned } = useExploreAppHref(eventConfig);
	const search = searchParams.toString();
	const from = search ? `${pathname}?${search}` : pathname;
	const value = useMemo<ExploreLinks>(
		() => ({
			from,
			appHref,
			openApp: (appId) => {
				void openApp(appId);
			},
			isOwned,
			packageHref: (id) => packageStoreHref({ id, from }),
		}),
		[from, appHref, openApp, isOwned],
	);
	return (
		<ExploreLinksContext.Provider value={value}>
			{children}
		</ExploreLinksContext.Provider>
	);
}

export function useExploreLinks(): ExploreLinks {
	const links = useContext(ExploreLinksContext);
	if (!links) {
		throw new Error(
			"useExploreLinks needs an ExploreLinksProvider above the Explore card",
		);
	}
	return links;
}

export function itemId(item: ExploreResolvedItem): string {
	if (item.kind === "app") return item.app.id;
	if (item.kind === "package") return item.package.id;
	return item.collection.id;
}

export function itemName(item: ExploreResolvedItem): string {
	if (item.kind === "app") return item.metadata?.name ?? item.app.id;
	if (item.kind === "package") {
		return item.package.metadata?.name ?? item.package.name;
	}
	return item.collection.title;
}

export function itemDescription(item: ExploreResolvedItem): string {
	if (item.kind === "app") return item.metadata?.description ?? "";
	if (item.kind === "package") {
		return item.package.metadata?.description ?? item.package.description;
	}
	return item.collection.blurb ?? "";
}

/** Cover art for an item: an app's thumbnail, a package's thumbnail, or a collection's first cover. */
export function itemCover(item: ExploreResolvedItem): string | undefined {
	if (item.kind === "app") return item.metadata?.thumbnail ?? undefined;
	if (item.kind === "package") return item.package.metadata?.thumbnail;
	for (const entry of item.collection.preview) {
		const cover = itemCover(entry);
		if (cover) return cover;
	}
	return undefined;
}

export function collectionHref(collectionId: string): string {
	return exploreSearchHref({ collection: collectionId });
}

/** Plain left clicks run `onOpen`; modified clicks keep the browser's own link handling. */
function isPlainClick(event: MouseEvent): boolean {
	return !(
		event.button !== 0 ||
		event.metaKey ||
		event.ctrlKey ||
		event.shiftKey ||
		event.altKey
	);
}

export function useItemHref() {
	const links = useExploreLinks();
	return useCallback(
		(item: ExploreResolvedItem) => {
			if (item.kind === "app") return links.appHref(item.app.id);
			if (item.kind === "package") return links.packageHref(item.package.id);
			return collectionHref(item.collection.id);
		},
		[links],
	);
}

/** A link to any Explore item; apps open straight into `/use` when the viewer owns them. */
export function ExploreItemLink({
	item,
	className,
	children,
	label,
	tile,
}: Readonly<{
	item: ExploreResolvedItem;
	className?: string;
	children: ReactNode;
	label?: string;
	/** `data-explore-tile` marker for tests and styling hooks. */
	tile?: string;
}>) {
	const links = useExploreLinks();
	const href = useItemHref()(item);
	return (
		<Link
			href={href}
			aria-label={label}
			data-explore-tile={tile}
			className={className}
			onClick={(event) => {
				if (item.kind !== "app" || !isPlainClick(event)) return;
				event.preventDefault();
				links.openApp(item.app.id);
			}}
		>
			{children}
		</Link>
	);
}

export function ExploreAppLink({
	appId,
	label,
	className,
	children,
}: Readonly<{
	appId: string;
	label: string;
	className?: string;
	children: ReactNode;
}>) {
	const links = useExploreLinks();
	return (
		<Link
			href={links.appHref(appId)}
			aria-label={label}
			data-explore-app={appId}
			className={cn(
				"block min-w-0 rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
				className,
			)}
			onClick={(event) => {
				if (!isPlainClick(event)) return;
				event.preventDefault();
				links.openApp(appId);
			}}
		>
			{children}
		</Link>
	);
}

export function ExploreAppCard({
	item,
	className,
}: Readonly<{ item: ExploreAppItem; className?: string }>) {
	const links = useExploreLinks();
	const name = itemName(item);
	return (
		<ExploreAppLink appId={item.app.id} label={name} className="h-full">
			<AppCard
				app={item.app}
				metadata={item.metadata ?? undefined}
				variant="extended"
				isOwned={links.isOwned(item.app.id)}
				href={links.appHref(item.app.id)}
				className={cn("h-full min-h-95 w-full", className)}
			/>
		</ExploreAppLink>
	);
}

export function ExplorePackageCard({
	pkg,
	variant = "standard",
	className,
	footer,
}: Readonly<{
	pkg: PackageSummary;
	variant?: PackageCardVariant;
	className?: string;
	footer?: ReactNode;
}>) {
	const links = useExploreLinks();
	return (
		<PackageCard
			pkg={pkg}
			variant={variant}
			href={links.packageHref(pkg.id)}
			className={className}
			footer={footer}
		/>
	);
}

/** Apps keep the AppCard grammar and packages the PackageCard grammar; the two never swap. */
export function ExploreItemCard({
	item,
	size = "standard",
	className,
}: Readonly<{
	item: ExploreResolvedItem;
	size?: "standard" | "landscape";
	className?: string;
}>) {
	if (item.kind === "app") {
		return <ExploreAppCard item={item} className={className} />;
	}
	if (item.kind === "package") {
		return (
			<ExplorePackageCard
				pkg={item.package}
				variant={size === "landscape" ? "landscape" : "standard"}
				className={className}
			/>
		);
	}
	return (
		<ExploreCollectionCard collection={item.collection} className={className} />
	);
}

function ExploreCollectionCard({
	collection,
	className,
}: Readonly<{ collection: ExploreCollectionSummary; className?: string }>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	return (
		<Link
			href={collectionHref(collection.id)}
			data-explore-collection={collection.id}
			className={cn(
				"group relative flex h-full min-h-95 w-full flex-col overflow-hidden rounded-xl border border-border/60 bg-card p-5 shadow-sm transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				className,
			)}
		>
			<CollectionStack
				items={collection.preview}
				className="h-36 w-full"
				cardClassName="h-24 w-36"
			/>
			<span className="mt-4 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
				<Layers aria-hidden="true" className="size-3.5 text-foreground" />
				{t("exploreCollection", "Collection")}
			</span>
			<h3 className="mt-1.5 line-clamp-2 text-lg font-semibold leading-snug tracking-tight">
				{collection.title}
			</h3>
			{collection.blurb && (
				<p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
					{collection.blurb}
				</p>
			)}
			<span className="mt-auto flex items-center justify-between gap-3 pt-4">
				<span className="font-mono text-xs tabular-nums text-muted-foreground">
					{labels.kindCounts(collection.apps, collection.packages)}
				</span>
				<span className="inline-flex items-center gap-1.5 text-sm font-semibold text-primary">
					{t("exploreOpenCollection", "Open collection")}
					<ArrowRight aria-hidden="true" className="size-3.5" />
				</span>
			</span>
		</Link>
	);
}

/** The store footer of a package card: installs · rating · price. */
export function PackageStatCells({ pkg }: Readonly<{ pkg: PackageSummary }>) {
	const { t } = useTranslation("store");
	const rated = (pkg.ratingCount ?? 0) > 0;
	return (
		<>
			<PackageCardStat
				value={formatCompact(pkg.downloadCount)}
				label={t("installs", "Installs")}
				valueClassName="tabular-nums"
			/>
			<PackageCardStat
				value={
					rated ? (
						<>
							<PackageRatingStar />
							{(pkg.avgRating ?? 0).toFixed(1)}
						</>
					) : (
						t("new", "New")
					)
				}
				label={t("rating", "Rating")}
				valueClassName="flex items-center gap-1 tabular-nums"
			/>
			<PackageCardStat
				value={pkg.price > 0 ? formatPrice(pkg.price) : t("free", "Free")}
				label={t("price", "Price")}
				valueClassName={pkg.price > 0 ? "text-primary" : undefined}
			/>
		</>
	);
}

/** A package's icon, or its hashed gradient with initials when it has none. */
export function PackageGlyph({
	pkg,
	className,
	textClassName,
}: Readonly<{
	pkg: PackageSummary;
	className?: string;
	textClassName?: string;
}>) {
	const gradient = usePackageGradient(pkg.id);
	const icon = useAssetImage(pkg.metadata?.icon);
	const name = pkg.metadata?.name ?? pkg.name;
	return (
		<span
			aria-hidden="true"
			className={cn(
				"relative flex shrink-0 items-center justify-center overflow-hidden",
				className,
			)}
			style={{ background: gradient }}
		>
			{icon.canRender ? (
				<img
					ref={icon.imgRef}
					src={icon.src}
					onLoad={icon.onLoad}
					onError={icon.onError}
					alt=""
					className="absolute inset-0 h-full w-full object-cover"
				/>
			) : (
				<span
					className={cn(
						"font-mono font-bold text-white/80",
						textClassName ?? "text-xs",
					)}
				>
					{getPackageInitials(name)}
				</span>
			)}
		</span>
	);
}

/** Small square thumbnail for any item: app cover or icon, package icon or gradient. */
export function ItemThumb({
	item,
	className,
}: Readonly<{ item: ExploreResolvedItem; className?: string }>) {
	if (item.kind === "package") {
		return (
			<PackageGlyph
				pkg={item.package}
				className={className}
				textClassName="text-[9px]"
			/>
		);
	}
	if (item.kind === "collection") {
		const first = item.collection.preview[0];
		return first ? (
			<ItemThumb item={first} className={className} />
		) : (
			<span className={cn("shrink-0 bg-muted", className)} />
		);
	}
	return <AppThumb item={item} className={className} />;
}

function AppThumb({
	item,
	className,
}: Readonly<{ item: ExploreAppItem; className?: string }>) {
	const cover = useAssetImage(item.metadata?.thumbnail ?? item.metadata?.icon);
	const fallback = usePackageGradient(item.app.id);
	return (
		<span
			aria-hidden="true"
			className={cn("relative block shrink-0 overflow-hidden", className)}
			style={{ background: fallback }}
		>
			{cover.canRender && (
				<img
					ref={cover.imgRef}
					src={cover.src}
					onLoad={cover.onLoad}
					onError={cover.onError}
					alt=""
					className="absolute inset-0 h-full w-full object-cover"
				/>
			)}
		</span>
	);
}

/** An app's icon cut to its type silhouette, on a translucent tile for dark artwork. */
export function AppIdentityMark({
	item,
	size = 44,
}: Readonly<{ item: ExploreAppItem; size?: number }>) {
	const name = itemName(item);
	return (
		<AppTypeMark
			type={item.app.app_type}
			size={size}
			src={item.metadata?.icon ?? "/app-logo.webp"}
			badgeOnDark
			background="oklch(1 0 0 / 0.12)"
			fallback={name.substring(0, 2).toUpperCase()}
		/>
	);
}

/** Cascading cards (front = first item), as on the collection tile and Browse's collection card. */
export function CollectionStack({
	items,
	className,
	cardClassName,
	max = 4,
}: Readonly<{
	items: readonly ExploreResolvedItem[];
	className?: string;
	cardClassName?: string;
	max?: number;
}>) {
	const shown = items.slice(0, max);
	const depth = shown.length;
	return (
		<div aria-hidden="true" className={cn("relative shrink-0", className)}>
			{shown
				.map((item, index) => ({ item, index }))
				.reverse()
				.map(({ item, index }) => {
					const step = depth - 1 - index;
					return (
						<div
							key={`${item.kind}:${itemId(item)}`}
							className={cn(
								"absolute overflow-hidden rounded-[10px] border border-white/15 shadow-[0_0_0_3px_var(--card)]",
								cardClassName ?? "h-25 w-37.5",
							)}
							style={{ left: step * 24, top: step * 20 }}
						>
							<StackFace item={item} front={index === 0} />
						</div>
					);
				})}
		</div>
	);
}

function StackFace({
	item,
	front,
}: Readonly<{ item: ExploreResolvedItem; front: boolean }>) {
	const cover = itemCover(item);
	const image = useAssetImage(cover);
	const gradient = usePackageGradient(itemId(item));
	return (
		<div className="absolute inset-0" style={{ background: gradient }}>
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
				<span className="absolute left-2 top-1.5 font-mono text-[10px] font-bold text-white/80">
					{getPackageInitials(itemName(item))}
				</span>
			)}
			{front && (
				<span className="absolute inset-x-0 bottom-0 truncate bg-linear-to-t from-black/85 to-transparent px-2.5 pb-2 pt-4 text-[11px] font-semibold leading-tight text-white">
					{itemName(item)}
				</span>
			)}
		</div>
	);
}

/** Name chip for a collection member: apps in the sans face with a cover, packages in mono with their gradient. */
export function ItemChip({ item }: Readonly<{ item: ExploreResolvedItem }>) {
	return (
		<span
			className={cn(
				"inline-flex h-6 max-w-full items-center gap-1.5 truncate rounded-md border border-border/70 bg-muted/40 pl-1 pr-2 text-foreground",
				item.kind === "package"
					? "font-mono text-[11px]"
					: "text-xs font-medium",
			)}
		>
			<ItemThumb item={item} className="size-4 rounded" />
			<span className="truncate">{itemName(item)}</span>
		</span>
	);
}
