"use client";

import { useTranslation } from "@flow-like/locales";
import AlertTriangle from "lucide-react/dist/esm/icons/triangle-alert.js";
import CheckCircle2 from "lucide-react/dist/esm/icons/circle-check.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import ChevronRight from "lucide-react/dist/esm/icons/chevron-right.js";
import Circle from "lucide-react/dist/esm/icons/circle.js";
import CircleMinus from "lucide-react/dist/esm/icons/circle-minus.js";
import DatabaseIcon from "lucide-react/dist/esm/icons/database.js";
import History from "lucide-react/dist/esm/icons/history.js";
import LayoutIcon from "lucide-react/dist/esm/icons/panels-top-left.js";
import Loader2 from "lucide-react/dist/esm/icons/loader-2.js";
import WorkflowIcon from "lucide-react/dist/esm/icons/workflow.js";
import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { formatDuration } from "../../../lib/date";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";
import type { IBuildLaneDetail, IPlanStep, PlanStepStatus } from "./chat-db";
import { ReasoningViewer } from "./reasoning-viewer";

interface PlanStepsProps {
	steps: IPlanStep[];
	currentStepId?: string;
	/**
	 * Whether the owning message is still streaming. Between agent tool rounds every step is
	 * momentarily settled, so step statuses alone cannot tell "done for good" from "next round
	 * pending" — without this the panel would snap shut and reopen once per round.
	 */
	loading?: boolean;
	/**
	 * Whether the owning turn is still generating, regardless of whether THIS group owns the
	 * active step. Inline groups only receive `loading` for the live segment (so finished groups
	 * don't spin at "Working…"), but every group must stay expanded until the turn ends —
	 * otherwise each one folds away the instant it settles and the run looks frozen mid-stream.
	 */
	turnActive?: boolean;
}

const VISIBLE_STEPS_COUNT = 4;

function StatusIcon({ status }: { status: PlanStepStatus }) {
	switch (status) {
		case "planned":
			return <Circle className="size-3.5 text-muted-foreground/60" />;
		case "progress":
			return <Loader2 className="size-3.5 text-primary animate-spin" />;
		case "done":
			return <CheckCircle2 className="size-3.5 text-emerald-500" />;
		case "failed":
			// A tool attempt can fail while the agent successfully takes another route. Keep the
			// outcome legible without making it look like the whole response failed.
			return <CircleMinus className="size-3.5 text-muted-foreground/70" />;
	}
}

const LANE_META: Record<
	IBuildLaneDetail["lane"],
	{ label: string; Icon: typeof DatabaseIcon }
> = {
	data: { label: "Data", Icon: DatabaseIcon },
	page: { label: "Page", Icon: LayoutIcon },
	workflow: { label: "Workflow", Icon: WorkflowIcon },
};

/** Status dots shown beyond this count are folded into a "+N". */
const SUMMARY_DOT_LIMIT = 8;

interface IStepSummary {
	readonly total: number;
	readonly laneCount: number;
	readonly doneCount: number;
	readonly failedCount: number;
	readonly settledCount: number;
	readonly durationLabel: string | null;
}

function summariseSteps(steps: IPlanStep[]): IStepSummary {
	let doneCount = 0;
	let failedCount = 0;
	// Same unit as step.startTime/endTime; formatDuration wants microseconds,
	// which is why the existing rows multiply by 1000 too.
	let elapsedMs = 0;
	const lanes = new Set<string>();

	for (const step of steps) {
		if (step.status === "done") doneCount += 1;
		if (step.status === "failed") failedCount += 1;
		if (step.detail?.kind === "build_lane") lanes.add(step.detail.lane);
		if (step.startTime && step.endTime) {
			elapsedMs += Math.max(0, step.endTime - step.startTime);
		}
	}

	return {
		total: steps.length,
		laneCount: lanes.size,
		doneCount,
		failedCount,
		settledCount: doneCount + failedCount,
		durationLabel: elapsedMs > 0 ? formatDuration(elapsedMs * 1000) : null,
	};
}

/**
 * The whole run at a glance: one dot per step, so a failure is visible without
 * expanding anything.
 */
