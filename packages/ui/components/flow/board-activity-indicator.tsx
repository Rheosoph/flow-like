"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import PuffLoader from "react-spinners/PuffLoader";
import { useShallow } from "zustand/react/shallow";
import { useRunExecutionStore } from "../../state/run-execution-state";

interface BoardActivityIndicatorProps {
	boardId: string;
}

type ActivityStatus = "active" | "warning" | "stale" | "inactive";

function getStatusColor(status: ActivityStatus): {
	border: string;
	bg: string;
	text: string;
	icon: string;
} {
	switch (status) {
		case "active":
			return {
				border: "border-green-500/35",
				bg: "bg-green-500/10",
				text: `text-green-600 dark:text-green-400`,
				icon: "text-green-500",
			};
		case "warning":
			return {
				border: "border-yellow-500/35",
				bg: "bg-yellow-500/10",
				text: `text-yellow-600 dark:text-yellow-400`,
				icon: "text-yellow-500",
			};
		case "stale":
			return {
				border: "border-red-500/35",
				bg: "bg-red-500/10",
				text: `text-red-600 dark:text-red-400`,
				icon: "text-red-500",
			};
		default:
			return {
				border: "border-muted-foreground/35",
				bg: "bg-muted/50",
				text: "text-muted-foreground",
				icon: "text-muted-foreground",
			};
	}
}

function formatDuration(ms: number): string {
	const seconds = Math.floor(ms / 1000);
	if (seconds < 60) return `${seconds}s`;
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
	const hours = Math.floor(minutes / 60);
	return `${hours}h ${minutes % 60}m`;
}

export function BoardActivityIndicator({
	boardId,
}: BoardActivityIndicatorProps) {
	const { t } = useTranslation("flow");
	const [now, setNow] = useState(Date.now());

	// Subscribe to primitive values to avoid infinite re-renders
	// Get the run IDs for this board (runs that have any activity)
	const activeRunIds = useRunExecutionStore(
		useShallow((state) => {
			const ids: string[] = [];
			for (const [runId, run] of state.runs) {
				// Show runs that have any activity
				if (
					run.boardId === boardId &&
					(run.nodes.size > 0 || run.totalExecutionsCompleted > 0)
				) {
					ids.push(runId);
				}
			}
			return ids;
		}),
	);

	// Get currently executing nodes count (unique nodes)
	const currentlyExecuting = useRunExecutionStore((state) => {
		let total = 0;
		for (const [, run] of state.runs) {
			if (run.boardId === boardId) {
				total += run.nodes.size;
			}
		}
		return total;
	});

	// Get total executions completed (counts loop iterations)
	const totalExecutionsCompleted = useRunExecutionStore((state) => {
		let total = 0;
		for (const [, run] of state.runs) {
			if (run.boardId === boardId) {
				total += run.totalExecutionsCompleted;
			}
		}
		return total;
	});

	const mostRecentUpdate = useRunExecutionStore((state) => {
		let latest = 0;
		for (const [, run] of state.runs) {
			if (run.boardId === boardId && run.lastNodeUpdateMs > latest) {
				latest = run.lastNodeUpdateMs;
			}
		}
		return latest;
	});

	// Update time every second when there are active runs
	useEffect(() => {
		if (activeRunIds.length === 0) return;

		const interval = setInterval(() => {
			setNow(Date.now());
		}, 1000);

		return () => clearInterval(interval);
	}, [activeRunIds.length]);

	if (activeRunIds.length === 0) return null;

	const timeSinceUpdate = now - mostRecentUpdate;

	// Determine status based on time since last update
	let status: ActivityStatus = "active";
	if (timeSinceUpdate > 60000) {
		status = "stale";
	} else if (timeSinceUpdate > 30000) {
		status = "warning";
	}

	const colors = getStatusColor(status);

	// Build the node count display - show execution count which properly counts loop iterations
	const nodeDisplay =
		currentlyExecuting > 0
			? totalExecutionsCompleted > 0
				? t('activeAndCompletedExecutions', '{{active}} active · {{completed}} completed', { active: currentlyExecuting, completed: totalExecutionsCompleted })
				: t('currentlyexecutingActive', '{{currentlyExecuting}} active', { currentlyExecuting })
			: totalExecutionsCompleted > 0
				? t('totalExecutionsCompleted', '{{totalExecutionsCompleted}} completed', { totalExecutionsCompleted })
				: t('starting', 'Starting…');

	// Only show time after 15 seconds
	const showTime = timeSinceUpdate >= 15000;

	return (
		<div
			className={`flex items-center gap-2 rounded-xl border ${colors.border} ${colors.bg} px-3 py-1.5 backdrop-blur-sm shadow-sm`}
		>
			<PuffLoader color="currentColor" size={14} className={colors.icon} />
			<div className="flex flex-col">
				<span className={`text-xs font-medium ${colors.text}`}>{t('countRuns', { defaultValue_one: '{{count}} run', defaultValue_other: '{{count}} runs', count: activeRunIds.length })} •{" "}
					{nodeDisplay}
				</span>
				{showTime && (
					<span className={`text-[10px] ${colors.text} opacity-75`}>
						{t('durationAgo', '{{duration}} ago', { duration: formatDuration(timeSinceUpdate) })}
					</span>
				)}
			</div>
		</div>
	);
}
