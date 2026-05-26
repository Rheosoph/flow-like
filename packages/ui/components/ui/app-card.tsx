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
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { formatAppCategory } from "../../lib/app-category";
import { type IApp, IAppVisibility } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";
import { Badge } from "./badge";

interface AppCardProps {
	app: IApp;
	metadata?: IMetadata;
	variant: "extended" | "small";
	onClick?: () => void;
	onSettingsClick?: () => void;
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
						App is currently offline
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
						Private access only
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
						Experimental prototype
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
						Public with access request
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
		| "className"
		| "multiSelected"
		| "href"
		| "isOwned"
	>
>) {
	const formatPrice = (price: number) => `€${(price / 100).toFixed(2)}`;
	const { primaryHue, isDark } = useThemeInfo();

	const itemVariants = {
		hidden: { opacity: 0, y: 20 },
		visible: { opacity: 1, y: 0 },
		hover: {},
	};

	return (
		<motion.div
			variants={itemVariants}
			whileHover="hover"
			whileTap={{ scale: 0.98 }}
			transition={{ type: "spring", stiffness: 300 }}
		>
			<button
				type="button"
				onClick={onClick}
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
						/>
					</div>
				)}
				<div className="absolute left-0 top-0 bottom-0 w-32 opacity-20 group-hover:opacity-50 transition-all duration-300 overflow-hidden">
					<img
						src={metadata?.thumbnail ?? "/placeholder-thumbnail-small.jpg"}
						alt={metadata?.name ?? app.id}
						className="w-full h-full object-cover object-right"
						width={1280}
						height={640}
						loading="lazy"
						decoding="async"
						fetchPriority="low"
					/>
					{!metadata?.thumbnail &&
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
						})()}
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
			</button>
		</motion.div>
	);
}

