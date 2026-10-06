"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Activity,
	Boxes,
	type LucideIcon,
	Play,
	RotateCw,
	Scaling,
	Square,
	Trash2,
	X,
	Zap,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { create } from "zustand";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeployRoute,
	DeviceRoute,
	DeviceTab,
	DevicesRoute,
	DevicesScope,
	ServiceRoute,
	ServiceTab,
} from "../../../../lib/device-management/model/types";
import type {
	ActivityAction,
	ActivityItem,
	ActivityKind,
	ActivityState,
} from "../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../lib/utils";
import { Sheet, SheetContent, SheetTitle } from "../../../ui/sheet";
import { isModelsWriteKind, trayTitle } from "../models/action-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { cx } from "../primitives/tone";
import { TrayItem } from "../primitives/tray-item";
import {
	useActivity,
	useAttentionState,
	useDeviceWorkspace,
	useOverlayStore,
	useWidthBucket,
} from "../workspace";
import type { ChromeNavigate } from "./rail-row";

export const ACTIVITY_TRAY_ID = "devices-activity-tray";

interface ActivityTrayState {
	open: boolean;
	setOpen: (open: boolean) => void;
	toggle: () => void;
}

const flipOpen = (state: ActivityTrayState) => {
	return { open: !state.open };
};

/** Open state of the one activity tray: the top bar button and screens ("Open activity") share it. */
export const useActivityTray = create<ActivityTrayState>((set) => {
	return {
		open: false,
		setOpen: (open) => {
			set({ open });
		},
		toggle: () => {
			set(flipOpen);
		},
	};
});

export interface ActivityTrayProps {
	scope: DevicesScope;
	onNavigate: ChromeNavigate;
	/** The screen in view: an item that finishes there gets no toast. */
	route?: DevicesRoute;
	/** Phone chrome: a modal sheet instead of the in-flow drawer. Defaults to the area's width bucket. */
	sheet?: boolean;
	className?: string;
}

/* Model (pure). */

const IN_PROGRESS = new Set<ActivityState>(["active", "paused", "waiting"]);
const FINISHED = new Set<ActivityState>(["done", "failed"]);

const inProgress = (item: ActivityItem) => IN_PROGRESS.has(item.state);
const isFinished = (item: ActivityItem) => FINISHED.has(item.state);
const isUnknown = (item: ActivityItem) => item.state === "unknown";
const isActive = (item: ActivityItem) => item.state === "active";
const isWaiting = (item: ActivityItem) => item.state === "waiting";

export interface TraySections {
	progress: ActivityItem[];
	unknown: ActivityItem[];
	finished: ActivityItem[];
}

/** SPEC §3.6: In progress · No reply received · Finished. */
export function traySections(items: readonly ActivityItem[]): TraySections {
	const progress = items.filter(inProgress);
	const unknown = items.filter(isUnknown);
	const finished = items.filter(isFinished);
	return { progress, unknown, finished };
}

/** In-flight count of the Activity button; `active` drives its spinner. */
export function trayCounts(items: readonly ActivityItem[]) {
	const active = items.filter(isActive).length;
	const waiting = items.filter(isWaiting).length;
	return { inFlight: active + waiting, active };
}

const COMMANDS = [
	"start",
	"stop",
	"restart",
	"scale",
	"remove",
	"apply",
] as const;
type TrayCommand = (typeof COMMANDS)[number];

const COMMAND_ICON: Record<TrayCommand, LucideIcon> = {
	start: Play,
	stop: Square,
	restart: RotateCw,
	scale: Scaling,
	remove: Trash2,
	apply: Zap,
};

const isCommand = (value: unknown): value is TrayCommand =>
	(COMMANDS as readonly unknown[]).includes(value);

function commandOf(item: ActivityItem): TrayCommand | undefined {
	const command = item.label.params?.command;
	return isCommand(command) ? command : undefined;
}

