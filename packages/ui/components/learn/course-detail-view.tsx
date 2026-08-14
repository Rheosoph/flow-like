"use client";
import { useTranslation } from "@flow-like/locales";
import {
	ArrowLeft,
	ArrowRight,
	Award,
	Check,
	Clock,
	PlayCircle,
	Workflow,
} from "lucide-react";
import { useMemo } from "react";
import type {
	CourseDetail,
	LessonSummary,
	ModuleWithLessons,
} from "../../lib/learn/types";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { CourseBoardGlyph } from "./course-board-glyph";

interface CourseDetailViewProps {
	readonly courseId: string;
	readonly course?: CourseDetail | null;
	readonly modules: ReadonlyArray<ModuleWithLessons>;
	readonly completedLessonIds: ReadonlySet<string>;
	readonly isEnrolled: boolean;
	readonly workspaceAppId?: string | null;
	readonly enrollPending?: boolean;
	readonly certificatePending?: boolean;
	readonly onBack: () => void;
	readonly onEnroll: () => void;
	readonly onOpenLesson: (moduleId: string, lessonId: string) => void;
	readonly onClaimCertificate: () => void;
	readonly onOpenWorkspace?: (appId: string) => void;
}

/**
 * The course page for every shell. Both apps used to keep their own copy, which
 * is how they drifted apart — routing and data stay in the page, the view lives
 * here.
 */
