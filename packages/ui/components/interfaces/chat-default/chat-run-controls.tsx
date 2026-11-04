"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ClockIcon,
	CornerDownRightIcon,
	Loader2Icon,
	SquareIcon,
	XIcon,
} from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { cn } from "../../../lib";
import { Button, Popover, PopoverContent, PopoverTrigger } from "../../ui";
import type { IChatActiveRun, IChatConcurrency } from "./chat";
import type { ChatBoxStop } from "./chatbox";

function truncate(text: string, max = 48) {
	const clean = text.replace(/\s+/g, " ").trim();
	return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean;
}

/** What the composer's send button does while responses are live: stop every one of them. */
export function useRunStop(
	concurrency: IChatConcurrency | undefined,
): ChatBoxStop | undefined {
	const { t } = useTranslation("chat");
	const runs = concurrency?.runs;
	const onStop = concurrency?.onStop;
	return useMemo(() => {
		if (!runs || !onStop || runs.length === 0) return undefined;
		return {
			label:
				runs.length > 1
					? t("stopAllResponses", "Stop all responses")
					: t("stopResponse", "Stop response"),
			stopping: runs.every((run) => run.status === "cancelling"),
			onStop: () => {
				for (const run of runs) {
					if (run.status === "streaming") onStop(run.runId);
				}
			},
		};
	}, [runs, onStop, t]);
}

/** The steering instructions the user pushed into a run, with their delivery outcome. */
function SteerTrail({ run }: { run: IChatActiveRun }) {
	const { t } = useTranslation("chat");
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
						<span className="opacity-80">
							{t("notDelivered", "not delivered")}
						</span>
					)}
				</div>
			))}
		</div>
	);
}

function ActivitySection({
	title,
	action,
	children,
}: {
	title: string;
	action?: ReactNode;
	children: ReactNode;
}) {
	return (
		<div className="flex flex-col">
			<div className="flex h-7 items-center justify-between px-2">
				<span className="text-xs font-medium text-muted-foreground">
					{title}
				</span>
				{action}
			</div>
			{children}
		</div>
	);
}

function RunRow({
	run,
	onStop,
}: {
	run: IChatActiveRun;
	onStop: (runId: string) => void;
}) {
	const { t } = useTranslation("chat");
	const cancelling = run.status === "cancelling";
	return (
		<div className="flex flex-col gap-0.5 rounded-md px-2 py-1 hover:bg-accent/50">
			<div className="flex items-center gap-2">
				<span
					className={cn(
						"size-1.5 shrink-0 rounded-full",
						cancelling ? "bg-muted-foreground" : "animate-pulse bg-primary",
					)}
				/>
				<span
					className={cn(
						"min-w-0 flex-1 truncate text-xs",
						cancelling && "text-muted-foreground",
					)}
				>
					{cancelling ? t("stopping", "Stopping —") : ""}
					{truncate(run.label)}
				</span>
				<Button
					variant="ghost"
					size="icon"
					className="size-6 shrink-0"
					disabled={cancelling}
					onClick={() => onStop(run.runId)}
					title={t("stopThisResponse", "Stop this response")}
					aria-label={t("stopResponseVal", "Stop response: {{val}}", {
						val: truncate(run.label),
					})}
				>
					<SquareIcon className="size-2.5 fill-current" />
				</Button>
			</div>
			<SteerTrail run={run} />
		</div>
	);
}

function QueuedRow({
	content,
	onRemove,
}: {
	content: string;
	onRemove: () => void;
}) {
	const { t } = useTranslation("chat");
	const removeLabel = t(
		"removeQueuedMessageVal",
		"Remove queued message: {{val}}",
		{ val: truncate(content) },
	);
	return (
		<div className="flex items-center gap-2 rounded-md px-2 py-1 hover:bg-accent/50">
			<ClockIcon className="size-3 shrink-0 text-muted-foreground" />
			<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
				{truncate(content)}
			</span>
			<Button
				variant="ghost"
				size="icon"
				className="size-6 shrink-0"
				onClick={onRemove}
				title={removeLabel}
				aria-label={removeLabel}
			>
				<XIcon className="size-3" />
			</Button>
		</div>
	);
}

