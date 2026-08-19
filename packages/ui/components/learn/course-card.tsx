"use client";
import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import {
	Atom,
	Boxes,
	Brain,
	Check,
	Clock,
	Compass,
	Database,
	Layers,
	Plug,
	Rocket,
	Sparkles,
	Workflow,
	Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useMemo, useState } from "react";
import type {
	CourseCategory,
	CourseDifficulty,
	CourseListItem,
} from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { CourseBoardGlyph } from "./course-board-glyph";

interface CourseCardProps {
	readonly course: CourseListItem;
	readonly progressPct?: number;
	readonly onSelect?: (course: CourseListItem) => void;
	readonly index?: number;
	readonly recommended?: boolean;
}

const categoryIcons: Record<CourseCategory, LucideIcon> = {
	GENERAL: Sparkles,
	GETTING_STARTED: Compass,
	FLOWS: Workflow,
	PAGES: Layers,
	EVENTS: Zap,
	DATA: Database,
	AI: Brain,
	INTEGRATIONS: Plug,
	DEPLOYMENT: Rocket,
	ADVANCED: Boxes,
	EXPERT: Atom,
};

const difficultyDots: Record<CourseDifficulty, number> = {
	BEGINNER: 1,
	INTERMEDIATE: 2,
	ADVANCED: 3,
	EXPERT: 4,
};

export function CourseCard({
	course,
	progressPct,
	onSelect,
	index = 0,
	recommended = false,
}: CourseCardProps) {
	const { t } = useTranslation();
	const Icon =
		categoryIcons[course.category as CourseCategory] ?? categoryIcons.GENERAL;
	const tags = useMemo(() => (course.tags ?? []).slice(0, 3), [course.tags]);
	const started = progressPct !== undefined && progressPct > 0;
	const completed = (progressPct ?? 0) >= 1;
	const dots = difficultyDots[course.difficulty as CourseDifficulty] ?? 1;
	const [failedBannerUrl, setFailedBannerUrl] = useState<string | null>(null);
	const [failedIconUrl, setFailedIconUrl] = useState<string | null>(null);
	const showBanner =
		Boolean(course.banner_url) && failedBannerUrl !== course.banner_url;
	const showIcon =
		Boolean(course.icon_url) && failedIconUrl !== course.icon_url;

	const categoryLabel = useMemo(() => {
		const raw = (course.category ?? "GENERAL").replace(/_/g, " ").toLowerCase();
		return raw.charAt(0).toUpperCase() + raw.slice(1);
	}, [course.category]);
	const difficultyLabel = useMemo(() => {
		const raw = course.difficulty.toLowerCase();
		return raw.charAt(0).toUpperCase() + raw.slice(1);
	}, [course.difficulty]);

	return (
		<motion.button
			type="button"
			onClick={() => onSelect?.(course)}
			initial={{ opacity: 0, y: 10 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.3, delay: Math.min(index, 8) * 0.03 }}
			whileHover={{ y: -3 }}
			whileTap={{ scale: 0.99 }}
			className="group h-full w-full rounded-xl text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
		>
			<article
				className={cn(
					"relative flex h-full flex-col overflow-hidden rounded-xl border bg-card transition-colors duration-200",
					recommended
						? "border-primary/60"
						: "border-border/70 group-hover:border-border",
				)}
			>
				<div className="relative aspect-video w-full shrink-0 overflow-hidden border-b border-border/60">
					{showBanner ? (
						<img
							src={course.banner_url ?? undefined}
							alt=""
							loading="lazy"
							decoding="async"
							draggable={false}
							onError={() => setFailedBannerUrl(course.banner_url)}
							className="size-full object-cover object-center transition-transform duration-500 group-hover:scale-[1.02]"
						/>
					) : (
						<CourseBoardGlyph seed={course.id} accent={started} />
					)}

					<div className="absolute left-3 top-3 grid size-9 place-items-center rounded-lg border border-border/60 bg-background/85 backdrop-blur-md">
						{showIcon ? (
							<img
								src={course.icon_url ?? undefined}
								alt=""
								loading="lazy"
								decoding="async"
								draggable={false}
								onError={() => setFailedIconUrl(course.icon_url)}
								className="size-7 rounded-md object-contain"
							/>
						) : (
							<Icon className="size-4 text-muted-foreground" />
						)}
					</div>

					{recommended && (
						<span className="absolute right-3 top-3 rounded-md bg-primary px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-primary-foreground">
							{t("startHere", "Start here")}
						</span>
					)}
					{!course.is_published && !recommended && (
						<Badge
							variant="outline"
							className="absolute right-3 top-3 border-border/60 bg-background/85 backdrop-blur-md"
						>
							{t("draft", "Draft")}
						</Badge>
					)}
				</div>

				<div className="flex flex-1 flex-col gap-2.5 px-4 pb-3 pt-3.5">
					<div className="flex items-center gap-2 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
						<span
							className={cn(
								"h-3 w-0.75 rounded-full",
								started ? "bg-primary" : "bg-muted-foreground/40",
							)}
						/>
						<span className="truncate text-foreground/70">{categoryLabel}</span>
						{course.estimated_minutes > 0 && (
							<>
								<span aria-hidden>·</span>
								<span className="inline-flex items-center gap-1 tabular-nums">
									<Clock className="size-3" />
									{t("valMin", "{{val}} min", {
										val: course.estimated_minutes,
									})}
								</span>
							</>
						)}
					</div>

					<h3 className="line-clamp-2 text-[17px] font-semibold leading-tight tracking-tight transition-colors group-hover:text-primary">
						{course.name ?? course.id}
					</h3>

					{course.description && (
						<p className="line-clamp-3 font-serif text-[14.5px] leading-[1.55] text-muted-foreground">
							{course.description}
						</p>
					)}

					{tags.length > 0 && (
						<div className="mt-auto flex flex-wrap gap-1.5 pt-1">
							{tags.map((tag) => (
								<span
									key={tag}
									className="rounded border border-border/60 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground"
								>
									{tag}
								</span>
							))}
						</div>
					)}
				</div>

				<div className="flex items-center gap-2.5 border-t border-border/60 px-4 py-2.5 text-xs text-muted-foreground">
					<span
						className="flex items-center gap-1"
						aria-label={`${difficultyLabel} difficulty`}
					>
						{[1, 2, 3, 4].map((i) => (
							<span
								key={i}
								className={cn(
									"block size-1.25 rounded-full",
									i <= dots ? "bg-foreground/70" : "bg-foreground/15",
								)}
							/>
						))}
					</span>
					<span className="font-mono text-[10px] uppercase tracking-wider">
						{difficultyLabel}
					</span>
					<span className="ml-auto font-mono text-[10px] uppercase tracking-wider">
						{completed ? (
							<span className="inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
								<Check className="size-3" />
								{t("built", "Built")}
							</span>
						) : started ? (
							<span className="text-primary">
								{t("inProgress", "In progress")}
							</span>
						) : (
							<span>{t("notStarted", "Not started")}</span>
						)}
					</span>
				</div>

				{started && (
					<div
						className={cn(
							"absolute inset-x-0 bottom-0 h-0.75",
							completed
								? "bg-primary"
								: "bg-[repeating-linear-gradient(90deg,var(--primary)_0_6px,transparent_6px_12px)]",
						)}
					/>
				)}
			</article>
		</motion.button>
	);
}
