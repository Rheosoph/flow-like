"use client";
import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import { ArrowRight, Check, Lock } from "lucide-react";
import { Fragment } from "react";
import type { CourseListItem, LearningPath } from "../../lib/learn/types";
import { cn } from "../../lib/utils";

interface LearningPathCardProps {
	readonly path: LearningPath;
	readonly progressByCourseId?: Record<string, number>;
	readonly onSelectCourse?: (course: CourseListItem) => void;
}

type StepState = "completed" | "active" | "upcoming";

export function LearningPathCard({
	path,
	progressByCourseId,
	onSelectCourse,
}: LearningPathCardProps) {
	const { t } = useTranslation();
	const progress = progressByCourseId ?? {};
	const ordered = [...path.steps].sort((a, b) => a.position - b.position);

	const stepStates: ReadonlyArray<StepState> = ordered.map((step) => {
		const p = step.course ? (progress[step.course.id] ?? 0) : 0;
		if (p >= 1) return "completed";
		if (p > 0) return "active";
		return "upcoming";
	});

	const firstUnfinished = stepStates.findIndex((s) => s !== "completed");
	const totalCompleted = stepStates.filter((s) => s === "completed").length;

	return (
		<motion.article
			initial={{ opacity: 0, y: 6 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.35, ease: "easeOut" }}
			className="rounded-xl border border-border/70 bg-card p-4 md:p-5"
		>
			<header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
				<h3 className="text-base font-semibold tracking-tight">{path.title}</h3>
				{path.description && (
					<p className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
						{path.description}
					</p>
				)}
				<span className="ml-auto font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
					{t("valOfValComplete", "{{done}} / {{total}} complete", {
						done: totalCompleted,
						total: ordered.length,
					})}
				</span>
			</header>

			<ol className="mt-4 flex flex-col lg:flex-row lg:items-stretch">
				{ordered.map((step, index) => {
					const state = stepStates[index] ?? "upcoming";
					const previousState = stepStates[index - 1];
					const isLocked =
						state === "upcoming" &&
						previousState !== undefined &&
						previousState !== "completed";
					const isNext = index === firstUnfinished && !isLocked;

					return (
						<Fragment key={`${step.course_id}-${step.position}`}>
							{index > 0 && <StepWire live={previousState === "completed"} />}
							<PathStep
								step={step}
								state={state}
								isLocked={isLocked}
								isNext={isNext}
								onSelectCourse={onSelectCourse}
							/>
						</Fragment>
					);
				})}
			</ol>
		</motion.article>
	);
}

/** The wire between two steps. Energized once the step before it is done. */
function StepWire({ live }: { readonly live: boolean }) {
	return (
		<li
			aria-hidden="true"
			className="relative h-5 shrink-0 lg:h-auto lg:w-8"
			role="presentation"
		>
			<span
				className={cn(
					"absolute left-4 top-0 bottom-0 w-px lg:hidden",
					live ? "bg-primary" : "border-l border-dashed border-border",
				)}
			/>
			<span
				className={cn(
					"absolute left-0 right-0 top-1/2 hidden -translate-y-1/2 lg:block",
					live
						? "h-0.5 bg-primary"
						: "h-px border-t border-dashed border-border",
				)}
			/>
		</li>
	);
}

interface PathStepProps {
	readonly step: {
		readonly course_id: string;
		readonly course: CourseListItem | null;
	};
	readonly state: StepState;
	readonly isLocked: boolean;
	readonly isNext: boolean;
	readonly onSelectCourse?: (course: CourseListItem) => void;
}

function PathStep({
	step,
	state,
	isLocked,
	isNext,
	onSelectCourse,
}: PathStepProps) {
	const { t } = useTranslation();
	const course = step.course;
	const interactive = !isLocked && course != null && onSelectCourse != null;

	return (
		<li
			className={cn("relative min-w-0", isNext ? "lg:flex-[1.6]" : "lg:flex-1")}
		>
			<button
				type="button"
				onClick={() => {
					if (interactive && course) onSelectCourse(course);
				}}
				disabled={!interactive}
				className={cn(
					"group relative flex size-full flex-col gap-1.5 rounded-lg border p-3 text-left transition-colors",
					isLocked && "border-dashed border-border/60 bg-transparent",
					!isLocked && state === "completed" && "border-border/70 bg-card",
					!isLocked && state !== "completed" && "border-border/70 bg-card",
					isNext && "border-primary bg-primary/5",
					interactive && !isNext && "hover:border-border",
					!interactive && "cursor-default",
				)}
			>
				<PathPin state={state} isLocked={isLocked} side="left" />
				<PathPin state={state} isLocked={isLocked} side="right" />

				<span className="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider">
					{state === "completed" && (
						<span className="inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
							<Check className="size-3" />
							{t("complete", "Complete")}
						</span>
					)}
					{state === "active" && (
						<span className="text-primary">
							{t("inProgress", "In progress")}
						</span>
					)}
					{state === "upcoming" && !isLocked && (
						<span className="text-muted-foreground">
							{t("upNext", "Up next")}
						</span>
					)}
					{isLocked && (
						<span className="inline-flex items-center gap-1 text-muted-foreground">
							<Lock className="size-3" />
							{t("locked", "Locked")}
						</span>
					)}
				</span>

				<span
					className={cn(
						"line-clamp-2 text-sm font-semibold leading-tight tracking-tight",
						isLocked && "text-muted-foreground",
					)}
				>
					{course?.name ?? t("untitledCourse", "Untitled course")}
				</span>

				<span className="font-mono text-[10px] uppercase tracking-wider tabular-nums text-muted-foreground">
					{course?.estimated_minutes
						? t("valMin", "{{val}} min", { val: course.estimated_minutes })
						: null}
				</span>

				{isNext && interactive && (
					<span className="mt-1 inline-flex w-fit items-center gap-1.5 rounded-md bg-primary px-2.5 py-1 text-xs font-semibold text-primary-foreground">
						{t("continue", "Continue")}
						<ArrowRight className="size-3" />
					</span>
				)}
			</button>
		</li>
	);
}

function PathPin({
	state,
	isLocked,
	side,
}: {
	readonly state: StepState;
	readonly isLocked: boolean;
	readonly side: "left" | "right";
}) {
	return (
		<span
			aria-hidden="true"
			className={cn(
				"absolute top-1/2 hidden size-2.5 -translate-y-1/2 rounded-full border-2 bg-background lg:block",
				side === "left" ? "-left-1.5" : "-right-1.5",
				state === "completed" &&
					"border-emerald-500 bg-emerald-500 dark:border-emerald-400 dark:bg-emerald-400",
				state === "active" && "border-primary bg-primary",
				(state === "upcoming" || isLocked) && "border-border",
			)}
		/>
	);
}