/** A `models` command: the tab's own items name it, the live manager's "no reply" item only its handle. */
const isModelsItem = (item: ActivityItem) =>
	item.label.params?.command === "models" ||
	(item.resume?.type === "operation" && item.resume.command === "models");

function modelsTitle(t: DevicesT, item: ActivityItem) {
	const request = item.label.params?.request;
	if (request === "push")
		return t("devices:models.send.tray", "Send a model file");
	return isModelsWriteKind(request)
		? trayTitle(t, request)
		: t("devices:models.actions.tray.any", "Model change");
}

function commandTitle(t: DevicesT, command: TrayCommand | undefined) {
	if (!command) return t("devices:chrome.tray.kind.command", "Command");
	const titles = {
		start: t("devices:chrome.tray.command.start", "Start"),
		stop: t("devices:chrome.tray.command.stop", "Stop"),
		restart: t("devices:chrome.tray.command.restart", "Restart"),
		scale: t("devices:chrome.tray.command.scale", "Change instances"),
		remove: t("devices:chrome.tray.command.remove", "Remove service"),
		apply: t("devices:chrome.tray.command.apply", "Quick update"),
	} satisfies Record<TrayCommand, string>;
	return titles[command];
}

/** The kind as a title ("Safe update"); R3: kind codes never reach the screen. */
export function activityTitle(t: DevicesT, item: ActivityItem): string {
	const titles = {
		safe_update: t("devices:chrome.tray.kind.safeUpdate", "Safe update"),
		upload: t("devices:chrome.tray.kind.upload", "Upload"),
		access_rules: t("devices:chrome.tray.kind.accessRules", "Access rules"),
		account_backup: t(
			"devices:chrome.tray.kind.accountBackup",
			"Account backup",
		),
		agent_update: t("devices:chrome.tray.kind.agentUpdate", "Agent update"),
		reboot: t("devices:chrome.tray.kind.reboot", "Reboot"),
		command: isModelsItem(item)
			? modelsTitle(t, item)
			: commandTitle(t, commandOf(item)),
		secret_write: t("devices:chrome.tray.kind.secretWrite", "Secret"),
		history_readers: t(
			"devices:chrome.tray.kind.historyReaders",
			"History readers",
		),
		metric_readers: t(
			"devices:chrome.tray.kind.metricReaders",
			"Shared live metrics",
		),
		offline_write_retry: t(
			"devices:chrome.tray.kind.offlineWriteRetry",
			"Buffered change retry",
		),
		signing_request: t(
			"devices:chrome.tray.kind.signingRequest",
			"Signing request",
		),
		setup: t("devices:chrome.tray.kind.setup", "Device setup"),
		event_run: t("devices:chrome.tray.kind.eventRun", "Action or form run"),
	} satisfies Record<ActivityKind, string>;
	return titles[item.kind];
}

/** "edge-berlin-01 › invoice-extractor". */
export function activityTarget(
	item: ActivityItem,
	deviceName?: (deviceId: string) => string | undefined,
): string {
	const { target } = item;
	const device =
		target.deviceName ??
		deviceName?.(target.deviceId) ??
		target.deviceId.slice(0, 8);
	return target.serviceId ? `${device} › ${target.serviceId}` : device;
}

const numberParam = (item: ActivityItem, key: string) => {
	const value = item.detail?.params?.[key];
	return typeof value === "number" ? value : undefined;
};

function progressOf(item: ActivityItem) {
	const { progress } = item;
	if (progress && progress !== "indeterminate") return progress;
	const done = numberParam(item, "done");
	const total = numberParam(item, "total");
	return done === undefined || total === undefined
		? undefined
		: { done, total };
}

