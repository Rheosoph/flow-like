import { HeartFilledIcon } from "@radix-ui/react-icons";
import { motion } from "framer-motion";
import {
	Check,
	CircleUserIcon,
	FlaskConicalIcon,
	GlobeLockIcon,
	LockIcon,
	Settings,
	Star,
} from "lucide-react";
import Link from "next/link";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { formatAppCategory } from "../../lib/app-category";
import { categoryColor } from "../../lib/category-meta";
import { type IApp, IAppVisibility } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";
import { VISIBILITY_META } from "../settings/visibility-status/visibility-meta";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";

const MotionLink = motion.create(Link);

interface AppCardProps {
	app: IApp;
	metadata?: IMetadata;
	variant: "extended" | "small";
	onClick?: () => void;
	onSettingsClick?: () => void;
	settingsHref?: string;
	multiSelected?: boolean;
	className?: string;
	isOwned?: boolean;
	href?: string;
}

export function AppCard({
	app,
	metadata,
	variant = "extended",
	onClick,
	onSettingsClick,
	settingsHref,
	multiSelected,
	className = "",
	isOwned,
	href,
}: Readonly<AppCardProps>) {
	if (variant === "small") {
		return (
			<SmallAppCard
				app={app}
				metadata={metadata}
				onClick={onClick}
				onSettingsClick={onSettingsClick}
				settingsHref={settingsHref}
				className={className}
				multiSelected={multiSelected}
				isOwned={isOwned}
				href={href}
			/>
		);
	}

	return (
		<ExtendedAppCard
			app={app}
			metadata={metadata}
			onClick={onClick}
			onSettingsClick={onSettingsClick}
			settingsHref={settingsHref}
			className={className}
			multiSelected={multiSelected}
			isOwned={isOwned}
			href={href}
		/>
	);
}

