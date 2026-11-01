"use client";

import {
	ArrowLeft,
	ArrowUpRight,
	Check,
	CheckCheck,
	CircleAlert,
	CircleHelp,
	Clock3,
	Loader2,
	Square,
	X,
} from "lucide-react";
import { Suspense, lazy, useEffect, useState } from "react";
import { useClientRouter } from "../lib/client-navigation";
import type { ExecutionTask } from "../lib/execution/types";
import type { IInteractionRequest } from "../lib/schema/interaction";
import { cn } from "../lib/utils";
import { useExecutionEngine } from "../state/execution-engine-context";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

const Interaction = lazy(() =>
	import("./interfaces/chat-default/interaction").then((module) => ({
		default: module.Interaction,
	})),
);

const statusDetails = {
	starting: {
		label: "Starting",
		Icon: Loader2,
		color: "text-muted-foreground",
	},
	running: {
		label: "Running",
		Icon: Loader2,
		color: "text-orange-700 dark:text-orange-400",
	},
	waiting: {
		label: "Needs your input",
		Icon: CircleHelp,
		color: "text-amber-700 dark:text-amber-400",
	},
	completed: {
		label: "Completed",
		Icon: Check,
		color: "text-emerald-700 dark:text-emerald-400",
	},
	failed: {
		label: "Failed",
		Icon: CircleAlert,
		color: "text-red-700 dark:text-red-400",
	},
	cancelled: { label: "Stopped", Icon: Square, color: "text-muted-foreground" },
} as const;

function isActive(task: ExecutionTask) {
	return ["starting", "running", "waiting"].includes(task.status);
}

function pendingQuestions(task: ExecutionTask) {
	return task.pendingInteractions.filter(
		(request) =>
			request.event.event_type === "interaction_request" &&
			(request.status === "pending" || request.status === "submitting"),
	);
}

function relativeTime(task: ExecutionTask, now: number) {
	const timestamp = task.completedAt ?? Date.parse(task.startedAt);
	const seconds = Math.max(0, Math.floor((now - timestamp) / 1000));
	if (!Number.isFinite(seconds) || seconds < 10) return "Just now";
	if (seconds < 60) return `${seconds}s`;
	if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
	if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
	return `${Math.floor(seconds / 86_400)}d`;
}

function taskPreview(task: ExecutionTask) {
	if (task.status === "failed" && task.error) return task.error;
	if (task.preview) return task.preview.replace(/\s+/g, " ").trim();
	if (task.status === "waiting") {
		const request = pendingQuestions(task)[0];
		if (typeof request?.event.payload?.name === "string")
			return request.event.payload.name;
		if (!request) return "Waiting for an app page or device to respond.";
	}
	if (task.status === "starting") return "Getting ready to run…";
	if (task.status === "running") return "Working on your request…";
	if (task.status === "completed") return "Your run has finished.";
	if (task.status === "cancelled") return "This run was stopped.";
	return "Open the conversation for details.";
}

export interface ExecutionActivityTrayProps {
	tasks: ExecutionTask[];
	onOpenTask: (task: ExecutionTask) => void;
	onCancelTask: (taskId: string) => Promise<void>;
	onDismissTask: (taskId: string) => void;
	onReply: (taskId: string, requestId: string, value: unknown) => Promise<void>;
}