function progressText(t: DevicesT, item: ActivityItem) {
	const progress = progressOf(item);
	if (!progress) return undefined;
	const { done, total } = progress;
	const texts = {
		files_progress: t(
			"devices:chrome.tray.detail.files",
			"{{done, number}} of {{total, number}} files",
			{ done, total },
		),
		bytes_progress: t(
			"devices:chrome.tray.detail.bytes",
			"{{done}} of {{total}}",
			{
				done: humanFileSize(done),
				total: humanFileSize(total),
			},
		),
		instances_progress: t(
			"devices:chrome.tray.detail.instances",
			"{{done, number}} of {{total, number}} instances ready",
			{ done, total },
		),
	};
	return texts[item.detail?.code as keyof typeof texts];
}

function stateText(t: DevicesT, time: AreaTime, item: ActivityItem) {
	const until = numberParam(item, "until");
	const texts: Partial<Record<string, string>> = {
		resumable_until:
			until === undefined
				? t(
						"devices:chrome.tray.detail.resumable",
						"Paused. You can resume it.",
					)
				: t(
						"devices:chrome.tray.detail.resumableUntil",
						"Paused. You can resume until {{time}}.",
						{ time: time.at(until / 1000) },
					),
		waiting_for_device:
			item.kind === "event_run"
				? t(
						"devices:chrome.tray.detail.waitingForRun",
						"Sent. Waiting for the device to start the run.",
					)
				: t(
						"devices:chrome.tray.detail.waitingForDevice",
						"Saved. The device applies it the next time it checks in.",
					),
		waiting_for_apply: t(
			"devices:chrome.tray.detail.waitingForApply",
			"Waiting for the device to apply it.",
		),
		waiting_for_heartbeat: t(
			"devices:chrome.tray.detail.waitingForHeartbeat",
			"Waiting for the device's first check-in.",
		),
		waiting_for_backup: t(
			"devices:chrome.tray.detail.waitingForBackup",
			"Waiting for the upload to your account.",
		),
		failed: t("devices:chrome.tray.detail.failed", "This didn't finish."),
		rolled_back: t(
			"devices:chrome.tray.detail.rolledBack",
			"The device went back to what it ran before.",
		),
		rejected: t(
			"devices:chrome.tray.detail.rejected",
			"The device refused this.",
		),
		cancelled: t(
			"devices:chrome.tray.detail.cancelled",
			"Cancelled before it finished.",
		),
	};
	return item.detail ? texts[item.detail.code] : undefined;
}

/** The detail line; `undefined` for "no reply" keeps the primitive's "It may have run" sentence. */
export function activityDetail(
	t: DevicesT,
	time: AreaTime,
	item: ActivityItem,
): string | undefined {
	const text = progressText(t, item) ?? stateText(t, time, item);
	if (item.state !== "active" || item.deadlineAt === undefined) return text;
	const limit = t(
		"devices:chrome.tray.detail.timeLimit",
		"time limit {{time}}",
		{ time: time.countdown(item.deadlineAt / 1000) },
	);
	return text ? `${text} · ${limit}` : limit;
}

function progressValue(item: ActivityItem) {
	const progress = progressOf(item);
	if (!progress || progress.total <= 0) return undefined;
	return Math.min(100, Math.round((progress.done / progress.total) * 100));
}

/* View. */

/** Tray buttons; Open and Dismiss render through the primitive's meta line. */
const BUTTON_ACTIONS = [
	"check_again",
	"resume",
	"activate",
	"discard",
	"cancel",
] as const satisfies readonly ActivityAction[];

function actionLabel(t: DevicesT, action: ActivityAction, item: ActivityItem) {
	const labels = {
		open: t("devices:chrome.tray.action.open", "Open"),
		check_again:
			item.state === "unknown"
				? t("devices:chrome.tray.action.checkResult", "Check result")
				: t("devices:chrome.tray.action.checkAgain", "Check again"),
		activate: t("devices:chrome.tray.action.activate", "Activate…"),
		discard: t("devices:chrome.tray.action.discard", "Discard…"),
		resume: t("devices:chrome.tray.action.resume", "Resume"),
		cancel: t("devices:chrome.tray.action.cancel", "Cancel…"),
		dismiss: t("devices:chrome.tray.action.dismiss", "Dismiss"),
	} satisfies Record<ActivityAction, string>;
	return labels[action];
}

