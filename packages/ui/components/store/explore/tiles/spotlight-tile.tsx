"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	ChevronLeft,
	ChevronRight,
	Pause,
	Play,
	Shield,
} from "lucide-react";
import {
	type ReactNode,
	type Ref,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAssetImage } from "../../../../hooks/use-asset-image";
import { useAppCategoryLabel } from "../../../../lib/app-category";
import { usePackageCapabilities } from "../../../../lib/package-capabilities";
import { cn } from "../../../../lib/utils";
import { formatCompact, usePackageGradient } from "../../package-card";
import {
	AppIdentityMark,
	type ExploreAppItem,
	ExploreItemLink,
	ItemThumb,
	PackageGlyph,
	itemCover,
	itemDescription,
	itemId,
	itemName,
	useExploreLinks,
} from "../explore-item-card";
import { formatPrice, useExploreLabels } from "../explore-labels";
import { accentColor } from "../explore-model";
import type {
	ExploreResolvedItem,
	ExploreSlide,
	ExploreSpotlight,
} from "../explore-types";

const MAX_SIDE_ITEMS = 4;
const MAX_CAPABILITIES = 3;

function usePrefersReducedMotion(): boolean {
	const [reduced, setReduced] = useState(false);
	useEffect(() => {
		const query = globalThis.matchMedia?.("(prefers-reduced-motion: reduce)");
		if (!query) return;
		setReduced(query.matches);
		const onChange = () => setReduced(query.matches);
		query.addEventListener?.("change", onChange);
		return () => query.removeEventListener?.("change", onChange);
	}, []);
	return reduced;
}

function usePageHidden(): boolean {
	const [hidden, setHidden] = useState(false);
	useEffect(() => {
		if (typeof document === "undefined") return;
		const onChange = () => setHidden(document.visibilityState === "hidden");
		onChange();
		document.addEventListener("visibilitychange", onChange);
		return () => document.removeEventListener("visibilitychange", onChange);
	}, []);
	return hidden;
}

/**
 * Only keyboard focus pauses the carousel. Chromium focuses a clicked button, and that focus would otherwise keep
 * the rotation paused after the pointer has left.
 */
function isKeyboardFocus(target: EventTarget): boolean {
	if (!(target instanceof Element)) return false;
	try {
		return target.matches(":focus-visible");
	} catch {
		return true;
	}
}

/**
 * The progress fill of the active slide doubles as its timer: a Web Animation that pauses with the carousel and
 * advances the slide when it finishes. Manual navigation starts a fresh animation, so the timer resets.
 */
