"use client";

import { AnimatePresence, motion } from "framer-motion";
import {
	AlertTriangleIcon,
	ChevronDown,
	LinkIcon,
	PencilIcon,
	PlayIcon,
	PlusCircleIcon,
	SparklesIcon,
	XCircleIcon,
	XIcon,
} from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";

import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { getCommandColor, getCommandIcon, getCommandSummary } from "./utils";

import type { BoardEditJobReview } from "../../lib/schema/copilot";
import type { BoardCommand } from "../../lib/schema/flow/copilot";

interface PendingCommandsViewProps {
	commands: BoardCommand[];
	flowscriptReady?: boolean;
	dismissOnly?: boolean;
	/** Host-retained review metadata when exact command payloads intentionally stay native. */
	retainedReview?: BoardEditJobReview;
	onExecute: () => void | Promise<void>;
	onExecuteSingle: (index: number) => void | Promise<void>;
	onDismiss: () => void;
}

export const PendingCommandsView = memo(function PendingCommandsView({
	commands,
	flowscriptReady = false,
	dismissOnly = false,
	retainedReview,
	onExecute,
	onExecuteSingle,
	onDismiss,
}: PendingCommandsViewProps) {
	const [expanded, setExpanded] = useState(false);
	const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
	const [destructiveConfirmationArmed, setDestructiveConfirmationArmed] =
		useState(false);
	const hasRetainedReview = retainedReview !== undefined;
	const retainedReviewCount = retainedReview?.commandCount;
	const retainedCommandSummaries = retainedReview?.commandSummaries ?? [];
	const retainedDestructiveEffects = retainedReview?.destructiveEffects ?? [];
	const retainedReviewIsDestructive = Boolean(
		retainedReview?.replacementMode || retainedDestructiveEffects.length > 0,
	);
	const keyedRetainedCommandSummaries = useMemo(() => {
		const occurrences = new Map<string, number>();
		return retainedCommandSummaries.map((summary) => {
			const occurrence = (occurrences.get(summary) ?? 0) + 1;
			occurrences.set(summary, occurrence);
			return { key: `${summary}:${occurrence}`, summary };
		});
	}, [retainedCommandSummaries]);
	const retainedCommandBreakdown = useMemo(
		() =>
			Object.entries(retainedReview?.commandCounts ?? {}).sort(
				([left], [right]) => left.localeCompare(right),
			),
		[retainedReview?.commandCounts],
	);
	const hasCommandDetails = hasRetainedReview
		? retainedCommandSummaries.length > 0
		: commands.length > 0;

	const toggleExpanded = useCallback(() => setExpanded((prev) => !prev), []);
	const handleExecute = useCallback(() => {
		if (retainedReviewIsDestructive && !destructiveConfirmationArmed) {
			setDestructiveConfirmationArmed(true);
			setExpanded(true);
			return;
		}
		setDestructiveConfirmationArmed(false);
		void onExecute();
	}, [destructiveConfirmationArmed, onExecute, retainedReviewIsDestructive]);
	const handleDismiss = useCallback(() => {
		setDestructiveConfirmationArmed(false);
		onDismiss();
	}, [onDismiss]);

	// Memoize command counts to avoid recalculation on every render
	const commandCounts = useMemo(
		() => ({
			add: commands.filter((c) => c.command_type === "AddNode").length,
			connect: commands.filter((c) => c.command_type === "ConnectPins").length,
			update: commands.filter((c) => c.command_type === "UpdateNodePin").length,
			remove: commands.filter((c) => c.command_type === "RemoveNode").length,
		}),
		[commands],
	);

	if (
		commands.length === 0 &&
		!flowscriptReady &&
		!dismissOnly &&
		!hasRetainedReview
	)
		return null;

	return (
		<motion.div
			initial={{ opacity: 0, y: 15, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			transition={{ type: "spring", stiffness: 400, damping: 25 }}
			className="w-full"
		>
			{/* Enhanced summary card */}
			<div className="relative overflow-hidden rounded-2xl border border-primary/20 bg-linear-to-br from-primary/5 via-violet-500/5 to-pink-500/5">
				{/* Animated background shimmer */}
				<motion.div
					className="absolute inset-0 bg-linear-to-r from-transparent via-white/5 to-transparent"
					animate={{ x: ["-100%", "200%"] }}
					transition={{
						duration: 2,
						repeat: Number.POSITIVE_INFINITY,
						repeatDelay: 1,
					}}
				/>

				<div className="relative p-3.5">
					<div className="flex items-center justify-between gap-3 mb-3">
						<div className="flex items-center gap-2.5">
							<div className="relative">
								<div className="absolute inset-0 bg-primary/30 rounded-lg blur-md" />
								<div className="relative p-1.5 bg-linear-to-br from-primary to-violet-600 rounded-lg">
									<SparklesIcon className="w-4 h-4 text-white" />
								</div>
							</div>
							<div>
								<div className="text-sm font-semibold text-foreground">
									{dismissOnly
										? "Review is stale"
										: retainedReviewIsDestructive
											? "Destructive change ready"
											: "Ready to Apply"}
								</div>
								<div className="text-[11px] text-muted-foreground">
									{dismissOnly
										? "The board changed; dismiss this review before regenerating"
										: flowscriptReady && commands.length === 0
											? "FlowScript workspace pending"
											: `${retainedReviewCount ?? commands.length} change${(retainedReviewCount ?? commands.length) > 1 ? "s" : ""} pending`}
								</div>
							</div>
						</div>

						<div className="flex items-center gap-1.5">
							{hasCommandDetails && (
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											size="sm"
											variant="ghost"
											className="h-8 w-8 p-0 rounded-lg hover:bg-background/60"
											onClick={toggleExpanded}
										>
											<ChevronDown
												className={`w-4 h-4 transition-transform duration-200 ${expanded ? "rotate-180" : ""}`}
											/>
										</Button>
									</TooltipTrigger>
									<TooltipContent side="top" className="text-xs">
										{expanded ? "Collapse" : "Expand"} details
									</TooltipContent>
								</Tooltip>
							)}

							{!dismissOnly && (
								<Button
									size="sm"
									className={`h-8 px-4 text-xs font-medium gap-1.5 shadow-lg rounded-lg ${destructiveConfirmationArmed ? "bg-destructive text-destructive-foreground hover:bg-destructive/90 shadow-destructive/20" : "bg-linear-to-r from-primary to-violet-600 hover:from-primary/90 hover:to-violet-600/90 shadow-primary/20"}`}
									onClick={handleExecute}
								>
									<PlayIcon className="w-3.5 h-3.5" />
									{destructiveConfirmationArmed
										? "Confirm Apply"
										: retainedReviewIsDestructive
											? "Review & Apply"
											: hasRetainedReview
												? "Apply Exact Batch"
												: "Apply All"}
								</Button>
							)}

							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										size="sm"
										variant="ghost"
										className="h-8 w-8 p-0 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/10"
										onClick={handleDismiss}
									>
										<XIcon className="w-4 h-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									{dismissOnly ? "Dismiss stale review" : "Dismiss changes"}
								</TooltipContent>
							</Tooltip>
						</div>
					</div>

					{/* Command type badges */}
					<div className="flex flex-wrap gap-1.5">
						{dismissOnly && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-amber-500/15 text-amber-700 dark:text-amber-400 border border-amber-500/20"
							>
								<XCircleIcon className="w-3 h-3" />
								Dismiss only
							</motion.span>
						)}
						{flowscriptReady && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-cyan-500/15 text-cyan-600 dark:text-cyan-400 border border-cyan-500/20"
							>
								<PencilIcon className="w-3 h-3" />
								FlowScript
							</motion.span>
						)}
						{hasRetainedReview && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-violet-500/15 text-violet-600 dark:text-violet-400 border border-violet-500/20"
							>
								<SparklesIcon className="w-3 h-3" />
								{retainedReviewCount} exact commands
							</motion.span>
						)}
						{hasRetainedReview &&
							retainedCommandBreakdown.map(([kind, count]) => (
								<motion.span
									key={kind}
									initial={{ scale: 0 }}
									animate={{ scale: 1 }}
									className="inline-flex items-center gap-1 rounded-full border border-border/50 bg-background/50 px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
								>
									{count} {kind.replace(/([a-z])([A-Z])/g, "$1 $2")}
								</motion.span>
							))}
						{!hasRetainedReview && commandCounts.add > 0 && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-green-500/15 text-green-600 dark:text-green-400 border border-green-500/20"
							>
								<PlusCircleIcon className="w-3 h-3" />
								{commandCounts.add} node{commandCounts.add > 1 ? "s" : ""}
							</motion.span>
						)}
						{!hasRetainedReview && commandCounts.connect > 0 && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								transition={{ delay: 0.05 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-blue-500/15 text-blue-600 dark:text-blue-400 border border-blue-500/20"
							>
								<LinkIcon className="w-3 h-3" />
								{commandCounts.connect} connection
								{commandCounts.connect > 1 ? "s" : ""}
							</motion.span>
						)}
						{!hasRetainedReview && commandCounts.update > 0 && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								transition={{ delay: 0.1 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-violet-500/15 text-violet-600 dark:text-violet-400 border border-violet-500/20"
							>
								<PencilIcon className="w-3 h-3" />
								{commandCounts.update} update
								{commandCounts.update > 1 ? "s" : ""}
							</motion.span>
						)}
						{!hasRetainedReview && commandCounts.remove > 0 && (
							<motion.span
								initial={{ scale: 0 }}
								animate={{ scale: 1 }}
								transition={{ delay: 0.15 }}
								className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-red-500/15 text-red-600 dark:text-red-400 border border-red-500/20"
							>
								<XCircleIcon className="w-3 h-3" />
								{commandCounts.remove} removal
								{commandCounts.remove > 1 ? "s" : ""}
							</motion.span>
						)}
					</div>
					{retainedReviewIsDestructive && (
						<div className="mt-3 rounded-xl border border-destructive/30 bg-destructive/10 p-2.5 text-xs text-destructive">
							<div className="flex items-center gap-1.5 font-semibold">
								<AlertTriangleIcon className="h-3.5 w-3.5 shrink-0" />
								This exact batch can remove or replace board content.
							</div>
							{retainedDestructiveEffects.length > 0 && (
								<ul className="mt-1 list-disc space-y-0.5 pl-5 text-[11px]">
									{retainedDestructiveEffects.map((effect) => (
										<li key={effect}>{effect}</li>
									))}
								</ul>
							)}
							{destructiveConfirmationArmed && (
								<div className="mt-1.5 font-medium">
									Review the effects above, then click Confirm Apply. The native
									host will re-check this exact batch before mutation.
								</div>
							)}
						</div>
					)}
				</div>
			</div>

			{/* Expanded command list */}
			<AnimatePresence>
				{expanded && hasCommandDetails && (
					<motion.div
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1 }}
						exit={{ height: 0, opacity: 0 }}
						transition={{ duration: 0.2 }}
						className="overflow-hidden"
					>
						<div className="pt-2 space-y-1.5 max-h-40 overflow-y-auto">
							{!hasRetainedReview &&
								commands.map((cmd, i) => (
									<motion.div
										key={`${cmd.command_type}:${JSON.stringify(cmd)}`}
										initial={{ opacity: 0, x: -10 }}
										animate={{ opacity: 1, x: 0 }}
										transition={{ delay: i * 0.03 }}
										className={`group relative flex items-center gap-2.5 p-2.5 rounded-xl bg-linear-to-r ${getCommandColor(cmd)} border cursor-pointer transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]`}
										onClick={() => onExecuteSingle(i)}
										onMouseEnter={() => setHoveredIndex(i)}
										onMouseLeave={() => setHoveredIndex(null)}
									>
										<div className="shrink-0">
											{getCommandIcon(cmd, "w-4 h-4")}
										</div>
										<span className="text-xs font-medium text-foreground truncate flex-1">
											{getCommandSummary(cmd)}
										</span>
										<motion.div
											initial={{ opacity: 0, scale: 0.8 }}
											animate={{
												opacity: hoveredIndex === i ? 1 : 0,
												scale: hoveredIndex === i ? 1 : 0.8,
											}}
											className="shrink-0 p-1 rounded-md bg-primary/20"
										>
											<PlayIcon className="w-3 h-3 text-primary" />
										</motion.div>
									</motion.div>
								))}
							{hasRetainedReview &&
								keyedRetainedCommandSummaries.map(({ key, summary }, index) => (
									<motion.div
										key={key}
										initial={{ opacity: 0, x: -10 }}
										animate={{ opacity: 1, x: 0 }}
										transition={{ delay: index * 0.03 }}
										className="flex items-start gap-2.5 rounded-xl border border-border/50 bg-background/60 p-2.5"
									>
										<SparklesIcon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
										<span className="text-xs text-foreground">{summary}</span>
									</motion.div>
								))}
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</motion.div>
	);
});