function ChipCount({
	icon,
	count,
	className,
}: {
	icon: ReactNode;
	count?: number;
	className?: string;
}) {
	return (
		<span className={cn("inline-flex items-center gap-1", className)}>
			{icon}
			{count !== undefined && <span>{count}</span>}
		</span>
	);
}

/**
 * The composer's live-turn status: a compact chip beside the send button that opens the full list —
 * one row per generating turn (own stop, steer trail) and one per queued message (remove). Hidden
 * while the send button's Stop already says everything, i.e. a single plain turn and an empty
 * composer.
 */
export function ChatRunStatus({
	concurrency,
	stopShown,
}: {
	concurrency: IChatConcurrency;
	stopShown: boolean;
}) {
	const { t } = useTranslation("chat");
	const { runs, queued, atCapacity, onStop, onRemoveQueued } = concurrency;
	const steers = runs.flatMap((run) => run.steers);
	const steerFailed = steers.some((steer) => steer.status === "failed");
	const steerPending = steers.some((steer) => steer.status === "pending");

	if (runs.length === 0 && queued.length === 0) return null;
	if (stopShown && runs.length <= 1 && queued.length === 0 && !steers.length)
		return null;

	const streaming = runs.filter((run) => run.status === "streaming");
	const label = t("responseActivity", "Running responses and queued messages");

	return (
		<Popover>
			<PopoverTrigger asChild>
				<button
					type="button"
					title={label}
					aria-label={label}
					className="inline-flex h-11 shrink-0 items-center gap-2 rounded-full px-2.5 text-xs tabular-nums text-muted-foreground transition-colors hover:bg-accent hover:text-foreground sm:h-8"
				>
					{runs.length > 0 && (
						<ChipCount
							icon={
								<Loader2Icon
									className={cn(
										"size-3.5",
										streaming.length > 0 && "animate-spin text-primary",
									)}
								/>
							}
							count={runs.length}
						/>
					)}
					{queued.length > 0 && (
						<ChipCount
							icon={<ClockIcon className="size-3.5" />}
							count={queued.length}
						/>
					)}
					{steers.length > 0 && (
						<ChipCount
							icon={<CornerDownRightIcon className="size-3.5" />}
							count={steers.length}
							className={cn(
								steerFailed && "text-destructive",
								!steerFailed && steerPending && "animate-pulse",
							)}
						/>
					)}
				</button>
			</PopoverTrigger>
			<PopoverContent side="top" align="end" className="mb-1 w-72 p-1">
				<div className="flex max-h-80 flex-col gap-1 overflow-y-auto">
					{runs.length > 0 && (
						<ActivitySection
							title={t("responding", "Responding")}
							action={
								streaming.length > 1 && (
									<Button
										variant="ghost"
										size="sm"
										className="h-6 px-2 text-xs"
										onClick={() => {
											for (const run of streaming) onStop(run.runId);
										}}
									>
										{t("stopAll", "Stop all")}
									</Button>
								)
							}
						>
							{runs.map((run) => (
								<RunRow key={run.runId} run={run} onStop={onStop} />
							))}
						</ActivitySection>
					)}
					{queued.length > 0 && (
						<ActivitySection title={t("queued", "Queued")}>
							{queued.map((entry) => (
								<QueuedRow
									key={entry.id}
									content={entry.content}
									onRemove={() => onRemoveQueued(entry.id)}
								/>
							))}
						</ActivitySection>
					)}
				</div>
				{atCapacity && (
					<p className="border-t px-2 pt-1.5 pb-1 text-[11px] text-muted-foreground">
						{t(
							"allResponseSlotsAreBusyTheNextMessageWillBeQueued",
							"All response slots are busy — the next message will be queued.",
						)}
					</p>
				)}
			</PopoverContent>
		</Popover>
	);
}
