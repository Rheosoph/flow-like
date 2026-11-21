"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlarmClockIcon, CircleAlertIcon, Settings2Icon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	useBotCalendar,
	useBotCalendarWake,
	useBotTasks,
	useCreateBotTask,
	useDeleteBotTask,
	useSetBotCalendarWake,
	useUpdateBotTask,
} from "../../hooks/use-bots";
import { getErrorMessage } from "../../lib/error-message";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IBotTask,
	ICalendarEntry,
	ICalendarSource,
	IEventWake,
} from "../../state/backend-state/bot-state";
import {
	botHasInboxHistory,
	jumpBotChat,
} from "../../state/global-chat/bot-inbox";
import {
	type BotFact,
	forgetBotFact,
	listBotFacts,
} from "../../state/global-chat/bot-memory-facts";
import {
	PlanningCalendar,
	type PlanningCalendarChange,
	type PlanningCalendarRange,
} from "../a2ui/display/planning-calendar";
import type { CalendarEvent, CalendarView } from "../a2ui/types";
import { Button } from "../ui/button";
import {
	CalendarItemDialog,
	EventEditorDialog,
	LayerDot,
	OwnEventActions,
	WakeSection,
	WakeSettingsDialog,
} from "./bot-calendar-dialogs";
import { CalendarFeedButton } from "./bot-calendar-feed";
import {
	CALENDAR_LAYERS,
	type CalendarItem,
	type CalendarLayer,
	type EventDraft,
	type WakeChoice,
	calendarItems,
	calendarWindow,
	createEventRequest,
	eventDraftOf,
	eventFields,
	eventWakeOf,
	eventWithWake,
	externalWakeRequest,
	movedEvent,
	newEventDraft,
	updateEventRequest,
	wakeChoiceOf,
} from "./bot-calendar-model";
import { botFactsKey } from "./bot-facts-list";
import { BotMemoryCard } from "./bot-memory-card";
import { memoryFormat } from "./bot-memory-model";
import { type BotPanelTab, botCalendarReach } from "./bot-panel-tabs";
import { browserTimeZone, describeSchedule } from "./bot-task-model";
import { useBotWorkspace } from "./use-bot-workspace";

const VIEWS: readonly CalendarView[] = ["month", "week", "day", "agenda"];

type EditorState =
	| { mode: "create"; draft: EventDraft }
	| { mode: "edit"; draft: EventDraft; task: IBotTask };

function useLayerLabels(): Readonly<Record<CalendarLayer, string>> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			events: t("calendar.layers.events", "Events"),
			external: t("calendar.layers.external", "Connected"),
			tasks: t("calendar.layers.tasks", "Tasks"),
			memories: t("calendar.layers.memories", "Memories"),
		}),
		[t],
	);
}

function useViewLabels(): Readonly<Record<CalendarView, string>> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			month: t("calendar.views.month", "Month"),
			week: t("calendar.views.week", "Week"),
			day: t("calendar.views.day", "Day"),
			agenda: t("calendar.views.agenda", "Agenda"),
		}),
		[t],
	);
}

/** The dates the bot remembers, read only while the Memories layer is on; forgetting one updates the Memory tab too. */
function useCalendarFacts(botId: string, enabled: boolean) {
	const token = useAuth().user?.access_token;
	const queryClient = useQueryClient();
	const { query: workspace } = useBotWorkspace(botId, enabled);
	const appId = workspace.data?.app_id;
	const facts = useQuery({
		queryKey: botFactsKey(botId),
		queryFn: () => listBotFacts({ botId, appId: appId as string, token }),
		enabled: enabled && Boolean(appId),
		retry: false,
	});
	const forget = useMutation({
		mutationFn: (fact: BotFact) => {
			if (!appId) {
				throw new Error(`The memory of bot ${botId} isn't loaded yet`);
			}
			return forgetBotFact({ botId, appId, token }, fact.id);
		},
		onSuccess: (_, fact) =>
			queryClient.setQueryData<BotFact[]>(botFactsKey(botId), (current) =>
				current?.filter((item) => item.id !== fact.id),
			),
	});
	return { facts, forget };
}