export interface TrayHandlers {
	deviceName?: (deviceId: string) => string | undefined;
	onOpen?: (item: ActivityItem) => void;
	onAction?: (item: ActivityItem, action: ActivityAction) => void;
	/** Ids with a result check in flight. */
	busy?: ReadonlySet<string>;
}

interface TrayEntryProps extends TrayHandlers {
	item: ActivityItem;
}

function EntryActions({ item, onAction, busy }: Readonly<TrayEntryProps>) {
	const { t } = useTranslation("devices");
	const shown = BUTTON_ACTIONS.filter((action) =>
		item.actions.includes(action),
	);
	const unfinishedDismiss =
		item.state === "unknown" && item.actions.includes("dismiss");
	if (!shown.length && !unfinishedDismiss) return null;
	return (
		<>
			{shown.map((action) => {
				return (
					<DvButton
						key={action}
						size="sm"
						busy={action === "check_again" && busy?.has(item.id)}
						onClick={() => onAction?.(item, action)}
					>
						{actionLabel(t, action, item)}
					</DvButton>
				);
			})}
			{unfinishedDismiss ? (
				<DvButton
					size="sm"
					variant="ghost"
					onClick={() => onAction?.(item, "dismiss")}
				>
					{actionLabel(t, "dismiss", item)}
				</DvButton>
			) : null}
		</>
	);
}

function TrayEntry(props: Readonly<TrayEntryProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { item, onOpen, onAction } = props;
	const command = item.kind === "command" ? commandOf(item) : undefined;
	const models = item.kind === "command" && isModelsItem(item);
	return (
		<TrayItem
			kind={item.kind}
			icon={command ? COMMAND_ICON[command] : models ? Boxes : undefined}
			state={item.state}
			title={activityTitle(t, item)}
			sub={activityTarget(item, props.deviceName)}
			progress={progressValue(item)}
			detail={activityDetail(t, time, item)}
			startedAt={item.startedAt / 1000}
			finishedAt={
				item.finishedAt === undefined ? undefined : item.finishedAt / 1000
			}
			by={t("chrome.tray.you", "you")}
			onOpen={onOpen ? () => onOpen(item) : undefined}
			actions={<EntryActions {...props} />}
			onDismiss={
				item.actions.includes("dismiss")
					? () => onAction?.(item, "dismiss")
					: undefined
			}
		/>
	);
}

interface TraySectionProps extends TrayHandlers {
	id: keyof TraySections;
	title: string;
	empty: string;
	items: readonly ActivityItem[];
}

function TraySection(props: Readonly<TraySectionProps>) {
	const { id, title, empty, items, ...handlers } = props;
	return (
		<section
			data-tray-section={id}
			aria-label={title}
			className="flex flex-col gap-2"
		>
			<h3 className="mt-2.5 text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
				{title} · <span className="font-mono tabular-nums">{items.length}</span>
			</h3>
			{items.length ? (
				items.map((item) => {
					return <TrayEntry key={item.id} item={item} {...handlers} />;
				})
			) : (
				<p className="text-xs text-muted-foreground">{empty}</p>
			)}
		</section>
	);
}

export interface ActivityTrayViewProps extends TrayHandlers {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Modal right sheet (phone) instead of the in-flow drawer. */
	sheet?: boolean;
	items: readonly ActivityItem[];
	className?: string;
}

interface TrayPanelProps extends TrayHandlers {
	items: readonly ActivityItem[];
	onClose: () => void;
	title: ReactNode;
}