function StatusDots({ steps }: { steps: IPlanStep[] }) {
	const { t } = useTranslation("chat");
	const shown = steps.slice(0, SUMMARY_DOT_LIMIT);
	const overflow = steps.length - shown.length;

	return (
		<span className="flex shrink-0 items-center gap-1" aria-hidden="true">
			{shown.map((step) => (
				<span
					key={step.id}
					className={cn(
						"size-1.5 rounded-full",
						step.status === "done" && "bg-emerald-500",
						step.status === "failed" && "bg-destructive/70",
						step.status === "progress" &&
							"bg-primary motion-safe:animate-pulse-soft",
						step.status === "planned" && "bg-muted-foreground/35",
					)}
				/>
			))}
			{overflow > 0 && (
				<span className="text-[10px] tabular-nums text-muted-foreground/60">{`+${overflow}`}</span>
			)}
		</span>
	);
}

/** The one line a settled run collapses to. */
function ActivitySummaryLabel({
	summary,
	running,
	activeTitle,
}: {
	summary: IStepSummary;
	running: boolean;
	activeTitle?: string;
}) {
	const { t } = useTranslation("chat");
	if (running) {
		return (
			<span className="min-w-0 flex-1 truncate">
				<span className="text-foreground">{activeTitle ?? "Working…"}</span>
				<span className="text-muted-foreground">
					{` · `}
					{`${summary.settledCount}/${summary.total}`}
				</span>
			</span>
		);
	}

	const parts = [t("countSteps", "{{count}} step", { count: summary.total })];
	if (summary.laneCount > 1) parts.push(`across ${summary.laneCount} lanes`);
	if (summary.durationLabel) parts.push(summary.durationLabel);
	if (summary.failedCount > 0) {
		parts.push(`${summary.failedCount} not completed`);
	}

	return <span className="min-w-0 flex-1 truncate">{parts.join(" · ")}</span>;
}

const ACTIVITY_TRIGGER_CLASS =
	"group flex w-full items-center gap-2.5 border-y py-2 text-left text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0";

/**
 * One concurrent branch of a build, rendered as a block rather than a row.
 *
 * The lanes run at the same time, so this is the piece that makes the parallelism legible: each
 * lane shows what it is building, how far its segments have got, and any function it handed back
 * for the user to implement. A gap the user is never shown is a workflow they think is finished.
 */
function BuildLaneRow({
	step,
	detail,
	isActive,
}: {
	step: IPlanStep;
	detail: IBuildLaneDetail;
	isActive: boolean;
}) {
	const { t } = useTranslation("chat");
	const { label, Icon } = LANE_META[detail.lane];
	const total = detail.segmentsTotal ?? detail.segments?.length ?? 0;
	const applied =
		detail.segmentsApplied ??
		detail.segments?.filter((segment) => segment.applied).length ??
		0;
	const showSegments = (detail.segments?.length ?? 0) > 1;

	return (
		<div
			className={cn(
				"rounded-md px-2 py-1.5 motion-safe:animate-in motion-safe:fade-in motion-safe:duration-300",
				isActive && "bg-primary/5",
			)}
		>
			<div className="flex items-center gap-2">
				<span className="shrink-0">
					<StatusIcon status={step.status} />
				</span>
				<Icon className="size-3.5 shrink-0 text-muted-foreground" />
				<span
					className={cn(
						"text-xs font-medium shrink-0",
						step.status === "failed"
							? "text-muted-foreground"
							: "text-foreground/90",
					)}
				>
					{label}
				</span>
				{detail.target && (
					<span className="text-xs text-muted-foreground truncate min-w-0 flex-1">
						{detail.target}
					</span>
				)}
				<span className="ml-auto flex items-center gap-1.5 shrink-0">
					{total > 1 && (
						<span className="text-[10px] font-mono text-muted-foreground/70">{`${applied}/${total}`}</span>
					)}
					{detail.earnedMinutes ? (
						<span className="text-[10px] font-mono text-muted-foreground/70">{`+${detail.earnedMinutes}m`}</span>
					) : null}
				</span>
			</div>
			{showSegments && (
				<ul className="mt-1 pl-7 grid gap-0.5">
					{detail.segments?.map((segment) => (
						<li
							key={segment.id}
							className="flex items-center gap-1.5 text-[11px] min-w-0"
						>
							{segment.applied ? (
								<CheckCircle2 className="size-3 shrink-0 text-emerald-500" />
							) : (
								<Circle className="size-3 shrink-0 text-muted-foreground/50" />
							)}
							<span
								className={cn(
									"truncate",
									segment.applied
										? "text-muted-foreground"
										: "text-muted-foreground/70",
								)}
							>
								{segment.title}
							</span>
						</li>
					))}
				</ul>
			)}
			{detail.gaps?.length ? (
				<div className="mt-1.5 ml-7 rounded-md border border-amber-500/30 bg-amber-500/5 px-2 py-1">
					<div className="flex items-center gap-1.5">
						<AlertTriangle className="size-3 shrink-0 text-amber-500" />
						<span className="text-[11px] font-medium text-amber-600 dark:text-amber-400">
							{t("countFunctionsNeedYourLogic", {
								defaultValue_one: "1 function needs your logic",
								defaultValue_other: "{{count}} functions need your logic",
								count: detail.gaps.length,
							})}
						</span>
					</div>
					<ul className="mt-0.5 grid gap-0.5">
						{detail.gaps.map((gap) => (
							<li
								key={`${gap.function ?? ""}:${gap.detail}`}
								className="text-[11px] text-muted-foreground min-w-0"
							>
								{gap.function && (
									<span className="font-mono text-foreground/80">
										{gap.function}
									</span>
								)}
								{gap.function && gap.detail ? ` — ` : ""}
								{gap.detail}
							</li>
						))}
					</ul>
				</div>
			) : null}
		</div>
	);
}

