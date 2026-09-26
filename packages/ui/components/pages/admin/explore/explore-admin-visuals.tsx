"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BadgeDollarSign,
	Code2,
	GalleryHorizontal,
	Globe,
	Languages,
	Layers,
	LogIn,
	LogOut,
	type LucideIcon,
	Megaphone,
	Monitor,
	Sparkles,
	Star,
	Users,
} from "lucide-react";
import { cn } from "../../../../lib/utils";
import type {
	ExplorePlacementKind,
	ExploreStatus,
	ExploreTone,
} from "../../../store/explore/explore-types";
import { audienceLabel, statusLabel } from "./explore-admin-model";

export const KIND_ICONS: Record<ExplorePlacementKind, LucideIcon> = {
	announcement: Megaphone,
	spotlight: Sparkles,
	feature: Star,
	collection: Layers,
	rail: GalleryHorizontal,
	sponsored: BadgeDollarSign,
};

export function audienceIcon(tag: string): LucideIcon {
	switch (tag) {
		case "dev":
			return Code2;
		case "signed_in":
			return LogIn;
		case "signed_out":
			return LogOut;
		case "desktop":
			return Monitor;
		case "web":
			return Globe;
		case "everyone":
			return Users;
		default:
			return Languages;
	}
}

const STATUS_DOT: Record<ExploreStatus, string> = {
	live: "bg-green-500",
	scheduled: "bg-blue-500",
	draft: "bg-muted-foreground/70",
	ended: "bg-muted-foreground/40",
};

const STATUS_PILL: Record<ExploreStatus, string> = {
	live: "bg-green-500/12 text-green-700 dark:text-green-400",
	scheduled: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
	draft: "bg-muted text-muted-foreground",
	ended: "border border-border text-muted-foreground",
};

/** The same tone colors as the viewer's AnnouncementTile, so the canvas and the tone picker preview the live tile. */
export const TONE_STYLES: Record<
	ExploreTone,
	{ dot: string; text: string; tint: string; pressed: string }
> = {
	info: {
		dot: "bg-sky-500",
		text: "text-sky-600 dark:text-sky-400",
		tint: "border-sky-500/40 bg-sky-500/8",
		pressed: "border-sky-500/50 bg-sky-500/15 text-sky-600 dark:text-sky-400",
	},
	launch: {
		dot: "bg-primary",
		text: "text-primary",
		tint: "border-primary/40 bg-primary/8",
		pressed: "border-primary/50 bg-primary/15 text-primary",
	},
	maintenance: {
		dot: "bg-amber-500",
		text: "text-amber-600 dark:text-amber-400",
		tint: "border-amber-500/40 bg-amber-500/8",
		pressed:
			"border-amber-500/50 bg-amber-500/15 text-amber-600 dark:text-amber-400",
	},
	warning: {
		dot: "bg-destructive",
		text: "text-destructive",
		tint: "border-destructive/45 bg-destructive/8",
		pressed: "border-destructive/50 bg-destructive/15 text-destructive",
	},
};

export const SECTION_TITLE =
	"text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground";

export const FALLBACK_CHIP =
	"inline-flex h-4 shrink-0 items-center rounded border border-dashed border-blue-500/60 px-1.5 text-[10px] text-blue-600 dark:text-blue-400";

export function StatusDot({
	status,
	className,
}: {
	status: ExploreStatus;
	className?: string;
}) {
	const { t } = useTranslation("admin");
	return (
		<span
			role="img"
			aria-label={statusLabel(status, t)}
			title={statusLabel(status, t)}
			className={cn(
				"inline-block size-1.75 shrink-0 rounded-full",
				STATUS_DOT[status],
				className,
			)}
		/>
	);
}

export function StatusPill({
	status,
	size = "sm",
}: {
	status: ExploreStatus;
	size?: "sm" | "md";
}) {
	const { t } = useTranslation("admin");
	return (
		<span
			className={cn(
				"inline-flex shrink-0 items-center gap-1.5 rounded-full font-medium",
				size === "md" ? "h-5.5 px-2.5 text-xs" : "h-4 px-1.75 text-[10.5px]",
				STATUS_PILL[status],
			)}
		>
			<span
				aria-hidden="true"
				className={cn("size-1.5 rounded-full", STATUS_DOT[status])}
			/>
			{statusLabel(status, t)}
		</span>
	);
}

export function AudienceIcons({
	audience,
	className,
}: {
	audience: readonly string[];
	className?: string;
}) {
	const { t } = useTranslation("admin");
	const tags = audience.length ? audience : ["everyone"];
	return (
		<span
			className={cn(
				"inline-flex min-w-0 items-center gap-1 text-muted-foreground",
				className,
			)}
		>
			{tags.map((tag) => {
				const Icon = audienceIcon(tag);
				const label = audienceLabel(tag, t);
				return (
					<Icon
						key={tag}
						role="img"
						aria-label={label}
						className="size-3 shrink-0"
					>
						<title>{label}</title>
					</Icon>
				);
			})}
		</span>
	);
}

export function KindIcon({
	kind,
	className,
}: {
	kind: ExplorePlacementKind;
	className?: string;
}) {
	const Icon = KIND_ICONS[kind];
	return <Icon aria-hidden="true" className={cn("size-3.5", className)} />;
}