/** The tray stays outside route content so switching pages cannot detach its controls. */
export function ExecutionActivityTray({
	tasks,
	onOpenTask,
	onCancelTask,
	onDismissTask,
	onReply,
}: ExecutionActivityTrayProps) {
	const [open, setOpen] = useState(false);
	const [selectedId, setSelectedId] = useState<string>();
	const [busy, setBusy] = useState<Set<string>>(new Set());
	const [errors, setErrors] = useState<Record<string, string>>({});
	const [now, setNow] = useState(Date.now);
	useEffect(() => {
		if (!open) return;
		setNow(Date.now());
		const timer = setInterval(() => setNow(Date.now()), 10_000);
		return () => clearInterval(timer);
	}, [open]);

	const waiting = tasks.filter(
		(task) => task.status === "waiting" && pendingQuestions(task).length > 0,
	);
	const running = tasks.filter(
		(task) => isActive(task) && !waiting.includes(task),
	);
	const recent = tasks
		.filter((task) => !isActive(task))
		.sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0));
	const activeCount = waiting.length + running.length;
	const selected = tasks.find((task) => task.taskId === selectedId);
	const selectedRequests = selected ? pendingQuestions(selected) : undefined;

	const perform = async (key: string, action: () => Promise<void>) => {
		setBusy((current) => new Set(current).add(key));
		setErrors((current) => {
			const next = { ...current };
			delete next[key];
			return next;
		});
		try {
			await action();
		} catch (error) {
			setErrors((current) => ({
				...current,
				[key]: error instanceof Error ? error.message : "Please try again.",
			}));
		} finally {
			setBusy((current) => {
				const next = new Set(current);
				next.delete(key);
				return next;
			});
		}
	};

	if (tasks.length === 0) return null;
	const openTask = (task: ExecutionTask) => {
		setOpen(false);
		onOpenTask(task);
	};

	const renderTask = (task: ExecutionTask) => {
		const needsInput =
			task.status === "waiting" && pendingQuestions(task).length > 0;
		const { Icon, label, color } =
			task.status === "waiting" && !needsInput
				? {
						Icon: Clock3,
						label: "Waiting for page or device",
						color: "text-muted-foreground",
					}
				: statusDetails[task.status];
		const active = isActive(task);
		const taskBusy =
			busy.has(task.taskId) || (active && task.cancellationRequested === true);
		return (
			<li
				key={task.taskId}
				data-task-id={task.taskId}
				data-task-status={task.status}
				className="group rounded-xl px-3 py-2.5 transition-colors hover:bg-muted/45"
			>
				<div className="flex gap-3">
					<div
						className={cn(
							"mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-background",
							needsInput && "border-amber-500/20 bg-amber-500/10",
							color,
						)}
					>
						<Icon
							aria-hidden="true"
							className={cn(
								"size-4",
								(task.status === "running" || task.status === "starting") &&
									"motion-safe:animate-spin",
							)}
						/>
					</div>
					<div className="min-w-0 flex-1">
						<div className="flex items-baseline gap-2">
							<p className="min-w-0 flex-1 truncate text-[13px] font-semibold leading-5">
								{task.title || "Workflow"}
							</p>
							<time
								dateTime={
									task.completedAt
										? new Date(task.completedAt).toISOString()
										: task.startedAt
								}
								className="shrink-0 text-[11px] tabular-nums text-muted-foreground"
								title={
									active
										? "Time since this run started"
										: "Time since this run ended"
								}
							>
								{relativeTime(task, now)}
							</time>
						</div>
						<p className="mt-0.5 line-clamp-2 break-words text-xs leading-[1.6] text-muted-foreground">
							{taskPreview(task)}
						</p>
						<div className="mt-2 flex min-h-6 items-center gap-1.5">
							<span className={cn("mr-auto text-[10px] font-medium", color)}>
								{active && task.cancellationRequested ? "Stopping…" : label}
							</span>
							{needsInput ? (
								<button
									type="button"
									disabled={task.cancellationRequested}
									onClick={() => setSelectedId(task.taskId)}
									className="rounded-md bg-amber-500/15 px-2.5 py-1 text-[11px] font-semibold text-amber-800 transition-colors hover:bg-amber-500/25 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring dark:text-amber-300"
								>
									Respond
								</button>
							) : task.path ? (
								<button
									type="button"
									aria-label={`Open ${task.title || "workflow"}`}
									onClick={() => openTask(task)}
									className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
								>
									Open <ArrowUpRight aria-hidden="true" className="size-3" />
								</button>
							) : null}
							<button
								type="button"
								aria-label={`${active ? "Stop" : "Dismiss"} ${task.title || "workflow"}`}
								title={active ? "Stop run" : "Dismiss"}
								disabled={taskBusy}
								onClick={() =>
									active
										? void perform(task.taskId, () => onCancelTask(task.taskId))
										: onDismissTask(task.taskId)
								}
								className={cn(
									"flex h-7 shrink-0 items-center justify-center gap-1 rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50",
									active ? "px-1.5" : "w-7",
								)}
							>
								{taskBusy ? (
									<Loader2
										aria-hidden="true"
										className="size-3 motion-safe:animate-spin"
									/>
								) : active ? (
									<Square aria-hidden="true" className="size-3" />
								) : (
									<X aria-hidden="true" className="size-3.5" />
								)}
								{active && (
									<span className="text-[11px] font-medium">Stop</span>
								)}
							</button>
						</div>
						{errors[task.taskId] && (
							<p
								role="alert"
								className="mt-1 text-xs text-red-700 dark:text-red-400"
							>
								{errors[task.taskId]}
							</p>
						)}
					</div>
				</div>
			</li>
		);
	};

	return (
		<div className="fixed bottom-[calc(5rem+var(--fl-safe-bottom,0px))] right-3 z-50 sm:bottom-[calc(1rem+var(--fl-safe-bottom,0px))] sm:right-4">
			<Popover
				open={open}
				onOpenChange={(next) => {
					setOpen(next);
					if (!next) setSelectedId(undefined);
				}}
			>
				<PopoverTrigger asChild>
					<button
						type="button"
						aria-label={`Activity, ${activeCount} active${waiting.length ? `, ${waiting.length} need your input` : ""}`}
						className="flex h-10 items-center gap-2 rounded-full border border-border/80 bg-popover/95 py-2 pl-3 pr-2 text-popover-foreground shadow-[0_4px_24px_-8px_rgb(0_0_0/0.24)] backdrop-blur-xl transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring"
					>
						{waiting.length > 0 ? (
							<CircleHelp
								aria-hidden="true"
								className="size-4 text-amber-700 dark:text-amber-400"
							/>
						) : activeCount > 0 ? (
							<Loader2
								aria-hidden="true"
								className="size-4 text-primary motion-safe:animate-spin"
							/>
						) : (
							<CheckCheck
								aria-hidden="true"
								className="size-4 text-muted-foreground"
							/>
						)}
						<span className="text-xs font-semibold">Activity</span>
						<span
							className={cn(
								"flex h-6 min-w-6 items-center justify-center rounded-full bg-muted px-1.5 text-[11px] font-semibold tabular-nums",
								waiting.length > 0 &&
									"bg-amber-500/15 text-amber-800 dark:text-amber-300",
							)}
						>
							{activeCount || recent.length}
						</span>
					</button>
				</PopoverTrigger>
				<PopoverContent
					align="end"
					side="top"
					sideOffset={12}
					collisionPadding={12}
					aria-label="Execution activity"
					className="flex max-h-[min(680px,var(--radix-popover-content-available-height))] w-[min(400px,calc(100vw-24px))] flex-col overflow-hidden rounded-2xl border-border/80 bg-popover p-0 shadow-[0_16px_60px_-12px_rgb(0_0_0/0.24)]"
				>
					<header className="flex shrink-0 items-center gap-3 border-b border-border/60 px-5 py-4">
						{selected && (
							<button
								type="button"
								aria-label="Back to activity"
								onClick={() => setSelectedId(undefined)}
								className="-ml-1 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
							>
								<ArrowLeft aria-hidden="true" className="size-4" />
							</button>
						)}
						<div className="min-w-0 flex-1">
							<h2 className="truncate text-sm font-semibold leading-5 tracking-tight">
								{selected ? selected.title : "Activity"}
							</h2>
							<p className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
								{selected
									? "Your response keeps this run moving."
									: waiting.length > 0
										? `${waiting.length} ${waiting.length === 1 ? "run needs" : "runs need"} your input`
										: activeCount > 0
											? `${activeCount} ${activeCount === 1 ? "run in progress" : "runs in progress"}`
											: "Your recent runs"}
							</p>
						</div>
						<button
							type="button"
							aria-label="Close activity"
							onClick={() => {
								setOpen(false);
								setSelectedId(undefined);
							}}
							className="-mr-1 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
						>
							<X aria-hidden="true" className="size-4" />
						</button>
					</header>
					<div className="min-h-0 overflow-y-auto overscroll-contain p-2">
						{selected ? (
							<div className="space-y-3 p-2">
								{selectedRequests?.length ? (
									selectedRequests.map((request) => {
										const key = `${selected.taskId}:${request.requestId}`;
										return (
											<div
												key={request.requestId}
												className="[&_[data-slot=badge]]:text-amber-800 dark:[&_[data-slot=badge]]:text-amber-300 [&_.tabular-nums]:whitespace-nowrap [&_[data-slot=button]]:bg-foreground [&_[data-slot=button]]:text-background"
											>
												<Suspense
													fallback={
														<output className="flex items-center gap-2 p-4 text-xs text-muted-foreground">
															<Loader2 className="size-4 animate-spin" />
															Loading question…
														</output>
													}
												>
													<Interaction
														interaction={
															request.event.payload as IInteractionRequest
														}
														disabled={
															busy.has(key) || request.status === "submitting"
														}
														onRespond={(_id, value) =>
															void perform(key, () =>
																onReply(
																	selected.taskId,
																	request.requestId,
																	value,
																),
															)
														}
													/>
												</Suspense>
												{(errors[key] || request.error) && (
													<p
														role="alert"
														className="mt-2 text-xs text-red-700 dark:text-red-400"
													>
														{errors[key] || request.error}
													</p>
												)}
											</div>
										);
									})
								) : (
									<output className="flex flex-col items-center px-5 py-9 text-center">
										<div className="mb-3 flex size-10 items-center justify-center rounded-full bg-emerald-500/10 text-emerald-700 dark:text-emerald-400">
											<Check className="size-5" />
										</div>
										<p className="text-sm font-medium">You’re all caught up</p>
										<p className="mt-1 text-xs text-muted-foreground">
											This run no longer needs a response.
										</p>
									</output>
								)}
								{selected.path && (
									<button
										type="button"
										onClick={() => openTask(selected)}
										className="flex w-full items-center justify-center gap-1.5 rounded-lg py-2 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
									>
										Open conversation <ArrowUpRight className="size-3.5" />
									</button>
								)}
							</div>
						) : (
							<>
								{[
									{ title: "Needs your input", tasks: waiting },
									{ title: "In progress", tasks: running },
									{ title: "Recent", tasks: recent },
								]
									.filter((section) => section.tasks.length > 0)
									.map((section) => (
										<section
											key={section.title}
											aria-label={section.title}
											className="mb-1 last:mb-0"
										>
											<div className="flex items-center gap-2 px-3 pb-1 pt-2">
												<h3 className="text-[10px] font-semibold leading-4 tracking-[0.08em] text-muted-foreground uppercase">
													{section.title}
												</h3>
												<span className="text-[10px] tabular-nums text-muted-foreground">
													{section.tasks.length}
												</span>
											</div>
											<ul>{section.tasks.map(renderTask)}</ul>
										</section>
									))}
							</>
						)}
					</div>
					{!selected && (
						<footer className="flex shrink-0 items-center gap-2 border-t border-border/60 bg-muted/20 px-5 py-3">
							<Clock3
								aria-hidden="true"
								className="size-3 text-muted-foreground"
							/>
							<p className="mr-auto text-[10px] leading-4 text-muted-foreground">
								Runs continue while you browse.
							</p>
							{recent.length > 0 && (
								<button
									type="button"
									onClick={() => {
										for (const task of recent) onDismissTask(task.taskId);
									}}
									className="rounded text-[10px] font-medium text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
								>
									Clear recent
								</button>
							)}
						</footer>
					)}
				</PopoverContent>
			</Popover>
		</div>
	);
}

export function RunningTasksIndicator() {
	const engine = useExecutionEngine();
	const router = useClientRouter();
	const [tasks, setTasks] = useState<ExecutionTask[]>([]);
	useEffect(() => {
		const update = () => setTasks(engine.getTasks());
		update();
		return engine.subscribeToGlobalUpdates(update);
	}, [engine]);
	return (
		<ExecutionActivityTray
			tasks={tasks}
			onOpenTask={(task) => {
				if (task.path) router.push(task.path);
			}}
			onCancelTask={(taskId) => engine.cancelTask(taskId)}
			onDismissTask={(taskId) => engine.dismissTask(taskId)}
			onReply={(taskId, requestId, value) =>
				engine.replyToTask(taskId, requestId, value)
			}
		/>
	);
}