function StepRow({
	step,
	isActive,
	isExpanded,
	onToggle,
}: {
	step: IPlanStep;
	isActive: boolean;
	isExpanded: boolean;
	onToggle: (stepId: string) => void;
}) {
	if (step.detail?.kind === "build_lane") {
		return (
			<BuildLaneRow step={step} detail={step.detail} isActive={isActive} />
		);
	}
	const hasReasoning = Boolean(step.reasoning?.trim());
	const duration =
		step.startTime && step.endTime
			? formatDuration((step.endTime - step.startTime) * 1000)
			: null;

	return (
		<div
			className={cn(
				"rounded-md motion-safe:animate-in motion-safe:fade-in motion-safe:duration-300",
				isActive && "bg-primary/5",
			)}
		>
			<button
				onClick={() => hasReasoning && onToggle(step.id)}
				type="button"
				className={cn(
					"w-full flex items-center gap-2 px-2 py-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0 rounded-md",
					hasReasoning
						? "cursor-pointer hover:bg-muted/40 transition-colors"
						: "cursor-default",
				)}
			>
				<span className="shrink-0">
					<StatusIcon status={step.status} />
				</span>
				<span
					className={cn(
						"text-xs font-medium truncate",
						step.status === "failed"
							? "text-muted-foreground"
							: "text-foreground/90",
					)}
				>
					{step.title}
				</span>
				{step.description && (
					<span className="text-xs text-muted-foreground truncate min-w-0 flex-1">
						{step.description}
					</span>
				)}
				<span className="ml-auto flex items-center gap-1.5 shrink-0">
					{duration && (
						<span className="text-[10px] font-mono text-muted-foreground/70">
							{duration}
						</span>
					)}
					{hasReasoning &&
						(isExpanded ? (
							<ChevronDown className="size-3.5 text-muted-foreground" />
						) : (
							<ChevronRight className="size-3.5 text-muted-foreground" />
						))}
				</span>
			</button>
			{isExpanded && hasReasoning && (
				<div className="pl-7 pr-2 pb-1.5">
					<ReasoningViewer
						reasoning={step.reasoning ?? ""}
						defaultExpanded={true}
						compact={true}
					/>
				</div>
			)}
		</div>
	);
}

/** Above this size an inline group stops showing bare rows and reuses the collapsible timeline. */
const INLINE_GROUP_COLLAPSE_THRESHOLD = 5;

/**
 * A run of actions rendered INLINE between the text segments they interrupted (steps carrying a
 * `content_offset` anchor). Small groups show their rows directly — one websearch reads as one
 * nicely rendered row in the reply's flow; large fan-outs fall back to the collapsible timeline.
 */