function useRotation(
	count: number,
	rotationSeconds: number,
	paused: boolean,
	disabled: boolean,
) {
	const [index, setIndex] = useState(0);
	const [cycle, setCycle] = useState(0);
	const fillRef = useRef<HTMLSpanElement | null>(null);
	const animation = useRef<Animation | null>(null);
	const active = count > 0 ? Math.min(index, count - 1) : 0;

	// biome-ignore lint/correctness/useExhaustiveDependencies: `cycle` restarts the timer after manual navigation.
	useEffect(() => {
		const fill = fillRef.current;
		if (!fill || count < 2 || disabled || typeof fill.animate !== "function") {
			return;
		}
		const running = fill.animate(
			[{ transform: "scaleX(0)" }, { transform: "scaleX(1)" }],
			{ duration: rotationSeconds * 1000, easing: "linear", fill: "forwards" },
		);
		running.onfinish = () => setIndex((current) => (current + 1) % count);
		animation.current = running;
		return () => {
			running.onfinish = null;
			running.cancel();
			animation.current = null;
		};
	}, [active, count, rotationSeconds, disabled, cycle]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: every new slide or cycle creates a new animation that must pick up the pause state.
	useEffect(() => {
		const running = animation.current;
		if (!running) return;
		if (paused) running.pause();
		else running.play();
	}, [paused, active, cycle]);

	const go = useCallback(
		(next: number) => {
			if (count === 0) return;
			setIndex(((next % count) + count) % count);
			setCycle((value) => value + 1);
		},
		[count],
	);

	return { active, go, fillRef };
}

interface SlideView {
	key: string;
	item: ExploreResolvedItem;
	name: string;
	headline: string;
	subline: string;
	cover?: string;
	accent: string;
	chip: string;
	chipMeta: string;
	cta: string;
}

function useSlideViews(slides: readonly ExploreSlide[]): SlideView[] {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const categoryLabel = useAppCategoryLabel();
	const links = useExploreLinks();
	return useMemo(
		() =>
			slides.map((slide, index) => {
				const item = slide.item;
				const name = itemName(item);
				const base = {
					key: `${index}:${item.kind}:${itemId(item)}`,
					item,
					name,
					headline: slide.headline?.trim() || name,
					subline: slide.subline?.trim() || itemDescription(item),
					cover: slide.artworkUrl ?? itemCover(item),
					accent: accentColor(slide.accent, item),
				};
				if (item.kind === "app") {
					const owned = links.isOwned(item.app.id);
					const price = item.app.price ?? 0;
					return {
						...base,
						chip: t("exploreKindApp", "App"),
						chipMeta: [
							labels.appType(item.app.app_type),
							categoryLabel(item.app.primary_category),
						]
							.filter(Boolean)
							.join(" · "),
						cta: owned
							? t("exploreOpen", "Open")
							: price > 0
								? formatPrice(price)
								: t("exploreGet", "Get"),
					};
				}
				if (item.kind === "package") {
					const category =
						item.package.primaryCategory ?? item.package.secondaryCategory;
					return {
						...base,
						chip: t("exploreKindPackage", "Package"),
						chipMeta: [
							category ? labels.packageCategory(category) : undefined,
							`v${item.package.latestVersion}`,
						]
							.filter(Boolean)
							.join(" · "),
						cta: t("exploreViewPackage", "View package"),
					};
				}
				return {
					...base,
					chip: t("exploreCollection", "Collection"),
					chipMeta: labels.kindCounts(
						item.collection.apps,
						item.collection.packages,
					),
					cta: t("exploreOpenCollection", "Open collection"),
				};
			}),
		[slides, t, labels, categoryLabel, links],
	);
}

export function SpotlightTile({
	spotlight,
	onCoverChange,
}: Readonly<{
	spotlight: ExploreSpotlight;
	onCoverChange?: (cover: string | undefined) => void;
}>) {
	const { t } = useTranslation("store");
	const views = useSlideViews(spotlight.slides);
	const [hovered, setHovered] = useState(false);
	const [focused, setFocused] = useState(false);
	const [stopped, setStopped] = useState(false);
	const hidden = usePageHidden();
	const reducedMotion = usePrefersReducedMotion();
	const paused = stopped || hovered || focused || hidden;
	const { active, go, fillRef } = useRotation(
		views.length,
		spotlight.rotationSeconds,
		paused,
		reducedMotion,
	);
	const current = views[active];
	const rotates = views.length > 1 && !reducedMotion;

	useEffect(() => {
		onCoverChange?.(current?.cover);
	}, [current?.cover, onCoverChange]);

	if (!current) return null;
	const wide = current.item.kind === "app";

	return (
		<article
			aria-roledescription="carousel"
			aria-label={t("exploreSpotlight", "Spotlight")}
			data-explore-tile="spotlight"
			className="relative isolate h-full min-h-104 overflow-hidden rounded-2xl border border-border/50 bg-neutral-950 text-white @3xl/explore:min-h-0"
			onMouseEnter={() => setHovered(true)}
			onMouseLeave={() => setHovered(false)}
			onFocusCapture={(event) => setFocused(isKeyboardFocus(event.target))}
			onBlurCapture={(event) => {
				if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
					setFocused(false);
				}
			}}
		>
			<SlideCover key={current.key} view={current} />
			<div
				aria-hidden="true"
				className="absolute inset-0 bg-linear-to-t from-black/90 via-black/50 to-black/10"
			/>
			<div
				aria-hidden="true"
				className="absolute inset-0 bg-linear-to-r from-black/60 via-black/0 to-black/0"
			/>

			<div className="absolute inset-x-4 top-12 flex items-center justify-between gap-3 @3xl/explore:inset-x-6 @3xl/explore:top-15.5">
				<span className="inline-flex h-6.5 min-w-0 items-center gap-2 rounded-full border border-white/15 bg-black/50 px-2.5 text-xs font-semibold text-white backdrop-blur-md">
					<span
						aria-hidden="true"
						className="size-1.75 shrink-0 rounded-full"
						style={{
							backgroundColor: current.accent,
							boxShadow: `0 0 0 3px color-mix(in oklch, ${current.accent} 25%, transparent)`,
						}}
					/>
					{current.chip}
					<span className="truncate font-medium text-white/60">
						{current.chipMeta}
					</span>
				</span>
				{views.length > 1 && (
					<div className="flex shrink-0 gap-2">
						{rotates && (
							<GlassButton
								label={
									stopped
										? t("explorePlaySpotlight", "Play spotlight")
										: t("explorePauseSpotlight", "Pause spotlight")
								}
								onClick={() => setStopped((value) => !value)}
							>
								{stopped ? (
									<Play className="size-3.5" />
								) : (
									<Pause className="size-3.5" />
								)}
							</GlassButton>
						)}
						<GlassButton
							label={t("explorePreviousSpotlight", "Previous spotlight")}
							onClick={() => go(active - 1)}
						>
							<ChevronLeft className="size-4" />
						</GlassButton>
						<GlassButton
							label={t("exploreNextSpotlight", "Next spotlight")}
							onClick={() => go(active + 1)}
						>
							<ChevronRight className="size-4" />
						</GlassButton>
					</div>
				)}
			</div>

			{views.length > 1 && (
				<SlidePicker
					views={views}
					active={active}
					onPick={go}
					fillRef={fillRef}
					reducedMotion={reducedMotion}
				/>
			)}

			<SideCard view={current} />

			<div
				aria-live={rotates && !paused ? "off" : "polite"}
				className={cn(
					"absolute inset-x-5 bottom-5 flex flex-col @3xl/explore:bottom-6.5 @3xl/explore:left-7 @3xl/explore:right-auto",
					wide
						? "@3xl/explore:w-[min(32.5rem,calc(100%-3.5rem))]"
						: "@3xl/explore:w-[min(25.875rem,calc(100%-3.5rem))]",
				)}
			>
				<h2 className="line-clamp-3 text-2xl font-bold leading-tight tracking-tight text-white @3xl/explore:text-[38px] @3xl/explore:leading-10.5">
					{current.headline}
				</h2>
				{current.subline && (
					<p className="mt-3 line-clamp-3 text-sm leading-relaxed text-white/80 @3xl/explore:text-[15px]">
						{current.subline}
					</p>
				)}
				<div className="mt-5 flex min-w-0 items-center gap-4">
					<ExploreItemLink
						item={current.item}
						className="inline-flex h-9.5 shrink-0 items-center gap-2 rounded-full bg-white px-4.5 text-[13px] font-semibold text-neutral-950 transition-colors hover:bg-white/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/70 focus-visible:ring-offset-2 focus-visible:ring-offset-black"
					>
						{current.cta}
						<ArrowRight aria-hidden="true" className="size-3.75" />
					</ExploreItemLink>
					<span aria-hidden="true" className="h-7 w-px shrink-0 bg-white/20" />
					<SlideIdentity view={current} />
				</div>
			</div>
		</article>
	);
}

