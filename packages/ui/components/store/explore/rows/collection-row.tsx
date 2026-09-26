"use client";

import { cn } from "../../../../lib/utils";
import { ExploreItemCard, itemId } from "../explore-item-card";
import type { ExploreCollection } from "../explore-types";
import { ExploreRowSection, ROW_ITEM, ROW_SCROLLER } from "./row-section";

const MAX_ITEMS = 4;

/** A curated collection as a mixed rail: app cards and package cards on one baseline. */
export function CollectionRow({
	collection,
	action,
}: Readonly<{
	collection: ExploreCollection;
	action?: { label: string; href: string };
}>) {
	const items = collection.items.slice(0, MAX_ITEMS);
	if (!items.length) return null;
	return (
		<ExploreRowSection
			title={collection.title}
			subtitle={collection.blurb}
			action={action}
		>
			<div
				className={cn(
					ROW_SCROLLER,
					"items-stretch @5xl/explore:grid-cols-4 @5xl/explore:auto-rows-[minmax(24rem,auto)]",
				)}
			>
				{items.map((item) => (
					<div key={`${item.kind}:${itemId(item)}`} className={ROW_ITEM}>
						<ExploreItemCard item={item} />
					</div>
				))}
			</div>
		</ExploreRowSection>
	);
}