function LayerToggles({
	layers,
	shown,
	onToggle,
}: Readonly<{
	layers: readonly CalendarLayer[];
	shown: ReadonlySet<CalendarLayer>;
	onToggle: (layer: CalendarLayer) => void;
}>) {
	const { t } = useTranslation("bots");
	const labels = useLayerLabels();
	return (
		<fieldset
			aria-label={t("calendar.layersLabel", "Show in the calendar")}
			className="flex flex-wrap gap-1.5 border-0 p-0"
		>
			{layers.map((layer) => {
				const on = shown.has(layer);
				return (
					<button
						key={layer}
						type="button"
						aria-pressed={on}
						data-layer={layer}
						onClick={() => onToggle(layer)}
						className={cn(
							"inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring",
							on
								? "border-border bg-muted text-foreground"
								: "border-dashed border-border/80 text-muted-foreground hover:text-foreground",
						)}
					>
						<span className={cn("inline-flex", !on && "opacity-40")}>
							<LayerDot layer={layer} />
						</span>
						{labels[layer]}
					</button>
				);
			})}
		</fieldset>
	);
}

function ViewSwitch({
	view,
	onChange,
}: Readonly<{ view: CalendarView; onChange: (view: CalendarView) => void }>) {
	const { t } = useTranslation("bots");
	const labels = useViewLabels();
	return (
		<fieldset
			aria-label={t("calendar.viewLabel", "Calendar view")}
			className="flex items-center gap-0.5 rounded-lg bg-muted p-0.5"
		>
			{VIEWS.map((item) => (
				<button
					key={item}
					type="button"
					aria-pressed={view === item}
					onClick={() => onChange(item)}
					className={cn(
						"h-6 rounded-md px-2 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-ring",
						view === item
							? "bg-background font-medium text-foreground shadow-xs"
							: "text-muted-foreground hover:text-foreground",
					)}
				>
					{labels[item]}
				</button>
			))}
		</fieldset>
	);
}

