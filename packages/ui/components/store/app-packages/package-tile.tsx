"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpCircle,
	Database,
	FolderOpen,
	Globe,
	KeyRound,
	type LucideIcon,
	MoreHorizontal,
	RefreshCw,
	Sparkles,
	Store,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useAssetImage } from "../../../hooks/use-asset-image";
import {
	ELEVATED_ACCESS_GROUPS,
	type ElevatedAccessGroup,
	type PackageAccess,
	groupAccessTags,
} from "../../../lib/app-package-overview";
import { usePackageCapabilities } from "../../../lib/package-capabilities";
import {
	type PackagePinState,
	formatPackagePrice,
	storePackageHref,
} from "../../../lib/package-license";
import type { AppPackage, PackageUpdate } from "../../../lib/schema/wasm";
import { cn } from "../../../lib/utils";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Skeleton } from "../../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { getPackageInitials } from "../package-card";
import { PackageStatusBadge, useAccessLabels } from "./parts";

const GROUP_ICONS: Record<ElevatedAccessGroup, LucideIcon> = {
	network: Globe,
	files: FolderOpen,
	accounts: KeyRound,
	models: Sparkles,
	data: Database,
};

export interface PackageTileActions {
	onToggleAutoUpdate: (autoUpdate: boolean) => void;
	onApplyUpdate: () => void;
	onReactivate: () => void;
	onRemove: () => void;
}

export interface PackageTilePending {
	autoUpdate: boolean;
	update: boolean;
	reactivate: boolean;
	remove: boolean;
}

export interface PackageTileProps {
	pkg: AppPackage;
	name: string;
	description?: string;
	pinState: PackagePinState;
	timeLeftLabel?: string;
	nodeCount: number;
	widgetCount: number;
	countsLoading: boolean;
	access: PackageAccess;
	update?: PackageUpdate;
	offline: boolean;
	canReactivate: boolean;
	pending: PackageTilePending;
	actions: PackageTileActions;
}

export function PackageTile({
	pkg,
	name,
	description,
	pinState,
	timeLeftLabel,
	nodeCount,
	widgetCount,
	countsLoading,
	access,
	update,
	offline,
	canReactivate,
	pending,
	actions,
}: Readonly<PackageTileProps>) {
	const { t } = useTranslation("store");
	const locked = pkg.stale || pinState !== "active";
	const muted = pinState === "stale" || pinState === "expired";
	const updatable = !!update && !locked && !offline;
	const price = pkg.price ?? 0;

	return (
		<article
			className={cn(
				"flex min-w-0 flex-col overflow-hidden rounded-xl border bg-card shadow-xs",
				muted && "border-dashed bg-card/60",
			)}
		>
			<TileCover pkg={pkg} name={name} muted={muted}>
				<PackageStatusBadge
					pinState={pinState}
					timeLeftLabel={timeLeftLabel}
					latestVersion={updatable ? update?.latestVersion : undefined}
				/>
			</TileCover>
			<div className="flex flex-1 flex-col gap-2 px-4 pt-8 pb-4">
				<div className="flex items-start gap-2">
					<div className="min-w-0 flex-1">
						<h3
							className={cn(
								"truncate text-[15px] font-semibold leading-tight",
								muted && "text-muted-foreground",
							)}
							title={name}
						>
							{name}
						</h3>
						<div className="mt-1 flex flex-wrap items-center gap-1.5">
							<span className="font-mono text-xs text-muted-foreground">{`v${pkg.version}`}</span>
							{price > 0 && (
								<Badge variant="outline" className="h-5 px-1.5 text-[10px]">
									{t("paidPrice", "Paid · {{price}}", {
										price: formatPackagePrice(price),
									})}
								</Badge>
							)}
						</div>
					</div>
					<PackageTileMenu
						pkg={pkg}
						name={name}
						update={updatable ? update : undefined}
						locked={locked}
						offline={offline}
						canReactivate={canReactivate}
						pending={pending}
						actions={actions}
					/>
				</div>
				<p className="line-clamp-3 text-sm leading-relaxed text-muted-foreground">
					{pinState === "stale"
						? t(
								"appPackagesStaleHint",
								"The member who added it left the project. It can't update or go on new boards until an admin reactivates it.",
							)
						: (description ??
							t("appPackagesNoDescription", "No description provided."))}
				</p>
				<div className="mt-auto flex items-center justify-between gap-2 border-t pt-3">
					{countsLoading ? (
						<Skeleton className="h-4 w-24" />
					) : (
						<span className="text-xs text-muted-foreground">
							{t("nodeCount", {
								defaultValue_one: "{{count}} node",
								defaultValue_other: "{{count}} nodes",
								count: nodeCount,
							})}
							{widgetCount > 0 && (
								<>
									{" · "}
									<span className="font-medium text-primary">
										{t("appPackagesWidgetCount", {
											defaultValue_one: "{{count}} widget",
											defaultValue_other: "{{count}} widgets",
											count: widgetCount,
										})}
									</span>
								</>
							)}
						</span>
					)}
					{pinState === "stale" && !offline ? (
						<Button
							size="sm"
							variant="outline"
							className="h-7 gap-1.5 px-2 text-xs"
							onClick={actions.onReactivate}
							disabled={pending.reactivate || !canReactivate}
						>
							<RefreshCw className="size-3" />
							{t("reactivate", "Reactivate")}
						</Button>
					) : (
						<TileAccessIcons access={access} />
					)}
				</div>
			</div>
		</article>
	);
}

