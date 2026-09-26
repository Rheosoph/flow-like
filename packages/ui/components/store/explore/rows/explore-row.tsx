"use client";

import { useTranslation } from "@flow-like/locales";
import { collectionHref } from "../explore-item-card";
import { layoutTopPaid, layoutTrending } from "../explore-model";
import type { ExploreRow } from "../explore-types";
import { BuildersRow } from "./builders-row";
import { CollectionRow } from "./collection-row";
import { NewRow } from "./new-row";
import type { RowContext } from "./row-section";
import { SuitesRow } from "./suites-row";
import { TopPaidRow } from "./top-paid-row";
import { TrendingRow } from "./trending-row";

/**
 * Whether a row shows hub content. The suites rail loads its own groups and hides itself without any, so it never
 * counts: a hub whose only row is suites is still an empty hub.
 */
export function exploreRowHasContent(
	row: ExploreRow,
	context: RowContext,
): boolean {
	if (row.kind === "collection") return row.collection.items.length > 0;
	switch (row.rail.rail) {
		case "trending":
			return layoutTrending(row.rail.items, context.dev).length > 0;
		case "top_paid":
			return layoutTopPaid(row.rail.items, context.dev).length > 0;
		case "for_builders":
		case "new":
			return row.rail.items.length > 0;
		default:
			return false;
	}
}

/** One landing row, by kind and rail key. */
export function ExploreRowView({
	row,
	context,
}: Readonly<{ row: ExploreRow; context: RowContext }>) {
	const { t } = useTranslation("store");
	if (row.kind === "collection") {
		return (
			<CollectionRow
				collection={row.collection}
				action={{
					label: t("seeAll", "See all"),
					href: collectionHref(row.collection.placementId),
				}}
			/>
		);
	}
	switch (row.rail.rail) {
		case "trending":
			return <TrendingRow rail={row.rail} context={context} />;
		case "top_paid":
			return <TopPaidRow rail={row.rail} context={context} />;
		case "for_builders":
			return <BuildersRow rail={row.rail} context={context} />;
		case "new":
			return <NewRow rail={row.rail} context={context} />;
		case "suites":
			return <SuitesRow />;
		default:
			return null;
	}
}
