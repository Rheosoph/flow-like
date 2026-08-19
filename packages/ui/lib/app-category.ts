"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback } from "react";
import { IAppCategory } from "./schema/app/app-search-query";

export function formatAppCategory(category?: string | null): string {
	if (!category) return "Other";

	const normalizedCategory = category
		.replace(/_/g, " ")
		.replace(/[A-Z](?=[A-Z][a-z])/g, "$& ")
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/\s+/g, " ")
		.trim();

	if (!normalizedCategory) return "Other";

	return normalizedCategory
		.split(" ")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
		.join(" ");
}

/**
 * Display labels live in the `store` namespace, keyed by the stable enum value.
 * Grouping, ordering and query params keep using the enum — only the rendered
 * label goes through translation.
 */
export const CATEGORY_TRANSLATION_KEYS: Record<IAppCategory, string> = {
	[IAppCategory.Anime]: "categoryAnime",
	[IAppCategory.Business]: "categoryBusiness",
	[IAppCategory.Communication]: "categoryCommunication",
	[IAppCategory.Education]: "categoryEducation",
	[IAppCategory.Entertainment]: "categoryEntertainment",
	[IAppCategory.Finance]: "categoryFinance",
	[IAppCategory.FoodAndDrink]: "categoryFoodAndDrink",
	[IAppCategory.Games]: "categoryGames",
	[IAppCategory.Health]: "categoryHealth",
	[IAppCategory.Lifestyle]: "categoryLifestyle",
	[IAppCategory.Music]: "categoryMusic",
	[IAppCategory.News]: "categoryNews",
	[IAppCategory.Other]: "categoryOther",
	[IAppCategory.Photography]: "categoryPhotography",
	[IAppCategory.Productivity]: "categoryProductivity",
	[IAppCategory.Shopping]: "categoryShopping",
	[IAppCategory.Social]: "categorySocial",
	[IAppCategory.Sports]: "categorySports",
	[IAppCategory.Travel]: "categoryTravel",
	[IAppCategory.Utilities]: "categoryUtilities",
	[IAppCategory.Weather]: "categoryWeather",
};

export function appCategoryTranslationKey(
	category?: string | null,
): string | undefined {
	return CATEGORY_TRANSLATION_KEYS[category as IAppCategory];
}

/**
 * Returns a translator for category labels. Unknown values (older apps, custom
 * strings) keep the prettified raw value instead of collapsing to "Other".
 */
export function useAppCategoryLabel(): (category?: string | null) => string {
	const { t } = useTranslation("store");
	return useCallback(
		(category?: string | null) => {
			const key = appCategoryTranslationKey(category);
			const fallback = formatAppCategory(category);
			return key ? t(key, fallback) : fallback;
		},
		[t],
	);
}
