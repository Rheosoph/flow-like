"use client";
import {
	Download,
	Heart,
	KeyRound,
	Play,
	Settings as SettingsIcon,
	ShoppingCart,
	Star,
} from "lucide-react";
import { useIsMobile } from "../../hooks/use-mobile";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { formatAppCategory } from "../../lib/app-category";
import type { IAppVisibility } from "../../lib/schema/app/app";
import { IAppVisibility as AppVis } from "../../lib/schema/app/app";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Button } from "../ui/button";
import { ShareButton } from "../ui/share-button";
import { visibilityLabel } from "./visibility";

export function StoreHero({
	appId,
	hasThumbnail,
	coverUrl,
	iconUrl,
	appName,
	priceLabel,
	category,
	isMember,
	ratingCount,
	avgRating,
	visibility,
	authors,
	canUseApp,
	price,
	isPurchasing,
	onUse,
	onSettings,
	onBuy,
	onJoinOrRequest,
	actionsExtra,
}: Readonly<{
	appId: string;
	hasThumbnail: boolean;
	coverUrl: string;
	iconUrl: string;
	appName: string;
	priceLabel: string;
	category: string;
	isMember: boolean;
	ratingCount: number;
	avgRating: number;
	visibility: IAppVisibility;
	authors: string[];
	canUseApp: boolean;
	price: number;
	isPurchasing: boolean;
	onUse: () => void;
	onSettings: () => void;
	onBuy: () => void;
	onJoinOrRequest: () => Promise<void> | void;
	actionsExtra?: React.ReactNode;
}>) {
	const { primaryHue, isDark } = useThemeInfo();
	const isMobile = useIsMobile();
	const primarySize = isMobile ? "default" : "sm";

	return (
		<section className="relative">
			{/* Full-bleed cover */}
			<div className="relative h-60 md:h-75 overflow-hidden">
				<img
					src={coverUrl}
					alt=""
					className="h-full w-full object-cover"
					loading="eager"
					decoding="async"
					onError={(e) => {
						(e.currentTarget as HTMLImageElement).style.display = "none";
					}}
				/>
				{!hasThumbnail &&
					(() => {
						const g = hashToGradient(appId, primaryHue, isDark);
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
				<div className="absolute inset-x-0 bottom-0 h-3/4 bg-linear-to-t from-background via-background/60 to-transparent" />
			</div>

			{/* Identity — overlaps cover */}
			<div className="relative -mt-16 max-w-5xl mx-auto px-6 md:px-10">
				<div className="flex flex-col sm:flex-row sm:items-end gap-4">
					<div className="shrink-0 rounded-full bg-background/60 backdrop-blur-xl p-1 shadow-2xl">
						<Avatar className="h-22 w-22">
							<AvatarImage
								src={iconUrl}
								alt={appName}
								className="object-cover"
							/>
							<AvatarFallback className="text-xl font-bold">
								{appName.slice(0, 2).toUpperCase()}
							</AvatarFallback>
						</Avatar>
					</div>

					<div className="flex-1 min-w-0">
						<div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
							<h1 className="text-2xl md:text-3xl font-bold tracking-tight">
								{appName}
							</h1>
							{isMember && (
								<span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
									<Heart className="h-3 w-3" /> Yours
								</span>
							)}
						</div>
						<div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground/60">
							<span>{formatAppCategory(category)}</span>
							<span className="select-none">·</span>
							<span className="capitalize">{visibilityLabel(visibility)}</span>
							{authors?.length > 0 && (
								<>
									<span className="select-none">·</span>
									<span className="truncate max-w-50">
										{authors.join(", ")}
									</span>
								</>
							)}
							{ratingCount > 0 && (
								<>
									<span className="select-none">·</span>
									<span className="inline-flex items-center gap-0.5">
										<Star className="h-3 w-3 text-yellow-500 fill-yellow-500" />
										{avgRating.toFixed(1)}
									</span>
								</>
							)}
						</div>
					</div>

					<div className="flex w-full flex-col items-stretch gap-2 sm:w-auto sm:flex-row sm:items-center sm:shrink-0 sm:pb-1">
						{isMember ? (
							<>
								{canUseApp && (
									<Button
										size={primarySize}
										className="w-full sm:w-auto"
										onClick={onUse}
									>
										<Play className="h-3.5 w-3.5 mr-1.5" /> Use
									</Button>
								)}
								<Button
									size={primarySize}
									variant="outline"
									className="w-full sm:w-auto"
									onClick={onSettings}
								>
									<SettingsIcon className="h-3.5 w-3.5 mr-1.5" /> Settings
								</Button>
							</>
						) : price > 0 ? (
							<Button
								size={primarySize}
								className="w-full sm:w-auto"
								onClick={onBuy}
								disabled={isPurchasing}
							>
								{isPurchasing ? (
									"Processing..."
								) : (
									<>
										<ShoppingCart className="h-3.5 w-3.5 mr-1.5" /> {priceLabel}
									</>
								)}
							</Button>
						) : visibility === AppVis.Public ? (
							<Button
								size={primarySize}
								className="w-full sm:w-auto"
								onClick={onJoinOrRequest}
							>
								<Download className="h-3.5 w-3.5 mr-1.5" /> Get
							</Button>
						) : (
							<Button
								size={primarySize}
								className="w-full sm:w-auto"
								onClick={onJoinOrRequest}
							>
								<KeyRound className="h-3.5 w-3.5 mr-1.5" />
								{visibility === AppVis.PublicRequestAccess
									? "Request join"
									: "Request access"}
							</Button>
						)}
						<div className="flex items-center gap-2 sm:contents">
							{actionsExtra}
							<ShareButton
								appId={appId}
								appName={appName}
								variant="outline"
								size={primarySize}
								className="flex-1 sm:flex-none"
							/>
						</div>
					</div>
				</div>
			</div>
		</section>
	);
}