function TrayPanel(props: Readonly<TrayPanelProps>) {
	const { t } = useTranslation("devices");
	const { items, onClose, title, ...handlers } = props;
	const sections = traySections(items);
	const closeRef = useRef<HTMLButtonElement>(null);
	useEffect(() => {
		closeRef.current?.focus();
	}, []);
	return (
		<>
			<div className="flex items-center gap-2 border-b border-hairline px-4 py-3">
				<Activity aria-hidden className="size-4 shrink-0" />
				{title}
				<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
					{t("chrome.tray.keptHere", "Kept on this computer")}
				</span>
				<DvButton
					ref={closeRef}
					variant="ghost"
					size="sm"
					iconOnly
					icon={X}
					aria-label={t("chrome.tray.close", "Close activity")}
					onClick={onClose}
				/>
			</div>
			<div
				data-tray-body=""
				className="flex min-h-0 flex-1 flex-col gap-2 overflow-auto px-4 pt-2 pb-4"
			>
				<TraySection
					id="progress"
					title={t("chrome.tray.inProgress", "In progress")}
					empty={t("chrome.tray.inProgressEmpty", "Nothing is running.")}
					items={sections.progress}
					{...handlers}
				/>
				<TraySection
					id="unknown"
					title={t("chrome.tray.noReply", "No reply received")}
					empty={t(
						"chrome.tray.noReplyEmpty",
						"Every command you sent got a reply.",
					)}
					items={sections.unknown}
					{...handlers}
				/>
				<TraySection
					id="finished"
					title={t("chrome.tray.finished", "Finished")}
					empty={t("chrome.tray.finishedEmpty", "Nothing has finished yet.")}
					items={sections.finished}
					{...handlers}
				/>
			</div>
			<p className="border-t border-hairline bg-surface-sunken px-4 py-2.5 text-xs text-muted-foreground">
				{t(
					"chrome.tray.foot",
					"Results stay until you dismiss them, across reloads and reconnects.",
				)}
			</p>
		</>
	);
}

/** Esc closes the non-modal drawer unless an overlay above it already took the key. */
function useEscape(active: boolean, onEscape: () => void) {
	useEffect(() => {
		if (!active) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key === "Escape" && !event.defaultPrevented) onEscape();
		};
		document.addEventListener("keydown", onKey);
		return () => document.removeEventListener("keydown", onKey);
	}, [active, onEscape]);
}

function focusTrayButton() {
	document
		.querySelector<HTMLElement>(`[aria-controls="${ACTIVITY_TRAY_ID}"]`)
		?.focus();
}

const TRAY_TITLE = "text-[15px]/5 font-semibold";
const TRAY_DRAWER =
	"absolute inset-y-0 right-0 z-20 flex w-[min(400px,100%)] flex-col border-l border-border-strong bg-popover";
const TRAY_SHEET =
	"w-full max-w-full gap-0 border-border-strong bg-popover p-0 shadow-none backdrop-blur-none sm:max-w-full [&>button]:hidden [&>div.pointer-events-none]:hidden [&>div.relative]:gap-0";

/** The tray over plain data: in-flow drawer (non-modal) or right sheet (modal). */
export function ActivityTrayView(props: Readonly<ActivityTrayViewProps>) {
	const { t } = useTranslation("devices");
	const { open, onOpenChange, sheet = false, className, ...panel } = props;
	const label = t("chrome.tray.title", "Activity");
	const dismiss = () => {
		onOpenChange(false);
	};
	const close = () => {
		onOpenChange(false);
		focusTrayButton();
	};
	useEscape(open && !sheet, close);
	if (sheet) {
		const title = <SheetTitle className={TRAY_TITLE}>{label}</SheetTitle>;
		return (
			<Sheet open={open} onOpenChange={onOpenChange}>
				<SheetContent
					side="right"
					id={ACTIVITY_TRAY_ID}
					overlayClassName="bg-scrim backdrop-blur-none"
					aria-describedby={undefined}
					className={cx(TRAY_SHEET, className)}
				>
					<TrayPanel {...panel} onClose={dismiss} title={title} />
				</SheetContent>
			</Sheet>
		);
	}
	if (!open) return null;
	const title = <h2 className={TRAY_TITLE}>{label}</h2>;
	return (
		<aside
			id={ACTIVITY_TRAY_ID}
			aria-label={label}
			className={cx(TRAY_DRAWER, className)}
		>
			<TrayPanel {...panel} onClose={close} title={title} />
		</aside>
	);
}