export function CourseDetailView({
	courseId,
	course,
	modules,
	completedLessonIds,
	isEnrolled,
	workspaceAppId,
	enrollPending = false,
	certificatePending = false,
	onBack,
	onEnroll,
	onOpenLesson,
	onClaimCertificate,
	onOpenWorkspace,
}: CourseDetailViewProps) {
	const { t } = useTranslation();

	const lessonCounts = useMemo(
		() =>
			modules.reduce(
				(counts, m) => ({
					all: counts.all + m.lessons.length,
					required:
						counts.required + m.lessons.filter((l) => !l.is_optional).length,
				}),
				{ all: 0, required: 0 },
			),
		[modules],
	);
	const completedRequiredLessons = useMemo(
		() =>
			modules.reduce(
				(sum, m) =>
					sum +
					m.lessons.filter(
						(l) => !l.is_optional && completedLessonIds.has(l.id),
					).length,
				0,
			),
		[completedLessonIds, modules],
	);
	const completedLessons = useMemo(
		() =>
			modules.reduce(
				(sum, m) =>
					sum + m.lessons.filter((l) => completedLessonIds.has(l.id)).length,
				0,
			),
		[completedLessonIds, modules],
	);
	const minutesLeft = useMemo(
		() =>
			modules.reduce(
				(sum, m) =>
					sum +
					m.lessons
						.filter((l) => !completedLessonIds.has(l.id))
						.reduce((acc, l) => acc + (l.estimated_minutes ?? 0), 0),
				0,
			),
		[completedLessonIds, modules],
	);

	const progressPct =
		lessonCounts.required === 0
			? lessonCounts.all > 0
				? 100
				: 0
			: Math.round((completedRequiredLessons / lessonCounts.required) * 100);

	/** The lesson to open next — first incomplete, in course order. */
	const nextLesson = useMemo(() => {
		for (const m of modules) {
			for (const l of m.lessons) {
				if (!completedLessonIds.has(l.id)) return { module: m, lesson: l };
			}
		}
		return null;
	}, [modules, completedLessonIds]);

	const difficultyLabel = course?.difficulty
		? course.difficulty.charAt(0) + course.difficulty.slice(1).toLowerCase()
		: "";
	const categoryLabel = (course?.category ?? "GENERAL")
		.replace(/_/g, " ")
		.toLowerCase();

	return (
		<div className="flex-1 overflow-auto">
			<div className="relative overflow-hidden border-b border-border/70">
				{/*
				 * The banner is texture, not content: masked to the top-right corner
				 * where no copy sits, then covered by a scrim so the title, blurb and
				 * meta row always read against the plain background.
				 */}
				{course?.banner_url && (
					<>
						<div
							className="absolute inset-0 bg-cover bg-center opacity-25 [mask-image:radial-gradient(90%_120%_at_100%_0%,#000_0%,transparent_62%)]"
							style={{ backgroundImage: `url(${course.banner_url})` }}
						/>
						<div className="absolute inset-0 bg-linear-to-r from-background via-background/80 to-background/25" />
						<div className="absolute inset-x-0 bottom-0 h-24 bg-linear-to-t from-background to-transparent" />
					</>
				)}
				<div className="absolute inset-0 [background-image:radial-gradient(var(--border)_1px,transparent_1px)] [background-size:22px_22px] [mask-image:radial-gradient(110%_100%_at_15%_0%,#000_10%,transparent_68%)]" />

				<div className="relative mx-auto max-w-6xl px-6 pb-7 pt-8 md:px-8 lg:px-10">
					<button
						type="button"
						onClick={onBack}
						className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
					>
						<ArrowLeft className="h-3.5 w-3.5" />
						{t("allCourses", "All courses")}
					</button>

					<div className="mt-5 flex flex-wrap items-center gap-2.5 font-mono text-[10px] uppercase tracking-wider">
						<span className="inline-flex items-center gap-2 text-primary">
							<span className="h-3 w-0.75 rounded-full bg-primary" />
							{categoryLabel}
						</span>
						<span className="h-3 w-px bg-border" />
						<span className="text-muted-foreground">{difficultyLabel}</span>
						{course?.estimated_minutes ? (
							<>
								<span className="h-3 w-px bg-border" />
								<span className="tabular-nums text-muted-foreground">
									{t("valMin", "{{val}} min", {
										val: course.estimated_minutes,
									})}
								</span>
							</>
						) : null}
					</div>

					<h1 className="mt-2.5 max-w-[18ch] text-3xl font-semibold leading-[1.05] tracking-tight text-balance md:text-[2.6rem]">
						{course?.name ?? courseId}
					</h1>

					{(course?.long_description || course?.description) && (
						<p className="mt-3.5 max-w-[62ch] whitespace-pre-wrap font-serif text-[17px] leading-[1.6] text-muted-foreground">
							{course?.long_description || course?.description}
						</p>
					)}

					<div className="mt-5 flex flex-wrap items-center gap-3">
						{!isEnrolled ? (
							<Button size="lg" disabled={enrollPending} onClick={onEnroll}>
								<PlayCircle className="mr-2 h-4 w-4" />
								{t("startCourse", "Start course")}
							</Button>
						) : nextLesson ? (
							<Button
								size="lg"
								onClick={() =>
									onOpenLesson(nextLesson.module.id, nextLesson.lesson.id)
								}
							>
								<PlayCircle className="mr-2 h-4 w-4" />
								{completedLessons === 0
									? t("startCourse", "Start course")
									: t("continue", "Continue")}
							</Button>
						) : (
							<Button
								size="lg"
								onClick={onClaimCertificate}
								disabled={certificatePending}
							>
								<Award className="mr-2 h-4 w-4" />
								{t("claimCertificate", "Claim certificate")}
							</Button>
						)}

						{workspaceAppId && onOpenWorkspace && (
							<Button
								size="lg"
								variant="outline"
								onClick={() => onOpenWorkspace(workspaceAppId)}
							>
								<Workflow className="mr-2 h-4 w-4" />
								{t("openTheWorkspace", "Open the workspace")}
							</Button>
						)}

						{isEnrolled && nextLesson && (
							<span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
								{t("nextVal", "Next: {{val}}", {
									val: nextLesson.lesson.title,
								})}
							</span>
						)}
					</div>
				</div>

				<div className="relative mx-auto max-w-6xl px-6 md:px-8 lg:px-10">
					<div className="flex flex-wrap border-t border-border/70">
						<MetaCell
							value={String(lessonCounts.all)}
							label={t("lessons", "Lessons")}
						/>
						<MetaCell
							value={String(modules.length)}
							label={t("modules", "Modules")}
						/>
						<MetaCell
							value={String(completedLessons)}
							label={t("completed", "Completed")}
						/>
						<MetaCell
							value={t("valMin", "{{val}} min", { val: minutesLeft })}
							label={t("remaining", "Remaining")}
						/>
						<MetaCell
							value={t("certificate", "Certificate")}
							label={t("onCompletion", "On completion")}
						/>
					</div>
				</div>
			</div>

			<div className="mx-auto grid max-w-6xl gap-10 px-6 py-9 md:px-8 lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start lg:px-10">
				<div className="min-w-0">
					{modules.length === 0 ? (
						<div className="rounded-xl border border-dashed border-border/70 p-8 text-center">
							<h2 className="text-sm font-semibold">
								{t("noContentYet", "No content yet")}
							</h2>
							<p className="mt-1 text-sm text-muted-foreground">
								{t(
									"thisCourseHasNoModulesYet",
									"This course has no modules yet. Check back soon.",
								)}
							</p>
						</div>
					) : (
						<>
							{modules.map((m, index) => (
								<ModuleSection
									key={m.id}
									module={m}
									position={index + 1}
									completedLessonIds={completedLessonIds}
									nextLessonId={nextLesson?.lesson.id ?? null}
									isLast={index === modules.length - 1}
									onOpen={(lessonId) => onOpenLesson(m.id, lessonId)}
								/>
							))}
							<div className="relative flex items-center gap-3 pl-8 pt-4">
								<span className="absolute left-2.5 top-0 h-[calc(50%+8px)] w-px border-l border-dashed border-border" />
								<span className="absolute left-0.5 top-[calc(50%+8px)] grid size-4 -translate-y-1/2 place-items-center rounded-full border-2 border-border bg-background text-muted-foreground">
									<Award className="size-2" />
								</span>
								<div>
									<p className="text-sm font-semibold">
										{t("certificateOfCompletion", "Certificate of completion")}
									</p>
									<p className="text-xs text-muted-foreground">
										{t(
											"signedAndAddedToYourProfile",
											"Signed and added to your profile once every required lesson is done.",
										)}
									</p>
								</div>
							</div>
						</>
					)}
				</div>

				<aside className="flex flex-col gap-4 lg:sticky lg:top-6">
					<section className="rounded-xl border border-border/70 bg-card">
						<header className="border-b border-border/60 bg-muted/40 px-3.5 py-2.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
							{t("yourProgress", "Your progress")}
						</header>
						<div className="flex flex-col gap-3.5 p-3.5">
							<div className="flex items-center gap-3.5">
								<ProgressRing value={progressPct} />
								<div>
									<p className="text-xl font-semibold tracking-tight tabular-nums">
										{`${progressPct}%`}
									</p>
									<p className="text-xs text-muted-foreground">
										{t("valOfValLessons", "{{done}} of {{total}} lessons", {
											done: completedLessons,
											total: lessonCounts.all,
										})}
									</p>
								</div>
							</div>
							<dl className="flex flex-col gap-2 text-sm">
								<TallyRow
									done={completedRequiredLessons >= lessonCounts.required}
									label={t("requiredLessons", "Required lessons")}
									value={`${completedRequiredLessons} / ${lessonCounts.required}`}
								/>
								<TallyRow
									label={t("timeRemaining", "Time remaining")}
									value={t("valMin", "{{val}} min", { val: minutesLeft })}
								/>
								<TallyRow
									done={progressPct === 100}
									label={t("certificate", "Certificate")}
									value={
										progressPct === 100
											? t("ready", "Ready")
											: t("locked", "Locked")
									}
								/>
							</dl>
							{progressPct === 100 && isEnrolled && (
								<Button
									size="sm"
									onClick={onClaimCertificate}
									disabled={certificatePending}
								>
									<Award className="mr-2 h-3.5 w-3.5" />
									{t("claimCertificate", "Claim certificate")}
								</Button>
							)}
						</div>
					</section>

					<section className="overflow-hidden rounded-xl border border-border/70 bg-card">
						<header className="border-b border-border/60 bg-muted/40 px-3.5 py-2.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
							{t("workspace", "Workspace")}
						</header>
						<div className="relative h-36 border-b border-border/60">
							<CourseBoardGlyph seed={courseId} accent={isEnrolled} />
						</div>
						<div className="flex items-center gap-2.5 p-3.5">
							<Workflow className="size-4 shrink-0 text-muted-foreground" />
							<p className="min-w-0 flex-1 text-xs text-muted-foreground">
								{isEnrolled
									? t(
											"clonedIntoYourLibraryChallengesReadThisBoard",
											"Cloned into your library. Challenges read this board.",
										)
									: t(
											"aSandboxAppClonesWhenYouStart",
											"A sandbox app clones into your library when you start.",
										)}
							</p>
							{workspaceAppId && onOpenWorkspace && (
								<button
									type="button"
									onClick={() => onOpenWorkspace(workspaceAppId)}
									className="inline-flex shrink-0 items-center gap-1.5 text-xs font-semibold text-primary hover:underline"
								>
									{t("open", "Open")}
									<ArrowRight className="size-3" />
								</button>
							)}
						</div>
					</section>

					{(course?.tags?.length ?? 0) > 0 && (
						<div className="flex flex-wrap gap-1.5">
							{course?.tags.map((tag) => (
								<Badge
									key={tag}
									variant="outline"
									className="font-mono text-[10px] uppercase tracking-wider"
								>
									{tag}
								</Badge>
							))}
						</div>
					)}
				</aside>
			</div>
		</div>
	);
}

