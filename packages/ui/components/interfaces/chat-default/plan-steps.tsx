"use client";

import {
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Circle,
	History,
	Loader2,
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
import type { IPlanStep, PlanStepStatus } from "./chat-db";
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

	// Expanded while running; auto-collapse once when everything settles cleanly. Messages loaded
	// from history (settled from the start) begin collapsed; failures keep the list open.
	const [open, setOpen] = useState(running || failedCount > 0);
	const wasRunningRef = useRef(running);
	useEffect(() => {
		if (wasRunningRef.current && !running && failedCount === 0) {
			setOpen(false);
		}
		if (!wasRunningRef.current && running) {
			setOpen(true);
		}
		wasRunningRef.current = running;
	}, [running, failedCount]);

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
		const cutoff = steps.length - VISIBLE_STEPS_COUNT;
		return {
			visibleSteps: steps.slice(cutoff),
			olderSteps: steps.slice(0, cutoff),
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