function GlassButton({
	label,
	onClick,
	children,
}: Readonly<{ label: string; onClick: () => void; children: ReactNode }>) {
	return (
		<button
			type="button"
			aria-label={label}
			onClick={onClick}
			className="inline-flex size-8 items-center justify-center rounded-full border border-white/20 bg-white/10 text-white backdrop-blur-md transition-colors hover:bg-white/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/70"
		>
			{children}
		</button>
	);
}

/** One progress segment per slide; the active one's fill is the rotation timer. After the controls in tab order. */
function SlidePicker({
	views,
	active,
	onPick,
	fillRef,
	reducedMotion,
}: Readonly<{
	views: readonly SlideView[];
	active: number;
	onPick: (index: number) => void;
	fillRef: Ref<HTMLSpanElement>;
	reducedMotion: boolean;
}>) {
	const { t } = useTranslation("store");
	return (
		<div
			className="absolute inset-x-4 top-3 grid gap-2 @3xl/explore:inset-x-6"
			style={{
				gridTemplateColumns: `repeat(${views.length}, minmax(0, 1fr))`,
			}}
		>
			{views.map((view, index) => (
				<button
					key={view.key}
					type="button"
					aria-label={t("exploreShowSpotlight", {
						defaultValue: "Show spotlight {{index}} of {{total}}: {{label}}",
						index: index + 1,
						total: views.length,
						label: view.name,
					})}
					aria-current={index === active ? "true" : undefined}
					onClick={() => onPick(index)}
					className="flex min-w-0 flex-col gap-1.5 rounded-sm pb-0.5 pt-1.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/70"
				>
					<span className="relative block h-0.75 overflow-hidden rounded-full bg-white/20">
						<span
							ref={index === active ? fillRef : undefined}
							className={cn(
								"absolute inset-0 origin-left rounded-full",
								index < active && "bg-white/85",
							)}
							style={{
								backgroundColor: index === active ? view.accent : undefined,
								transform:
									index < active || (index === active && reducedMotion)
										? "scaleX(1)"
										: "scaleX(0)",
							}}
						/>
					</span>
					<span
						className={cn(
							"hidden min-w-0 gap-1.5 whitespace-nowrap text-[11px] leading-3.5 @3xl/explore:flex",
							index === active
								? "font-semibold text-white"
								: "font-medium text-white/60",
						)}
					>
						<span className="font-mono tabular-nums">
							{String(index + 1).padStart(2, "0")}
						</span>
						<span className="truncate">{view.name}</span>
					</span>
				</button>
			))}
		</div>
	);
}