export function VisibilityIcon({
	visibility,
}: Readonly<{ visibility: IAppVisibility }>) {
	const [isOpen, setIsOpen] = useState(false);
	const [position, setPosition] = useState({ x: 0, y: 0 });
	const triggerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (isOpen && triggerRef.current) {
			const rect = triggerRef.current.getBoundingClientRect();
			setPosition({
				x: rect.left + rect.width / 2,
				y: rect.bottom + 8,
			});
		}
	}, [isOpen]);

	const renderTooltip = (content: React.ReactNode, icon: React.ReactNode) => (
		<>
			<div
				ref={triggerRef}
				className="relative group cursor-pointer"
				onMouseEnter={() => setIsOpen(true)}
				onMouseLeave={() => setIsOpen(false)}
			>
				{icon}
			</div>
			{isOpen &&
				createPortal(
					<div
						className="fixed z-9999 pointer-events-none"
						style={{
							left: position.x,
							top: position.y,
							transform: "translateX(-50%)",
						}}
					>
						<div className="bg-white/80 dark:bg-gray-900/80 backdrop-blur-xl border border-white/30 dark:border-white/10 shadow-2xl rounded-lg p-3 animate-in fade-in-0 zoom-in-95 duration-200">
							{content}
						</div>
					</div>,
					document.body,
				)}
		</>
	);

	switch (visibility) {
		case IAppVisibility.Offline:
			return renderTooltip(
				<div className="flex items-center gap-2 text-red-700 dark:text-red-300">
					<div className="w-2 h-2 bg-red-500/70 rounded-full shadow-sm" />
					<p className="text-xs font-medium whitespace-nowrap">
						{VISIBILITY_META[IAppVisibility.Offline].tooltip}
					</p>
				</div>,
				<div className="relative bg-white/15 dark:bg-white/8 backdrop-blur-md rounded-full p-2 border border-white/25 dark:border-white/15 shadow-lg group-hover:shadow-xl transition-all duration-300">
					<div className="absolute inset-0 bg-red-500/25 rounded-full group-hover:bg-red-500/35 transition-all duration-300" />
					<LockIcon className="w-3 h-3 text-red-100 relative z-10 drop-shadow-xs group-hover:scale-110 group-hover:rotate-12 transition-all duration-300" />
				</div>,
			);

		case IAppVisibility.Private:
			return renderTooltip(
				<div className="flex items-center gap-2 text-purple-700 dark:text-purple-300">
					<div className="w-2 h-2 bg-linear-to-r from-purple-500/70 to-pink-500/70 rounded-full shadow-sm" />
					<p className="text-xs font-medium whitespace-nowrap">
						{VISIBILITY_META[IAppVisibility.Private].tooltip}
					</p>
				</div>,
				<div className="relative bg-white/15 dark:bg-white/8 backdrop-blur-md rounded-full p-2 border border-white/25 dark:border-white/15 shadow-lg group-hover:shadow-xl transition-all duration-300">
					<div className="absolute inset-0 bg-linear-to-br from-purple-500/30 to-pink-500/30 rounded-full group-hover:from-purple-500/40 group-hover:to-pink-500/40 transition-all duration-300" />
					<CircleUserIcon className="w-3 h-3 text-purple-100 relative z-10 drop-shadow-xs group-hover:scale-110 group-hover:rotate-12 transition-all duration-300" />
				</div>,
			);

		case IAppVisibility.Prototype:
			return renderTooltip(
				<div className="flex items-center gap-2 text-orange-700 dark:text-orange-300">
					<div className="w-2 h-2 bg-linear-to-r from-orange-500/70 to-yellow-500/70 rounded-full shadow-sm" />
					<p className="text-xs font-medium whitespace-nowrap">
						{VISIBILITY_META[IAppVisibility.Prototype].tooltip}
					</p>
				</div>,
				<div className="relative group cursor-pointer">
					<div className="relative bg-white/15 dark:bg-white/8 backdrop-blur-md rounded-full p-2 border border-white/25 dark:border-white/15 shadow-lg group-hover:shadow-xl transition-all duration-300">
						<div className="absolute inset-0 bg-linear-to-br from-orange-400/30 to-yellow-400/30 rounded-full group-hover:from-orange-400/45 group-hover:to-yellow-400/45 transition-all duration-300" />
						<FlaskConicalIcon className="w-3 h-3 text-orange-100 relative z-10 drop-shadow-xs transition-all duration-300 group-hover:rotate-12 group-hover:scale-110" />
					</div>
					<div className="absolute top-0 left-1/2 w-1 h-1 bg-linear-to-r from-orange-400/90 to-yellow-400/90 backdrop-blur-xs rounded-full -translate-x-1/2 shadow-sm group-hover:scale-125 group-hover:-translate-y-0.5 transition-all duration-300" />
					<div className="absolute top-1 right-0 w-0.5 h-0.5 bg-yellow-400/90 backdrop-blur-xs rounded-full shadow-sm group-hover:scale-150 group-hover:-translate-y-0.5 transition-all duration-300" />
				</div>,
			);

		case IAppVisibility.Public:
			return null;

		case IAppVisibility.PublicRequestAccess:
			return renderTooltip(
				<div className="flex items-center gap-2 text-blue-700 dark:text-blue-300">
					<div className="w-2 h-2 bg-linear-to-r from-blue-500/70 to-cyan-500/70 rounded-full shadow-sm" />
					<p className="text-xs font-medium whitespace-nowrap">
						{VISIBILITY_META[IAppVisibility.PublicRequestAccess].tooltip}
					</p>
				</div>,
				<div className="relative group cursor-pointer">
					<div className="absolute -inset-1 bg-linear-to-r from-blue-500/20 via-cyan-500/20 to-teal-500/20 rounded-full opacity-60 group-hover:opacity-90 group-hover:scale-105 transition-all duration-500 backdrop-blur-xs" />
					<div className="relative bg-white/20 dark:bg-white/8 backdrop-blur-lg rounded-full p-2 border border-white/30 dark:border-white/20 shadow-xl group-hover:shadow-2xl transition-all duration-300">
						<div className="absolute inset-0 bg-linear-to-br from-blue-400/25 to-cyan-400/25 rounded-full group-hover:from-blue-400/35 group-hover:to-cyan-400/35 transition-all duration-300" />
						<GlobeLockIcon className="w-3 h-3 text-blue-100 relative z-10 drop-shadow-xs transition-all duration-300 group-hover:scale-110 group-hover:-rotate-6" />
					</div>
				</div>,
			);
	}
}