function ExtendedAppCard({
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
	const showSettingsButton =
		onSettingsClick &&
		(app.visibility === IAppVisibility.Offline ||
			app.visibility === IAppVisibility.Private ||
			isOwned);

	const itemVariants = {
		hidden: { opacity: 0, y: 20 },
		visible: { opacity: 1, y: 0 },
		hover: {},
	};

	return (
		<motion.div
			variants={itemVariants}
			whileHover="hover"
			whileTap={{ scale: 0.98 }}
			transition={{ type: "spring", stiffness: 300 }}
		>
			<button
				type="button"
				onClick={onClick}
				data-href={href}
				data-title={metadata?.name ?? app.id}
				className={cn(
					"group cursor-pointer relative flex flex-col transition-all duration-300 rounded-xl border border-border/40 bg-card/80 backdrop-blur-sm shadow-sm hover:shadow-lg hover:border-primary/20 hover:bg-card/95 w-72 h-[375px] overflow-hidden",
					className,
				)}
			>
				{typeof multiSelected !== "undefined" && onClick && (
					<div className="relative shrink-0 z-10">
						<Checkbox
							checked={multiSelected ?? false}
							onCheckedChange={onClick}
						/>
					</div>
				)}
				<div className="relative w-full h-40 overflow-hidden">
					<motion.img
						className="absolute inset-0 w-full h-full object-cover "
						src={metadata?.thumbnail ?? "/placeholder-thumbnail.webp"}
						alt={appName}
						width={1280}
						height={640}
						loading="lazy"
						decoding="async"
						fetchPriority="low"
						variants={{
							hover: { scale: 1.02 },
						}}
						transition={{ type: "spring", stiffness: 300 }}
					/>
					{!metadata?.thumbnail &&
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
						})()}
					<div className="absolute inset-0 bg-gradient-to-t from-black/60 via-black/20 to-transparent" />

					<div className="absolute top-3 right-3 z-10 flex items-center gap-2">
						{showSettingsButton && (
							<motion.div
								role="button"
								tabIndex={0}
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									onSettingsClick?.();
								}}
								onPointerDown={(e) => {
									e.stopPropagation();
								}}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										e.stopPropagation();
										onSettingsClick?.();
									}
								}}
								className="relative bg-white/20 dark:bg-white/10 backdrop-blur-md rounded-full p-2 border border-white/30 dark:border-white/20 shadow-lg hover:shadow-xl hover:bg-white/30 dark:hover:bg-white/15 transition-all duration-300 cursor-pointer"
								whileHover={{ scale: 1.05 }}
								whileTap={{ scale: 0.95 }}
							>
								<Settings className="w-3.5 h-3.5 text-white drop-shadow-xs" />
							</motion.div>
						)}
						<VisibilityIcon visibility={app.visibility} />
					</div>

					<div className="absolute bottom-3 left-3 right-3 flex items-end justify-between">
						<Avatar className="w-16 h-16 shadow-lg bg-white/10 backdrop-blur-md">
							<motion.div
								variants={{
									hover: { scale: 0.9 },
								}}
								transition={{ type: "spring", stiffness: 300 }}
							>
								<AvatarImage
									className="scale-100"
									src={appIcon}
									alt={`${appName} icon`}
								/>
							</motion.div>
							<AvatarFallback className="text-lg font-bold bg-white/20 backdrop-blur-md text-white border border-white/30">
								{appName.substring(0, 2).toUpperCase()}
							</AvatarFallback>
						</Avatar>
						{app.visibility === IAppVisibility.Public && (
							<div className="mb-2">
								{app.price && app.price > 0 ? (
									<div className="bg-white/90 backdrop-blur-xs text-gray-900 rounded-full px-3 py-1 text-sm font-bold shadow-lg">
										{formatPrice(app.price)}
									</div>
								) : isOwned ? (
									<div className="bg-emerald-500/20 backdrop-blur-xs text-emerald-400/90 rounded-full px-3 py-1 text-sm font-medium shadow-lg border border-white/30 flex flex-row items-center gap-2">
										<HeartFilledIcon className="size-4" />
										Yours
									</div>
								) : (
									<div className="bg-white/20 backdrop-blur-xs text-white/90 rounded-full px-3 py-1 text-sm font-medium shadow-lg border border-white/30">
										GET
									</div>
								)}
							</div>
						)}
					</div>
				</div>

				<div className="flex flex-col p-5 flex-1">
					<h3 className="font-bold text-lg text-foreground text-left leading-tight line-clamp-1 min-h-6 mb-2">
						{appName}
					</h3>

					<div className="flex items-center gap-2 mb-3">
						<Badge variant="default" className="text-xs px-2 py-1">
							{formatAppCategory(app.primary_category)}
						</Badge>
						<Badge variant="outline" className="text-xs px-2 py-1">
							{metadata?.age_rating ?? 0}+
						</Badge>
					</div>

					<p className="text-sm text-muted-foreground text-left line-clamp-3 leading-relaxed min-h-[4.4rem] mb-3 overflow-hidden">
						{metadata?.description ?? "No description available"}
					</p>

					<div className="flex items-center justify-between">
						<div className="flex items-center gap-2 h-5">
							{hasRating ? (
								<>
									<div className="flex items-center gap-1">
										<Star className="w-4 h-4 fill-yellow-400 text-yellow-400" />
										<span className="font-semibold text-sm">
											{(app.avg_rating ?? 0).toFixed(1)}
										</span>
									</div>
									<span className="text-xs text-muted-foreground">
										({app.rating_count.toLocaleString()})
									</span>
								</>
							) : (
								<span className="text-xs text-muted-foreground">
									No ratings yet
								</span>
							)}
						</div>
					</div>
				</div>
			</button>
		</motion.div>
	);
}

function Checkbox({
	checked,
	onCheckedChange,
}: { checked: boolean; onCheckedChange: () => void }) {
	return (
		<div className="relative cursor-pointer" onClick={onCheckedChange}>
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
