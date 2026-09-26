import {
	BombIcon,
	CircleXIcon,
	InfoIcon,
	type LucideIcon,
	ScrollIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { memo } from "react";
import { cn } from "../../../lib/utils";

const LEVEL_ICONS: readonly LucideIcon[] = [
	ScrollIcon,
	InfoIcon,
	TriangleAlertIcon,
	CircleXIcon,
	BombIcon,
];

/** Severity decides the colour: debug recedes, errors carry the only red. */
export const LEVEL_TONES: readonly string[] = [
	"text-muted-foreground",
	"text-sky-500",
	"text-amber-500",
	"text-destructive",
	"text-pink-500",
];

export const LEVEL_SHORT = ["D", "I", "W", "E", "F"] as const;

export const LevelIcon = memo(function LevelIcon({
	level,
	className,
}: Readonly<{ level: number; className?: string }>) {
	const Icon = LEVEL_ICONS[level] ?? ScrollIcon;
	return (
		<Icon
			aria-hidden
			className={cn("size-3.5 shrink-0", LEVEL_TONES[level], className)}
		/>
	);
});
