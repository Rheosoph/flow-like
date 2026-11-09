import { useTranslation } from "@flow-like/locales";
import {
	BookOpenIcon,
	CalendarDaysIcon,
	CheckCheckIcon,
	HeartIcon,
	type LucideIcon,
	UserRoundIcon,
} from "lucide-react";
import { useCallback } from "react";

export interface MemoryKindStyle {
	icon: LucideIcon;
	/** Tinted badge: background and text. */
	color: string;
	/** Text color for strokes and glyphs drawn in `currentColor`. */
	accent: string;
	/** Solid fill for rail dots and map pins. */
	solid: string;
	/** Soft card wash, used with `bg-linear-to-b`. */
	wash: string;
	/** MapLibre paint needs a literal color. */
	hex: string;
}

export type MemoryKind =
	| "profile"
	| "preference"
	| "event"
	| "decision"
	| "knowledge";

const kindStyles: Record<MemoryKind, MemoryKindStyle> = {
	profile: {
		icon: UserRoundIcon,
		color: "bg-sky-500/10 text-sky-700 dark:text-sky-300",
		accent: "text-sky-600 dark:text-sky-400",
		solid: "bg-sky-500",
		wash: "from-sky-500/6",
		hex: "#0ea5e9",
	},
	preference: {
		icon: HeartIcon,
		color: "bg-rose-500/10 text-rose-700 dark:text-rose-300",
		accent: "text-rose-600 dark:text-rose-400",
		solid: "bg-rose-500",
		wash: "from-rose-500/6",
		hex: "#f43f5e",
	},
	event: {
		icon: CalendarDaysIcon,
		color: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
		accent: "text-amber-600 dark:text-amber-400",
		solid: "bg-amber-500",
		wash: "from-amber-500/6",
		hex: "#f59e0b",
	},
	decision: {
		icon: CheckCheckIcon,
		color: "bg-violet-500/10 text-violet-700 dark:text-violet-300",
		accent: "text-violet-600 dark:text-violet-400",
		solid: "bg-violet-500",
		wash: "from-violet-500/6",
		hex: "#8b5cf6",
	},
	knowledge: {
		icon: BookOpenIcon,
		color: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
		accent: "text-emerald-600 dark:text-emerald-400",
		solid: "bg-emerald-500",
		wash: "from-emerald-500/6",
		hex: "#10b981",
	},
};

export function memoryKindStyle(kind: string): MemoryKindStyle {
	return Object.hasOwn(kindStyles, kind)
		? kindStyles[kind as MemoryKind]
		: kindStyles.knowledge;
}

export function memoryLabel(value: string) {
	const words = value.replaceAll("_", " ");
	return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Names a kind; `common.memory` is a plain string, so the key carries its namespace. */
export function useMemoryKindLabel() {
	const { t } = useTranslation("bots");
	return useCallback(
		(kind: string) =>
			t(`bots:memory.kinds.${kind as MemoryKind}`, memoryLabel(kind)),
		[t],
	);
}