export function InlineStepGroup({
	steps,
	currentStepId,
	loading,
	turnActive,
}: PlanStepsProps) {
	const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set());

	// Open for as long as the turn is generating — not merely while this group owns
	// the active step — then fold to its summary line so the finished answer reads
	// as prose rather than as a log.
	const isLive =
		Boolean(loading) ||
		Boolean(turnActive) ||
		steps.some((s) => s.status === "progress" || s.status === "planned");
	const [open, setOpen] = useState(isLive);
	const wasLiveRef = useRef(isLive);
	useEffect(() => {
		if (wasLiveRef.current && !isLive) setOpen(false);
		if (!wasLiveRef.current && isLive) setOpen(true);
		wasLiveRef.current = isLive;
	}, [isLive]);

	useEffect(() => {
		if (!currentStepId) return;
		setExpandedSteps((prev) => {
			if (prev.has(currentStepId)) return prev;
			const next = new Set(prev);
			next.add(currentStepId);
			return next;
		});
	}, [currentStepId]);

	if (steps.length === 0) return null;
	if (steps.length > INLINE_GROUP_COLLAPSE_THRESHOLD) {
		return (
			<PlanSteps
				steps={steps}
				currentStepId={currentStepId}
				loading={loading}
				turnActive={turnActive}
			/>
		);
	}

	const toggleStep = (stepId: string) => {
		setExpandedSteps((prev) => {
			const next = new Set(prev);
			if (next.has(stepId)) next.delete(stepId);
			else next.add(stepId);
			return next;
		});
	};

	const summary = summariseSteps(steps);
	const running =
		Boolean(loading) ||
		steps.some((s) => s.status === "progress" || s.status === "planned");
	const activeStep = currentStepId
		? steps.find((s) => s.id === currentStepId)
		: undefined;

	return (
		<Collapsible
			open={open}
			onOpenChange={setOpen}
			className="my-3"
			data-fl-plan-group
			style={{ maxWidth: "var(--fl-chat-measure, 38rem)" }}
		>
			<CollapsibleTrigger
				className={ACTIVITY_TRIGGER_CLASS}
				style={{ borderColor: "var(--fl-chat-rule, var(--border))" }}
			>
				{running ? (
					<Loader2 className="size-3.5 shrink-0 animate-spin text-primary" />
				) : (
					<StatusDots steps={steps} />
				)}
				<ActivitySummaryLabel
					summary={summary}
					running={running}
					activeTitle={activeStep?.title}
				/>
				<ChevronDown
					className={cn(
						"size-3.5 shrink-0 transition-transform",
						open ? "rotate-180" : "",
					)}
				/>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="space-y-0.5 pt-2 pb-1">
					{steps.map((step) => (
						<StepRow
							key={step.id}
							step={step}
							isActive={currentStepId === step.id}
							isExpanded={expandedSteps.has(step.id)}
							onToggle={toggleStep}
						/>
					))}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}

/**
 * Compact tool/step activity timeline for a chat message. Expanded while the agent is working;
 * collapses to a one-line summary once every step has settled.
 */