/* Binding. */

/** Handles the hub answers for; every other handle needs the device's keys. */
const HUB_HANDLES = new Set(["policy", "account_backup", "setup"]);

const needsKeys = (item: ActivityItem) =>
	item.resume !== undefined && !HUB_HANDLES.has(item.resume.type);

const DEVICE_TAB: Partial<Record<ActivityKind, DeviceTab>> = {
	access_rules: "access",
	account_backup: "keys",
	agent_update: "settings",
	reboot: "settings",
	history_readers: "activity",
	metric_readers: "metrics",
	signing_request: "certificates",
};

const SERVICE_TAB: Partial<Record<ActivityKind, ServiceTab>> = {
	offline_write_retry: "offline",
	secret_write: "configuration",
	history_readers: "activity",
	metric_readers: "metrics",
};

function uploadRoute(item: ActivityItem) {
	const { deviceId, projectId } = item.target;
	const route: DeployRoute = {
		screen: "deploy",
		deviceIds: [deviceId],
		mode: "update",
		step: "copy_upload",
	};
	if (projectId) route.appId = projectId;
	return route;
}

function setupRoute(item: ActivityItem) {
	const handle = item.resume;
	if (!handle || handle.type !== "setup") return undefined;
	const route: DevicesRoute = {
		screen: "setup",
		enrollmentId: handle.enrollmentId,
	};
	return route;
}

function serviceRoute(item: ActivityItem, serviceId: string) {
	const tab = SERVICE_TAB[item.kind] ?? "status";
	const { deviceId } = item.target;
	const route: DevicesRoute = { screen: "service", deviceId, serviceId, tab };
	return route;
}

function deviceRoute(item: ActivityItem) {
	const tab: DeviceTab = isModelsItem(item)
		? "models"
		: (DEVICE_TAB[item.kind] ?? "overview");
	const { deviceId } = item.target;
	const route: DevicesRoute = { screen: "device", deviceId, tab };
	return route;
}

function targetRoute(item: ActivityItem) {
	const { serviceId } = item.target;
	return serviceId ? serviceRoute(item, serviceId) : deviceRoute(item);
}

/** Where an item's own controls live: its link, else the page of what it works on. */
export function activityRoute(item: ActivityItem) {
	if (item.href) return item.href;
	if (item.kind === "upload") return uploadRoute(item);
	return setupRoute(item) ?? targetRoute(item);
}

/** The Rollout step of a deploy shows every target of its own run inline. */
function showsRun(route: DeployRoute, item: ActivityItem) {
	const run = item.href;
	if (route.step !== "rollout" || run?.screen !== "deploy") return false;
	return (
		route.appId === run.appId &&
		route.serviceId === run.serviceId &&
		route.deviceIds.includes(item.target.deviceId)
	);
}

/** The device page shows every item of its device, a service page those of its service. */
function showsObject(route: DeviceRoute | ServiceRoute, item: ActivityItem) {
	if (route.deviceId !== item.target.deviceId) return false;
	return route.screen === "device" || route.serviceId === item.target.serviceId;
}

/** The "Run now…" sheet of that event is open: it shows the outcome itself. */
function sheetShows(item: ActivityItem) {
	const overlay = useOverlayStore.getState().overlay;
	const { target } = item;
	return (
		item.kind === "event_run" &&
		overlay.kind === "run_now" &&
		overlay.deviceId === target.deviceId &&
		overlay.serviceId === target.serviceId &&
		overlay.eventId === target.eventId
	);
}

