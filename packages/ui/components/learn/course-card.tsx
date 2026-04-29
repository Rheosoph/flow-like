"use client";
import { motion } from "framer-motion";
import {
	Atom,
	BookOpen,
	Boxes,
	Brain,
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
import { useMemo } from "react";
import type {
	CourseCategory,
	CourseDifficulty,
	CourseListItem,
} from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";

interface CourseCardProps {
	readonly course: CourseListItem;
	readonly progressPct?: number;
	readonly onSelect?: (course: CourseListItem) => void;
	readonly index?: number;
}

const categoryStyles: Record<
	CourseCategory,
	{
		readonly gradient: string;
		readonly icon: LucideIcon;
		readonly label: string;
	}
> = {
	GENERAL: {
		gradient: "from-slate-500/40 via-slate-500/20 to-slate-700/30",
		icon: Sparkles,
		label: "General",
	},
	GETTING_STARTED: {
		gradient: "from-emerald-500/50 via-teal-400/30 to-cyan-500/40",
		icon: Compass,
		label: "Getting started",
	},
	FLOWS: {
		gradient: "from-violet-500/50 via-fuchsia-500/30 to-pink-500/40",
		icon: Workflow,
		label: "Flows",
	},
	PAGES: {
		gradient: "from-sky-500/50 via-blue-500/30 to-indigo-500/40",
		icon: Layers,
		label: "Pages",
	},
	EVENTS: {
		gradient: "from-amber-500/50 via-orange-500/30 to-rose-500/40",
		icon: Zap,
		label: "Events",
	},
	DATA: {
		gradient: "from-emerald-600/40 via-teal-500/30 to-cyan-600/40",
		icon: Database,
		label: "Data",
	},
	AI: {
		gradient: "from-fuchsia-500/50 via-violet-500/40 to-indigo-500/40",
		icon: Brain,
		label: "AI",
	},
	INTEGRATIONS: {
		gradient: "from-orange-500/40 via-amber-500/30 to-yellow-500/30",
		icon: Plug,
		label: "Integrations",
	},
	DEPLOYMENT: {
		gradient: "from-blue-600/50 via-indigo-500/30 to-violet-600/40",
		icon: Rocket,
		label: "Deployment",
	},
	ADVANCED: {
		gradient: "from-rose-500/50 via-red-500/30 to-orange-600/40",
		icon: Boxes,
		label: "Advanced",
	},
	EXPERT: {
		gradient: "from-zinc-700/60 via-zinc-800/40 to-zinc-900/50",
		icon: Atom,
		label: "Expert",
	},
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
}: CourseCardProps) {
	const style =
		categoryStyles[course.category as CourseCategory] ?? categoryStyles.GENERAL;
	const Icon = style.icon;
	const tags = useMemo(() => (course.tags ?? []).slice(0, 3), [course.tags]);
	const isEnrolled = progressPct !== undefined && progressPct > 0;
	const progressPercent = Math.round((progressPct ?? 0) * 100);
	const dots = difficultyDots[course.difficulty as CourseDifficulty] ?? 1;

	return (
		<motion.button
			type="button"
			onClick={() => onSelect?.(course)}
			initial={{ opacity: 0, y: 12 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.35, delay: index * 0.04, ease: "easeOut" }}
			whileHover={{ y: -4 }}
			whileTap={{ scale: 0.985 }}
			className="group text-left w-full focus:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-2xl"
		>
			<article className="relative h-full overflow-hidden rounded-2xl border border-border/50 bg-card/80 backdrop-blur-sm shadow-sm transition-all duration-300 group-hover:shadow-xl group-hover:border-primary/30">
				{/* banner */}
				<div className="relative h-36 overflow-hidden">
					{course.banner_url ? (
						<div
							className="h-full bg-cover bg-center transition-transform duration-500 group-hover:scale-105"
							style={{ backgroundImage: `url(${course.banner_url})` }}
						/>
					) : (
						<div
							className={cn(
								"h-full bg-linear-to-br transition-all duration-500 group-hover:scale-105",
								style.gradient,
							)}
						/>
					)}
					{/* dotted texture overlay */}
					<div
						className="absolute inset-0 opacity-30 mix-blend-overlay pointer-events-none"
						style={{
							backgroundImage:
								"radial-gradient(circle at 1px 1px, rgba(255,255,255,0.5) 1px, transparent 0)",
							backgroundSize: "16px 16px",
						}}
					/>
					{/* floating icon */}
					<div className="absolute top-3 left-3 size-11 rounded-xl bg-background/80 backdrop-blur-md grid place-items-center shadow-lg ring-1 ring-border/50 group-hover:ring-primary/30 transition">
						{course.icon_url ? (
							<img
								src={course.icon_url}
								alt=""
								className="size-8 rounded-lg object-cover"
							/>
						) : (
							<Icon className="size-5 text-foreground" />
						)}
					</div>
					<Badge
						variant="outline"
						className={cn(
							"absolute top-3 right-3 bg-background/80 backdrop-blur-md",
							course.is_published
								? "border-emerald-500/40 text-emerald-700 dark:text-emerald-400"
								: "border-yellow-500/40 text-yellow-700 dark:text-yellow-400",
						)}
					>
						{course.is_published ? "Public" : "Draft"}
					</Badge>
					{/* difficulty dots */}
					<div className="absolute bottom-3 right-3 flex gap-1 items-center bg-background/70 backdrop-blur-md rounded-full px-2 py-1 ring-1 ring-border/50">
						{[1, 2, 3, 4].map((i) => (
							<span
								key={i}
								className={cn(
									"block size-1.5 rounded-full transition-colors",
									i <= dots ? "bg-foreground" : "bg-foreground/15",
								)}
							/>
						))}
						<span className="ml-1 text-[10px] uppercase tracking-wide text-foreground/80 font-medium">
							{course.difficulty.toLowerCase()}
						</span>
					</div>
				</div>

				{/* body */}
				<div className="px-5 pt-4 pb-5 space-y-3">
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<span className="font-medium text-foreground/70">
							{style.label}
						</span>
						{course.estimated_minutes > 0 && (
							<>
								<span aria-hidden>·</span>
								<span className="inline-flex items-center gap-1">
									<Clock className="size-3" />
									{course.estimated_minutes} min
								</span>
							</>
						)}
						{isEnrolled && (
							<>
								<span aria-hidden>·</span>
								<span className="inline-flex items-center gap-1 text-primary font-medium">
									<BookOpen className="size-3" />
									{progressPercent}%
								</span>
							</>
						)}
					</div>
					<h3 className="text-lg font-semibold leading-snug line-clamp-2 group-hover:text-primary transition-colors">
						{course.name ?? course.id}
					</h3>
					{course.description && (
						<p className="text-sm text-muted-foreground line-clamp-2">
							{course.description}
						</p>
					)}
					{tags.length > 0 && (
						<div className="flex flex-wrap gap-1.5 pt-1">
							{tags.map((t) => (
								<span
									key={t}
									className="text-[10px] uppercase tracking-wide font-medium text-muted-foreground bg-muted/60 rounded-md px-1.5 py-0.5"
								>
									{t}
								</span>
							))}
						</div>
					)}
				</div>

				{/* progress bar */}
				{isEnrolled && (
					<div className="absolute bottom-0 left-0 right-0 h-1 bg-muted/40">
						<div
							className="h-full bg-linear-to-r from-primary via-primary to-primary/70 transition-all duration-700"
							style={{ width: `${progressPercent}%` }}
						/>
					</div>
				)}
			</article>
		</motion.button>
	);
}
