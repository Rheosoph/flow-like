"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlarmClockIcon,
	Loader2Icon,
	PauseIcon,
	PlayIcon,
	PlusIcon,
	Trash2Icon,
} from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";
import {
	useBotTasks,
	useCreateBotTask,
	useDeleteBotTask,
	useUpdateBotTask,
} from "../../hooks/use-bots";
import { formatRelativeTime } from "../../lib/date";
import type {
	IBot,
	IBotTask,
	ICreateBotTaskRequest,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";
import {
	MAX_BOT_TASKS,
	type TaskDraft,
	browserTimeZone,
	describeSchedule,
	emptyTaskDraft,
	taskRequest,
	taskState,
} from "./bot-task-model";

/** Whether the Details tab offers tasks for this bot. */
export function offersTasks(bot: IBot): boolean {
	return bot.capabilities.includes("scheduling");
}

function NewTask({
	busy,
	onCreate,
}: Readonly<{
	busy: boolean;
	onCreate: (request: ICreateBotTaskRequest) => void;
}>) {
	const { t } = useTranslation("bots");
	const monitoringId = useId();
	const [draft, setDraft] = useState<TaskDraft>(() =>
		emptyTaskDraft(browserTimeZone()),
	);
	const request = taskRequest(draft);
	const set = (patch: Partial<TaskDraft>) =>
		setDraft((current) => ({ ...current, ...patch }));
	return (
		<form
			className="flex flex-col gap-2"
			data-slot="bot-task-form"
			onSubmit={(event) => {
				event.preventDefault();
				if (!request) return;
				onCreate(request);
				setDraft(emptyTaskDraft(draft.tz));
			}}
		>
			<div className="flex gap-1.5">
				{(["once", "cron"] as const).map((kind) => (
					<Button
						key={kind}
						type="button"
						size="sm"
						variant={draft.kind === kind ? "secondary" : "ghost"}
						aria-pressed={draft.kind === kind}
						onClick={() => set({ kind })}
					>
						{kind === "once"
							? String(t("panel.tasks.once", "Once"))
							: String(t("panel.tasks.repeat", "Repeat"))}
					</Button>
				))}
			</div>
			{draft.kind === "once" ? (
				<Input
					type="datetime-local"
					value={draft.at}
					onChange={(event) => set({ at: event.target.value })}
					aria-label={String(t("panel.tasks.at", "When"))}
				/>
			) : (
				<div className="flex gap-1.5">
					<Input
						value={draft.cron}
						onChange={(event) => set({ cron: event.target.value })}
						placeholder={t("panel.tasks.cron", "0 9 * * 1-5")}
						aria-label={t("panel.tasks.cronLabel", "Cron schedule")}
						className="font-mono text-xs"
					/>
					<Input
						value={draft.tz}
						onChange={(event) => set({ tz: event.target.value })}
						aria-label={t("panel.tasks.tz", "Time zone")}
						className="w-40 text-xs"
					/>
				</div>
			)}
			<Input
				value={draft.title}
				maxLength={200}
				onChange={(event) => set({ title: event.target.value })}
				placeholder={t("panel.tasks.name", "Name (optional)")}
				aria-label={t("panel.tasks.nameLabel", "Task name")}
			/>
			<Textarea
				value={draft.instruction}
				maxLength={4000}
				rows={2}
				onChange={(event) => set({ instruction: event.target.value })}
				placeholder={t("panel.tasks.instruction", "What should the bot do?")}
				aria-label={t("panel.tasks.instructionLabel", "Instruction")}
			/>
			<label htmlFor={monitoringId} className="flex items-start gap-2 text-sm">
				<Switch
					id={monitoringId}
					checked={draft.monitoring ?? false}
					onCheckedChange={(monitoring) => set({ monitoring })}
					aria-label={t(
						"panel.tasks.monitoring",
						"Only report meaningful changes",
					)}
				/>
				<span>
					{t("panel.tasks.monitoring", "Only report meaningful changes")}
					<span className="block text-xs text-muted-foreground">
						{t(
							"panel.tasks.monitoringHint",
							"Uses the bot's decision model after each check. Leave off for reminders and regular reports.",
						)}
					</span>
				</span>
			</label>
			<Button
				type="submit"
				size="sm"
				className="self-start"
				disabled={busy || !request}
			>
				{busy ? (
					<Loader2Icon className="animate-spin" aria-hidden />
				) : (
					<PlusIcon aria-hidden />
				)}
				{t("panel.tasks.create", "Add task")}
			</Button>
		</form>
	);
}

function nextRun(task: IBotTask, label: string, fallback: string): string {
	if (!task.next_run_at) return fallback;
	return `${label} ${formatRelativeTime(task.next_run_at, "long", "")}`;
}

function TaskRow({
	task,
	busy,
	onToggle,
	onDelete,
}: Readonly<{
	task: IBotTask;
	busy: boolean;
	onToggle: (task: IBotTask) => void;
	onDelete: (task: IBotTask) => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	const state = taskState(task);
	const fallback =
		state === "done"
			? t("panel.tasks.done", "Done")
			: t("panel.tasks.paused", "Paused");
	return (
		<li
			data-slot="bot-task"
			data-state={state}
			className="flex items-center gap-2 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3"
		>
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm font-medium">{task.title}</p>
				<p className="truncate text-xs text-muted-foreground">
					{task.monitoring &&
						`${t("panel.tasks.monitoringLabel", "Monitoring")} · `}
					{describeSchedule(task, i18n?.language)}
					{" · "}
					{nextRun(task, t("panel.tasks.next", "Next"), fallback)}
				</p>
			</div>
			{state !== "done" && (
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-7 shrink-0 text-muted-foreground"
					disabled={busy}
					aria-label={
						task.enabled
							? t("panel.tasks.pause", "Pause {{name}}", { name: task.title })
							: t("panel.tasks.resume", "Resume {{name}}", { name: task.title })
					}
					onClick={() => onToggle(task)}
				>
					{task.enabled ? <PauseIcon aria-hidden /> : <PlayIcon aria-hidden />}
				</Button>
			)}
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0 text-muted-foreground"
				disabled={busy}
				aria-label={t("panel.tasks.delete", "Delete {{name}}", {
					name: task.title,
				})}
				onClick={() => onDelete(task)}
			>
				<Trash2Icon aria-hidden />
			</Button>
		</li>
	);
}

/** The tasks list and the new-task form, apart from where the tasks come from. */
export function BotTasksView({
	tasks,
	busy,
	onCreate,
	onToggle,
	onDelete,
}: Readonly<{
	tasks: readonly IBotTask[];
	busy: boolean;
	onCreate: (request: ICreateBotTaskRequest) => void;
	onToggle: (task: IBotTask) => void;
	onDelete: (task: IBotTask) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<section className="flex flex-col gap-3" data-slot="bot-tasks">
			<h3 className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<AlarmClockIcon className="size-3.5" aria-hidden />
				{t("panel.tasks.title", "Tasks")}
			</h3>
			<ul className="flex flex-col gap-1.5">
				{tasks.map((task) => (
					<TaskRow
						key={task.id}
						task={task}
						busy={busy}
						onToggle={onToggle}
						onDelete={onDelete}
					/>
				))}
			</ul>
			{tasks.length === 0 && (
				<p className="text-sm text-muted-foreground">
					{t(
						"panel.tasks.none",
						"No tasks yet. Schedule one here, or ask the bot to remind you.",
					)}
				</p>
			)}
			{tasks.length < MAX_BOT_TASKS ? (
				<NewTask busy={busy} onCreate={onCreate} />
			) : (
				<p className="text-xs text-muted-foreground">
					{t(
						"panel.tasks.full",
						"This bot has {{count}} tasks, the most it can keep. Delete one to add another.",
						{ count: MAX_BOT_TASKS },
					)}
				</p>
			)}
		</section>
	);
}

/** Scheduled work: what the bot runs on its own, when it runs next, pause and delete. */
export function BotTasks({ bot }: Readonly<{ bot: IBot }>) {
	const tasks = useBotTasks(bot.id);
	const create = useCreateBotTask();
	const update = useUpdateBotTask();
	const remove = useDeleteBotTask();
	const onError = (error: Error) => toast.error(error.message);
	return (
		<BotTasksView
			tasks={tasks.data ?? []}
			busy={create.isPending || update.isPending || remove.isPending}
			onCreate={(request) =>
				create.mutate({ botId: bot.id, request }, { onError })
			}
			onToggle={(task) =>
				update.mutate(
					{
						botId: bot.id,
						taskId: task.id,
						request: { enabled: !task.enabled },
					},
					{ onError },
				)
			}
			onDelete={(task) =>
				remove.mutate({ botId: bot.id, taskId: task.id }, { onError })
			}
		/>
	);
}