function TileCover({
	pkg,
	name,
	muted,
	children,
}: Readonly<{
	pkg: AppPackage;
	name: string;
	muted: boolean;
	children: React.ReactNode;
}>) {
	const icon = useAssetImage(pkg.metadata?.icon);
	const thumbnail = useAssetImage(pkg.metadata?.thumbnail);

	return (
		<div
			className={cn(
				"relative h-20 shrink-0 border-b bg-muted/40",
				muted && "border-dashed",
			)}
		>
			{thumbnail.canRender ? (
				<img
					ref={thumbnail.imgRef}
					src={thumbnail.src}
					onLoad={thumbnail.onLoad}
					onError={thumbnail.onError}
					alt=""
					className={cn(
						"absolute inset-0 h-full w-full object-cover",
						muted && "opacity-50 grayscale",
					)}
				/>
			) : (
				<div
					aria-hidden="true"
					className="absolute inset-0 bg-[radial-gradient(var(--border)_1px,transparent_1px)] bg-size-[14px_14px]"
				/>
			)}
			<div className="absolute top-3 right-3">{children}</div>
			<div
				className={cn(
					"absolute -bottom-5 left-4 flex size-11 items-center justify-center overflow-hidden rounded-lg border bg-card shadow-sm",
					muted && "border-dashed",
				)}
			>
				{icon.canRender ? (
					<img
						ref={icon.imgRef}
						src={icon.src}
						onLoad={icon.onLoad}
						onError={icon.onError}
						alt=""
						className={cn("h-full w-full object-cover", muted && "grayscale")}
					/>
				) : (
					<span
						aria-hidden="true"
						className="font-mono text-sm font-bold text-muted-foreground"
					>
						{getPackageInitials(name)}
					</span>
				)}
			</div>
		</div>
	);
}

function TileAccessIcons({ access }: Readonly<{ access: PackageAccess }>) {
	const { t } = useTranslation("store");
	const labels = useAccessLabels();
	const capabilities = usePackageCapabilities(access.tags);
	const groups = useMemo(() => groupAccessTags(access.tags), [access.tags]);
	const elevated = ELEVATED_ACCESS_GROUPS.filter(
		(group) => groups[group].length > 0,
	);

	if (elevated.length === 0) return null;

	return (
		<div className="flex items-center gap-2">
			{elevated.map((group) => {
				const Icon = GROUP_ICONS[group];
				const lines = capabilities
					.filter((capability) => groups[group].includes(capability.key))
					.map((capability) => capability.label);
				const hosts =
					group === "network" && groups.network.includes("net.http")
						? access.hosts.length
							? access.hosts.join(", ")
							: t("appPackagesAnyHost", "Any host")
						: undefined;
				const label = [...lines, hosts].filter(Boolean).join(" · ");
				return (
					<Tooltip key={group}>
						<TooltipTrigger asChild>
							<button
								type="button"
								aria-label={label}
								className="flex rounded-sm text-primary outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<Icon className="size-4" aria-hidden="true" />
							</button>
						</TooltipTrigger>
						<TooltipContent className="max-w-64">
							{lines.map((line) => (
								<p key={line}>{line}</p>
							))}
							{hosts && <p className="font-mono text-xs">{hosts}</p>}
							{group === "files" && (
								<p className="text-xs opacity-80">
									{groups.files.map(labels.tag).join(", ")}
								</p>
							)}
						</TooltipContent>
					</Tooltip>
				);
			})}
		</div>
	);
}

function PackageTileMenu({
	pkg,
	name,
	update,
	locked,
	offline,
	canReactivate,
	pending,
	actions,
}: Readonly<{
	pkg: AppPackage;
	name: string;
	update?: PackageUpdate;
	locked: boolean;
	offline: boolean;
	canReactivate: boolean;
	pending: PackageTilePending;
	actions: PackageTileActions;
}>) {
	const { t } = useTranslation("store");
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className="-mt-1 -mr-2 size-8 shrink-0"
					aria-label={t("appPackagesActionsFor", "Actions for {{name}}", {
						name,
					})}
				>
					<MoreHorizontal className="size-4" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-56">
				{update && (
					<DropdownMenuItem
						onSelect={actions.onApplyUpdate}
						disabled={pending.update}
					>
						<ArrowUpCircle />
						{t("appPackagesUpdateAvailableTo", "Update to {{version}}", {
							version: update.latestVersion,
						})}
					</DropdownMenuItem>
				)}
				{!offline && (
					<DropdownMenuCheckboxItem
						checked={pkg.autoUpdate}
						onCheckedChange={actions.onToggleAutoUpdate}
						disabled={pending.autoUpdate || locked}
					>
						{t("autoupdate", "Auto-update")}
					</DropdownMenuCheckboxItem>
				)}
				{locked && !offline && (
					<DropdownMenuItem
						onSelect={actions.onReactivate}
						disabled={pending.reactivate || !canReactivate}
					>
						<RefreshCw />
						{t("reactivate", "Reactivate")}
					</DropdownMenuItem>
				)}
				{!offline && (
					<DropdownMenuItem asChild>
						<Link href={storePackageHref(pkg.packageId)}>
							<Store />
							{t("appPackagesViewInStore", "View in store")}
						</Link>
					</DropdownMenuItem>
				)}
				<DropdownMenuSeparator />
				<DropdownMenuItem
					variant="destructive"
					onSelect={actions.onRemove}
					disabled={pending.remove}
				>
					<Trash2 />
					{t("appPackagesRemoveFromApp", "Remove from app")}
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