export function PlanSteps({
	steps,
	currentStepId,
	loading,
	turnActive,
}: PlanStepsProps) {
	const { t } = useTranslation("chat");
	const [expandedSteps, setExpandedSteps] = useState<Set<string>>(
		new Set(currentStepId ? [currentStepId] : []),
	);
	const [showOlderSteps, setShowOlderSteps] = useState(false);

	const doneCount = steps.filter((s) => s.status === "done").length;
	const failedCount = steps.filter((s) => s.status === "failed").length;
	const settledCount = doneCount + failedCount;
	const allSettled =
		steps.length > 0 &&
		steps.every((s) => s.status === "done" || s.status === "failed");
	const running = Boolean(loading) || Boolean(turnActive) || !allSettled;
	// A build that handed functions back for the user to implement must not fold itself away on
	// success: an unread gap is a workflow the user believes is finished.
	const hasGaps = steps.some(
		(s) => s.detail?.kind === "build_lane" && (s.detail.gaps?.length ?? 0) > 0,
	);

	// Expanded while running; auto-collapse once everything settles. A failed tool attempt is
	// activity detail, not proof that the response failed, so only actionable build gaps stay open.
	// Messages loaded from history (settled from the start) begin collapsed for the same reason.
	const [open, setOpen] = useState(running || hasGaps);
	const wasRunningRef = useRef(running);
	useEffect(() => {
		if (wasRunningRef.current && !running && !hasGaps) {
			setOpen(false);
		}
		if (!wasRunningRef.current && running) {
			setOpen(true);
		}
		wasRunningRef.current = running;
	}, [running, hasGaps]);

	useEffect(() => {
		if (currentStepId) {
			setExpandedSteps((prev) => {
				if (prev.has(currentStepId)) return prev;
				const next = new Set(prev);
				next.add(currentStepId);
				return next;
			});
		}
	}, [currentStepId]);

	const { visibleSteps, olderSteps } = useMemo(() => {
		if (steps.length <= VISIBLE_STEPS_COUNT) {
			return { visibleSteps: steps, olderSteps: [] as IPlanStep[] };
		}
		// Build lanes are the summary of the whole run and are published as soon as each lane
		// starts, so a plain "last N steps" window would push them into the collapsed older
		// section exactly as the build gets interesting. Pin them, and window only the rest.
		const lanes = steps.filter((step) => step.detail?.kind === "build_lane");
		const rest = steps.filter((step) => step.detail?.kind !== "build_lane");
		if (lanes.length === 0) {
			const cutoff = steps.length - VISIBLE_STEPS_COUNT;
			return {
				visibleSteps: steps.slice(cutoff),
				olderSteps: steps.slice(0, cutoff),
			};
		}
		const restCutoff = Math.max(0, rest.length - VISIBLE_STEPS_COUNT);
		return {
			visibleSteps: [...lanes, ...rest.slice(restCutoff)],
			olderSteps: rest.slice(0, restCutoff),
		};
	}, [steps]);

	if (!steps || steps.length === 0) return null;

	const toggleStep = (stepId: string) => {
		setExpandedSteps((prev) => {
			const next = new Set(prev);
			if (next.has(stepId)) next.delete(stepId);
			else next.add(stepId);
			return next;
		});
	};

	const activeStep = currentStepId
		? steps.find((s) => s.id === currentStepId)
		: undefined;

	const summary = summariseSteps(steps);

	return (
		<Collapsible
			open={open}
			onOpenChange={setOpen}
			className="my-3"
			data-fl-plan-group
			style={{ maxWidth: "var(--fl-chat-measure, 38rem)" }}
		>
			<CollapsibleTrigger
				className={ACTIVITY_TRIGGER_CLASS}
				style={{ borderColor: "var(--fl-chat-rule, var(--border))" }}
			>
				{running ? (
					<Loader2 className="size-3.5 shrink-0 animate-spin text-primary" />
				) : (
					<StatusDots steps={steps} />
				)}
				<ActivitySummaryLabel
					summary={summary}
					running={running}
					activeTitle={activeStep?.title}
				/>
				<ChevronDown
					className={cn(
						"size-3.5 shrink-0 transition-transform",
						open ? "rotate-180" : "",
					)}
				/>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="space-y-0.5 pt-2 pb-1">
					{olderSteps.length > 0 && (
						<button
							type="button"
							onClick={() => setShowOlderSteps((show) => !show)}
							className="w-full flex items-center gap-2 px-2 py-1 rounded-md text-left text-xs text-muted-foreground hover:bg-muted/40 transition-colors outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						>
							<History className="size-3.5 shrink-0" />
							<span className="flex-1">
								{showOlderSteps ? t("hide", "Hide") : t("show", "Show")}{" "}
								{t("countEarlierSteps", {
									defaultValue_one: "{{count}} earlier step",
									defaultValue_other: "{{count}} earlier steps",
									count: olderSteps.length,
								})}
							</span>
							<ChevronDown
								className={cn(
									"size-3.5 transition-transform",
									showOlderSteps ? "rotate-180" : "",
								)}
							/>
						</button>
					)}
					{showOlderSteps &&
						olderSteps.map((step) => (
							<StepRow
								key={step.id}
								step={step}
								isActive={currentStepId === step.id}
								isExpanded={expandedSteps.has(step.id)}
								onToggle={toggleStep}
							/>
						))}
					{visibleSteps.map((step) => (
						<StepRow
							key={step.id}
							step={step}
							isActive={currentStepId === step.id}
							isExpanded={expandedSteps.has(step.id)}
							onToggle={toggleStep}
						/>
					))}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}
