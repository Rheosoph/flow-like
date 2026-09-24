"use client";

import { cn } from "../../../../lib/utils";
import { ExploreItemCard, itemId } from "../explore-item-card";
import type { ExploreRail } from "../explore-types";
import {
	ExploreRowSection,
	ROW_ITEM,
	ROW_SCROLLER,
	type RowContext,
	useRailHeading,
} from "./row-section";

const MAX_ITEMS = 4;

/** "New this week": also the fallback of the builders row for non-dev viewers and the Apps filter. */
export function NewRow({
	rail,
	context,
}: Readonly<{ rail: ExploreRail; context: RowContext }>) {
	const heading = useRailHeading(rail, context.mix);
	const items = rail.items.slice(0, MAX_ITEMS);
	if (!items.length) return null;
	return (
		<ExploreRowSection {...heading}>
			<div className={cn(ROW_SCROLLER, "@5xl/explore:grid-cols-4")}>
				{items.map((item) => (
					<div key={`${item.kind}:${itemId(item)}`} className={ROW_ITEM}>
						<ExploreItemCard item={item} />
					</div>
				))}
			</div>
		</ExploreRowSection>
	);
}
