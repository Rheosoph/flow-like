"use client";

import { useTranslation } from "@flow-like/locales";
import { TEMPLATE_LANGUAGES } from "../../../../lib/schema/developer";
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
const STRIP_LANGUAGES = [
	"rust",
	"python",
	"typescript",
	"go",
	"zig",
	"cpp",
	"moonbit",
	"assemblyscript",
];

function LanguageStrip() {
	const { t } = useTranslation("store");
	const languages = TEMPLATE_LANGUAGES.filter((language) =>
		STRIP_LANGUAGES.includes(language.value),
	);
	return (
		<span className="hidden items-center gap-2.5 text-xs text-muted-foreground @3xl/explore:flex">
			<span className="flex">
				{languages.map((language, index) => (
					<img
						key={language.value}
						src={language.img}
						alt={language.label}
						title={language.label}
						className={cn(
							"size-5.5 rounded-full border-2 border-background object-cover",
							index > 0 && "-ml-1.5",
						)}
					/>
				))}
			</span>
			{t("exploreLanguagesStrip", {
				count: TEMPLATE_LANGUAGES.length,
				defaultValue_one: "Written in {{count}} language, compiled to WASM",
				defaultValue_other: "Written in {{count}} languages, compiled to WASM",
			})}
		</span>
	);
}

/** Packages for builders in the standard PackageCard grammar, with the languages they can be written in. */
export function BuildersRow({
	rail,
	context,
}: Readonly<{ rail: ExploreRail; context: RowContext }>) {
	const heading = useRailHeading(rail, context.mix);
	const items = rail.items.slice(0, MAX_ITEMS);
	if (!items.length) return null;
	return (
		<ExploreRowSection {...heading} extra={<LanguageStrip />}>
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
