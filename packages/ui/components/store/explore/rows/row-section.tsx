"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronRight } from "lucide-react";
import Link from "next/link";
import { type ReactNode, useId } from "react";
import { cn } from "../../../../lib/utils";
import { exploreSearchHref } from "../explore-href";
import { useExploreLabels } from "../explore-labels";
import type { ExploreRail, ExploreTypeFilter } from "../explore-types";

/** Below xl a row scrolls sideways with snap; at xl it becomes a grid. */
export const ROW_SCROLLER =
	"flex snap-x snap-mandatory gap-4 overflow-x-auto pb-2 @5xl/explore:grid @5xl/explore:overflow-visible @5xl/explore:pb-0";
export const ROW_ITEM = "w-64 shrink-0 snap-start @5xl/explore:w-auto";

export function ExploreRowSection({
	title,
	subtitle,
	action,
	extra,
	children,
	className,
}: Readonly<{
	title: string;
	subtitle?: string | null;
	action?: { label: string; href: string };
	extra?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	const headingId = useId();
	return (
		<section
			aria-labelledby={headingId}
			data-explore-row
			className={cn("min-w-0 space-y-4", className)}
		>
			<div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
				<div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
					<h2 id={headingId} className="text-lg font-semibold tracking-tight">
						{title}
					</h2>
					{subtitle && (
						<span className="text-[13px] text-muted-foreground">
							{subtitle}
						</span>
					)}
				</div>
				<div className="flex min-w-0 items-center gap-4">
					{extra}
					{action && (
						<Link
							href={action.href}
							className="inline-flex shrink-0 items-center gap-1 rounded-sm text-[13px] font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						>
							{action.label}
							<ChevronRight aria-hidden="true" className="size-3.5" />
						</Link>
					)}
				</div>
			</div>
			{children}
		</section>
	);
}

export interface RowContext {
	/** Which kinds the active view shows (a non-dev viewer only ever sees apps). */
	mix: ExploreTypeFilter;
	dev: boolean;
}

function mixType(mix: ExploreTypeFilter) {
	return mix === "all" ? undefined : mix;
}

export function useRailHeading(rail: ExploreRail, mix: ExploreTypeFilter) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const title = rail.title?.trim() || labels.railTitle(rail.rail);
	const subtitle = labels.railSubtitle(rail.rail, mix);
	const seeAll = t("seeAll", "See all");
	const type = mixType(mix);
	const action = ((): { label: string; href: string } | undefined => {
		switch (rail.rail) {
			case "trending":
				return {
					label: seeAll,
					href: exploreSearchHref({ type, sort: "installs" }),
				};
			case "new":
				return {
					label: seeAll,
					href: exploreSearchHref({ type, sort: "newest" }),
				};
			case "top_paid":
				return {
					label: seeAll,
					href: exploreSearchHref({ type, price: "paid", sort: "installs" }),
				};
			case "for_builders":
				return {
					label: t("exploreAllPackages", "All packages"),
					href: exploreSearchHref({ type: "packages" }),
				};
			default:
				return undefined;
		}
	})();
	return { title, subtitle, action };
}
