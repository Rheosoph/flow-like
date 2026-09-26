"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BadgeCheck,
	Check,
	Cpu,
	Download,
	Loader2,
	Plus,
	ShoppingBag,
} from "lucide-react";
import Link from "next/link";
import { memo, useMemo } from "react";
import { usePackageCapabilities } from "../../lib/package-capabilities";
import {
	formatPackagePrice,
	storePackageHref,
} from "../../lib/package-license";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	type ShelfAccessFacet,
	type ShelfPackage,
	type ShelfPin,
	memoryLimitLabel,
	packageInitials,
	shelfPins,
} from "./package-shelf-model";

export type ShelfCardState =
	| "idle"
	| "adding"
	| "removing"
	| "added"
	| "inProject";

interface ShelfCardProps {
	pkg: ShelfPackage;
	state: ShelfCardState;
	canRemove: boolean;
	onAdd: (pkg: ShelfPackage) => void;
	onRemove: (pkg: ShelfPackage) => void;
	onGetFirst: () => void;
}

export const ShelfCard = memo(function ShelfCard({
	pkg,
	state,
	canRemove,
	onAdd,
	onRemove,
	onGetFirst,
}: Readonly<ShelfCardProps>) {
	const { t } = useTranslation("store");
	const pins = useMemo(() => shelfPins(pkg), [pkg]);
	const pinLabel = usePinLabel();
	const accessSummary = pins.map(pinLabel).join(", ");
	const needsHolder =
		pkg.license === "needed" && state !== "inProject" && state !== "added";
	const author = pkg.authors.join(", ");

	return (
		<article
			aria-label={pkg.name}
			className={cn(
				"relative flex flex-col overflow-hidden rounded-xl border bg-card transition-colors",
				state === "added" ? "border-primary/50" : "border-border/70",
			)}
		>
			<NodeCover pkg={pkg} pins={pins} />
			<div className="flex flex-1 flex-col gap-1 px-4 pt-3">
				<div className="flex min-w-0 items-center gap-2">
					<h3 className="truncate text-sm font-semibold">{pkg.name}</h3>
					<span className="shrink-0 font-mono text-[11px] text-muted-foreground">
						{pkg.version}
					</span>
					{pkg.verified && (
						<BadgeCheck
							className="size-3.5 shrink-0 text-primary"
							aria-label={t("verified", "Verified")}
						/>
					)}
				</div>
				{author && (
					<p className="truncate text-xs text-muted-foreground">
						{t("packageShelfByAuthor", "by {{author}}", { author })}
					</p>
				)}
				<p
					className={cn(
						"mt-1 line-clamp-3 text-xs leading-relaxed text-muted-foreground",
						!pkg.description && "italic opacity-70",
					)}
				>
					{pkg.description ||
						t("packageShelfNoDescription", "No description in manifest")}
				</p>
				{accessSummary && <p className="sr-only">{accessSummary}</p>}
				{needsHolder && (
					<p className="mt-1 text-[11px] leading-snug text-muted-foreground">
						{t(
							"packageNeedsHolderToAdd",
							"An admin or the owner needs this package before adding it to a project.",
						)}
					</p>
				)}
			</div>
			<div className="flex items-center gap-2 px-4 pt-3 pb-3.5">
				<CardMeta pkg={pkg} />
				<span className="flex-1" />
				{needsHolder ? (
					<Button asChild size="sm" variant="outline">
						<Link href={storePackageHref(pkg.id)} onClick={onGetFirst}>
							<ShoppingBag />
							{t("getFirst", "Get first")}
						</Link>
					</Button>
				) : (
					<ShelfCardAction
						pkg={pkg}
						state={state}
						canRemove={canRemove}
						onAdd={onAdd}
						onRemove={onRemove}
					/>
				)}
			</div>
		</article>
	);
});

function CardMeta({ pkg }: Readonly<{ pkg: ShelfPackage }>) {
	const { t } = useTranslation("store");
	const memory = memoryLimitLabel(pkg.memory);
	return (
		<div className="flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground">
			{memory && (
				<span
					className="inline-flex items-center gap-1"
					title={t("packageShelfMemoryLimit", "Memory limit")}
				>
					<Cpu className="size-3.5" />
					{memory}
				</span>
			)}
			{pkg.downloadCount !== undefined && (
				<span className="inline-flex items-center gap-1">
					<Download className="size-3.5" />
					{t("downloadCount", "{{formattedCount}} downloads", {
						count: pkg.downloadCount,
						formattedCount: pkg.downloadCount.toLocaleString(),
					})}
				</span>
			)}
			{pkg.price > 0 && (
				<Badge variant="outline" className="text-[11px]">
					{formatPackagePrice(pkg.price)}
				</Badge>
			)}
			{pkg.license === "owned" && (
				<Badge variant="secondary" className="text-[11px]">
					{t("owned", "Owned")}
				</Badge>
			)}
		</div>
	);
}

