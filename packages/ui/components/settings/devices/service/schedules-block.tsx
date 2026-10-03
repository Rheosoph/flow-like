"use client";

import { useTranslation } from "@flow-like/locales";
import { Clock, MapPin } from "lucide-react";
import { useState } from "react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import {
	scheduleRun,
	serviceScheduleIds,
	whereOf,
} from "../../../../lib/device-management/model/schedule-where";
import type {
	DeviceViewModel,
	ServiceSchedule,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { eligibilityCopy } from "../copy/eligibility-copy";
import {
	type OnceText,
	type ScheduleText,
	scheduleRunLines,
	scheduleRunNames,
	scheduleSkip,
	scheduleWords,
	scheduleZone,
} from "../copy/schedule-copy";
import { useDeployRole } from "../deploy/use-deploy-reads";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { InlineConfirm } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { stampOf } from "../shell/attention-popover";
import { type ScheduleMoves, useScheduleMoves } from "../workspace";
import { type AppEventRow, appEventRows } from "./service-events";
import type { ServiceApp } from "./service-header";

/* Service › Status: the schedules of one service, what each does now and why one does not run here. */

export interface ScheduleRow {
	eventId: string;
	/** The event's name; undefined when the app no longer has it, or its names can't be read. */
	name?: string;
	/** A repeating schedule as the device runs it when it says so, else as the app's event has it now. */
	text: ScheduleText | null;
	/** A one-time schedule: its local date and time, the device's instant when it reports one. */
	once?: OnceText;
}

const localFormats = new Map<string, Intl.DateTimeFormat>();

/** "2026-09-24" and "09:00" of an instant in a zone; null for a zone this browser doesn't know. */
function localDateTime(
	atS: number,
	zone: string,
): { date: string; time: string } | null {
	try {
		let format = localFormats.get(zone);
		if (!format) {
			format = new Intl.DateTimeFormat("en-CA", {
				timeZone: zone,
				year: "numeric",
				month: "2-digit",
				day: "2-digit",
				hour: "2-digit",
				minute: "2-digit",
				hourCycle: "h23",
			});
			localFormats.set(zone, format);
		}
		const parts = format.formatToParts(atS * 1_000);
		const part = (type: Intl.DateTimeFormatPartTypes) =>
			parts.find((entry) => entry.type === type)?.value ?? "";
		return {
			date: `${part("year")}-${part("month")}-${part("day")}`,
			time: `${part("hour")}:${part("minute")}`,
		};
	} catch {
		return null;
	}
}

/** The device's one-time instant in its zone; the event's own words when they name the same instant. */
function reportedOnce(
	atS: number,
	zone: string,
	saved: (OnceText & { at: number }) | undefined,
): OnceText | undefined {
	if (saved?.at === atS && saved.timezone === zone) return saved;
	const local = localDateTime(atS, zone);
	return local ? { ...local, timezone: zone } : saved;
}

type RowText = Pick<ScheduleRow, "text" | "once">;
type Eligibility = AppEventRow["eligibility"];

/** What the device reports about the schedule's time; null when it reports nothing. */
function reportedText(
	entry: ServiceSchedule,
	saved: Eligibility["once"],
): RowText | null {
	const { once_at: onceAt, expression, timezone } = entry;
	if (onceAt !== undefined) {
		const once = reportedOnce(onceAt, timezone, saved);
		return once ? { text: null, once } : { text: null };
	}
	return expression === undefined ? null : { text: { expression, timezone } };
}

/** What the app's event says about the schedule's time. */
const savedText = (eligibility: Eligibility | undefined): RowText =>
	eligibility?.once
		? { text: null, once: eligibility.once }
		: { text: eligibility?.schedule ?? null };

const rowText = (
	entry: ServiceSchedule | undefined,
	eligibility: Eligibility | undefined,
) =>
	(entry && reportedText(entry, eligibility?.once)) ?? savedText(eligibility);

/**
 * The schedules of a service: what its process reported, and the events it
 * serves that the app lists as a schedule. A service that is stopped, or whose
 * agent can't say, still lists the ones its events name.
 */
export function scheduleRows(
	service: Pick<ServiceView, "schedules" | "events">,
	app: Pick<ServiceApp, "view">,
): ScheduleRow[] {
	const known = appEventRows(app);
	const reported: readonly ServiceSchedule[] = Array.isArray(service.schedules)
		? service.schedules
		: [];
	const ids = serviceScheduleIds(
		service,
		(id) => known.get(id)?.eligibility.kind === "scheduled",
	);
	return ids.map((eventId) => {
		const row = known.get(eventId);
		const entry = reported.find((value) => value.event_id === eventId);
		const name = row?.name;
		return {
			eventId,
			...(name ? { name } : {}),
			...rowText(entry, row?.eligibility),
		};
	});
}

type ReleaseResult = Awaited<ReturnType<ScheduleMoves["release"]>>;

/** Why the hub did not move the schedule or bot to this service. */
function releaseRefusal(
	t: DevicesT,
	result: ReleaseResult,
	bot: boolean,
): string {
	if (result.kind === "schedule_role")
		return t(
			"devices:events.pop.runOnHub.needsRole",
			"Only someone who can edit this app's events can move it.",
		);
	if (result.kind === "schedule_elsewhere")
		return bot
			? t(
					"devices:serviceStatus.bots.runHereElsewhere",
					"Another service runs it. A bot runs in one place: take it back in Events first.",
				)
			: t(
					"devices:serviceStatus.schedules.runHereElsewhere",
					"Another service runs it. A schedule runs in one place: run it on the hub again in Events first.",
				);
	if (result.kind === "schedule_returning")
		return t(
			"devices:serviceStatus.schedules.runHereReturning",
			"It is still returning to the hub from where it ran. Try again in a few minutes.",
		);
	return bot
		? eligibilityCopy(t, { code: "hub_type", eventType: "telegram" }).long
		: eligibilityCopy(t, { code: "hub_schedules", eventType: "cron" }).long;
}

export interface RunHereTarget {
	appId: string;
	eventId: string;
	event: string;
	deviceId: string;
	serviceId: string;
	/** A bot: it connects from this service instead of the hub running it. */
	bot?: boolean;
}

/** What moving a schedule or a bot here does, in the confirm's rows. */
function runHereRows(t: DevicesT, target: RunHereTarget) {
	const values = { event: target.event, service: target.serviceId };
	const when = t(
		"devices:serviceStatus.schedules.runHereWhen",
		"At the service's next check, within 5 minutes.",
	);
	if (target.bot)
		return {
			what: t(
				"devices:serviceStatus.bots.runHereConseq",
				"{{service}} connects {{event}} at its next check, within 5 minutes.",
				values,
			),
			who: t(
				"devices:serviceStatus.bots.runHereWho",
				"A bot runs in one place: no other device answers its messages afterwards.",
			),
			when,
			undo: {
				reversible: true,
				text: t(
					"devices:serviceStatus.bots.runHereUndo",
					"Choose Take it back in Events.",
				),
			},
		};
	return {
		what: t(
			"devices:serviceStatus.schedules.runHereConseq",
			"{{service}} takes {{event}} over at its next check, within 5 minutes. The hub stops running it then.",
			values,
		),
		who: t(
			"devices:serviceStatus.schedules.runHereWho",
			"Until then the hub keeps running it. No run happens twice.",
		),
		when,
		undo: {
			reversible: true,
			text: t(
				"devices:serviceStatus.schedules.runHereUndo",
				"Choose Run it on the hub again in Events.",
			),
		},
	};
}

/**
 * A schedule or bot nobody moved to this service: the hub runs the schedule,
 * nothing on a device answers the bot. Someone who may edit the app's events
 * can move it here; anyone else sees who can.
 */
export function RunItHere({ target }: Readonly<{ target: RunHereTarget }>) {
	const { t } = useTranslation("devices");
	const { release } = useScheduleMoves(target.appId);
	const { canEditEvents } = useDeployRole(target.appId);
	const [asking, setAsking] = useState(false);
	const [moved, setMoved] = useState(false);
	const label = t("serviceStatus.schedules.runHere", "Run it here");
	const values = { event: target.event, service: target.serviceId };
	if (moved)
		return (
			<InlineResult tone="good" onDismiss={() => setMoved(false)}>
				{target.bot
					? t(
							"serviceStatus.bots.runHereDone",
							"{{service}} connects {{event}} at its next check, within 5 minutes.",
							values,
						)
					: t(
							"serviceStatus.schedules.runHereDone",
							"{{service}} takes {{event}} over at its next check, within 5 minutes.",
							values,
						)}
			</InlineResult>
		);
	if (canEditEvents === false)
		return (
			<span className="flex flex-wrap items-center gap-2">
				<DvButton size="xs" icon={MapPin} aria-disabled data-gated="">
					{label}
				</DvButton>
				<GateInline kind="role">
					{t(
						"events.pop.runOnHub.needsRole",
						"Only someone who can edit this app's events can move it.",
					)}
				</GateInline>
			</span>
		);
	if (!asking)
		return (
			<DvButton
				size="xs"
				icon={MapPin}
				className="w-fit"
				onClick={() => setAsking(true)}
			>
				{label}
			</DvButton>
		);
	return (
		<InlineConfirm
			label={label}
			title={label}
			confirmLabel={label}
			tone="default"
			rows={runHereRows(t, target)}
			onConfirm={async () => {
				const result = await release(
					target.eventId,
					target.deviceId,
					target.serviceId,
				);
				if (result.kind !== "ok")
					throw new Error(releaseRefusal(t, result, target.bot === true));
				setAsking(false);
				setMoved(true);
			}}
			onCancel={() => setAsking(false)}
		/>
	);
}

/** "At 02:00 every day · Europe/Berlin" or "Once on 2026-10-15 at 09:00 · Europe/Berlin". */
function ScheduleHead({ row }: Readonly<{ row: ScheduleRow }>) {
	const { t } = useTranslation("devices");
	const words = row.once
		? t("eligibility.runs.onceAt", "Once on {{date}} at {{time}} · {{zone}}", {
				date: row.once.date,
				time: row.once.time,
				zone: scheduleZone(t, row.once),
			})
		: row.text
			? t("eligibility.runs.schedule", "{{words}} · {{zone}}", {
					words: scheduleWords(t, row.text.expression),
					zone: scheduleZone(t, row.text),
				})
			: null;
	return words ? <span className="text-muted-foreground">{words}</span> : null;
}

function ScheduleItem({
	row,
	device,
	service,
	app,
}: Readonly<{
	row: ScheduleRow;
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const locked = service.freshness.age === "locked";
	const run = scheduleRun(service, row.eventId, time.nowS);
	const name = row.name ?? t("service.serves.removed", "A removed event");
	const names = new Map(
		(app.view?.groups ?? []).map((group) => [group.deviceId, group.name]),
	);
	const lines = locked
		? [t("view.matrix.locked", "Unknown until unlocked")]
		: scheduleRunLines(
				t,
				run,
				scheduleRunNames(t, {
					deviceId: service.deviceId,
					serviceId: service.serviceId,
					device: deviceName(device.row),
					eventId: row.eventId,
					...(app.view
						? { where: whereOf(app.view.schedules, row.eventId) }
						: {}),
					deviceName: (id) => names.get(id) ?? id,
					...(Array.isArray(device.services)
						? { siblings: device.services }
						: {}),
				}),
				time,
				{ noNext: true },
			);
	// Counters belong to the process that was read: a status too old to say when it runs next doesn't count for it.
	const entry = run.state === "armed" && !run.stale ? run.entry : null;
	const skipped = entry?.skipped ?? 0;
	const onceState =
		!locked && "entry" in run ? run.entry.once_state : undefined;
	return (
		<li
			data-schedule={row.eventId}
			data-schedule-state={locked ? "locked" : run.state}
			data-once={onceState}
			className="flex min-w-0 flex-col gap-1 border-t border-hairline py-2.5 text-ui first:border-t-0 first:pt-0 last:pb-0"
		>
			<p className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
				<b className="font-semibold">{name}</b>
				<ScheduleHead row={row} />
			</p>
			{lines.map((line) => (
				<p key={line} data-schedule-run="" className="text-ink-2">
					{line}
				</p>
			))}
			{skipped > 0 ? (
				<p data-schedule-skipped="" className="text-xs text-muted-foreground">
					{entry?.last_skip
						? t(
								"serviceStatus.schedules.skipped",
								"Skipped {{count, number}} · last: {{reason}}",
								{
									count: skipped,
									reason: scheduleSkip(t, entry.last_skip.reason),
								},
							)
						: t(
								"serviceStatus.schedules.skippedCount",
								"Skipped {{count, number}}",
								{ count: skipped },
							)}
				</p>
			) : null}
			{!locked &&
			run.state === "held" &&
			run.hold === "not_released" &&
			app.appId ? (
				<RunItHere
					target={{
						appId: app.appId,
						eventId: row.eventId,
						event: name,
						deviceId: service.deviceId,
						serviceId: service.serviceId,
					}}
				/>
			) : null}
		</li>
	);
}

/**
 * The schedules of the service with what each does now: its next and last
 * run while it runs, the reason while it is held, and "not running" or
 * "unknown" where nothing can be said. Never "never ran" for a row that is
 * only not readable.
 */
export function SchedulesBlock({
	device,
	service,
	app,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
}>) {
	const { t } = useTranslation("devices");
	const rows = scheduleRows(service, app);
	if (!rows.length) return null;
	const rules = [
		...(rows.some((row) => !row.once)
			? [
					t(
						"serviceStatus.schedules.rule",
						"A run is skipped when the device is off, asleep or restarting at its time, and when the previous run is still going.",
					),
					t(
						"serviceStatus.schedules.clockChange",
						"When the clocks change, a time that comes twice runs once, and a time that doesn't come runs right after the change.",
					),
				]
			: []),
		...(rows.some((row) => row.once)
			? [
					t(
						"serviceStatus.schedules.onceRule",
						"A one-time schedule runs once. If {{device}} isn't running at its time, or within 15 minutes after, it doesn't run at all.",
						{ device: deviceName(device.row) },
					),
				]
			: []),
	];
	return (
		<Block
			id="service-schedules"
			icon={Clock}
			title={t("serviceStatus.schedules.title", "Schedules")}
			count={rows.length}
			stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
			foot={rules.join(" ")}
		>
			<ul className="flex min-w-0 flex-col">
				{rows.map((row) => (
					<ScheduleItem
						key={row.eventId}
						row={row}
						device={device}
						service={service}
						app={app}
					/>
				))}
			</ul>
			{service.schedulesTruncated ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceStatus.schedules.truncated",
						"The device reports 16 schedules at once; this service has more.",
					)}
				</p>
			) : null}
		</Block>
	);
}