function SlideCover({ view }: Readonly<{ view: SlideView }>) {
	const image = useAssetImage(view.cover);
	const gradient = usePackageGradient(itemId(view.item));
	return (
		<div aria-hidden="true" className="absolute inset-0 -z-10">
			<div className="absolute inset-0" style={{ background: gradient }} />
			{image.canRender && (
				<img
					ref={image.imgRef}
					src={image.src}
					onLoad={image.onLoad}
					onError={image.onError}
					alt=""
					className={cn(
						"absolute inset-0 h-full w-full object-cover transition-opacity duration-500",
						image.loaded ? "opacity-100" : "opacity-0",
					)}
				/>
			)}
		</div>
	);
}

function SlideIdentity({ view }: Readonly<{ view: SlideView }>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const categoryLabel = useAppCategoryLabel();
	const item = view.item;
	let mark: ReactNode;
	let name = view.name;
	let meta: string;
	let mono = false;
	if (item.kind === "app") {
		mark = <AppIdentityMark item={item as ExploreAppItem} size={34} />;
		meta = [
			categoryLabel(item.app.primary_category),
			item.app.rating_count > 0
				? `★ ${(item.app.avg_rating ?? 0).toFixed(1)} (${item.app.rating_count})`
				: undefined,
			(item.app.price ?? 0) > 0
				? formatPrice(item.app.price ?? 0)
				: t("free", "Free"),
		]
			.filter(Boolean)
			.join(" · ");
	} else if (item.kind === "package") {
		mono = true;
		mark = (
			<PackageGlyph
				pkg={item.package}
				className="size-8.5 rounded-full border border-white/20"
				textClassName="text-[10px]"
			/>
		);
		meta = [
			t("exploreInstallCount", {
				count: item.package.downloadCount,
				formatted: formatCompact(item.package.downloadCount),
				defaultValue_one: "{{formatted}} install",
				defaultValue_other: "{{formatted}} installs",
			}),
			item.package.price > 0
				? formatPrice(item.package.price)
				: t("free", "Free"),
			item.package.verified ? t("exploreVerifiedLower", "verified") : "",
		]
			.filter(Boolean)
			.join(" · ");
	} else {
		name = t("exploreCuratedCollection", "Curated collection");
		mark = (
			<ItemThumb
				item={item}
				className="size-8.5 rounded-lg border border-white/20"
			/>
		);
		meta = labels.kindCounts(item.collection.apps, item.collection.packages);
	}
	return (
		<div className="flex min-w-0 items-center gap-2.5">
			{mark}
			<div className="flex min-w-0 flex-col">
				<span
					className={cn(
						"truncate text-[13px] font-semibold leading-4.5 text-white",
						mono && "font-mono",
					)}
				>
					{name}
				</span>
				<span className="truncate text-xs text-white/60">{meta}</span>
			</div>
		</div>
	);
}

const SIDE_CARD =
	"absolute bottom-6 right-6 hidden rounded-[14px] border border-white/12 bg-black/60 backdrop-blur-xl @3xl/explore:block @5xl/explore:hidden @7xl/explore:block";

function SideCard({ view }: Readonly<{ view: SlideView }>) {
	if (view.item.kind === "package") {
		return <PackageSideCard item={view.item} />;
	}
	if (view.item.kind === "collection") {
		return <CollectionSideCard items={view.item.collection.preview} />;
	}
	return null;
}

