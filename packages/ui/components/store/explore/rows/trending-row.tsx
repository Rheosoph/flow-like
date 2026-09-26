"use client";

import type { CSSProperties } from "react";
import { cn } from "../../../../lib/utils";
import { ExploreItemCard, itemId } from "../explore-item-card";
import { type ExploreRowCell, layoutTrending } from "../explore-model";
import type { ExploreRail } from "../explore-types";
import {
	ExploreRowSection,
	ROW_ITEM,
	ROW_SCROLLER,
	type RowContext,
	useRailHeading,
} from "./row-section";

const CELL =
	"min-w-0 @5xl/explore:col-(--cell-col) @5xl/explore:row-(--cell-row)";
const ROW_HEIGHT_PX = 182;

function cellStyle(cell: ExploreRowCell): CSSProperties {
	return {
		"--cell-col": `${cell.col} / span ${cell.span}`,
		"--cell-row": `${cell.row} / span ${cell.rowSpan}`,
	} as CSSProperties;
}

/** Tall cells stand alone; one-row package cells in the same column form a pair that scrolls as one. */
function groupCells(cells: readonly ExploreRowCell[]): ExploreRowCell[][] {
	const groups = new Map<string, ExploreRowCell[]>();
	for (const cell of cells) {
		const key = cell.rowSpan > 1 ? `tall:${cell.col}` : `pair:${cell.col}`;
		groups.set(key, [...(groups.get(key) ?? []), cell]);
	}
	return [...groups.values()].sort((a, b) => a[0].col - b[0].col);
}

/** As many 182 px rows as the cells reach, so one or two packages never leave an empty second row. */
function rowTracks(cells: readonly ExploreRowCell[]): CSSProperties {
	const rows = Math.max(...cells.map((cell) => cell.row + cell.rowSpan - 1));
	return {
		"--trending-rows": `repeat(${rows}, ${ROW_HEIGHT_PX}px)`,
	} as CSSProperties;
}

/** Apps portrait, packages in landscape pairs: app | pkg + pkg | app on 12 columns × up to two 182 px rows. */
export function TrendingRow({
	rail,
	context,
}: Readonly<{ rail: ExploreRail; context: RowContext }>) {
	const heading = useRailHeading(rail, context.mix);
	const cells = layoutTrending(rail.items, context.dev);
	if (!cells.length) return null;
	return (
		<ExploreRowSection {...heading}>
			<div
				className={cn(
					ROW_SCROLLER,
					"@5xl/explore:grid-cols-12 @5xl/explore:grid-rows-(--trending-rows)",
				)}
				style={rowTracks(cells)}
			>
				{groupCells(cells).map((group) => {
					const [first] = group;
					if (first.rowSpan > 1) {
						return (
							<div
								key={itemId(first.item)}
								className={cn(ROW_ITEM, CELL)}
								style={cellStyle(first)}
							>
								<ExploreItemCard item={first.item} />
							</div>
						);
					}
					return (
						<div
							key={`pair:${itemId(first.item)}`}
							className="flex w-[min(34rem,85cqw)] shrink-0 snap-start flex-col gap-4 @5xl/explore:contents"
						>
							{group.map((cell) => (
								<div
									key={itemId(cell.item)}
									className={CELL}
									style={cellStyle(cell)}
								>
									<ExploreItemCard item={cell.item} size="landscape" />
								</div>
							))}
						</div>
					);
				})}
			</div>
		</ExploreRowSection>
	);
}
