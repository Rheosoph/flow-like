"use client";

import type { CSSProperties, ReactNode } from "react";
import { useAssetImage } from "../../hooks/use-asset-image";
import { appTypeMeta } from "../../lib/app-type";
import type { IAppType } from "../../lib/schema/app/app";
import { cn } from "../../lib/utils";

/**
 * Below this the badge covers more of the tile than it explains — the
 * silhouette carries the type on its own, and callers with room for words
 * should reach for {@link AppTypeLabel} instead.
 */
const MIN_SIZE_FOR_BADGE = 30;

function badgeSize(size: number): number {
	return Math.max(12, Math.round(size * 0.4));
}

export interface AppTypeMarkProps {
	type?: IAppType | null;
	/** Edge length of the tile in px. */
	size?: number;
	/** App icon URL. Falls back to `fallback` when absent or broken. */
	src?: string | null;
	/** Usually the app's initials. */
	fallback?: ReactNode;
	/** Hide the corner glyph — for very small marks where it would not read. */
	hideBadge?: boolean;
	/** Tile background when there is no icon, e.g. the app's hash gradient. */
	background?: string;
	/** Contrasting surface the badge sits on, so it reads on dark artwork. */
	badgeOnDark?: boolean;
	className?: string;
	style?: CSSProperties;
}

/**
 * An app's icon, cut to the silhouette of its type, with the type's glyph
 * notched into the corner.
 *
 * Type is carried by shape rather than colour on purpose: topic categories
 * already own a hue each (`CATEGORY_COLORS`) and visibility owns the coloured
 * status badges, so a third colour code would read as one of those. Shape is
 * the channel nothing else is using, and it costs no extra space — the icon
 * tile was already on the card.
 */
export function AppTypeMark({
	type,
	size = 40,
	src,
	fallback,
	hideBadge,
	background,
	badgeOnDark,
	className,
	style,
}: Readonly<AppTypeMarkProps>) {
	const meta = appTypeMeta(type);
	const Icon = meta.icon;
	const badge = badgeSize(size);
	const showBadge = !hideBadge && size >= MIN_SIZE_FOR_BADGE;

	// A plain <img> paints a cached icon immediately, unlike Radix's Avatar, but
	// it has no fallback of its own — a 404 would leave a broken image where the
	// initials used to be.
	const image = useAssetImage(src);

	return (
		<span
			className={cn("relative inline-block shrink-0 align-middle", className)}
			style={{ width: size, height: size, ...style }}
			title={meta.label}
		>
			<span
				className="relative block h-full w-full overflow-hidden bg-muted bg-cover bg-center font-bold leading-none text-white"
				style={{
					...meta.shape,
					background,
					fontSize: Math.max(8, Math.round(size * 0.3)),
				}}
			>
				{/* The initials sit underneath rather than instead of the icon, so an
				    icon that arrives late fades over them instead of replacing them. */}
				<span
					className={cn(
						"absolute inset-0 flex items-center justify-center transition-opacity duration-200",
						image.loaded && "opacity-0",
					)}
				>
					{fallback}
				</span>
				{image.canRender && (
					<img
						ref={image.imgRef}
						src={image.src}
						alt=""
						aria-hidden="true"
						loading="lazy"
						decoding="async"
						className={cn(
							"absolute inset-0 h-full w-full object-cover transition-opacity duration-200",
							image.loaded ? "opacity-100" : "opacity-0",
						)}
						onLoad={image.onLoad}
						onError={image.onError}
					/>
				)}
			</span>

			{showBadge && (
				<span
					className={cn(
						"absolute grid place-items-center rounded-full border shadow-sm",
						badgeOnDark
							? "border-white/25 bg-neutral-900 text-white"
							: "border-border bg-background text-foreground",
					)}
					style={{
						width: badge,
						height: badge,
						right: -Math.round(badge * 0.22),
						bottom: -Math.round(badge * 0.22),
					}}
				>
					<Icon
						style={{ width: badge * 0.6, height: badge * 0.6 }}
						strokeWidth={2.2}
					/>
				</span>
			)}
		</span>
	);
}

/**
 * Inline "glyph + label" for places that have room for words — the config
 * header, list rows, and the type dropdown.
 */
export function AppTypeLabel({
	type,
	className,
	showUnclassified = true,
}: Readonly<{
	type?: IAppType | null;
	className?: string;
	showUnclassified?: boolean;
}>) {
	if (!type && !showUnclassified) return null;
	const meta = appTypeMeta(type);
	const Icon = meta.icon;
	return (
		<span className={cn("inline-flex items-center gap-1.5", className)}>
			<Icon className="h-3 w-3 shrink-0" />
			{meta.label}
		</span>
	);
}
