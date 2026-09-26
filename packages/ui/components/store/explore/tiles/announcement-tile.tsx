"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Info,
	type LucideIcon,
	Rocket,
	TriangleAlert,
	Wrench,
	X,
} from "lucide-react";
import Link from "next/link";
import type { Ref } from "react";
import { useAssetImage } from "../../../../hooks/use-asset-image";
import { cn } from "../../../../lib/utils";
import { useExploreLabels } from "../explore-labels";
import type { ExploreAnnouncement, ExploreTone } from "../explore-types";

const TONES: Record<
	ExploreTone,
	{
		icon: LucideIcon;
		border: string;
		wash: string;
		badge: string;
		accent: string;
	}
> = {
	info: {
		icon: Info,
		border: "border-sky-500/40",
		wash: "bg-sky-500/8",
		badge: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
		accent: "text-sky-600 dark:text-sky-400",
	},
	launch: {
		icon: Rocket,
		border: "border-primary/40",
		wash: "bg-primary/8",
		badge: "bg-primary/15 text-primary",
		accent: "text-primary",
	},
	maintenance: {
		icon: Wrench,
		border: "border-amber-500/40",
		wash: "bg-amber-500/8",
		badge: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
		accent: "text-amber-600 dark:text-amber-400",
	},
	warning: {
		icon: TriangleAlert,
		border: "border-destructive/45",
		wash: "bg-destructive/8",
		badge: "bg-destructive/15 text-destructive",
		accent: "text-destructive",
	},
};

function isExternal(href: string): boolean {
	return href.startsWith("https://");
}

export function AnnouncementTile({
	announcement,
	onDismiss,
	dismissRef,
}: Readonly<{
	announcement: ExploreAnnouncement;
	onDismiss?: () => void;
	dismissRef?: Ref<HTMLButtonElement>;
}>) {
	const { t } = useTranslation("store");
	const labels = useExploreLabels();
	const tone = TONES[announcement.tone];
	const Icon = tone.icon;
	const image = useAssetImage(announcement.imageUrl);
	const href = announcement.ctaHref;
	const ctaClass = cn(
		"mt-auto inline-flex items-center gap-1.5 self-start rounded-sm pt-3 text-[13px] font-semibold hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
		tone.accent,
	);
	const ctaContent = (
		<>
			{announcement.ctaLabel}
			<ArrowRight aria-hidden="true" className="size-3.5" />
		</>
	);

	return (
		<article
			aria-label={t("exploreAnnouncement", "Announcement")}
			data-explore-tile="announcement"
			data-tone={announcement.tone}
			className={cn(
				"relative flex h-full min-w-0 gap-4.5 overflow-hidden rounded-2xl border bg-card p-4.5 pl-5",
				tone.border,
			)}
		>
			<div
				aria-hidden="true"
				className={cn("pointer-events-none absolute inset-0", tone.wash)}
			/>
			<div className="relative flex min-w-0 flex-1 flex-col">
				<div className="flex items-center gap-2 pr-8">
					<span
						aria-hidden="true"
						className={cn(
							"inline-flex size-6.5 shrink-0 items-center justify-center rounded-md",
							tone.badge,
						)}
					>
						<Icon className="size-3.75" />
					</span>
					<span className={cn("text-xs font-semibold", tone.accent)}>
						{labels.tone(announcement.tone)}
					</span>
					<span className="truncate text-xs text-muted-foreground">
						{`· ${t("exploreAnnouncement", "Announcement")}`}
					</span>
				</div>
				<h2 className="mt-3 line-clamp-2 text-[19px] font-semibold leading-6 tracking-tight">
					{announcement.title}
				</h2>
				<p className="mt-1.5 line-clamp-3 text-[13.5px] leading-5 text-foreground/80">
					{announcement.body}
				</p>
				{href &&
					announcement.ctaLabel &&
					(isExternal(href) ? (
						<a
							href={href}
							target="_blank"
							rel="noopener noreferrer"
							className={ctaClass}
						>
							{ctaContent}
						</a>
					) : (
						<Link href={href} className={ctaClass}>
							{ctaContent}
						</Link>
					))}
			</div>
			{image.canRender && (
				<div className="relative hidden w-35 shrink-0 overflow-hidden rounded-[10px] border border-border/60 @7xl/explore:block">
					<img
						ref={image.imgRef}
						src={image.src}
						onLoad={image.onLoad}
						onError={image.onError}
						alt=""
						className="absolute inset-0 h-full w-full object-cover"
					/>
				</div>
			)}
			{announcement.dismissible && onDismiss && (
				<button
					ref={dismissRef}
					type="button"
					aria-label={t("exploreDismissAnnouncement", "Dismiss announcement")}
					onClick={onDismiss}
					className="absolute right-2.5 top-2.5 inline-flex size-7 items-center justify-center rounded-full border border-border bg-background/80 text-foreground backdrop-blur-sm transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<X aria-hidden="true" className="size-3.5" />
				</button>
			)}
		</article>
	);
}