/** Connected calendars that failed, or a pointer to connecting one when there is none. */
function SourceNotices({
	sources,
	name,
	onSettings,
}: Readonly<{
	sources: readonly ICalendarSource[];
	name: string;
	onSettings?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const failed = sources.filter((source) => source.error);
	if (sources.length === 0) {
		return (
			<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
				{t(
					"calendar.noSources",
					"No calendar connected. Add Google Calendar, Outlook or a CalDAV calendar such as iCloud or Nextcloud under Calendar in {{name}}'s settings.",
					{ name },
				)}
				{onSettings && (
					<Button
						type="button"
						variant="link"
						size="sm"
						className="h-auto p-0 text-xs"
						onClick={onSettings}
					>
						{t("calendar.openSettings", "Open settings")}
					</Button>
				)}
			</p>
		);
	}
	if (failed.length === 0) return null;
	return (
		<ul className="flex flex-col gap-1" data-slot="calendar-source-errors">
			{failed.map((source) => (
				<li
					key={source.connector_id}
					className="flex items-start gap-1.5 text-xs text-warning"
				>
					<CircleAlertIcon className="mt-px size-3.5 shrink-0" aria-hidden />
					<span className="min-w-0 wrap-anywhere">
						{t("calendar.sourceFailed", "Couldn't read {{label}}: {{error}}", {
							label: source.label,
							error: source.error,
						})}
					</span>
				</li>
			))}
		</ul>
	);
}

/**
 * A bot's calendar: its own events, the connected calendars, its tasks and the dates it
 * remembers, laid over each other in the planning calendar the A2UI Calendar uses.
 */
export function BotCalendarPane({
	bot,
	name,
	onEdit,
	onTab,
}: Readonly<{
	bot: IBot;
	name: string;
	/** Opens the bot's settings, where its connectors are. */
	onEdit?: (bot: IBot) => void;
	onTab?: (tab: BotPanelTab) => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	const locale = i18n?.language;
	const state = useBackend().botState;
	const signedIn = Boolean(useAuth().user?.access_token);
	const layerLabels = useLayerLabels();
	const reach = useMemo(() => botCalendarReach(bot), [bot]);
	// Signed out there is no account to read a connected calendar through.
	const connects = reach.events && (signedIn || !bot.local);
	const available = useMemo(
		() =>
			CALENDAR_LAYERS.filter((layer) => {
				if (layer === "tasks") return reach.tasks;
				if (layer === "memories") return reach.memories;
				if (layer === "external") return connects;
				return reach.events;
			}),
		[reach, connects],
	);
	const [hidden, setHidden] = useState<ReadonlySet<CalendarLayer>>(
		() => new Set(),
	);
	const shown = useMemo(
		() => new Set(available.filter((layer) => !hidden.has(layer))),
		[available, hidden],
	);
	const toggleLayer = useCallback(
		(layer: CalendarLayer) =>
			setHidden((current) => {
				const next = new Set(current);
				if (!next.delete(layer)) next.add(layer);
				return next;
			}),
		[],
	);
	const [view, setView] = useState<CalendarView>("agenda");
	const [range, setRange] = useState<PlanningCalendarRange | null>(null);
	const [today] = useState(() => new Date().toISOString());
	const [now] = useState(() => Date.now());
	const listing = range ? calendarWindow(range) : null;

	const calendar = useBotCalendar(
		reach.events ? bot.id : null,
		listing?.from ?? null,
		listing?.to ?? null,
	);
	const tasks = useBotTasks(reach.events || reach.tasks ? bot.id : null);
	const { facts, forget } = useCalendarFacts(bot.id, shown.has("memories"));
	const wake = useBotCalendarWake(reach.events ? bot.id : null);
	const create = useCreateBotTask();
	const update = useUpdateBotTask();
	const remove = useDeleteBotTask();
	const setWake = useSetBotCalendarWake();
	const busy =
		create.isPending ||
		update.isPending ||
		remove.isPending ||
		forget.isPending;
	const editable = reach.events && Boolean(state?.createTask);

	const items = useMemo(
		() =>
			range
				? calendarItems(
						{
							range: calendar.data,
							tasks: tasks.data,
							facts: facts.data,
						},
						shown,
						{ from: range.from.getTime(), to: range.to.getTime() },
						now,
						editable,
					)
				: [],
		[range, calendar.data, tasks.data, facts.data, shown, now, editable],
	);
	const events = useMemo(() => items.map((item) => item.event), [items]);
	const byId = useMemo(
		() => new Map(items.map((item) => [item.event.id, item])),
		[items],
	);
	const taskOf = useCallback(
		(taskId?: string | null) =>
			taskId ? tasks.data?.find((task) => task.id === taskId) : undefined,
		[tasks.data],
	);

	const [selectedId, setSelectedId] = useState<string | null>(null);
	const selected = selectedId ? (byId.get(selectedId) ?? null) : null;
	const [editor, setEditor] = useState<EditorState | null>(null);
	const [settingsOpen, setSettingsOpen] = useState(false);

	const onError = useCallback(
		(error: Error) => toast.error(getErrorMessage(error)),
		[],
	);

	const openEditor = useCallback((task: IBotTask) => {
		const draft = eventDraftOf(task);
		if (!draft) return;
		setSelectedId(null);
		setEditor({ mode: "edit", draft, task });
	}, []);

	const saveEditor = useCallback(() => {
		if (!editor) return;
		const base = editor.mode === "edit" ? editor.task.event : null;
		const fields = eventFields(editor.draft, browserTimeZone(), base);
		if (!fields) return;
		const done = { onSuccess: () => setEditor(null), onError };
		if (editor.mode === "create") {
			create.mutate(
				{ botId: bot.id, request: createEventRequest(fields) },
				done,
			);
		} else {
			update.mutate(
				{
					botId: bot.id,
					taskId: editor.task.id,
					request: updateEventRequest(fields, editor.task),
				},
				done,
			);
		}
	}, [editor, bot.id, create, update, onError]);

	const deleteEvent = useCallback(
		(entry: ICalendarEntry) => {
			if (!entry.task_id) return;
			remove.mutate(
				{ botId: bot.id, taskId: entry.task_id },
				{ onSuccess: () => setSelectedId(null), onError },
			);
		},
		[bot.id, remove, onError],
	);

	const changeWake = useCallback(
		(entry: ICalendarEntry, choice: WakeChoice, offsets: number[]) => {
			const wake = eventWakeOf(choice, offsets);
			const task = taskOf(entry.task_id);
			if (task?.event) {
				update.mutate(
					{
						botId: bot.id,
						taskId: task.id,
						request: { event: eventWithWake(task.event, wake) },
					},
					{ onError },
				);
				return;
			}
			const request = externalWakeRequest(entry, wake, browserTimeZone());
			if (request) create.mutate({ botId: bot.id, request }, { onError });
		},
		[bot.id, create, update, taskOf, onError],
	);

	const onChange = useCallback(
		(change: PlanningCalendarChange) => {
			const subject =
				change.type === "create" && change.sourceId
					? byId.get(change.sourceId)?.subject
					: byId.get(change.event.id)?.subject;
			if (subject?.layer !== "events") return;
			const task = taskOf(subject.entry.task_id);
			if (!task?.event) return;
			if (change.type === "delete") deleteEvent(subject.entry);
			if (change.type === "move" || change.type === "resize") {
				update.mutate(
					{
						botId: bot.id,
						taskId: task.id,
						request: {
							event: movedEvent(
								task.event,
								change.event,
								change.type === "resize",
							),
						},
					},
					{ onError },
				);
			}
			if (change.type === "create") {
				create.mutate(
					{
						botId: bot.id,
						request: {
							kind: "event",
							title: task.title,
							instruction: task.instruction,
							tz: task.tz ?? browserTimeZone(),
							event: eventWithWake(task.event, task.event.wake ?? undefined),
						},
					},
					{ onError },
				);
			}
		},
		[bot.id, byId, taskOf, create, update, deleteEvent, onError],
	);

	const onOpenEvent = useCallback(
		(event: CalendarEvent) => setSelectedId(event.id),
		[],
	);
	const onCreateEvent = useCallback(
		(start: Date, end: Date, allDay: boolean) =>
			setEditor({ mode: "create", draft: newEventDraft(start, end, allDay) }),
		[],
	);
	const onEditEvent = useCallback(
		(event: CalendarEvent) => {
			const subject = byId.get(event.id)?.subject;
			const task =
				subject?.layer === "events" ? taskOf(subject.entry.task_id) : undefined;
			if (task) openEditor(task);
		},
		[byId, taskOf, openEditor],
	);

	const editorReady = editor
		? eventFields(editor.draft, browserTimeZone()) !== null
		: false;
	const loading = calendar.isFetching || tasks.isFetching || facts.isFetching;

	return (
		<div
			className="flex min-h-0 min-w-0 flex-1 flex-col gap-2 overflow-y-auto px-3 pb-3"
			data-slot="bot-calendar"
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<ViewSwitch view={view} onChange={setView} />
				<CalendarFeedButton bot={bot} name={name} />
				{reach.events && (
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="h-7 gap-1.5 px-2 text-xs"
						onClick={() => setSettingsOpen(true)}
					>
						<AlarmClockIcon className="size-3.5" aria-hidden />
						{t("calendar.wakeButton", "Wake-ups")}
						{wake.data?.enabled === false && (
							<span className="text-muted-foreground">
								· {t("calendar.wakeOff", "off")}
							</span>
						)}
						<Settings2Icon
							className="size-3.5 text-muted-foreground"
							aria-hidden
						/>
					</Button>
				)}
			</div>
			{available.length > 1 && (
				<LayerToggles layers={available} shown={shown} onToggle={toggleLayer} />
			)}
			{calendar.isError && (
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-destructive">
					<CircleAlertIcon className="size-3.5 shrink-0" aria-hidden />
					{t(
						"calendar.loadFailed",
						"Couldn't load {{name}}'s calendar: {{error}}",
						{
							name,
							error: getErrorMessage(calendar.error),
						},
					)}
					<Button
						type="button"
						variant="link"
						size="sm"
						className="h-auto p-0 text-xs"
						onClick={() => void calendar.refetch()}
					>
						{t("calendar.retry", "Try again")}
					</Button>
				</p>
			)}
			{shown.has("external") && calendar.data && (
				<SourceNotices
					sources={calendar.data.sources}
					name={name}
					onSettings={onEdit ? () => onEdit(bot) : undefined}
				/>
			)}
			<div className="min-h-88 flex-1">
				<PlanningCalendar
					events={events}
					view={view}
					date={today}
					density="compact"
					editable={editable}
					locale={locale}
					height="100%"
					responsive={false}
					showViewSwitcher={false}
					busy={loading}
					className="shadow-none"
					onChange={onChange}
					onOpenEvent={onOpenEvent}
					onCreateEvent={onCreateEvent}
					onEditEvent={onEditEvent}
					onRangeChange={setRange}
				/>
			</div>
			<ItemDetails
				item={selected}
				name={name}
				locale={locale}
				now={now}
				busy={busy}
				editable={editable}
				taskOf={taskOf}
				layerLabels={layerLabels}
				onClose={() => setSelectedId(null)}
				onEdit={openEditor}
				onDelete={deleteEvent}
				onWake={changeWake}
				onForget={(fact) =>
					forget.mutate(fact, {
						onSuccess: () => setSelectedId(null),
						onError,
					})
				}
				onTab={onTab}
				botId={bot.id}
			/>
			{editor && (
				<EventEditorDialog
					open
					mode={editor.mode}
					draft={editor.draft}
					ready={editorReady}
					busy={create.isPending || update.isPending}
					name={name}
					onDraftChange={(draft) => setEditor({ ...editor, draft })}
					onSubmit={saveEditor}
					onOpenChange={(open) => !open && setEditor(null)}
				/>
			)}
			{reach.events && (
				<WakeSettingsDialog
					open={settingsOpen}
					wake={wake.data ?? null}
					busy={setWake.isPending}
					name={name}
					onSave={(next) =>
						setWake.mutate(
							{ botId: bot.id, update: next },
							{ onSuccess: () => setSettingsOpen(false), onError },
						)
					}
					onOpenChange={setSettingsOpen}
				/>
			)}
		</div>
	);
}

/** The details of the chip that was clicked, with what its layer can do. */
function ItemDetails({
	item,
	name,
	locale,
	now,
	busy,
	editable,
	botId,
	layerLabels,
	taskOf,
	onClose,
	onEdit,
	onDelete,
	onWake,
	onForget,
	onTab,
}: Readonly<{
	item: CalendarItem | null;
	name: string;
	locale?: string;
	now: number;
	busy: boolean;
	editable: boolean;
	botId: string;
	layerLabels: Readonly<Record<CalendarLayer, string>>;
	taskOf: (taskId?: string | null) => IBotTask | undefined;
	onClose: () => void;
	onEdit: (task: IBotTask) => void;
	onDelete: (entry: ICalendarEntry) => void;
	onWake: (
		entry: ICalendarEntry,
		choice: WakeChoice,
		offsets: number[],
	) => void;
	onForget: (fact: BotFact) => void;
	onTab?: (tab: BotPanelTab) => void;
}>) {
	const { t } = useTranslation("bots");
	const subject = item?.subject;
	const event = item?.event ?? null;
	const open = item !== null;
	const close = (next: boolean) => !next && onClose();
	if (!subject || subject.layer === "events" || subject.layer === "external") {
		const entry = subject?.entry;
		const task = taskOf(entry?.task_id);
		const own: IEventWake | null | undefined = task?.event?.wake;
		const isOwn = subject?.layer === "events";
		return (
			<CalendarItemDialog
				open={open}
				event={event}
				layer={subject?.layer ?? "events"}
				source={
					isOwn
						? t("calendar.details.sourceBot", "{{name}}'s calendar", { name })
						: (entry?.calendar ?? layerLabels.external)
				}
				recurring={entry?.recurring}
				link={entry?.link}
				onOpenChange={close}
				actions={
					isOwn && editable && task && entry ? (
						<OwnEventActions
							busy={busy}
							onEdit={() => onEdit(task)}
							onDelete={() => onDelete(entry)}
						/>
					) : undefined
				}
			>
				{entry && editable && (isOwn ? task : true) && (
					<WakeSection
						choice={wakeChoiceOf(own)}
						offsets={own?.offsets_minutes ?? []}
						wake={entry.wake}
						name={name}
						busy={busy}
						onChange={(choice, offsets) => onWake(entry, choice, offsets)}
					/>
				)}
			</CalendarItemDialog>
		);
	}
	if (subject.layer === "tasks") {
		return (
			<CalendarItemDialog
				open={open}
				event={event}
				layer="tasks"
				source={describeSchedule(subject.task, locale, t)}
				onOpenChange={close}
				actions={
					onTab ? (
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => onTab("details")}
						>
							{t("calendar.details.showTasks", "Show tasks")}
						</Button>
					) : undefined
				}
			>
				<p className="whitespace-pre-wrap text-sm text-muted-foreground">
					{subject.task.instruction}
				</p>
			</CalendarItemDialog>
		);
	}
	return (
		<CalendarItemDialog
			open={open}
			event={
				event
					? {
							...event,
							title: t("calendar.details.memoryTitle", "{{name}} remembers", {
								name,
							}),
						}
					: null
			}
			layer="memories"
			onOpenChange={close}
			actions={
				onTab ? (
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() => onTab("memory")}
					>
						{t("calendar.details.showMemory", "Show in Memory")}
					</Button>
				) : undefined
			}
		>
			<ul className="flex flex-col">
				<BotMemoryCard
					fact={subject.fact}
					now={now}
					format={memoryFormat(locale ?? "en")}
					busy={busy}
					disabled={busy}
					onForget={() => onForget(subject.fact)}
					onJump={(seq) => {
						if (botHasInboxHistory(botId)) void jumpBotChat(botId, seq);
					}}
				/>
			</ul>
		</CalendarItemDialog>
	);
}
