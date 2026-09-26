"use client";

import { useTranslation } from "@flow-like/locales";
import type { CSSProperties, ReactNode, Ref } from "react";
import {
	type BentoArea,
	type BentoTilePlacement,
	bentoPlacement,
	bentoRows,
} from "./explore-model";
import {
	type ExploreGrid,
	type ExploreGridSlotKey,
	type ExploreTypeFilter,
	GRID_SLOT_KEYS,
} from "./explore-types";
import { AnnouncementTile } from "./tiles/announcement-tile";
import { CategoryTiles } from "./tiles/category-tiles";
import { CollectionTile } from "./tiles/collection-tile";
import { FeatureTile } from "./tiles/feature-tile";
import { SpotlightTile } from "./tiles/spotlight-tile";
import { StatTile } from "./tiles/stat-tile";

function isPresent(grid: ExploreGrid, slot: ExploreGridSlotKey): boolean {
	switch (slot) {
		case "hero":
			return (grid.hero?.slides.length ?? 0) > 0;
		case "stat":
			return !!grid.stat?.stat;
		case "categories":
			return (grid.categories?.categories?.length ?? 0) > 0;
		default:
			return !!grid[slot];
	}
}

/** The grid slots a view resolved, minus a dismissed announcement. */
export function presentSlots(
	grid: ExploreGrid,
	noticeDismissed: boolean,
): ExploreGridSlotKey[] {
	return GRID_SLOT_KEYS.filter(
		(slot) => isPresent(grid, slot) && !(slot === "notice" && noticeDismissed),
	);
}

function track(area: BentoArea): { column: string; row: string } {
	return {
		column: `${area.col} / span ${area.span}`,
		row: `${area.row} / span ${area.rowSpan}`,
	};
}

function areaStyle(placement: BentoTilePlacement): CSSProperties {
	const xl = track(placement.xl);
	const md = track(placement.md);
	return {
		"--xl-col": xl.column,
		"--xl-row": xl.row,
		"--md-col": md.column,
		"--md-row": md.row,
	} as CSSProperties;
}

function heights(rows: readonly number[]): string {
	return rows.map((height) => `${height}px`).join(" ");
}

export interface BentoGridProps {
	grid: ExploreGrid;
	type: ExploreTypeFilter;
	noticeDismissed: boolean;
	onDismissNotice: () => void;
	dismissNoticeRef?: Ref<HTMLButtonElement>;
	onCoverChange?: (cover: string | undefined) => void;
}

/**
 * 12 columns at xl, 6 at md, one stacked column below; areas come from the §2.3 collapse tables, so a missing
 * tile never leaves a hole.
 */
export function BentoGrid({
	grid,
	type,
	noticeDismissed,
	onDismissNotice,
	dismissNoticeRef,
	onCoverChange,
}: Readonly<BentoGridProps>) {
	const { t } = useTranslation("store");
	const present = presentSlots(grid, noticeDismissed);
	if (!present.length) return null;
	const placement = bentoPlacement(present);
	const rows = bentoRows(present);

	const tile = (slot: ExploreGridSlotKey, area: BentoTilePlacement) => {
		const shape = {
			short: area.size === "short",
			mdShort: area.mdSize === "short",
		};
		const content: Record<ExploreGridSlotKey, () => ReactNode> = {
			hero: () =>
				grid.hero && (
					<SpotlightTile spotlight={grid.hero} onCoverChange={onCoverChange} />
				),
			notice: () =>
				grid.notice && (
					<AnnouncementTile
						announcement={grid.notice}
						onDismiss={onDismissNotice}
						dismissRef={dismissNoticeRef}
					/>
				),
			feature: () =>
				grid.feature && <FeatureTile feature={grid.feature} size={area.size} />,
			collection: () =>
				grid.collection && <CollectionTile collection={grid.collection} />,
			stat: () => grid.stat && <StatTile rail={grid.stat} {...shape} />,
			categories: () =>
				grid.categories && (
					<CategoryTiles rail={grid.categories} type={type} {...shape} />
				),
		};
		return content[slot]();
	};

	return (
		<section
			aria-label={t("exploreFeatured", "Featured")}
			data-explore-bento
			className="grid grid-cols-1 gap-4 @3xl/explore:grid-cols-6 @3xl/explore:grid-rows-(--bento-md-rows) @5xl/explore:grid-cols-12 @5xl/explore:grid-rows-(--bento-xl-rows)"
			style={
				{
					"--bento-md-rows": heights(rows.md),
					"--bento-xl-rows": heights(rows.xl),
				} as CSSProperties
			}
		>
			{present.map((slot) => {
				const area = placement[slot];
				if (!area) return null;
				return (
					<div
						key={slot}
						data-slot={slot}
						data-size={area.size}
						data-row-span={area.xl.rowSpan}
						className="min-w-0 @3xl/explore:col-(--md-col) @3xl/explore:row-(--md-row) @5xl/explore:col-(--xl-col) @5xl/explore:row-(--xl-row)"
						style={areaStyle(area)}
					>
						{tile(slot, area)}
					</div>
				);
			})}
		</section>
	);
}
