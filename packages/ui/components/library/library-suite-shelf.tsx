"use client";

import { ChevronDown, Layers } from "lucide-react";
import { useRef, useState } from "react";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { cn } from "../../lib/utils";
import type { IGroup } from "../../state/backend-state/types";
import {
	VISIBILITY_META,
	fromWireVisibility,
} from "../settings/visibility-status/visibility-meta";
import { AppCard } from "../ui/app-card";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import type { LibraryItem } from "./library-types";
import { CARD_MIN_W_DESKTOP, CARD_MIN_W_MOBILE } from "./library-types";

/** A suite plus the library items that actually resolved for its members. */
export interface SuiteGroup {
	group: IGroup;
	items: LibraryItem[];
}

interface SuiteShelfProps {
	suites: SuiteGroup[];
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	appHref?: (id: string) => string;
	visibilityMode?: boolean;
	activeAppIds?: Set<string>;
	onToggleVisibility?: (id: string) => void;
	isMobile?: boolean;
}

export function SuiteShelf({ suites, ...rowProps }: Readonly<SuiteShelfProps>) {
	if (suites.length === 0) return null;

	return (
		<section>
			<div className="flex items-center gap-2 mb-3">
				<Layers className="h-3.5 w-3.5 text-muted-foreground/50" />
				<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
					Suites
				</h2>
				<span className="text-xs text-muted-foreground/30">
					{suites.length}
				</span>
			</div>

			<div className="space-y-2.5">
				{suites.map((suite) => (
					<SuiteRow key={suite.group.id} suite={suite} {...rowProps} />
				))}
			</div>
		</section>
	);
}