function MetaCell({
	value,
	label,
}: {
	readonly value: string;
	readonly label: string;
}) {
	return (
		<div className="mr-6 flex flex-col gap-0.5 border-r border-border/70 py-3.5 pr-6 last:mr-0 last:border-r-0 last:pr-0">
			<span className="text-[15px] font-semibold tracking-tight tabular-nums">
				{value}
			</span>
			<span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
				{label}
			</span>
		</div>
	);
}

function ProgressRing({ value }: { readonly value: number }) {
	const circumference = 2 * Math.PI * 24;
	return (
		<svg width="58" height="58" viewBox="0 0 58 58" aria-hidden="true">
			<title>Progress</title>
			<circle
				cx="29"
				cy="29"
				r="24"
				fill="none"
				stroke="var(--border)"
				strokeWidth="6"
			/>
			<circle
				cx="29"
				cy="29"
				r="24"
				fill="none"
				stroke="var(--primary)"
				strokeWidth="6"
				strokeLinecap="round"
				strokeDasharray={circumference}
				strokeDashoffset={circumference * (1 - Math.min(100, value) / 100)}
				transform="rotate(-90 29 29)"
			/>
		</svg>
	);
}

function TallyRow({
	label,
	value,
	done = false,
}: {
	readonly label: string;
	readonly value: string;
	readonly done?: boolean;
}) {
	return (
		<div className="flex items-center gap-2.5">
			<span
				className={
					done
						? "text-emerald-600 dark:text-emerald-400"
						: "text-muted-foreground"
				}
			>
				{done ? <Check className="size-3.5" /> : <Clock className="size-3.5" />}
			</span>
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="ml-auto font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
				{value}
			</dd>
		</div>
	);
}