function CollectionSideCard({
	items,
}: Readonly<{ items: readonly ExploreResolvedItem[] }>) {
	const { t } = useTranslation("store");
	const shown = items.slice(0, MAX_SIDE_ITEMS);
	return (
		<div className={cn(SIDE_CARD, "w-64 p-3")}>
			<div className="flex items-center justify-between px-0.5 pb-2 text-[11px] font-semibold uppercase tracking-wider text-white/60">
				<span>{t("exploreInThisCollection", "In this collection")}</span>
				<span className="font-mono font-medium normal-case tracking-normal">
					{t("exploreItemCount", {
						count: items.length,
						defaultValue_one: "{{count}} item",
						defaultValue_other: "{{count}} items",
					})}
				</span>
			</div>
			<ul className="flex flex-col gap-1">
				{shown.map((item) => (
					<li
						key={`${item.kind}:${itemId(item)}`}
						className="flex h-9.5 min-w-0 items-center gap-2.5 rounded-lg bg-white/5 pl-1.25 pr-2"
					>
						<ItemThumb
							item={item}
							className="size-7 rounded-md border border-white/12"
						/>
						<span
							className={cn(
								"min-w-0 truncate text-white",
								item.kind === "package"
									? "font-mono text-xs font-medium"
									: "text-[13px] font-semibold",
							)}
						>
							{itemName(item)}
						</span>
						<span className="ml-auto shrink-0 font-mono text-[10px] uppercase tracking-wider text-white/60">
							{item.kind === "package"
								? t("exploreKindPackage", "Package")
								: t("exploreKindApp", "App")}
						</span>
					</li>
				))}
			</ul>
		</div>
	);
}

function PackageSideCard({
	item,
}: Readonly<{ item: Extract<ExploreResolvedItem, { kind: "package" }> }>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const pkg = item.package;
	const capabilities = usePackageCapabilities(pkg.capabilities);
	const category = pkg.primaryCategory ?? pkg.secondaryCategory;
	const rated = (pkg.ratingCount ?? 0) > 0;
	const stats: [value: string, label: string][] = [
		[formatCompact(pkg.downloadCount), t("installs", "Installs")],
		[
			rated ? (pkg.avgRating ?? 0).toFixed(1) : t("new", "New"),
			t("rating", "Rating"),
		],
		[
			pkg.price > 0 ? formatPrice(pkg.price) : t("free", "Free"),
			t("price", "Price"),
		],
	];
	return (
		<div className={cn(SIDE_CARD, "w-68 p-3.5")}>
			<div className="flex min-w-0 items-center gap-2.5">
				<PackageGlyph
					pkg={pkg}
					className="size-9 rounded-lg border border-white/15"
				/>
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-1.5">
						<span className="truncate font-mono text-[13px] font-semibold leading-4.5 text-white">
							{itemName(item)}
						</span>
						{pkg.verified && (
							<Shield
								aria-label={t("verified", "Verified")}
								className="size-3.5 shrink-0 text-sky-400"
							/>
						)}
					</div>
					<div className="truncate font-mono text-[10px] uppercase leading-3.5 tracking-wider text-white/60">
						{[
							category ? labels.packageCategory(category) : undefined,
							`v${pkg.latestVersion}`,
						]
							.filter(Boolean)
							.join(" · ")}
					</div>
				</div>
			</div>
			{pkg.capabilities && (
				<div className="mt-3 flex flex-wrap gap-1">
					{capabilities.length === 0 ? (
						<span className="rounded border border-dashed border-white/25 px-1.5 py-1 font-mono text-[10px] leading-none text-white/70">
							{t("noPermissionsRequested", "no permissions requested")}
						</span>
					) : (
						capabilities.slice(0, MAX_CAPABILITIES).map((capability) => (
							<span
								key={capability.key}
								title={capability.label}
								className={cn(
									"rounded border px-1.5 py-1 font-mono text-[10px] leading-none",
									capability.severity === "elevated"
										? "border-primary/40 bg-primary/15 text-primary"
										: "border-white/20 bg-white/5 text-white/70",
								)}
							>
								{capability.key}
							</span>
						))
					)}
					{capabilities.length > MAX_CAPABILITIES && (
						<span className="rounded border border-white/20 bg-white/5 px-1.5 py-1 font-mono text-[10px] leading-none text-white/70">
							{`+${capabilities.length - MAX_CAPABILITIES}`}
						</span>
					)}
				</div>
			)}
			<div className="mt-3 grid grid-cols-3 divide-x divide-white/12 border-t border-white/12 pt-2.5">
				{stats.map(([value, label]) => (
					<div key={label} className="px-2.5 first:pl-0 last:pr-0">
						<div className="font-mono text-[13px] font-semibold tabular-nums text-white">
							{value}
						</div>
						<div className="mt-0.5 font-mono text-[9px] uppercase tracking-wider text-white/60">
							{label}
						</div>
					</div>
				))}
			</div>
		</div>
	);
}
