"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpRight, KeyRound, Lock, Shield, Star } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useAssetImage } from "../../hooks/use-asset-image";
import { hashToGradient, useThemeInfo } from "../../hooks/use-theme-gradient";
import { usePackageCapabilities } from "../../lib/package-capabilities";
import type { PackageSummary } from "../../lib/schema/wasm";
import { cn } from "../../lib/utils";

function getPackageInitials(name: string): string {
	const words = name
		.replace(/[()]/g, " ")
		.split(/\s+/)
		.filter((word) => {
			const normalized = word.toLowerCase();
			return normalized !== "custom" && normalized !== "node";
		});
	if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
	const initials = (words.length ? words : [name])
		.slice(0, 2)
		.map((word) => word[0]?.toUpperCase() ?? "")
		.join("");

	return initials || "PK";
}

function formatCompact(n: number): string {
	if (n >= 1_000_000)
		return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
	return `${n}`;
}

function prettyCategory(category: string): string {
	return category
		.toLowerCase()
		.split("_")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(" ");
}

const MAX_VISIBLE_CAPABILITIES = 3;

export type PackageCardVariant = "standard" | "compact" | "featured";

export function PackageCard({
	pkg,
	variant = "standard",
	className,
}: { pkg: PackageSummary; variant?: PackageCardVariant; className?: string }) {
	const { t } = useTranslation("store");
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.name;
	const displayDesc = pkg.metadata?.description ?? pkg.description;
	const icon = useAssetImage(pkg.metadata?.icon);
	const thumbnail = useAssetImage(pkg.metadata?.thumbnail);
	const rated = (pkg.ratingCount ?? 0) > 0;
	const category = pkg.primaryCategory ?? pkg.secondaryCategory;
	const capabilities = usePackageCapabilities(pkg.capabilities);
	const visibleCapabilities = capabilities.slice(0, MAX_VISIBLE_CAPABILITIES);
	const hiddenCapabilityCount =
		capabilities.length - visibleCapabilities.length;
	const featured = variant === "featured";
	const compact = variant === "compact";

	return (
		<Link
			href={`/store/packages?id=${encodeURIComponent(pkg.id)}`}
			data-package-card={variant}
			className={cn(
				"group relative flex h-full min-w-0 w-full flex-col overflow-hidden rounded-xl border border-border/60 bg-card p-2.5 shadow-sm transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-lg outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
				featured && "rounded-2xl p-3.5",
				compact && "p-4",
				className,
			)}
		>
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--border)_0.5px,transparent_0.5px)] bg-size-[7px_7px] opacity-50"
			/>

			{!compact && (
				<div
					className={cn(
						"relative aspect-video w-full shrink-0 overflow-hidden rounded-lg border border-border/60 bg-muted",
						featured && "aspect-[16/9] rounded-xl",
					)}
				>
					{thumbnail.canRender ? (
						<img
							ref={thumbnail.imgRef}
							src={thumbnail.src}
							onLoad={thumbnail.onLoad}
							onError={thumbnail.onError}
							alt=""
							className="absolute inset-0 h-full w-full object-cover"
						/>
					) : (
						<>
							<div
								className="absolute inset-0"
								style={{
									background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
								}}
							/>
							{icon.canRender ? (
								<img
									src={icon.src}
									alt=""
									aria-hidden="true"
									className="absolute left-1/2 top-1/2 h-[150%] w-[150%] -translate-x-1/2 -translate-y-1/2 object-contain opacity-40 blur-2xl saturate-150"
								/>
							) : (
								<span
									className={cn(
										"absolute inset-0 flex items-center justify-center font-mono text-2xl font-bold text-white/50",
										featured && "text-6xl tracking-tighter text-white/65",
									)}
								>
									{getPackageInitials(displayName)}
								</span>
							)}
						</>
					)}
					<span className="absolute bottom-1.5 left-1.5 rounded-md border border-white/20 bg-black/50 px-1.5 py-0.5 font-mono text-[10px] leading-none text-white/90 backdrop-blur-sm">
						{`v${pkg.latestVersion}`}
					</span>
					{featured && (
						<span
							aria-hidden="true"
							className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-full border border-white/25 bg-black/15 text-white backdrop-blur-sm transition-transform group-hover:-rotate-12"
						>
							<ArrowUpRight className="size-4" />
						</span>
					)}
				</div>
			)}

			<div
				className={cn(
					"relative mt-2.5 flex items-center gap-2",
					featured && "mt-4 gap-3",
					compact && "mt-0 gap-3",
				)}
			>
				<div
					className={cn(
						"h-9 w-9 shrink-0 overflow-hidden rounded-lg border border-border/60 bg-muted",
						(featured || compact) && "size-11 rounded-xl",
					)}
				>
					{icon.canRender ? (
						<img
							ref={icon.imgRef}
							src={icon.src}
							onLoad={icon.onLoad}
							onError={icon.onError}
							alt=""
							className="h-full w-full object-cover"
						/>
					) : (
						<div className="flex h-full w-full items-center justify-center font-mono text-[10px] font-semibold text-muted-foreground">
							{getPackageInitials(displayName)}
						</div>
					)}
				</div>
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-1.5">
						<h3
							className={cn(
								"truncate font-mono text-[13px] font-semibold",
								featured && "font-sans text-base font-bold tracking-tight",
								compact && "font-sans text-sm",
							)}
						>
							{displayName}
						</h3>
						{pkg.verified && (
							<Shield
								aria-label={t("verified", "Verified")}
								className="h-3.5 w-3.5 shrink-0 text-sky-500 dark:text-sky-400"
							/>
						)}
						{pkg.visibility !== "public" && (
							<span
								className="shrink-0 rounded-md border border-border/60 p-1 text-muted-foreground"
								title={pkg.visibility}
							>
								{pkg.visibility === "private" ? (
									<Lock className="h-3 w-3" />
								) : (
									<KeyRound className="h-3 w-3" />
								)}
							</span>
						)}
					</div>
					{category && (
						<div className="truncate font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
							{prettyCategory(category)}
						</div>
					)}
					{compact && (
						<span className="mt-0.5 block font-mono text-[10px] text-muted-foreground">
							v{pkg.latestVersion}
						</span>
					)}
				</div>
			</div>

			<p
				className={cn(
					"relative mt-2 line-clamp-2 text-xs leading-relaxed text-muted-foreground",
					featured && "mt-3 text-[13px]",
				)}
			>
				{displayDesc}
			</p>

			{/* Capability labels come from the package manifest. */}
			{pkg.capabilities && (
				<div className="relative mb-2.5 mt-2.5 flex flex-wrap gap-1">
					{visibleCapabilities.map((capability) => (
						<span
							key={capability.key}
							title={capability.label}
							className={
								capability.severity === "elevated"
									? "rounded border border-primary/35 bg-primary/10 px-1.5 py-1 font-mono text-[10px] leading-none text-primary"
									: "rounded border border-border/60 bg-muted/40 px-1.5 py-1 font-mono text-[10px] leading-none text-muted-foreground"
							}
						>
							{capability.key}
						</span>
					))}
					{hiddenCapabilityCount > 0 && (
						<span
							title={capabilities.map((c) => c.label).join("\n")}
							className="rounded border border-border/60 bg-muted/40 px-1.5 py-1 font-mono text-[10px] leading-none text-muted-foreground"
						>
							{`+${hiddenCapabilityCount}`}
						</span>
					)}
					{capabilities.length === 0 && (
						<span className="rounded border border-dashed border-border/60 px-1.5 py-1 font-mono text-[10px] leading-none text-muted-foreground">
							{t("noPermissionsRequested", "no permissions requested")}
						</span>
					)}
				</div>
			)}

			<div className="relative mt-auto grid grid-cols-3 divide-x divide-border/60 border-t border-border/60 pt-2.5">
				<div className="pr-2.5">
					<div className="font-mono text-[13px] font-semibold tabular-nums">
						{formatCompact(pkg.downloadCount)}
					</div>
					<div className="mt-0.5 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
						{t("installs", "Installs")}
					</div>
				</div>
				<div className="px-2.5">
					<div className="flex items-center gap-1 font-mono text-[13px] font-semibold tabular-nums">
						{rated ? (
							<>
								<Star className="h-3 w-3 fill-yellow-500 text-yellow-500 dark:fill-yellow-400 dark:text-yellow-400" />
								{(pkg.avgRating ?? 0).toFixed(1)}
							</>
						) : (
							t("new", "New")
						)}
					</div>
					<div className="mt-0.5 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
						{t("rating", "Rating")}
					</div>
				</div>
				<div className="pl-2.5">
					<div
						className={`font-mono text-[13px] font-semibold ${pkg.price > 0 ? "text-primary" : ""}`}
					>
						{pkg.price > 0
							? `€${(pkg.price / 100).toFixed(2)}`
							: t("free", "Free")}
					</div>
					<div className="mt-0.5 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
						{t("price", "Price")}
					</div>
				</div>
			</div>
			{featured && (
				<span className="relative mt-4 flex items-center justify-between border-t border-border/60 pt-3 text-xs font-semibold text-primary">
					{t("explorePackage", "Explore package")}
					<ArrowUpRight className="size-4 transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
				</span>
			)}
		</Link>
	);
}