interface ModuleSectionProps {
	readonly module: ModuleWithLessons;
	readonly position: number;
	readonly completedLessonIds: ReadonlySet<string>;
	readonly nextLessonId: string | null;
	readonly isLast: boolean;
	readonly onOpen: (lessonId: string) => void;
}

function ModuleSection({
	module: m,
	position,
	completedLessonIds,
	nextLessonId,
	isLast,
	onOpen,
}: ModuleSectionProps) {
	const { t } = useTranslation();
	const doneCount = m.lessons.filter((l) =>
		completedLessonIds.has(l.id),
	).length;
	const state =
		doneCount === m.lessons.length && m.lessons.length > 0
			? "complete"
			: doneCount > 0
				? "active"
				: "upcoming";

	return (
		<section className="mb-7">
			<div className="flex items-baseline gap-3">
				<span className="font-mono text-xs tabular-nums text-muted-foreground">
					{String(position).padStart(2, "0")}
				</span>
				<h2 className="text-[19px] font-semibold tracking-tight">{m.title}</h2>
				<span className="ml-auto font-mono text-[10px] uppercase tracking-wider">
					{state === "complete" && (
						<span className="text-emerald-600 dark:text-emerald-400">
							{t("complete", "Complete")}
						</span>
					)}
					{state === "active" && (
						<span className="text-primary">
							{t("inProgress", "In progress")}
						</span>
					)}
					{state === "upcoming" && (
						<span className="text-muted-foreground">
							{t("notStarted", "Not started")}
						</span>
					)}
				</span>
			</div>
			{m.description && (
				<p className="ml-8 mt-1 max-w-[60ch] font-serif text-[15px] leading-normal text-muted-foreground">
					{m.description}
				</p>
			)}

			<ul className="mt-3">
				{m.lessons.map((lesson, index) => (
					<LessonRow
						key={lesson.id}
						lesson={lesson}
						done={completedLessonIds.has(lesson.id)}
						isNext={lesson.id === nextLessonId}
						isFirst={index === 0}
						isLast={index === m.lessons.length - 1 && !isLast}
						onOpen={() => onOpen(lesson.id)}
					/>
				))}
			</ul>
		</section>
	);
}

