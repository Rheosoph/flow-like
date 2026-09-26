"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight, Layers } from "lucide-react";
import Link from "next/link";
import { cn } from "../../../../lib/utils";
import {
	CollectionStack,
	ItemChip,
	collectionHref,
	itemId,
} from "../explore-item-card";
import { useExploreLabels } from "../explore-labels";
import type { ExploreCollection } from "../explore-types";

const MAX_CHIPS = 3;

/** Grid tile for a curated collection; "Open collection" opens its Browse page. */
export function CollectionTile({
	collection,
	className,
}: Readonly<{ collection: ExploreCollection; className?: string }>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const chips = collection.items.slice(0, MAX_CHIPS);
	const hidden = collection.items.length - chips.length;
	return (
		<article
			aria-label={t("exploreCollectionNamed", {
				defaultValue: "Collection: {{title}}",
				title: collection.title,
			})}
			data-explore-tile="collection"
			className={cn(
				"group relative flex h-full min-w-0 gap-5.5 overflow-hidden rounded-2xl border border-border/70 bg-card p-4 pr-5 transition-colors hover:border-primary/40",
				className,
			)}
		>
			<CollectionStack
				items={collection.items}
				className="hidden w-57 @xl/explore:block"
			/>
			<div className="flex min-w-0 flex-1 flex-col">
				<span className="flex min-w-0 items-center gap-1.75 text-[11px] font-semibold uppercase leading-3.5 tracking-wider text-muted-foreground">
					<Layers aria-hidden="true" className="size-3.25 text-foreground" />
					{t("exploreCollection", "Collection")}
					<span className="truncate font-medium normal-case tracking-normal">
						{collection.source === "rule"
							? t("exploreCollectionRule", "· updated automatically")
							: t("exploreCollectionHandPicked", "· hand-picked")}
					</span>
				</span>
				<h3 className="mt-2 line-clamp-2 text-xl font-semibold leading-6.5 tracking-tight">
					{collection.title}
				</h3>
				{collection.blurb && (
					<p className="mt-0.75 truncate text-[13px] leading-4.75 text-muted-foreground">
						{collection.blurb}
					</p>
				)}
				<div className="mt-3 flex min-w-0 flex-wrap gap-1.5 overflow-hidden">
					{chips.map((item) => (
						<ItemChip key={`${item.kind}:${itemId(item)}`} item={item} />
					))}
					{hidden > 0 && (
						<span className="inline-flex h-6 items-center rounded-md border border-border/70 px-2 font-mono text-[11px] text-muted-foreground">
							{`+${hidden}`}
						</span>
					)}
				</div>
				<div className="mt-auto flex items-center justify-between gap-3 pt-3">
					<span className="font-mono text-xs tabular-nums text-muted-foreground">
						{labels.kindCounts(collection.apps, collection.packages)}
					</span>
					<Link
						href={collectionHref(collection.placementId)}
						className="inline-flex shrink-0 items-center gap-1.5 rounded-sm text-[13px] font-semibold text-primary after:absolute after:inset-0 after:content-[''] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						{t("exploreOpenCollection", "Open collection")}
						<ArrowRight aria-hidden="true" className="size-3.5" />
					</Link>
				</div>
			</div>
		</article>
	);
}