function ShelfCardAction({
	pkg,
	state,
	canRemove,
	onAdd,
	onRemove,
}: Readonly<Omit<ShelfCardProps, "onGetFirst">>) {
	const { t } = useTranslation("store");

	if (state === "inProject")
		return (
			<Button size="sm" variant="ghost" disabled>
				<Check />
				{t("packageShelfInProject", "In project")}
			</Button>
		);

	if (state === "adding" || state === "removing")
		return (
			<Button size="sm" variant="outline" disabled>
				<Loader2 className="animate-spin" />
				{state === "adding"
					? t("packageShelfAdding", "Adding…")
					: t("packageShelfRemoving", "Removing…")}
			</Button>
		);

	if (state === "added")
		return (
			<Button
				size="sm"
				variant="outline"
				disabled={!canRemove}
				onClick={() => onRemove(pkg)}
				title={
					canRemove
						? t("packageShelfAddedUndoHint", "Added. Click to remove it again.")
						: undefined
				}
				className="border-primary/50 bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary disabled:opacity-100"
			>
				<Check />
				{t("packageShelfAdded", "Added")}
			</Button>
		);

	return (
		<Button
			size="sm"
			variant="outline"
			onClick={() => onAdd(pkg)}
			title={t("packageShelfAddTitle", "Add {{name}} to this project", {
				name: pkg.name,
			})}
		>
			<Plus />
			{t("packageShelfAdd", "Add")}
		</Button>
	);
}

export function useShelfFacetLabel() {
	const { t } = useTranslation("store");
	return useMemo(() => {
		const labels: Record<ShelfAccessFacet, string> = {
			network: t("packageShelfFacetNetwork", "Network"),
			files: t("packageShelfFacetFiles", "Files"),
			accounts: t("packageShelfFacetAccounts", "Accounts (OAuth)"),
			models: t("packageShelfFacetModels", "Language models"),
			data: t("packageShelfFacetData", "Databases"),
			widgets: t("packageShelfFacetWidgets", "Widgets"),
		};
		return (facet: ShelfAccessFacet) => labels[facet];
	}, [t]);
}

function usePinLabel() {
	const { t } = useTranslation("store");
	const facetLabel = useShelfFacetLabel();
	return (pin: ShelfPin) => {
		if (pin.facet === "more")
			return t("packageShelfPinMore", "+{{count}} more", {
				count: pin.count ?? 0,
			});
		if (pin.facet === "widgets" && pin.count)
			return t("packageShelfPinWidgets", "{{count}} widgets", {
				count: pin.count,
			});
		if (pin.host)
			return t("packageShelfPinHost", "Network · {{host}}", {
				host: pin.host,
			});
		return facetLabel(pin.facet);
	};
}

function NodeCover({
	pkg,
	pins,
}: Readonly<{ pkg: ShelfPackage; pins: ShelfPin[] }>) {
	const pinLabel = usePinLabel();
	const elevated = pins.some((pin) => pin.elevated);
	return (
		<div
			aria-hidden="true"
			className="flex h-28 shrink-0 items-center border-b border-border/60 bg-muted/30 bg-[radial-gradient(var(--border)_1.2px,transparent_1.3px)] pl-6 [background-size:16px_16px]"
		>
			<div className="w-44 rounded-lg border border-border bg-card shadow-md">
				<div className="flex items-center gap-1.5 border-b border-border/70 px-2 py-1.5">
					<span className="-ml-[13px] size-2 shrink-0 rounded-full border-[1.5px] border-muted-foreground bg-card" />
					<span
						className={cn(
							"flex size-[18px] shrink-0 items-center justify-center rounded text-[9px] font-bold",
							elevated
								? "bg-primary/15 text-primary"
								: "bg-muted text-foreground/80",
						)}
					>
						{packageInitials(pkg.name)}
					</span>
					<span className="truncate text-[11px] font-semibold">{pkg.name}</span>
				</div>
				<div className="flex flex-col gap-1.5 py-2">
					{pins.length === 0 ? (
						<CoverPin elevated={false} />
					) : (
						pins.map((pin) => (
							<CoverPin
								key={pin.facet}
								label={pinLabel(pin)}
								tags={pin.tags}
								elevated={pin.elevated}
							/>
						))
					)}
				</div>
			</div>
		</div>
	);
}

function CoverPin({
	label,
	tags,
	elevated,
}: Readonly<{ label?: string; tags?: string[]; elevated: boolean }>) {
	const capabilities = usePackageCapabilities(tags);
	const title = capabilities.map((c) => c.label).join(", ") || undefined;
	return (
		<div className="flex h-3 items-center justify-end gap-1.5" title={title}>
			{label && (
				<span className="truncate text-[10px] text-muted-foreground">
					{label}
				</span>
			)}
			<span
				className={cn(
					"-mr-[5px] size-[9px] shrink-0 rounded-full border-[1.5px]",
					elevated
						? "border-primary bg-primary/35"
						: "border-muted-foreground bg-card",
				)}
			/>
		</div>
	);
}