/** The screen already shows this device (or service), or the run the item belongs to: no toast needed there. */
function showsItem(route: DevicesRoute | undefined, item: ActivityItem) {
	if (sheetShows(item)) return true;
	if (!route) return false;
	switch (route.screen) {
		case "deploy":
			return showsRun(route, item);
		case "device":
		case "service":
			return showsObject(route, item);
		default:
			return false;
	}
}

function finishedText(t: DevicesT, item: ActivityItem, target: string) {
	const title = activityTitle(t, item);
	if (item.state === "done")
		return t(
			"devices:chrome.tray.toast.done",
			"{{title}} finished · {{target}}",
			{
				title,
				target,
			},
		);
	if (item.state === "failed")
		return t(
			"devices:chrome.tray.toast.failed",
			"{{title}} failed · {{target}}",
			{
				title,
				target,
			},
		);
	return t(
		"devices:chrome.tray.toast.unknown",
		"{{title}}: no reply received · {{target}}",
		{ title, target },
	);
}

interface TrayBinding {
	route?: DevicesRoute;
	open: boolean;
	deviceName: (deviceId: string) => string | undefined;
	onNavigate: ChromeNavigate;
}

/** SPEC §3.6: an item that finishes while its page isn't showing it gets a toast that links back. */
function useFinishedToast(binding: TrayBinding) {
	const { t } = useTranslation("devices");
	const { activity } = useDeviceWorkspace();
	const latest = useRef(binding);
	latest.current = binding;
	useEffect(
		() =>
			activity.onFinishedElsewhere((item) => {
				const now = latest.current;
				if (now.open || showsItem(now.route, item)) return;
				toast(finishedText(t, item, activityTarget(item, now.deviceName)), {
					action: {
						label: t("chrome.tray.action.open", "Open"),
						onClick: () => now.onNavigate(activityRoute(item)),
					},
				});
			}),
		[activity, t],
	);
}

/** SPEC §3.6: the only drawer. In progress · No reply received · Finished. */
export function ActivityTray({
	onNavigate,
	route,
	sheet,
	className,
}: Readonly<ActivityTrayProps>) {
	const { open, setOpen } = useActivityTray();
	const { items, dismiss, checkAgain } = useActivity();
	const { input, workspace } = useAttentionState();
	const phone = useWidthBucket() === "phone";
	const asSheet = sheet ?? phone;
	const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
	const names = useMemo(
		() => new Map(input.devices.map((row) => [row.device_id, deviceName(row)])),
		[input.devices],
	);
	const nameOf = useCallback(
		(deviceId: string) => names.get(deviceId),
		[names],
	);
	useEffect(() => () => useActivityTray.getState().setOpen(false), []);
	useFinishedToast({ route, open, deviceName: nameOf, onNavigate });

	const openItem = (item: ActivityItem) => {
		if (asSheet) setOpen(false);
		onNavigate(activityRoute(item));
	};
	const check = async (item: ActivityItem) => {
		const { deviceId } = item.target;
		if (
			needsKeys(item) &&
			workspace.keys.snapshot(deviceId).state !== "unlocked"
		) {
			useOverlayStore.getState().openUnlock(deviceId, { connectLive: true });
			return;
		}
		setBusy((current) => new Set(current).add(item.id));
		try {
			await checkAgain(deviceId);
		} finally {
			setBusy((current) => {
				const next = new Set(current);
				next.delete(item.id);
				return next;
			});
		}
	};
	const act = (item: ActivityItem, action: ActivityAction) => {
		if (action === "dismiss") dismiss(item.id);
		else if (action === "check_again") void check(item);
		else openItem(item);
	};
	return (
		<ActivityTrayView
			open={open}
			onOpenChange={setOpen}
			sheet={asSheet}
			items={items}
			deviceName={nameOf}
			onOpen={openItem}
			onAction={act}
			busy={busy}
			className={className}
		/>
	);
}
