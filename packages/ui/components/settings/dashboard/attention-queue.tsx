"use client";

import {
	AlertTriangleIcon,
	ArrowRightIcon,
	CheckCircle2Icon,
} from "lucide-react";
import Link from "next/link";
import { cn } from "../../../lib/utils";
import { Card } from "../../ui/card";
import { StateDot } from "./dashboard-primitives";
import type {
	AttentionSignal,
	InspectorPanel,
	SignalTone,
} from "./use-project-signals";

const TONE_DOT: Record<SignalTone, "critical" | "warn" | "idle"> = {
	critical: "critical",
	warning: "warn",
	info: "idle",
};

function SignalChip({
	signal,
	onOpenPanel,
}: Readonly<{
	signal: AttentionSignal;
	onOpenPanel: (panel: InspectorPanel) => void;
}>) {
	const body = (
		<>
			<StateDot tone={TONE_DOT[signal.tone]} />
			<span className="truncate">
				{signal.label}
				{signal.subject && (
					<>
						{" — "}
						<span className="font-medium text-foreground">
							{signal.subject}
						</span>
					</>
				)}
			</span>
			<ArrowRightIcon className="h-3 w-3 shrink-0 text-muted-foreground" />
		</>
	);

	const className =
		"flex max-w-full items-center gap-2 rounded-full border bg-card px-3 py-1 text-xs text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground";

	if (signal.href) {
		return (
			<Link href={signal.href} className={className} title={signal.actionLabel}>
				{body}
			</Link>
		);
	}

	return (
		<button
			type="button"
			className={className}
			onClick={() => signal.panel && onOpenPanel(signal.panel)}
			title={signal.actionLabel}
		>
			{body}
		</button>
	);
}

/**
 * The ranked "needs you" strip. Only renders when there is something real to
 * act on — an all-clear state is a single quiet line rather than an empty box.
 */
export function AttentionQueue({
	signals,
	onOpenPanel,
	className,
}: Readonly<{
	signals: AttentionSignal[];
	onOpenPanel: (panel: InspectorPanel) => void;
	className?: string;
}>) {
	if (signals.length === 0) {
		return (
			<div
				className={cn(
					"flex items-center gap-2 px-1 text-xs text-muted-foreground",
					className,
				)}
			>
				<CheckCircle2Icon className="h-3.5 w-3.5 text-emerald-500" />
				Nothing needs your attention.
			</div>
		);
	}

	return (
		<Card
			className={cn(
				"flex flex-row flex-wrap items-center gap-2 border-amber-500/40 bg-amber-500/5 px-3 py-2.5",
				className,
			)}
		>
			<span className="flex items-center gap-1.5 whitespace-nowrap text-xs font-semibold text-amber-600 dark:text-amber-400">
				<AlertTriangleIcon className="h-3.5 w-3.5" />
				Needs you · {signals.length}
			</span>
			<div className="flex min-w-0 flex-wrap items-center gap-2">
				{signals.map((signal) => (
					<SignalChip
						key={signal.id}
						signal={signal}
						onOpenPanel={onOpenPanel}
					/>
				))}
			</div>
		</Card>
	);
}
