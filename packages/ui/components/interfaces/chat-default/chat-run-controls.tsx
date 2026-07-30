"use client";

import {
	ClockIcon,
	CornerDownRightIcon,
	SquareIcon,
	XIcon,
} from "lucide-react";
import { cn } from "../../../lib";
import { Button } from "../../ui";
import type { IChatActiveRun, IChatConcurrency } from "./chat";

function truncate(text: string, max = 48) {
	const clean = text.replace(/\s+/g, " ").trim();
	return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

/** The steering instructions the user pushed into a run, with their delivery outcome. */
function SteerTrail({ run }: { run: IChatActiveRun }) {
	if (run.steers.length === 0) return null;
	return (
		<div className="flex flex-col gap-0.5 pl-5">
			{run.steers.map((steer) => (
				<div
					key={steer.id}
					className={cn(
						"flex items-center gap-1 text-[11px]",
						steer.status === "failed"
							? "text-destructive"
							: "text-muted-foreground",
					)}
					title={steer.error ?? steer.content}
				>
					<CornerDownRightIcon className="size-3 shrink-0" />
					<span className="truncate">{truncate(steer.content, 40)}</span>
					{steer.status === "pending" && <span className="opacity-60">…</span>}
					{steer.status === "failed" && (
						<span className="opacity-80">not delivered</span>
					)}
				</div>
			))}
		</div>
	);
}

/**
 * The strip above the composer: one row per generating turn (with its own stop button) and one per
 * queued message (with remove). This is what makes concurrency legible — without it, several
 * bubbles streaming at once reads as chaos, and a queued message looks like a dropped one.
 */
export function ChatRunControls({
	concurrency,
}: {
	concurrency: IChatConcurrency;
}) {
	const { runs, queued, atCapacity, onStop, onRemoveQueued } = concurrency;
	if (runs.length === 0 && queued.length === 0) return null;

	return (
		<div className="flex flex-col gap-1 rounded-lg border border-border bg-muted/40 px-2 py-1.5">
			{runs.map((run) => (
				<div key={run.runId} className="flex flex-col gap-0.5">
					<div className="flex items-center gap-2">
						<span
							className={cn(
								"size-1.5 shrink-0 rounded-full",
								run.status === "cancelling"
									? "bg-muted-foreground"
									: "animate-pulse bg-primary",
							)}
						/>
						<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
							{run.status === "cancelling" ? "Stopping — " : ""}
							{truncate(run.label)}
						</span>
						<Button
							variant="ghost"
							size="icon"
							className="size-6 shrink-0"
							disabled={run.status === "cancelling"}
							onClick={() => onStop(run.runId)}
							title="Stop this response"
							aria-label={`Stop response: ${truncate(run.label)}`}
						>
							<SquareIcon className="size-3 fill-current" />
						</Button>
					</div>
					<SteerTrail run={run} />
				</div>
			))}
			{queued.map((entry) => (
				<div key={entry.id} className="flex items-center gap-2">
					<ClockIcon className="size-3 shrink-0 text-muted-foreground" />
					<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
						{truncate(entry.content)}
					</span>
					<Button
						variant="ghost"
						size="icon"
						className="size-6 shrink-0"
						onClick={() => onRemoveQueued(entry.id)}
						title="Remove from the queue"
						aria-label={`Remove queued message: ${truncate(entry.content)}`}
					>
						<XIcon className="size-3" />
					</Button>
				</div>
			))}
			{atCapacity && (
				<p className="pl-3.5 text-[11px] text-muted-foreground">
					All response slots are busy — the next message will be queued.
				</p>
			)}
		</div>
	);
}
