"use client";
import { motion } from "framer-motion";
import {
	ArrowRight,
	Award,
	BookOpenCheck,
	Compass,
	GraduationCap,
	Search,
	Settings2,
	SlidersHorizontal,
	Sparkles,
	Trophy,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import type {
	CourseCategory,
	CourseDifficulty,
	CourseListItem,
	LearningPath,
} from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { EmptyState } from "../ui/empty-state";
import { Input } from "../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { CourseCard } from "./course-card";
import { LearningPathCard } from "./learning-path-card";
import { UniversityHeroArt } from "./university-hero-art";

interface CourseCatalogProps {
	readonly courses: ReadonlyArray<CourseListItem>;
	readonly paths?: ReadonlyArray<LearningPath>;
	readonly progressByCourseId?: Record<string, number>;
	readonly onSelect?: (course: CourseListItem) => void;
	readonly stats?: {
		readonly enrolled: number;
		readonly completed: number;
		readonly points?: number;
		readonly certificates: number;
	};
	readonly displayName?: string | null;
	readonly onOpenLeaderboard?: () => void;
	readonly onOpenCertificates?: () => void;
	readonly onOpenAuthoring?: () => void;
}

const categories: ReadonlyArray<{
	readonly value: CourseCategory | "ALL";
	readonly label: string;
}> = [
	{ value: "ALL", label: "All topics" },
	{ value: "GETTING_STARTED", label: "Getting started" },
	{ value: "FLOWS", label: "Flows" },
	{ value: "PAGES", label: "Pages" },
	{ value: "EVENTS", label: "Events" },
	{ value: "DATA", label: "Data" },
	{ value: "AI", label: "AI" },
	{ value: "INTEGRATIONS", label: "Integrations" },
	{ value: "DEPLOYMENT", label: "Deployment" },
	{ value: "ADVANCED", label: "Advanced" },
	{ value: "EXPERT", label: "Expert" },
];

const difficulties: ReadonlyArray<CourseDifficulty | "ALL"> = [
	"ALL",
	"BEGINNER",
	"INTERMEDIATE",
	"ADVANCED",
	"EXPERT",
];

export function CourseCatalog({
	courses,
	paths = [],
	progressByCourseId,
	onSelect,
	stats,
	displayName,
	onOpenLeaderboard,
	onOpenCertificates,
	onOpenAuthoring,
}: CourseCatalogProps) {
	const [query, setQuery] = useState("");
	const [category, setCategory] =
		useState<(typeof categories)[number]["value"]>("ALL");
	const [difficulty, setDifficulty] =
		useState<(typeof difficulties)[number]>("ALL");

	const filtered = useMemo(() => {
		const q = query.trim().toLowerCase();
		return courses.filter((c) => {
			if (category !== "ALL" && c.category !== category) return false;
			if (difficulty !== "ALL" && c.difficulty !== difficulty) return false;
			if (!q) return true;
			const haystack = [c.name ?? "", c.description ?? "", ...(c.tags ?? [])]
				.join(" ")
				.toLowerCase();
			return haystack.includes(q);
		});
	}, [courses, query, category, difficulty]);

	const inProgress = useMemo(() => {
		const map = progressByCourseId ?? {};
		return filtered.filter((c) => {
			const p = map[c.id];
			return p !== undefined && p > 0 && p < 1;
		});
	}, [filtered, progressByCourseId]);

	const remainingCourses = useMemo(
		() => filtered.filter((c) => !inProgress.find((i) => i.id === c.id)),
		[filtered, inProgress],
	);

	const availableCategories = useMemo(() => {
		const present = new Set<CourseCategory>();
		for (const c of courses) {
			if (c.category) present.add(c.category as CourseCategory);
		}
		return categories.filter(
			(c) => c.value === "ALL" || present.has(c.value as CourseCategory),
		);
	}, [courses]);

	const greetingName = displayName?.split(" ")[0] || displayName || null;
	const isFiltering =
		query.trim().length > 0 || difficulty !== "ALL" || category !== "ALL";
	const activeFilterCount =
		(difficulty !== "ALL" ? 1 : 0) + (category !== "ALL" ? 1 : 0);
	const clearFilters = () => {
		setDifficulty("ALL");
		setCategory("ALL");
	};

	return (
		<div className="flex flex-col gap-4">
			<header className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
				<div>
					<h1 className="text-xl font-semibold tracking-tight">University</h1>
					<p className="text-xs text-muted-foreground">
						{filtered.length} {filtered.length === 1 ? "course" : "courses"}{" "}
						available
					</p>
				</div>
				<div className="flex w-full gap-2 md:w-auto">
					<div className="relative min-w-0 flex-1 md:w-80">
						<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={query}
							onChange={(e) => setQuery(e.target.value)}
							placeholder="Search by title, tag, or topic…"
							className="h-9 rounded-lg border-border/60 bg-background/70 pl-9 backdrop-blur-sm"
						/>
					</div>
					<Popover>
						<PopoverTrigger asChild>
							<button
								type="button"
								className={cn(
									"inline-flex h-9 shrink-0 items-center gap-2 rounded-lg border border-border/60 bg-background/70 px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground",
									activeFilterCount > 0 &&
										"border-primary/40 bg-primary/10 text-foreground",
								)}
							>
								<SlidersHorizontal className="size-4" />
								<span className="hidden sm:inline">Filters</span>
								{activeFilterCount > 0 && (
									<span className="grid size-4 place-items-center rounded-full bg-primary text-[10px] font-semibold text-primary-foreground">
										{activeFilterCount}
									</span>
								)}
							</button>
						</PopoverTrigger>
						<PopoverContent
							align="end"
							className="w-[min(92vw,34rem)] space-y-4 p-4"
						>
							<div className="flex items-start justify-between gap-3">
								<div>
									<h3 className="text-sm font-semibold">Refine courses</h3>
									<p className="text-xs text-muted-foreground">
										Filter by difficulty or topic.
									</p>
								</div>
								{activeFilterCount > 0 && (
									<button
										type="button"
										onClick={clearFilters}
										className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-muted-foreground hover:bg-muted/60 hover:text-foreground"
									>
										<X className="size-3" />
										Clear
									</button>
								)}
							</div>
							<div className="space-y-4">
								<SegmentedSelect
									label="Difficulty"
									options={difficulties.map((d) => ({
										value: d,
										label:
											d === "ALL"
												? "All"
												: d.charAt(0) + d.slice(1).toLowerCase(),
									}))}
									value={difficulty}
									onChange={(v) =>
										setDifficulty(v as (typeof difficulties)[number])
									}
								/>
								<SegmentedSelect
									label="Topic"
									options={categories.map((c) => ({
										value: c.value,
										label: c.label,
									}))}
									value={category}
									onChange={(v) =>
										setCategory(v as (typeof categories)[number]["value"])
									}
								/>
							</div>
						</PopoverContent>
					</Popover>
				</div>
			</header>

			<motion.section
				initial={{ opacity: 0, y: 8 }}
				animate={{ opacity: 1, y: 0 }}
				transition={{ duration: 0.45, ease: "easeOut" }}
				className="relative overflow-hidden rounded-2xl border border-border/60 bg-linear-to-br from-sky-500/15 via-card/85 to-amber-500/10 p-5 shadow-sm md:p-6"
			>
				<div className="absolute inset-x-0 top-0 h-1 bg-linear-to-r from-sky-400/80 via-emerald-400/80 to-amber-300/80" />
				<div className="absolute inset-0 bg-[linear-gradient(135deg,transparent_0%,rgba(255,255,255,0.08)_42%,transparent_74%)]" />
				<UniversityHeroArt className="absolute right-4 top-1/2 hidden h-44 w-64 -translate-y-1/2 opacity-90 md:block lg:right-8 lg:h-52 lg:w-80" />
				<div className="relative flex flex-col gap-5">
					<div className="space-y-3">
						<div className="inline-flex items-center gap-2 rounded-full border border-border/50 bg-background/45 px-2.5 py-1 text-xs font-medium text-muted-foreground backdrop-blur-sm">
							<GraduationCap className="size-3.5" />
							FlowLike University
						</div>
						<div className="space-y-1.5">
							<h1 className="max-w-2xl text-2xl font-semibold tracking-tight md:text-3xl">
								{greetingName ? (
									<>
										Welcome back,{" "}
										<span className="bg-linear-to-r from-sky-300 via-emerald-300 to-amber-200 bg-clip-text text-transparent">
											{greetingName}
										</span>
										.
									</>
								) : (
									"Learn flows by doing."
								)}
							</h1>
							<p className="max-w-xl text-sm leading-6 text-muted-foreground">
								Short, practical lessons paired with real apps. Build something
								useful and keep your momentum visible.
							</p>
						</div>
					</div>
					{stats && (
						<dl className="flex flex-wrap items-center gap-1.5">
							<HeroChip
								icon={BookOpenCheck}
								label={`${stats.enrolled} enrolled`}
								tone="primary"
							/>
							<HeroChip
								icon={GraduationCap}
								label={`${stats.completed} completed`}
								tone="green"
							/>
							<HeroChip
								icon={Award}
								label={`${stats.certificates} ${
									stats.certificates === 1 ? "certificate" : "certificates"
								}`}
								tone="amber"
							/>
							<HeroChip
								icon={Trophy}
								label={`${(stats.points ?? 0).toLocaleString()} pts`}
								tone="violet"
							/>
						</dl>
					)}
				</div>
			</motion.section>

			<section className="grid gap-3 grid-cols-[repeat(auto-fit,minmax(min(100%,16rem),1fr))]">
				<HubStripCard
					icon={Trophy}
					title="Leaderboard"
					value={`${(stats?.points ?? 0).toLocaleString()} pts`}
					description="Track your challenge points."
					tone="leaderboard"
					onClick={onOpenLeaderboard}
				/>
				<HubStripCard
					icon={Award}
					title="Certificates"
					value={`${stats?.certificates ?? 0}`}
					description="Review signed course certificates."
					tone="certificates"
					onClick={onOpenCertificates}
				/>
				{onOpenAuthoring && (
					<HubStripCard
						icon={Settings2}
						title="Course admin"
						value="Drafts"
						description="Manage learning content."
						tone="authoring"
						onClick={onOpenAuthoring}
					/>
				)}
			</section>

			{paths.length > 0 && (
				<section className="space-y-3">
					<div className="space-y-1">
						<h2 className="text-sm font-semibold tracking-tight">
							Learning paths
						</h2>
						<p className="text-xs text-muted-foreground">
							Curated journeys — finish one course to unlock the next.
						</p>
					</div>
					<div className="space-y-3">
						{paths.map((path) => (
							<LearningPathCard
								key={path.id}
								path={path}
								progressByCourseId={progressByCourseId}
								onSelectCourse={onSelect}
							/>
						))}
					</div>
				</section>
			)}

			{inProgress.length > 0 && !isFiltering && (
				<section className="space-y-3">
					<div className="flex items-end justify-between gap-2">
						<div>
							<h2 className="text-sm font-semibold tracking-tight">
								Continue learning
							</h2>
							<p className="text-xs text-muted-foreground">
								Pick up where you left off.
							</p>
						</div>
						{inProgress.length > 4 && (
							<span className="text-xs text-muted-foreground">
								Showing 4 of {inProgress.length}
							</span>
						)}
					</div>
					<div className="grid items-stretch gap-4 grid-cols-[repeat(auto-fill,minmax(min(100%,18rem),1fr))]">
						{inProgress.slice(0, 4).map((course, i) => (
							<CourseCard
								key={course.id}
								course={course}
								progressPct={progressByCourseId?.[course.id]}
								onSelect={onSelect}
								index={i}
							/>
						))}
					</div>
				</section>
			)}

			{availableCategories.length > 2 && (
				<TopicChips
					options={availableCategories}
					value={category}
					onChange={(v) => setCategory(v)}
				/>
			)}

			<section className="grid items-stretch gap-4 grid-cols-[repeat(auto-fill,minmax(min(100%,18rem),1fr))]">
				{(isFiltering ? filtered : remainingCourses).map((course, i) => (
					<CourseCard
						key={course.id}
						course={course}
						progressPct={progressByCourseId?.[course.id]}
						onSelect={onSelect}
						index={i}
					/>
				))}
				{(isFiltering ? filtered : remainingCourses).length === 0 && (
					<div className="sm:col-span-2 lg:col-span-3">
						<EmptyState
							title={
								isFiltering
									? "No courses match these filters"
									: "No courses yet"
							}
							description={
								isFiltering
									? "Try clearing some filters or searching for something different."
									: "Courses will show up here once published. Check back soon."
							}
							icons={[Compass, GraduationCap, Sparkles]}
							className="h-full min-h-[220px] max-w-none p-10"
						/>
					</div>
				)}
			</section>
		</div>
	);
}

interface TopicChipsProps {
	readonly options: ReadonlyArray<{
		readonly value: CourseCategory | "ALL";
		readonly label: string;
	}>;
	readonly value: CourseCategory | "ALL";
	readonly onChange: (value: CourseCategory | "ALL") => void;
}

function TopicChips({ options, value, onChange }: TopicChipsProps) {
	return (
		<div
			className="flex flex-wrap items-center gap-1.5"
			role="tablist"
			aria-label="Filter by topic"
		>
			{options.map((opt) => {
				const active = opt.value === value;
				return (
					<button
						key={opt.value}
						type="button"
						role="tab"
						aria-selected={active}
						onClick={() => onChange(opt.value)}
						className={cn(
							"inline-flex h-7 items-center rounded-full border px-3 text-xs font-medium transition-colors",
							active
								? "border-primary/60 bg-primary/10 text-foreground"
								: "border-border/60 bg-background/40 text-muted-foreground hover:border-border hover:bg-muted/50 hover:text-foreground",
						)}
					>
						{opt.label}
					</button>
				);
			})}
		</div>
	);
}

interface HeroChipProps {
	readonly icon: typeof Sparkles;
	readonly label: string;
	readonly tone: "primary" | "green" | "amber" | "violet";
}

const chipTones: Record<HeroChipProps["tone"], string> = {
	primary:
		"text-sky-900 bg-sky-100/80 ring-sky-300/70 dark:text-sky-100 dark:bg-sky-500/15 dark:ring-sky-400/30",
	green:
		"text-emerald-900 bg-emerald-100/80 ring-emerald-300/70 dark:text-emerald-100 dark:bg-emerald-500/15 dark:ring-emerald-400/30",
	amber:
		"text-amber-950 bg-amber-100/80 ring-amber-300/70 dark:text-amber-100 dark:bg-amber-500/15 dark:ring-amber-400/30",
	violet:
		"text-violet-950 bg-violet-100/80 ring-violet-300/70 dark:text-violet-100 dark:bg-violet-500/15 dark:ring-violet-400/30",
};

function HeroChip({ icon: Icon, label, tone }: HeroChipProps) {
	return (
		<span
			className={cn(
				"inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium ring-1",
				chipTones[tone],
			)}
		>
			<Icon className="size-3 text-muted-foreground" />
			{label}
		</span>
	);
}

interface HubStripCardProps {
	readonly icon: typeof Sparkles;
	readonly title: string;
	readonly value: string;
	readonly description: string;
	readonly tone: "leaderboard" | "certificates" | "authoring";
	readonly onClick?: () => void;
}

const hubCardTones: Record<
	HubStripCardProps["tone"],
	{
		readonly card: string;
		readonly icon: string;
		readonly value: string;
	}
> = {
	leaderboard: {
		card: "from-amber-500/10 via-card/95 to-card hover:border-amber-400/50",
		icon: "bg-amber-500/15 text-amber-500 ring-amber-400/25",
		value: "text-amber-500",
	},
	certificates: {
		card: "from-sky-500/10 via-card/95 to-card hover:border-sky-400/50",
		icon: "bg-sky-500/15 text-sky-400 ring-sky-400/25",
		value: "text-sky-400",
	},
	authoring: {
		card: "from-emerald-500/10 via-card/95 to-card hover:border-emerald-400/50",
		icon: "bg-emerald-500/15 text-emerald-400 ring-emerald-400/25",
		value: "text-emerald-400",
	},
};

function HubStripCard({
	icon: Icon,
	title,
	value,
	description,
	tone,
	onClick,
}: HubStripCardProps) {
	const style = hubCardTones[tone];

	return (
		<motion.button
			type="button"
			onClick={onClick}
			disabled={!onClick}
			initial={{ opacity: 0, y: 6 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.35, ease: "easeOut" }}
			whileTap={onClick ? { scale: 0.985 } : undefined}
			className={cn(
				"group relative flex items-center gap-3 overflow-hidden rounded-xl border border-border/60 bg-linear-to-br p-3 text-left shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				style.card,
				onClick ? "cursor-pointer" : "cursor-default",
			)}
		>
			<div
				className={cn(
					"grid size-9 shrink-0 place-items-center rounded-lg ring-1",
					style.icon,
				)}
			>
				<Icon className="size-4" />
			</div>
			<div className="min-w-0 flex-1">
				<div className="flex items-baseline gap-2">
					<h3 className="truncate text-sm font-semibold">{title}</h3>
					<span
						className={cn(
							"shrink-0 text-xs font-semibold tabular-nums",
							style.value,
						)}
					>
						{value}
					</span>
				</div>
				<p className="truncate text-xs text-muted-foreground">{description}</p>
			</div>
			<ArrowRight className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
		</motion.button>
	);
}

interface SegmentedSelectProps {
	readonly label: string;
	readonly options: ReadonlyArray<{
		readonly value: string;
		readonly label: string;
	}>;
	readonly value: string;
	readonly onChange: (value: string) => void;
}

function SegmentedSelect({
	label,
	options,
	value,
	onChange,
}: SegmentedSelectProps) {
	return (
		<div className="grid gap-2 md:grid-cols-[5.5rem_1fr] md:items-center">
			<span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
				{label}
			</span>
			<div className="flex flex-wrap gap-1.5">
				{options.map((opt) => {
					const active = opt.value === value;
					return (
						<button
							key={opt.value}
							type="button"
							onClick={() => onChange(opt.value)}
							className={cn(
								"rounded-md px-2.5 py-1.5 text-xs font-medium ring-1 transition-colors",
								active
									? "bg-background text-foreground ring-border shadow-sm"
									: "bg-transparent text-muted-foreground ring-border/50 hover:bg-muted/50 hover:text-foreground",
							)}
						>
							{opt.label}
						</button>
					);
				})}
			</div>
		</div>
	);
}
