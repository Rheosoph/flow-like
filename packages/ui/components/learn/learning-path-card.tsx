"use client";
import { motion } from "framer-motion";
import { Check, ChevronRight, Lock, Route } from "lucide-react";
import type { CourseListItem, LearningPath } from "../../lib/learn/types";
import { cn } from "../../lib/utils";

interface LearningPathCardProps {
	readonly path: LearningPath;
	readonly progressByCourseId?: Record<string, number>;
	readonly onSelectCourse?: (course: CourseListItem) => void;
}

export function LearningPathCard({
	path,
	progressByCourseId,
	onSelectCourse,
}: LearningPathCardProps) {
	const progress = progressByCourseId ?? {};
	const ordered = [...path.steps].sort((a, b) => a.position - b.position);

	const stepStates = ordered.map((step) => {
		const p = step.course ? (progress[step.course.id] ?? 0) : 0;
		if (p >= 1) return "completed" as const;
		if (p > 0) return "active" as const;
		return "upcoming" as const;
	});

	const firstUnfinished = stepStates.findIndex((s) => s !== "completed");
	const totalCompleted = stepStates.filter((s) => s === "completed").length;

	return (
		<motion.article
			initial={{ opacity: 0, y: 6 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.35, ease: "easeOut" }}
			className="relative overflow-hidden rounded-2xl border border-border/60 bg-linear-to-br from-violet-500/10 via-card/95 to-card p-5 shadow-sm"
		>
			<div className="absolute inset-x-0 top-0 h-1 bg-linear-to-r from-violet-400/80 via-fuchsia-400/70 to-pink-400/70" />

			<header className="relative flex flex-wrap items-start justify-between gap-3">
				<div className="flex items-start gap-3 min-w-0">
					<div className="grid size-9 shrink-0 place-items-center rounded-lg bg-violet-500/15 text-violet-400 ring-1 ring-violet-400/25">
						<Route className="size-4" />
					</div>
					<div className="min-w-0 space-y-0.5">
						<h3 className="truncate text-base font-semibold tracking-tight">
							{path.title}
						</h3>
						{path.description && (
							<p className="text-xs text-muted-foreground line-clamp-2">
								{path.description}
							</p>
						)}
					</div>
				</div>
				<div className="text-xs text-muted-foreground tabular-nums">
					{totalCompleted}/{ordered.length} complete
				</div>
			</header>

			<ol className="relative mt-5 grid gap-3 lg:grid-cols-[repeat(auto-fill,minmax(min(100%,12rem),1fr))]">
				{ordered.map((step, index) => {
					const state = stepStates[index] ?? "upcoming";
					const previousState = stepStates[index - 1];
					const isLocked =
						state === "upcoming" &&
						previousState !== undefined &&
						previousState !== "completed";
					const isNext = index === firstUnfinished && !isLocked;
					return (
						<PathStep
							key={`${step.course_id}-${step.position}`}
							step={step}
							index={index}
							state={state}
							isLocked={isLocked}
							isNext={isNext}
							onSelectCourse={onSelectCourse}
						/>
					);
				})}
			</ol>
		</motion.article>
	);
}

interface PathStepProps {
	readonly step: { course_id: string; course: CourseListItem | null };
	readonly index: number;
	readonly state: "completed" | "active" | "upcoming";
	readonly isLocked: boolean;
	readonly isNext: boolean;
	readonly onSelectCourse?: (course: CourseListItem) => void;
}

function PathStep({
	step,
	index,
	state,
	isLocked,
	isNext,
	onSelectCourse,
}: PathStepProps) {
	const course = step.course;
	const interactive = !isLocked && course != null && onSelectCourse != null;

	const stepNumber = index + 1;

	return (
		<li className="relative">
			<div
				className="absolute left-1/2 top-4 hidden h-px w-full translate-x-0 bg-border/60 lg:block"
				aria-hidden
			/>
			<button
				type="button"
				onClick={() => {
					if (interactive && course) onSelectCourse(course);
				}}
				disabled={!interactive}
				className={cn(
					"group relative flex w-full items-start gap-3 rounded-xl border bg-card/60 p-3 text-left transition-colors",
					isLocked
						? "border-border/40 opacity-60"
						: "border-border/60 hover:border-violet-400/50 hover:bg-violet-500/5",
					isNext && "ring-1 ring-violet-400/40",
					!interactive && "cursor-default",
				)}
			>
				<div
					className={cn(
						"grid size-8 shrink-0 place-items-center rounded-full text-xs font-semibold ring-1 transition-colors",
						state === "completed" &&
							"bg-emerald-500/15 text-emerald-400 ring-emerald-400/30",
						state === "active" &&
							"bg-violet-500/15 text-violet-300 ring-violet-400/40",
						state === "upcoming" &&
							!isLocked &&
							"bg-background text-muted-foreground ring-border",
						isLocked && "bg-muted/50 text-muted-foreground/70 ring-border/40",
					)}
				>
					{state === "completed" ? (
						<Check className="size-4" />
					) : isLocked ? (
						<Lock className="size-3.5" />
					) : (
						stepNumber
					)}
				</div>
				<div className="min-w-0 flex-1 space-y-0.5">
					<div className="flex items-center gap-1">
						<p
							className={cn(
								"truncate text-sm font-medium",
								state === "completed" && "text-muted-foreground",
							)}
						>
							{course?.name ?? "Untitled course"}
						</p>
						{interactive && (
							<ChevronRight className="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
						)}
					</div>
					{course?.description && (
						<p className="line-clamp-2 text-xs text-muted-foreground">
							{course.description}
						</p>
					)}
					<div className="flex flex-wrap items-center gap-1.5 pt-1 text-[10px] uppercase tracking-wide text-muted-foreground">
						{course?.difficulty && <span>{course.difficulty}</span>}
						{course?.estimated_minutes ? (
							<>
								<span aria-hidden>·</span>
								<span>{course.estimated_minutes} min</span>
							</>
						) : null}
					</div>
				</div>
			</button>
		</li>
	);
}