function SuiteRow({
	suite,
	onAppClick,
	settingsHref,
	appHref,
	visibilityMode,
	activeAppIds,
	onToggleVisibility,
	isMobile = false,
}: Readonly<{ suite: SuiteGroup } & Omit<SuiteShelfProps, "suites">>) {
	const { group, items } = suite;
	const [expanded, setExpanded] = useState(false);
	const [bannerFailed, setBannerFailed] = useState(false);
	const [iconFailed, setIconFailed] = useState(false);
	const containerRef = useRef<HTMLDivElement>(null);
	const { primaryHue, isDark } = useThemeInfo();
	const cardMin = isMobile ? CARD_MIN_W_MOBILE : CARD_MIN_W_DESKTOP;

	const title = group.name || "Untitled suite";
	const subtitle = group.use_case || group.description || null;
	const meta = VISIBILITY_META[fromWireVisibility(group.visibility)];
	const gradient = hashToGradient(group.id, primaryHue, isDark);

	const handleClick = (id: string) => {
		if (visibilityMode && onToggleVisibility) {
			onToggleVisibility(id);
		} else {
			onAppClick(id);
		}
	};

	return (
		<div
			className={cn(
				"group/suite relative rounded-xl border border-border/40 bg-card/80 backdrop-blur-sm overflow-hidden transition-all duration-300",
				expanded ? "border-primary/20 bg-card/95" : "hover:border-primary/20",
			)}
		>
			{/* Suite artwork sits behind the row as a soft edge wash, matching AppCard. */}
			<div className="pointer-events-none absolute left-0 top-0 bottom-0 w-64 opacity-20 group-hover/suite:opacity-40 transition-opacity duration-300 overflow-hidden">
				{group.banner && !bannerFailed ? (
					// eslint-disable-next-line @next/next/no-img-element
					<img
						src={group.banner}
						alt=""
						className="w-full h-full object-cover object-right"
						loading="lazy"
						decoding="async"
						onError={() => setBannerFailed(true)}
					/>
				) : (
					<div
						className="absolute inset-0"
						style={{
							background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
							opacity: gradient.opacity,
						}}
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-r from-transparent to-card" />
			</div>

			<button
				type="button"
				onClick={() => setExpanded((value) => !value)}
				aria-expanded={expanded}
				className="relative z-10 w-full flex items-center gap-3 px-3 py-2.5 text-left cursor-pointer"
			>
				<SuiteGlyph
					group={group}
					items={items}
					iconFailed={iconFailed}
					onIconError={() => setIconFailed(true)}
					primaryHue={primaryHue}
					isDark={isDark}
				/>

				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						<h3 className="font-semibold text-sm truncate">{title}</h3>
						<span className="inline-flex items-center gap-1 rounded-full border border-border/50 bg-background/60 px-1.5 py-px text-[10px] font-medium text-muted-foreground shrink-0">
							<meta.BadgeIcon className="h-2.5 w-2.5" />
							{meta.badgeLabel}
						</span>
					</div>
					<p className="text-xs text-muted-foreground/80 truncate mt-0.5">
						{subtitle}
					</p>
				</div>

				<div className="flex items-center gap-2.5 shrink-0 text-muted-foreground/70">
					<span className="text-xs font-medium tabular-nums">
						{items.length} app{items.length === 1 ? "" : "s"}
					</span>
					<ChevronDown
						className={cn(
							"h-4 w-4 transition-transform duration-300",
							expanded && "rotate-180",
						)}
					/>
				</div>
			</button>

			{expanded && (
				<div className="relative z-10 border-t border-border/40 bg-background/30 px-3 py-3">
					<div
						ref={containerRef}
						className="grid gap-3"
						style={{
							gridTemplateColumns: `repeat(auto-fill, minmax(${cardMin}px, 1fr))`,
						}}
					>
						{items.map((item) => {
							const isActive = activeAppIds?.has(item.id) ?? true;
							return (
								<div
									key={`${group.id}-${item.id}`}
									className={cn(
										"transition-all duration-300",
										visibilityMode &&
											!isActive &&
											"opacity-35 hover:opacity-70",
									)}
								>
									<AppCard
										isOwned
										app={item.app}
										metadata={item}
										variant={isMobile ? "small" : "extended"}
										onClick={() => handleClick(item.id)}
										settingsHref={
											visibilityMode ? undefined : settingsHref?.(item.id)
										}
										href={!visibilityMode ? appHref?.(item.id) : undefined}
										className="w-full"
									/>
								</div>
							);
						})}
					</div>
				</div>
			)}
		</div>
	);
}

/**
 * A suite's identity comes from what is inside it: a mosaic of its member app
 * icons, the way a folder shows its contents. The suite's own uploaded artwork
 * wins when it has any.
 */
function SuiteGlyph({
	group,
	items,
	iconFailed,
	onIconError,
	primaryHue,
	isDark,
}: Readonly<{
	group: IGroup;
	items: LibraryItem[];
	iconFailed: boolean;
	onIconError: () => void;
	primaryHue: number;
	isDark: boolean;
}>) {
	if (group.icon && !iconFailed) {
		return (
			<Avatar className="h-11 w-11 rounded-xl shrink-0 ring-1 ring-border/50">
				<AvatarImage
					src={group.icon}
					alt=""
					className="rounded-xl"
					onError={onIconError}
				/>
				<AvatarFallback className="rounded-xl">
					<Layers className="h-5 w-5 text-muted-foreground" />
				</AvatarFallback>
			</Avatar>
		);
	}

	const tiles = items.slice(0, 4);

	// Tile the 11x11 square edge-to-edge for any member count, so a suite never
	// renders with a dead quadrant: 1 fills it, 2 split vertically, 3 puts the
	// first app full-height beside two stacked, 4 is an even quarter grid.
	const spanFor = (index: number) => {
		if (tiles.length === 1) return "col-span-2 row-span-2";
		if (tiles.length === 2) return "row-span-2";
		if (tiles.length === 3 && index === 0) return "row-span-2";
		return "";
	};

	return (
		<div className="h-11 w-11 shrink-0 rounded-xl overflow-hidden ring-1 ring-border/50 grid grid-cols-2 grid-rows-2 gap-px bg-border/40">
			{tiles.map((item, index) => (
				<GlyphTile
					key={item.id}
					item={item}
					span={spanFor(index)}
					large={tiles.length === 1}
					primaryHue={primaryHue}
					isDark={isDark}
				/>
			))}
		</div>
	);
}

function GlyphTile({
	item,
	span,
	large,
	primaryHue,
	isDark,
}: Readonly<{
	item: LibraryItem;
	span: string;
	large: boolean;
	primaryHue: number;
	isDark: boolean;
}>) {
	const [failed, setFailed] = useState(false);
	const gradient = hashToGradient(item.id, primaryHue, isDark);

	return (
		<div className={cn("relative overflow-hidden bg-muted", span)}>
			{item.icon && !failed ? (
				// eslint-disable-next-line @next/next/no-img-element
				<img
					src={item.icon}
					alt=""
					className="absolute inset-0 h-full w-full object-cover"
					loading="lazy"
					decoding="async"
					onError={() => setFailed(true)}
				/>
			) : (
				<div
					className="absolute inset-0 flex items-center justify-center"
					style={{
						background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
					}}
				>
					<span
						className={cn(
							"font-semibold text-white/90 leading-none",
							large ? "text-sm" : "text-[8px]",
						)}
					>
						{(item.name ?? item.id).substring(0, large ? 2 : 1).toUpperCase()}
					</span>
				</div>
			)}
		</div>
	);
}
