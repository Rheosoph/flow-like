"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, LayoutGrid, ListFilter } from "lucide-react";
import { useLayoutEffect, useRef } from "react";
import { useAppCategoryLabel } from "../../lib/app-category";
import { APP_CATEGORY_ORDER, CATEGORY_ICONS } from "../../lib/category-meta";
import type { IAppCategory } from "../../lib/schema/app/app-search-query";
import { cn } from "../../lib/utils";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuLabel,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";

const PRIMARY_CATEGORIES = APP_CATEGORY_ORDER.slice(0, 5);
const MORE_CATEGORIES = APP_CATEGORY_ORDER.slice(5);
const CHIP_CLASS_NAME =
	"inline-flex min-h-10 shrink-0 cursor-pointer items-center justify-center gap-2 whitespace-nowrap rounded-xl border px-3 py-2 text-sm font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

export function ExploreCategoryFilter({
	selected,
	onSelect,
}: Readonly<{
	selected?: IAppCategory;
	onSelect: (category: IAppCategory | undefined) => void;
}>) {
	const { t } = useTranslation("store");
	const categoryLabel = useAppCategoryLabel();
	const scrollRef = useRef<HTMLDivElement>(null);
	const selectedChipRef = useRef<HTMLButtonElement>(null);
	const hasMoreSelection =
		selected !== undefined && !PRIMARY_CATEGORIES.includes(selected);
	const visibleCategories = hasMoreSelection
		? [...PRIMARY_CATEGORIES, selected]
		: PRIMARY_CATEGORIES;
	const browseLabel = t("browseCategories", {
		defaultValue: "Browse categories",
	});
	const moreLabel = t("moreCategories", { defaultValue: "More categories" });

	useLayoutEffect(() => {
		const scroller = scrollRef.current;
		if (!scroller) return;
		const chip = selectedChipRef.current;
		const keepSelectedVisible = () => {
			let nextScrollLeft = 0;
			if (selected !== undefined) {
				if (!chip) return;
				nextScrollLeft = scroller.scrollLeft;
				const left = chip.offsetLeft - 4;
				const right = chip.offsetLeft + chip.offsetWidth + 4;
				if (
					chip.offsetWidth + 8 > scroller.clientWidth ||
					left < scroller.scrollLeft
				) {
					nextScrollLeft = left;
				} else if (right > scroller.scrollLeft + scroller.clientWidth) {
					nextScrollLeft = right - scroller.clientWidth;
				}
			}
			const maxScrollLeft = Math.max(
				0,
				scroller.scrollWidth - scroller.clientWidth,
			);
			nextScrollLeft = Math.max(0, Math.min(nextScrollLeft, maxScrollLeft));
			if (Math.abs(scroller.scrollLeft - nextScrollLeft) > 1) {
				scroller.scrollLeft = nextScrollLeft;
			}
		};
		keepSelectedVisible();
		const observer = new ResizeObserver(keepSelectedVisible);
		observer.observe(scroller);
		if (chip) observer.observe(chip);
		return () => observer.disconnect();
	}, [selected]);

	return (
		<fieldset className="flex min-w-0 items-center gap-2 border-0 p-0">
			<legend className="sr-only">{browseLabel}</legend>
			<div
				ref={scrollRef}
				className="relative flex min-w-0 flex-1 gap-2 overflow-x-auto p-1 scrollbar-hide"
			>
				<button
					type="button"
					aria-pressed={selected === undefined}
					onClick={() => onSelect(undefined)}
					className={cn(
						CHIP_CLASS_NAME,
						selected === undefined
							? "border-primary/25 bg-primary/10 text-primary"
							: "border-border/60 bg-background text-muted-foreground hover:bg-muted hover:text-foreground",
					)}
				>
					<LayoutGrid className="size-4" aria-hidden="true" />
					{t("allApps", { defaultValue: "All apps" })}
				</button>
				{visibleCategories.map((category) => {
					const Icon = CATEGORY_ICONS[category];
					const isSelected = selected === category;
					return (
						<button
							key={category}
							ref={isSelected ? selectedChipRef : undefined}
							type="button"
							aria-pressed={isSelected}
							onClick={() => onSelect(isSelected ? undefined : category)}
							className={cn(
								CHIP_CLASS_NAME,
								isSelected
									? "border-primary/25 bg-primary/10 text-primary"
									: "border-border/60 bg-background text-muted-foreground hover:bg-muted hover:text-foreground",
							)}
						>
							<Icon className="size-4" aria-hidden="true" />
							{categoryLabel(category)}
						</button>
					);
				})}
			</div>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						aria-label={moreLabel}
						className={cn(
							CHIP_CLASS_NAME,
							"border-border/60 bg-background text-foreground hover:bg-muted data-[state=open]:bg-muted",
							hasMoreSelection &&
								"border-primary/25 bg-primary/10 text-primary",
						)}
					>
						<ListFilter className="size-4 sm:hidden" aria-hidden="true" />
						<span className="hidden sm:inline">{moreLabel}</span>
						<ChevronDown className="size-4" aria-hidden="true" />
					</button>
				</DropdownMenuTrigger>
				<DropdownMenuContent
					align="end"
					className="max-h-[min(24rem,var(--radix-dropdown-menu-content-available-height))] w-60 rounded-xl"
				>
					<DropdownMenuLabel>{browseLabel}</DropdownMenuLabel>
					<DropdownMenuSeparator />
					<DropdownMenuRadioGroup
						aria-label={browseLabel}
						value={selected ?? ""}
						onValueChange={(value) => onSelect(value as IAppCategory)}
					>
						{MORE_CATEGORIES.map((category) => {
							const Icon = CATEGORY_ICONS[category];
							return (
								<DropdownMenuRadioItem
									key={category}
									value={category}
									className="min-h-10 cursor-pointer rounded-lg data-[state=checked]:bg-primary/10 data-[state=checked]:text-primary"
								>
									<Icon className="size-4" aria-hidden="true" />
									{categoryLabel(category)}
								</DropdownMenuRadioItem>
							);
						})}
					</DropdownMenuRadioGroup>
				</DropdownMenuContent>
			</DropdownMenu>
		</fieldset>
	);
}
