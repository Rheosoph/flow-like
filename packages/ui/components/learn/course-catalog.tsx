"use client";
import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import {
	ArrowRight,
	Award,
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
import { useSearch } from "../../hooks/use-search-index";
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
import { CourseBoardGlyph } from "./course-board-glyph";
import { CourseCard } from "./course-card";
import { LearningPathCard } from "./learning-path-card";

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

/** Difficulty is the only ordering the catalogue actually knows, so it is what groups the shelf. */
const TIERS: ReadonlyArray<{
	readonly id: string;
	readonly levels: ReadonlyArray<CourseDifficulty>;
}> = [
	{ id: "foundation", levels: ["BEGINNER"] },
	{ id: "systems", levels: ["INTERMEDIATE"] },
	{ id: "depth", levels: ["ADVANCED", "EXPERT"] },
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
	const { t } = useTranslation();
	const [query, setQuery] = useState("");
	const [category, setCategory] =
		useState<(typeof categories)[number]["value"]>("ALL");
	const [difficulty, setDifficulty] =
		useState<(typeof difficulties)[number]>("ALL");

	const matched = useSearch(courses, query, {
		fields: ["name", "description", "tags"],
		boost: { name: 3, tags: 1.5 },
	});

	const filtered = useMemo(
		() =>
			matched.filter((c) => {
				if (category !== "ALL" && c.category !== category) return false;
				if (difficulty !== "ALL" && c.difficulty !== difficulty) return false;
				return true;
			}),
		[matched, category, difficulty],
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

	const isFiltering =
		query.trim().length > 0 || difficulty !== "ALL" || category !== "ALL";
	const activeFilterCount =
		(difficulty !== "ALL" ? 1 : 0) + (category !== "ALL" ? 1 : 0);
	const clearFilters = () => {
		setDifficulty("ALL");
		setCategory("ALL");
	};

	const firstVisit = (stats?.enrolled ?? 0) === 0;
	const greetingName = displayName?.split(" ")[0] || displayName || null;

	const totalMinutes = useMemo(
		() => courses.reduce((sum, c) => sum + (c.estimated_minutes ?? 0), 0),
		[courses],
	);

	/** The one course a brand-new learner should open: step one of a path, else the shortest beginner course. */
	const recommended = useMemo(() => {
		const firstStep = [...paths]
			.sort((a, b) => a.position - b.position)
			.flatMap((p) => [...p.steps].sort((a, b) => a.position - b.position))
			.find((s) => s.course != null)?.course;
		if (firstStep) return firstStep;
		return (
			[...courses]
				.filter((c) => c.difficulty === "BEGINNER")
				.sort((a, b) => a.estimated_minutes - b.estimated_minutes)[0] ??
			courses[0] ??
			null
		);
	}, [paths, courses]);

	const inProgress = useMemo(() => {
		const map = progressByCourseId ?? {};
		return courses.filter((c) => {
			const p = map[c.id];
			return p !== undefined && p > 0 && p < 1;
		});
	}, [courses, progressByCourseId]);

	const resume = inProgress[0] ?? null;

	const tiers = useMemo(
		() =>
			TIERS.map((tier) => ({
				...tier,
				items: filtered.filter((c) =>
					tier.levels.includes(c.difficulty as CourseDifficulty),
				),
			})).filter((tier) => tier.items.length > 0),
		[filtered],
	);

	const tierCopy: Record<string, { title: string; note: string }> = {
		foundation: {
			title: t("foundation", "Foundation"),
			note: t(
				"buildOneThingEndToEndBeforeYouBuildMany",
				"Build one thing end to end before you build many.",
			),
		},
		systems: {
			title: t("systems", "Systems"),
			note: t(
				"giveItDataEntryPointsAndOtherPeople",
				"Give it data, entry points, and other people.",
			),
		},
		depth: {
			title: t("depth", "Depth"),
			note: t(
				"whereFlowLikeStopsBeingAToolAndStartsBeingAPlatform",
				"Where Flow-Like stops being a tool and starts being a platform.",
			),
		},
	};

	return (
		<div className="flex flex-col gap-8">
			<header className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
				<div className="flex items-baseline gap-3">
					<h1 className="text-lg font-semibold tracking-tight">
						{t("university", "University")}
					</h1>
					<span className="font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
						{t("valCoursesValHours", "{{courses}} courses · {{hours}} h", {
							courses: courses.length,
							hours: Math.max(1, Math.round(totalMinutes / 60)),
						})}
					</span>
				</div>
				<div className="flex w-full gap-2 md:w-auto">
					<div className="relative min-w-0 flex-1 md:w-80">
						<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={query}
							onChange={(e) => setQuery(e.target.value)}
							placeholder={t(
								"searchWhatYouWantToBuild",
								"Search what you want to build",
							)}
							className="h-9 rounded-lg border-border/60 pl-9"
						/>
					</div>
					<Popover>
						<PopoverTrigger asChild>
							<button
								type="button"
								className={cn(
									"inline-flex h-9 shrink-0 items-center gap-2 rounded-lg border border-border/60 px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground",
									activeFilterCount > 0 &&
										"border-primary/40 bg-primary/10 text-foreground",
								)}
							>
								<SlidersHorizontal className="size-4" />
								<span className="hidden sm:inline">
									{t("filters", "Filters")}
								</span>
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
									<h3 className="text-sm font-semibold">
										{t("refineCourses", "Refine courses")}
									</h3>
									<p className="text-xs text-muted-foreground">
										{t(
											"filterByDifficultyOrTopic",
											"Filter by difficulty or topic.",
										)}
									</p>
								</div>
								{activeFilterCount > 0 && (
									<button
										type="button"
										onClick={clearFilters}
										className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-muted-foreground hover:bg-muted/60 hover:text-foreground"
									>
										<X className="size-3" />
										{t("clear", "Clear")}
									</button>
								)}
							</div>
							<div className="space-y-4">
								<SegmentedSelect
									label={t("difficulty", "Difficulty")}
									options={difficulties.map((d) => ({
										value: d,
										label:
											d === "ALL"
												? t("all", "All")
												: d.charAt(0) + d.slice(1).toLowerCase(),
									}))}
									value={difficulty}
									onChange={(v) =>
										setDifficulty(v as (typeof difficulties)[number])
									}
								/>
								<SegmentedSelect
									label={t("topic", "Topic")}
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

			<section className="flex flex-col gap-3">
				<span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
					{greetingName && !firstVisit
						? t("welcomeBackVal", "Welcome back, {{val}}", {
								val: greetingName,
							})
						: t("flowLikeUniversity", "Flow-Like University")}
				</span>

				{firstVisit && (
					<>
						<h2 className="max-w-[18ch] text-3xl font-semibold leading-[1.05] tracking-tight text-balance md:text-4xl">
							{t(
								"learnFlowLikeByBuildingIt",
								"Learn Flow-Like by building it.",
							)}
						</h2>
						<p className="max-w-[58ch] font-serif text-[17.5px] leading-[1.6] text-muted-foreground">
							{t(
								"everyCourseShipsARealApp",
								"Every course ships a real App that clones into your library when you enroll. The lessons are the commentary — the board is the deliverable.",
							)}
						</p>
					</>
				)}

				<div className="flex flex-wrap items-center gap-x-6 gap-y-1.5 pt-1 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
					{firstVisit ? (
						<>
							<Fact
								value={courses.length}
								label={t("appsToBuild", "to build")}
							/>
							<Fact
								value={Math.max(1, Math.round(totalMinutes / 60))}
								label={t("hoursOfMaterial", "hours of material")}
							/>
							{paths.length > 0 && (
								<Fact
									value={paths.length}
									label={t("learningPaths", "learning paths")}
								/>
							)}
							<span>
								{t("runsLocallyOrInTheCloud", "Runs locally or in the cloud")}
							</span>
						</>
					) : (
						<>
							<Fact value={stats?.completed ?? 0} label={t("built", "built")} />
							<Fact
								value={inProgress.length}
								label={t("inProgress", "in progress")}
							/>
							<Fact
								value={stats?.certificates ?? 0}
								label={t("certificates", "certificates")}
							/>
							<Fact value={stats?.points ?? 0} label={t("points", "points")} />
						</>
					)}
				</div>
			</section>

			{firstVisit && recommended && (
				<FeatureCourse
					course={recommended}
					kicker={t(
						"startHereNothingToSetUp",
						"Start here · nothing to set up",
					)}
					action={t("startCourse", "Start course")}
					onSelect={onSelect}
				/>
			)}

			{!firstVisit && resume && (
				<FeatureCourse
					course={resume}
					kicker={t("inProgress", "In progress")}
					action={t("continue", "Continue")}
					onSelect={onSelect}
				/>
			)}

			{paths.length > 0 && !isFiltering && (
				<section className="flex flex-col gap-3">
					<div className="flex items-baseline gap-3">
						<h2 className="text-sm font-semibold tracking-tight">
							{t("learningPaths", "Learning paths")}
						</h2>
						<p className="text-xs text-muted-foreground">
							{t(
								"finishACourseToEnergizeTheNextWire",
								"Finish a course to energize the next wire.",
							)}
						</p>
					</div>
					<div className="flex flex-col gap-3">
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

			{availableCategories.length > 2 && (
				<TopicChips
					options={availableCategories}
					value={category}
					onChange={(v) => setCategory(v)}
				/>
			)}

			{filtered.length === 0 ? (
				<EmptyState
					title={
						isFiltering
							? t(
									"noCoursesMatchTheseFilters",
									"No courses match these filters",
								)
							: t("noCoursesYet", "No courses yet")
					}
					description={
						isFiltering
							? t(
									"tryClearingSomeFilters",
									"Try clearing some filters or searching for something different.",
								)
							: t(
									"coursesWillShowUpHereOncePublished",
									"Courses will show up here once published. Check back soon.",
								)
					}
					icons={[Compass, GraduationCap, Sparkles]}
					className="min-h-55 max-w-none p-10"
				/>
			) : isFiltering ? (
				<section className="flex flex-col gap-3">
					<div className="flex items-baseline gap-3 border-b border-border/60 pb-3">
						<h2 className="text-sm font-semibold tracking-tight">
							{t("matchingCourses", "Matching courses")}
						</h2>
						<span className="font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
							{filtered.length}
						</span>
					</div>
					<CourseGrid
						courses={filtered}
						progressByCourseId={progressByCourseId}
						onSelect={onSelect}
					/>
				</section>
			) : (
				<div className="flex flex-col gap-9">
					{tiers.map((tier) => (
						<section key={tier.id} className="flex flex-col gap-3">
							<div className="flex items-baseline gap-3">
								<span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
									{tierCopy[tier.id]?.title}
								</span>
								<span className="h-px flex-1 bg-border" />
								<span className="hidden font-mono text-[10px] uppercase tracking-wider text-muted-foreground sm:inline">
									{tierCopy[tier.id]?.note}
								</span>
							</div>
							<CourseGrid
								courses={tier.items}
								progressByCourseId={progressByCourseId}
								onSelect={onSelect}
								recommendedId={firstVisit ? recommended?.id : undefined}
							/>
						</section>
					))}
				</div>
			)}

			<section className="grid gap-3 pb-2 sm:grid-cols-2 lg:grid-cols-3">
				<UtilityLink
					icon={Trophy}
					title={t("leaderboard", "Leaderboard")}
					description={
						firstVisit
							? t("youreUnranked", "You're unranked — finish a challenge")
							: t("valPts", "{{val}} pts", { val: stats?.points ?? 0 })
					}
					onClick={onOpenLeaderboard}
				/>
				<UtilityLink
					icon={Award}
					title={t("certificates", "Certificates")}
					description={
						(stats?.certificates ?? 0) === 0
							? t(
									"noneYetFinishACourse",
									"None yet — finish a course to earn one",
								)
							: t("valSigned", "{{val}} signed", { val: stats?.certificates })
					}
					onClick={onOpenCertificates}
				/>
				{onOpenAuthoring && (
					<UtilityLink
						icon={Settings2}
						title={t("courseAdmin", "Course admin")}
						description={t("manageLearningContent", "Manage learning content")}
						onClick={onOpenAuthoring}
					/>
				)}
			</section>
		</div>
	);
}

function Fact({
	value,
	label,
}: {
	readonly value: number;
	readonly label: string;
}) {
	return (
		<span className="inline-flex items-baseline gap-1.5">
			<span className="text-base font-semibold tracking-tight tabular-nums text-foreground">
				{value.toLocaleString()}
			</span>
			{label}
		</span>
	);
}

interface CourseGridProps {
	readonly courses: ReadonlyArray<CourseListItem>;
	readonly progressByCourseId?: Record<string, number>;
	readonly onSelect?: (course: CourseListItem) => void;
	readonly recommendedId?: string;
}

function CourseGrid({
	courses,
	progressByCourseId,
	onSelect,
	recommendedId,
}: CourseGridProps) {
	return (
		<div className="grid grid-cols-1 items-stretch gap-4 md:grid-cols-2 xl:grid-cols-3">
			{courses.map((course, i) => (
				<CourseCard
					key={course.id}
					course={course}
					progressPct={progressByCourseId?.[course.id]}
					onSelect={onSelect}
					index={i}
					recommended={course.id === recommendedId}
				/>
			))}
		</div>
	);
}

interface FeatureCourseProps {
	readonly course: CourseListItem;
	readonly kicker: string;
	readonly action: string;
	readonly onSelect?: (course: CourseListItem) => void;
}

/** The single course the learner should open next — the artifact first, the words second. */
function FeatureCourse({
	course,
	kicker,
	action,
	onSelect,
}: FeatureCourseProps) {
	const { t } = useTranslation();
	const [bannerFailed, setBannerFailed] = useState(false);
	const showBanner = Boolean(course.banner_url) && !bannerFailed;
	const difficultyLabel =
		course.difficulty.charAt(0) + course.difficulty.slice(1).toLowerCase();

	return (
		<motion.section
			initial={{ opacity: 0, y: 8 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.4, ease: "easeOut" }}
			className="grid gap-5 overflow-hidden rounded-xl border border-border/70 border-l-2 border-l-primary bg-card p-4 md:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] md:items-center md:p-5"
		>
			<div className="relative aspect-video overflow-hidden rounded-lg border border-border/60">
				{showBanner ? (
					<img
						src={course.banner_url ?? undefined}
						alt=""
						loading="lazy"
						decoding="async"
						draggable={false}
						onError={() => setBannerFailed(true)}
						className="size-full object-cover object-center"
					/>
				) : (
					<CourseBoardGlyph seed={course.id} accent />
				)}
			</div>

			<div className="flex flex-col gap-2">
				<span className="font-mono text-[10px] uppercase tracking-wider text-primary">
					{kicker}
				</span>
				<h2 className="text-xl font-semibold leading-tight tracking-tight text-balance md:text-2xl">
					{course.name ?? course.id}
				</h2>
				{course.description && (
					<p className="max-w-[56ch] text-sm leading-relaxed text-muted-foreground">
						{course.description}
					</p>
				)}
				<div className="mt-1 flex flex-wrap items-center gap-3">
					<button
						type="button"
						onClick={() => onSelect?.(course)}
						className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2.5 text-sm font-semibold text-primary-foreground transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						{action}
						<ArrowRight className="size-4" />
					</button>
					<span className="font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
						{course.estimated_minutes > 0
							? t("valMinValDifficulty", "{{min}} min · {{difficulty}}", {
									min: course.estimated_minutes,
									difficulty: difficultyLabel,
								})
							: difficultyLabel}
					</span>
				</div>
			</div>
		</motion.section>
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
			className="flex flex-wrap items-center gap-2"
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
							"inline-flex h-8 items-center rounded-full border px-3.5 text-xs font-medium transition-colors",
							active
								? "border-foreground bg-foreground text-background"
								: "border-border/60 text-muted-foreground hover:border-border hover:text-foreground",
						)}
					>
						{opt.label}
					</button>
				);
			})}
		</div>
	);
}

interface UtilityLinkProps {
	readonly icon: typeof Sparkles;
	readonly title: string;
	readonly description: string;
	readonly onClick?: () => void;
}

function UtilityLink({
	icon: Icon,
	title,
	description,
	onClick,
}: UtilityLinkProps) {
	return (
		<button
			type="button"
			onClick={onClick}
			disabled={!onClick}
			className={cn(
				"group flex items-center gap-3 rounded-lg border border-border/70 p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				onClick
					? "cursor-pointer hover:border-border hover:bg-card"
					: "cursor-default",
			)}
		>
			<Icon className="size-4 shrink-0 text-muted-foreground" />
			<span className="min-w-0 flex-1">
				<span className="block truncate text-sm font-semibold">{title}</span>
				<span className="block truncate text-xs text-muted-foreground">
					{description}
				</span>
			</span>
			<ArrowRight className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
		</button>
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
									? "bg-background text-foreground ring-border"
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
