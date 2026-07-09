import {
	Briefcase,
	Camera,
	Clapperboard,
	CloudSun,
	Dumbbell,
	Gamepad2,
	GraduationCap,
	HeartPulse,
	type LucideIcon,
	MessagesSquare,
	Music,
	Newspaper,
	Plane,
	Shapes,
	ShoppingBag,
	Sparkles,
	Sun,
	UsersRound,
	UtensilsCrossed,
	Wallet,
	Wrench,
	Zap,
} from "lucide-react";
import { formatAppCategory } from "./app-category";
import { IAppCategory } from "./schema/app/app-search-query";

// Keyed by the FORMATTED label (formatAppCategory output), not the enum value.
export const CATEGORY_COLORS: Record<string, string> = {
	Business: "oklch(0.65 0.15 250)",
	Communication: "oklch(0.65 0.15 290)",
	Education: "oklch(0.65 0.15 145)",
	Entertainment: "oklch(0.65 0.15 330)",
	Finance: "oklch(0.65 0.15 160)",
	"Food And Drink": "oklch(0.65 0.15 55)",
	Games: "oklch(0.65 0.15 310)",
	Health: "oklch(0.65 0.15 145)",
	Lifestyle: "oklch(0.65 0.15 20)",
	Music: "oklch(0.65 0.15 280)",
	News: "oklch(0.65 0.15 220)",
	Other: "oklch(0.65 0.08 250)",
	Photography: "oklch(0.65 0.15 80)",
	Productivity: "oklch(0.65 0.15 240)",
	Shopping: "oklch(0.65 0.15 40)",
	Social: "oklch(0.65 0.15 200)",
	Sports: "oklch(0.65 0.15 130)",
	Travel: "oklch(0.65 0.15 180)",
	Utilities: "oklch(0.65 0.10 230)",
	Weather: "oklch(0.65 0.15 210)",
	Anime: "oklch(0.65 0.15 350)",
};

// Curated browse order for category rails and chip rows ("Other" always last).
export const APP_CATEGORY_ORDER: readonly IAppCategory[] = [
	IAppCategory.Productivity,
	IAppCategory.Business,
	IAppCategory.Utilities,
	IAppCategory.Communication,
	IAppCategory.Social,
	IAppCategory.Education,
	IAppCategory.Entertainment,
	IAppCategory.Games,
	IAppCategory.Music,
	IAppCategory.Photography,
	IAppCategory.News,
	IAppCategory.Finance,
	IAppCategory.Health,
	IAppCategory.Lifestyle,
	IAppCategory.Sports,
	IAppCategory.Travel,
	IAppCategory.FoodAndDrink,
	IAppCategory.Shopping,
	IAppCategory.Weather,
	IAppCategory.Anime,
	IAppCategory.Other,
];

export const CATEGORY_ICONS: Record<IAppCategory, LucideIcon> = {
	[IAppCategory.Anime]: Sparkles,
	[IAppCategory.Business]: Briefcase,
	[IAppCategory.Communication]: MessagesSquare,
	[IAppCategory.Education]: GraduationCap,
	[IAppCategory.Entertainment]: Clapperboard,
	[IAppCategory.Finance]: Wallet,
	[IAppCategory.FoodAndDrink]: UtensilsCrossed,
	[IAppCategory.Games]: Gamepad2,
	[IAppCategory.Health]: HeartPulse,
	[IAppCategory.Lifestyle]: Sun,
	[IAppCategory.Music]: Music,
	[IAppCategory.News]: Newspaper,
	[IAppCategory.Other]: Shapes,
	[IAppCategory.Photography]: Camera,
	[IAppCategory.Productivity]: Zap,
	[IAppCategory.Shopping]: ShoppingBag,
	[IAppCategory.Social]: UsersRound,
	[IAppCategory.Sports]: Dumbbell,
	[IAppCategory.Travel]: Plane,
	[IAppCategory.Utilities]: Wrench,
	[IAppCategory.Weather]: CloudSun,
};

export function categoryColor(category?: string | null): string {
	return CATEGORY_COLORS[formatAppCategory(category)] ?? CATEGORY_COLORS.Other;
}

export function categoryIcon(category?: string | null): LucideIcon {
	return CATEGORY_ICONS[category as IAppCategory] ?? Shapes;
}
