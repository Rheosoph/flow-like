import { useTranslation } from "@flow-like/locales";
import { ScrollTextIcon } from "lucide-react";
import { type MouseEvent, memo, useCallback } from "react";
import { useShallow } from "zustand/react/shallow";
import { cn } from "../../../lib/utils";
import {
	nodeLogCounts,
	useLogAggregation,
} from "../../../state/log-aggregation-state";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

const compact = new Intl.NumberFormat(undefined, {
	notation: "compact",
	maximumFractionDigits: 1,
});

/**
 * The node's log count in the current run: problems in the error tone, else
 * the total. Before the run's summary is known it falls back to the plain
 * icon on nodes that executed.
 */
export const NodeLogBadge = memo(function NodeLogBadge({
	nodeId,
	nodeName,
	executed,
	onFilterLogs,
}: Readonly<{
	nodeId: string;
	nodeName: string;
	executed: boolean;
	onFilterLogs?: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const counts = useLogAggregation(
		useShallow((state) => nodeLogCounts(state, nodeId)),
	);
	const onClick = useCallback(
		(event: MouseEvent<Element>) => {
			event.stopPropagation();
			onFilterLogs?.(nodeId);
		},
		[onFilterLogs, nodeId],
	);

	if (!counts || counts.total === 0) {
		if (!executed) return null;
		return (
			<ScrollTextIcon
				onClick={onClick}
				className="nodrag h-2 w-2 cursor-pointer hover:text-primary"
			/>
		);
	}

	const problems = counts.problems > 0;
	const label = t(
		"logViewShowNodeLogs",
		"Show {{count, number}} logs from {{name}}",
		{
			count: counts.total,
			name: nodeName,
		},
	);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					onClick={onClick}
					className={cn(
						"nodrag nopan inline-flex h-3 shrink-0 items-center gap-0.5 rounded-full border px-1 font-mono text-[8px] leading-none font-semibold transition-colors",
						problems
							? "border-destructive/40 bg-destructive/15 text-destructive hover:bg-destructive/25"
							: "border-border bg-secondary text-muted-foreground hover:text-foreground",
					)}
				>
					<ScrollTextIcon aria-hidden className="size-2" />
					{compact.format(problems ? counts.problems : counts.total)}
					{!problems && counts.warnings > 0 ? (
						<span aria-hidden className="size-1 rounded-full bg-amber-500" />
					) : null}
				</button>
			</TooltipTrigger>
			<TooltipContent side="top">{label}</TooltipContent>
		</Tooltip>
	);
});