interface LessonRowProps {
	readonly lesson: LessonSummary;
	readonly done: boolean;
	readonly isNext: boolean;
	readonly isFirst: boolean;
	readonly isLast: boolean;
	readonly onOpen: () => void;
}

function LessonRow({
	lesson,
	done,
	isNext,
	isFirst,
	isLast,
	onOpen,
}: LessonRowProps) {
	const { t } = useTranslation();

	return (
		<li className="relative">
			<span
				aria-hidden="true"
				className={cn(
					"absolute left-2.5 w-px",
					isFirst ? "top-1/2" : "top-0",
					isLast ? "bottom-1/2" : "bottom-0",
					done || isNext
						? "bg-primary"
						: "border-l border-dashed border-border",
				)}
			/>
			<button
				type="button"
				onClick={onOpen}
				className={cn(
					"relative flex w-full items-center gap-3 rounded-lg py-3 pl-8 pr-3 text-left transition-colors",
					isNext ? "bg-primary/5" : "hover:bg-muted/50",
				)}
			>
				<span
					className={cn(
						"absolute left-0.5 top-1/2 grid size-4 -translate-y-1/2 place-items-center rounded-full bg-background",
						done
							? "text-emerald-600 dark:text-emerald-400"
							: isNext
								? "text-primary"
								: "text-muted-foreground/50",
					)}
				>
					{done ? (
						<Check className="size-3" strokeWidth={3} />
					) : lesson.has_video ? (
						<PlayCircle className="size-3.5" />
					) : (
						<span className="size-2 rounded-full bg-current" />
					)}
				</span>

				<span className="min-w-0 flex-1">
					<span
						className={cn(
							"block truncate text-[15px] font-medium",
							done && "text-muted-foreground",
						)}
					>
						{lesson.title}
					</span>
					<span className="mt-0.5 flex items-center gap-2 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
						{isNext
							? t("upNext", "Up next")
							: lesson.has_video
								? t("watch", "Watch")
								: t("read", "Read")}
						{lesson.is_optional && (
							<span className="rounded border border-border/60 px-1.5 py-px">
								{t("optional", "Optional")}
							</span>
						)}
					</span>
				</span>

				<span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">
					{t("valMin", "{{val}} min", { val: lesson.estimated_minutes })}
				</span>

				{isNext && (
					<span className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-primary px-2.5 py-1 text-xs font-semibold text-primary-foreground">
						{t("open", "Open")}
						<ArrowRight className="size-3" />
					</span>
				)}
			</button>
		</li>
	);
}
