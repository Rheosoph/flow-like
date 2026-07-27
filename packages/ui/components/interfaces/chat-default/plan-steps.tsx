"use client";

import {
	AlertTriangle,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Circle,
	DatabaseIcon,
	History,
	LayoutIcon,
	Loader2,
	WorkflowIcon,
	XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib";
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
			return <XCircle className="size-3.5 text-red-500" />;
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
						step.status === "failed" ? "text-red-500" : "text-foreground/90",
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
						<span className="text-[10px] font-mono text-muted-foreground/70">
							{applied}/{total}
						</span>
					)}
					{detail.earnedMinutes ? (
						<span className="text-[10px] font-mono text-muted-foreground/70">
							+{detail.earnedMinutes}m
						</span>
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
							{detail.gaps.length === 1
								? "1 function needs your logic"
								: `${detail.gaps.length} functions need your logic`}
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
								{gap.function && gap.detail ? " — " : ""}
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
						step.status === "failed" ? "text-red-500" : "text-foreground/90",
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
}: PlanStepsProps) {
	const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set());

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

	return (
		<div className="my-2 rounded-lg border border-border/40 bg-muted/20 px-1 py-1 space-y-0.5">
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
	);
}

/**
 * Compact tool/step activity timeline for a chat message. Expanded while the agent is working;
 * collapses to a one-line summary once every step has settled (stays open on failures).
 */
export function PlanSteps({ steps, currentStepId, loading }: PlanStepsProps) {
	const [expandedSteps, setExpandedSteps] = useState<Set<string>>(
		new Set(currentStepId ? [currentStepId] : []),
	);
	const [showOlderSteps, setShowOlderSteps] = useState(false);

	const doneCount = steps.filter((s) => s.status === "done").length;
	const failedCount = steps.filter((s) => s.status === "failed").length;
	const allSettled =
		steps.length > 0 &&
		steps.every((s) => s.status === "done" || s.status === "failed");
	const running = Boolean(loading) || !allSettled;
	// A build that handed functions back for the user to implement must not fold itself away on
	// success: an unread gap is a workflow the user believes is finished.
	const hasGaps = steps.some(
		(s) => s.detail?.kind === "build_lane" && (s.detail.gaps?.length ?? 0) > 0,
	);

	// Expanded while running; auto-collapse once when everything settles cleanly. Messages loaded
	// from history (settled from the start) begin collapsed; failures and gaps keep the list open.
	const [open, setOpen] = useState(running || failedCount > 0 || hasGaps);
	const wasRunningRef = useRef(running);
	useEffect(() => {
		if (wasRunningRef.current && !running && failedCount === 0 && !hasGaps) {
			setOpen(false);
		}
		if (!wasRunningRef.current && running) {
			setOpen(true);
		}
		wasRunningRef.current = running;
	}, [running, failedCount, hasGaps]);

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

	return (
		<Collapsible
			open={open}
			onOpenChange={setOpen}
			className="my-2 rounded-lg border border-border/40 bg-muted/20"
		>
			<CollapsibleTrigger className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left rounded-lg hover:bg-muted/40 transition-colors outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0">
				{running ? (
					<Loader2 className="size-3.5 text-primary animate-spin shrink-0" />
				) : failedCount > 0 ? (
					<XCircle className="size-3.5 text-red-500 shrink-0" />
				) : (
					<CheckCircle2 className="size-3.5 text-emerald-500 shrink-0" />
				)}
				<span className="text-xs font-medium text-foreground/90 shrink-0">
					{running
						? (activeStep?.title ?? "Working…")
						: `${steps.length} step${steps.length === 1 ? "" : "s"}`}
				</span>
				<span className="text-xs text-muted-foreground truncate min-w-0 flex-1">
					{running
						? `${doneCount}/${steps.length}`
						: failedCount > 0
							? `${failedCount} failed`
							: "completed"}
				</span>
				<ChevronDown
					className={cn(
						"size-3.5 text-muted-foreground shrink-0 transition-transform",
						open ? "rotate-180" : "",
					)}
				/>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="px-1 pb-1 space-y-0.5">
					{olderSteps.length > 0 && (
						<button
							type="button"
							onClick={() => setShowOlderSteps((show) => !show)}
							className="w-full flex items-center gap-2 px-2 py-1 rounded-md text-left text-xs text-muted-foreground hover:bg-muted/40 transition-colors outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						>
							<History className="size-3.5 shrink-0" />
							<span className="flex-1">
								{showOlderSteps ? "Hide" : "Show"} {olderSteps.length} earlier
								step{olderSteps.length === 1 ? "" : "s"}
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