function SmallAppCard({
	app,
	metadata,
	onClick,
	onSettingsClick,
	className,
	multiSelected,
	isOwned,
	href,
}: Readonly<
	Pick<
		AppCardProps,
		| "app"
		| "metadata"
		| "onClick"
		| "onSettingsClick"
		| "settingsHref"
		| "className"
		| "multiSelected"
		| "href"
		| "isOwned"
	>
>) {
	const formatPrice = (price: number) => `€${(price / 100).toFixed(2)}`;
	const { primaryHue, isDark } = useThemeInfo();
	const [thumbFailed, setThumbFailed] = useState(false);
	const handleCardKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
		if (!onClick || event.defaultPrevented) return;
		if (event.key === "Enter" || event.key === " ") {
			event.preventDefault();
			onClick();
		}
	};

	const itemVariants = {
		hidden: { opacity: 0, y: 20 },
		visible: { opacity: 1, y: 0 },
		hover: {},
	};

	return (
		<motion.div
			className="w-full h-full min-w-0"
			variants={itemVariants}
			whileHover="hover"
			whileTap={{ scale: 0.98 }}
			transition={{ type: "spring", stiffness: 300 }}
		>
			<div
				role={onClick ? "button" : undefined}
				tabIndex={onClick ? 0 : undefined}
				onClick={onClick}
				onKeyDown={handleCardKeyDown}
				data-href={href}
				data-title={metadata?.name ?? app.id}
				className={cn(
					"group cursor-pointer relative flex items-center gap-3 p-3 transition-all duration-300 rounded-xl border border-border/40 bg-card/80 backdrop-blur-sm hover:border-primary/20 hover:bg-card/95 hover:shadow-md w-full overflow-hidden",
					className,
				)}
			>
				{typeof multiSelected !== "undefined" && onClick && (
					<div className="relative shrink-0 z-10">
						<Checkbox
							checked={multiSelected ?? false}
							onCheckedChange={onClick}
							label={`Select ${metadata?.name ?? app.id}`}
						/>
					</div>
				)}
				<div className="absolute left-0 top-0 bottom-0 w-32 opacity-20 group-hover:opacity-50 transition-all duration-300 overflow-hidden">
					{metadata?.thumbnail && !thumbFailed ? (
						<img
							src={metadata.thumbnail}
							alt={metadata.name ?? app.id}
							className="w-full h-full object-cover object-right"
							width={1280}
							height={640}
							loading="lazy"
							decoding="async"
							fetchPriority="low"
							onError={() => setThumbFailed(true)}
						/>
					) : (
						(() => {
							const g = hashToGradient(app.id, primaryHue, isDark);
							return (
								<div
									className="absolute inset-0"
									style={{
										background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})`,
										opacity: g.opacity,
									}}
								/>
							);
						})()
					)}
					<div className="absolute inset-0 bg-gradient-to-r from-transparent to-card" />
				</div>

				<div className="relative shrink-0 z-10">
					<Avatar className="w-12 h-12 rounded-xl shadow-sm">
						<motion.div
							variants={{
								hover: { scale: 0.9 },
							}}
							transition={{ type: "spring", stiffness: 300 }}
						>
							<AvatarImage
								src={metadata?.icon ?? "/app-logo.webp"}
								alt={`${metadata?.name ?? app.id} icon`}
								className="rounded-xl"
							/>
						</motion.div>
						<AvatarFallback className="rounded-xl text-xs font-semibold bg-gradient-to-br from-primary/20 to-primary/10">
							{(metadata?.name ?? app.id).substring(0, 2).toUpperCase()}
						</AvatarFallback>
					</Avatar>
					{app.visibility !== IAppVisibility.Public && (
						<div className="absolute -top-0.5 -right-0.5 scale-[0.6]">
							<VisibilityIcon visibility={app.visibility} />
						</div>
					)}
				</div>

				<div className="flex-1 min-w-0 text-left relative z-10">
					<div className="flex items-start justify-between mb-1">
						<h4 className="font-semibold text-sm text-foreground truncate pr-2">
							{metadata?.name ?? app.id}
						</h4>

						{app.visibility === IAppVisibility.Public && (
							<div className="shrink-0">
								{app.price && app.price > 0 ? (
									<div className="bg-primary text-primary-foreground rounded-full px-2.5 py-0.5 text-xs font-semibold">
										{formatPrice(app.price)}
									</div>
								) : isOwned ? (
									<div className="bg-emerald-500/20 rounded-full px-2.5 py-0.5 text-xs text-emerald-500/80 border-emerald-500/80 border font-medium flex flex-row items-center gap-1">
										<HeartFilledIcon className="size-3" />
										Yours
									</div>
								) : (
									<div className="bg-muted/20 text-muted-foreground rounded-full px-2.5 py-0.5 text-xs font-medium">
										GET
									</div>
								)}
							</div>
						)}
					</div>

					<div className="flex items-center justify-between">
						<p className="text-xs text-muted-foreground truncate flex-1 mr-2">
							{metadata?.description ?? "No description available"}
						</p>

						{app.rating_count > 0 && (
							<div className="flex items-center gap-1 shrink-0">
								<Star className="w-2.5 h-2.5 fill-yellow-400 text-yellow-400" />
								<span className="text-xs font-medium">
									{(app.avg_rating ?? 0).toFixed(1)}
								</span>
							</div>
						)}
					</div>
				</div>
			</div>
		</motion.div>
	);
}

function ExtendedAppCard({
	app,
	metadata,
	onClick,
	onSettingsClick,
	settingsHref,
	className,
	multiSelected,
	isOwned,
	href,
}: Readonly<
	Pick<
		AppCardProps,
		| "app"
		| "metadata"
		| "onClick"
		| "onSettingsClick"
		| "settingsHref"
		| "className"
		| "multiSelected"
		| "isOwned"
		| "href"
	>
>) {
	const formatPrice = (price: number) => `€${(price / 100).toFixed(2)}`;
	const appName = metadata?.name ?? app.id;
	const appIcon = metadata?.icon ?? "/app-logo.webp";
	const hasRating = app.rating_count > 0;
	const { primaryHue, isDark } = useThemeInfo();
	const [thumbFailed, setThumbFailed] = useState(false);
	const [iconFailed, setIconFailed] = useState(false);
	const hasThumb = Boolean(metadata?.thumbnail) && !thumbFailed;
	const grad = hashToGradient(app.id, primaryHue, isDark);
	const eyebrowColor = app.primary_category
		? categoryColor(app.primary_category)
		: "var(--primary)";
	const showSettingsButton =
		(onSettingsClick || settingsHref) &&
		(app.visibility === IAppVisibility.Offline ||
			app.visibility === IAppVisibility.Private ||
			isOwned);
	const handleCardKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
		if (!onClick || event.defaultPrevented) return;
		if (event.key === "Enter" || event.key === " ") {
			event.preventDefault();
			onClick();
		}
	};

	const itemVariants = {
		hidden: { opacity: 0, y: 20 },
		visible: { opacity: 1, y: 0 },
		hover: {},
	};

	return (
		<motion.div
			className="w-full h-full min-w-0"
			variants={itemVariants}
			whileHover="hover"
			whileTap={{ scale: 0.98 }}
			transition={{ type: "spring", stiffness: 300 }}
		>
			<div
				role={onClick ? "button" : undefined}
				tabIndex={onClick ? 0 : undefined}
				onClick={onClick}
				onKeyDown={handleCardKeyDown}
				data-href={href}
				data-title={metadata?.name ?? app.id}
				className={cn(
					"group relative flex min-h-95 w-72 cursor-pointer flex-col justify-end overflow-hidden rounded-xl border border-border/40 bg-card shadow-sm transition-all duration-300 hover:-translate-y-1 hover:border-primary/30 hover:shadow-xl",
					className,
				)}
			>
				{/* full-bleed cover art (or the app's icon as ambient art) behind everything */}
				<div className="absolute inset-0">
					{hasThumb ? (
						<motion.img
							className="absolute inset-0 h-full w-full object-cover"
							src={metadata?.thumbnail ?? ""}
							alt=""
							width={1280}
							height={640}
							loading="lazy"
							decoding="async"
							fetchPriority="low"
							onError={() => setThumbFailed(true)}
							variants={{ hover: { scale: 1.04 } }}
							transition={{ type: "spring", stiffness: 200 }}
						/>
					) : (
						<>
							<div
								className="absolute inset-0"
								style={{
									background: `linear-gradient(${grad.angle}deg, ${grad.from}, ${grad.to})`,
									opacity: grad.opacity,
								}}
							/>
							{appIcon && !iconFailed && (
								<img
									src={appIcon}
									alt=""
									aria-hidden="true"
									loading="lazy"
									decoding="async"
									className="absolute left-1/2 top-[38%] h-[150%] w-[150%] -translate-x-1/2 -translate-y-1/2 object-contain opacity-40 blur-2xl saturate-150"
									onError={() => setIconFailed(true)}
								/>
							)}
						</>
					)}
					<div className="absolute inset-0 bg-linear-to-t from-black/90 via-black/45 to-black/5" />
				</div>

				{typeof multiSelected !== "undefined" && onClick && (
					<div className="absolute left-3 top-3 z-20">
						<Checkbox
							checked={multiSelected ?? false}
							onCheckedChange={onClick}
							label={`Select ${appName}`}
						/>
					</div>
				)}

				<div className="absolute right-3 top-3 z-20 flex items-center gap-2">
					{showSettingsButton &&
						(settingsHref ? (
							<MotionLink
								href={settingsHref}
								data-href={settingsHref}
								aria-label={`Open settings for ${appName}`}
								onClick={(e) => {
									e.stopPropagation();
									onSettingsClick?.();
								}}
								onPointerDown={(e) => {
									e.stopPropagation();
								}}
								onKeyDown={(e) => {
									e.stopPropagation();
								}}
								className="relative cursor-pointer rounded-full border border-white/30 bg-white/20 p-2 shadow-lg backdrop-blur-md transition-all duration-300 hover:bg-white/30 hover:shadow-xl"
								whileHover={{ scale: 1.05 }}
								whileTap={{ scale: 0.95 }}
							>
								<Settings className="h-3.5 w-3.5 text-white drop-shadow-xs" />
							</MotionLink>
						) : (
							<motion.button
								type="button"
								aria-label={`Open settings for ${appName}`}
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									onSettingsClick?.();
								}}
								onPointerDown={(e) => {
									e.stopPropagation();
								}}
								className="relative cursor-pointer rounded-full border border-white/30 bg-white/20 p-2 shadow-lg backdrop-blur-md transition-all duration-300 hover:bg-white/30 hover:shadow-xl"
								whileHover={{ scale: 1.05 }}
								whileTap={{ scale: 0.95 }}
							>
								<Settings className="h-3.5 w-3.5 text-white drop-shadow-xs" />
							</motion.button>
						))}
					<VisibilityIcon visibility={app.visibility} />
				</div>

				{/* meta overlaid on the scrim at the bottom */}
				<div className="relative z-10 flex flex-col gap-2.5 p-5">
					<div className="flex items-center gap-3">
						<Avatar className="size-11 shrink-0 rounded-xl border border-white/15 bg-white/10 shadow-lg backdrop-blur-md">
							<motion.div
								variants={{ hover: { scale: 0.92 } }}
								transition={{ type: "spring", stiffness: 300 }}
							>
								<AvatarImage
									src={appIcon}
									alt={`${appName} icon`}
									className="rounded-xl"
								/>
							</motion.div>
							<AvatarFallback className="rounded-xl bg-white/15 text-sm font-bold text-white">
								{appName.substring(0, 2).toUpperCase()}
							</AvatarFallback>
						</Avatar>
						<div className="min-w-0">
							<div
								className="truncate text-[11px] font-semibold uppercase tracking-wider"
								style={{ color: eyebrowColor }}
							>
								{formatAppCategory(app.primary_category)}
							</div>
							<h3 className="truncate text-lg font-bold leading-tight text-white">
								{appName}
							</h3>
						</div>
					</div>

					{/* reserve two lines so the title + medallion sit at the same height on every card */}
					<p className="line-clamp-2 min-h-12 text-sm leading-relaxed text-white/75">
						{metadata?.description ?? "No description available"}
					</p>

					<div className="mt-1 flex items-center justify-between gap-2">
						<div className="flex items-center gap-1.5 text-sm text-white/90">
							{hasRating ? (
								<>
									<Star className="size-4 fill-yellow-400 text-yellow-400" />
									<span className="font-semibold">
										{(app.avg_rating ?? 0).toFixed(1)}
									</span>
									<span className="text-xs text-white/50">
										({app.rating_count.toLocaleString()})
									</span>
								</>
							) : (
								<span className="text-xs text-white/50">No ratings yet</span>
							)}
						</div>
						{app.visibility === IAppVisibility.Public &&
							(app.price && app.price > 0 ? (
								<span className="rounded-full bg-white/90 px-3 py-1 text-xs font-bold text-gray-900 shadow-lg backdrop-blur-xs">
									{formatPrice(app.price)}
								</span>
							) : isOwned ? (
								<span className="flex items-center gap-1.5 rounded-full border border-emerald-400/40 bg-emerald-500/20 px-3 py-1 text-xs font-semibold text-emerald-300 backdrop-blur-xs">
									<HeartFilledIcon className="size-3.5" />
									Yours
								</span>
							) : (
								<span className="rounded-full border border-white/25 bg-white/15 px-3 py-1 text-xs font-semibold text-white backdrop-blur-xs">
									GET
								</span>
							))}
					</div>
				</div>
			</div>
		</motion.div>
	);
}

function formatCompact(n: number): string {
	if (n >= 1_000_000)
		return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
	return `${n}`;
}

/**
 * Wide, editorial "featured" card: a cover/ambient media panel beside a body that
 * leads with the app's use case, real tags and stats. Stacks vertically on narrow
 * widths. Apps with no thumbnail fall back to their own icon blurred as ambient art
 * (never a flat gradient slab).
 */
export function SpotlightCard({
	app,
	metadata,
	isOwned,
	onClick,
	href,
	className,
}: Readonly<
	Pick<
		AppCardProps,
		"app" | "metadata" | "isOwned" | "onClick" | "href" | "className"
	>
>) {
	const { primaryHue, isDark } = useThemeInfo();
	const [thumbFailed, setThumbFailed] = useState(false);
	const [iconFailed, setIconFailed] = useState(false);

	const appName = metadata?.name ?? app.id;
	const appIcon = metadata?.icon ?? "/app-logo.webp";
	const hasThumb = Boolean(metadata?.thumbnail) && !thumbFailed;
	const useCase = metadata?.use_case?.trim() || metadata?.description || "";
	const tags = (metadata?.tags ?? []).filter(Boolean).slice(0, 4);
	const author = app.authors?.find(Boolean);
	const hasRating = app.rating_count > 0;
	const grad = hashToGradient(app.id, primaryHue, isDark);

	const cta = isOwned ? (
		<span className="flex shrink-0 items-center gap-1.5 rounded-full border border-emerald-500/40 bg-emerald-500/15 px-3.5 py-1.5 text-xs font-semibold text-emerald-400">
			<HeartFilledIcon className="size-3.5" />
			Yours
		</span>
	) : (
		<span className="flex shrink-0 items-center gap-1.5 rounded-full bg-primary px-4 py-1.5 text-xs font-bold text-primary-foreground shadow-sm transition-transform group-hover:scale-[1.03]">
			{app.price && app.price > 0 ? `€${(app.price / 100).toFixed(2)}` : "Get"}
		</span>
	);

	const body = (
		<>
			<div className="relative min-h-40 overflow-hidden sm:min-h-0">
				{hasThumb ? (
					// real cover art speaks for itself — the icon lives in the body title
					<img
						src={metadata?.thumbnail ?? ""}
						alt=""
						loading="lazy"
						decoding="async"
						className="absolute inset-0 h-full w-full object-cover transition-transform duration-500 ease-out group-hover:scale-105"
						onError={() => setThumbFailed(true)}
					/>
				) : (
					// no thumbnail → the app's own icon becomes blurred ambient art,
					// with a crisp medallion on top (never a flat gradient slab)
					<>
						<div
							className="absolute inset-0"
							style={{
								background: `linear-gradient(${grad.angle}deg, ${grad.from}, ${grad.to})`,
								opacity: grad.opacity,
							}}
						/>
						{appIcon && !iconFailed && (
							<img
								src={appIcon}
								alt=""
								aria-hidden="true"
								loading="lazy"
								decoding="async"
								className="absolute left-1/2 top-1/2 h-[200%] w-[200%] -translate-x-1/2 -translate-y-1/2 object-contain opacity-45 blur-2xl saturate-150"
								onError={() => setIconFailed(true)}
							/>
						)}
						<div className="absolute inset-0 grid place-items-center">
							<Avatar className="size-16 rounded-2xl border border-white/15 bg-white/10 shadow-lg backdrop-blur-md">
								<AvatarImage
									src={appIcon}
									alt={`${appName} icon`}
									className="rounded-2xl"
								/>
								<AvatarFallback className="rounded-2xl bg-white/15 text-lg font-bold text-white">
									{appName.substring(0, 2).toUpperCase()}
								</AvatarFallback>
							</Avatar>
						</div>
					</>
				)}
				<div className="absolute inset-0 bg-linear-to-t from-card/80 via-card/10 to-transparent sm:bg-linear-to-r sm:from-transparent sm:to-card/40" />
			</div>

			<div className="flex min-w-0 flex-col p-5">
				<div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider">
					<span className="truncate text-primary">
						{formatAppCategory(app.primary_category)}
					</span>
					<span className="shrink-0 text-muted-foreground/50">· Featured</span>
				</div>

				<div className="mt-1.5 flex items-baseline gap-2">
					<h3 className="truncate text-xl font-bold tracking-tight text-foreground">
						{appName}
					</h3>
					{author && (
						<span className="shrink-0 truncate text-xs text-muted-foreground">
							by {author}
						</span>
					)}
				</div>

				{useCase && (
					<p className="mt-2 line-clamp-2 text-sm leading-relaxed text-muted-foreground">
						{useCase}
					</p>
				)}

				{tags.length > 0 && (
					<div className="mt-3 flex flex-wrap gap-1.5">
						{tags.map((tag) => (
							<span
								key={tag}
								className="rounded-md border border-border bg-muted/40 px-2 py-1 text-[11px] text-foreground/80"
							>
								{tag}
							</span>
						))}
					</div>
				)}

				<div className="mt-auto flex items-center justify-between gap-3 pt-4">
					<div className="flex items-center gap-3.5 text-xs tabular-nums text-muted-foreground">
						{hasRating && (
							<span className="flex items-center gap-1 font-semibold text-foreground">
								<Star className="size-3.5 fill-yellow-400 text-yellow-400" />
								{(app.avg_rating ?? 0).toFixed(1)}
							</span>
						)}
						{app.download_count > 0 && (
							<span>{formatCompact(app.download_count)} installs</span>
						)}
						{app.version && <span>v{app.version}</span>}
					</div>
					{cta}
				</div>
			</div>
		</>
	);

	const cardClass = cn(
		"group grid h-full grid-cols-1 overflow-hidden rounded-2xl border border-border/60 bg-card/80 shadow-sm backdrop-blur-sm transition-all duration-300 hover:-translate-y-1 hover:border-primary/30 hover:shadow-xl sm:grid-cols-[200px_1fr]",
		className,
	);

	return (
		<motion.div
			className="w-full min-w-0"
			whileHover={{ scale: 1 }}
			transition={{ type: "spring", stiffness: 300 }}
		>
			{href ? (
				<Link href={href} className={cardClass} onClick={onClick}>
					{body}
				</Link>
			) : (
				<div
					role={onClick ? "button" : undefined}
					tabIndex={onClick ? 0 : undefined}
					onClick={onClick}
					onKeyDown={(event) => {
						if (!onClick) return;
						if (event.key === "Enter" || event.key === " ") {
							event.preventDefault();
							onClick();
						}
					}}
					className={cardClass}
				>
					{body}
				</div>
			)}
		</motion.div>
	);
}

function Checkbox({
	checked,
	onCheckedChange,
	label,
}: { checked: boolean; onCheckedChange: () => void; label?: string }) {
	return (
		// biome-ignore lint/a11y/useSemanticElements: animated div checkbox; a native input cannot host the motion check mark
		<div
			role="checkbox"
			aria-checked={checked}
			aria-label={label ?? "Select item"}
			tabIndex={0}
			className="relative cursor-pointer"
			onClick={(e) => {
				e.stopPropagation();
				onCheckedChange();
			}}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					e.stopPropagation();
					onCheckedChange();
				}
			}}
		>
			<motion.div
				className={`w-5 h-5 rounded border-2 transition-all duration-200 ${
					checked
						? "bg-primary border-primary"
						: "bg-background border-border hover:border-primary/50"
				}`}
				whileTap={{ scale: 0.9 }}
			>
				<motion.div
					initial={{ scale: 0, opacity: 0 }}
					animate={{
						scale: checked ? 1 : 0,
						opacity: checked ? 1 : 0,
					}}
					transition={{ type: "spring", stiffness: 300, damping: 20 }}
					className="flex items-center justify-center h-full"
				>
					<Check className="w-3 h-3 text-primary-foreground" />
				</motion.div>
			</motion.div>
		</div>
	);
}
