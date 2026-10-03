"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleArrowUp,
	CirclePlay,
	CircleSlash,
	LockOpen,
	Rocket,
	Settings,
	Undo2,
} from "lucide-react";
import { Fragment, type ReactNode, useId, useState } from "react";
import type {
	EventEligibility,
	EventIneligibleCode,
} from "../../../../lib/device-management/deployment";
import type { AppView } from "../../../../lib/device-management/model/app-plan";
import {
	type ScheduleWhere,
	botRun,
	holdsSchedule,
	scheduleRun,
} from "../../../../lib/device-management/model/schedule-where";
import {
	type OnceSchedule,
	onceAhead,
} from "../../../../lib/device-management/schedule";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { botLine } from "../app/kind-lines";
import { openRunNow } from "../app/run-now";
import { appCopy } from "../copy/app-copy";
import {
	agentTooOldCopy,
	cantHereCopy,
	eligibilityCopy,
	eligibilityInput,
	eventTypeLabel,
	howItRunsCopy,
} from "../copy/eligibility-copy";
import { gateCopy } from "../copy/gate-copy";
import {
	type WhereKind,
	type WhereNames,
	canRunOnHubAgain,
	scheduleHead,
	scheduleOnceHead,
	scheduleRunLines,
	scheduleRunNames,
	scheduleWhereText,
	whereNames,
} from "../copy/schedule-copy";
import { ModeChip } from "../primitives/app-chips";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { LatestTag } from "../primitives/event-cell";
import {
	FreshnessStamp,
	type FreshnessStampProps,
	MixedSourcesStamp,
	sameSource,
} from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { InlineConfirm } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { MatrixCell } from "../primitives/matrix-cell";
import { PairedPins } from "../primitives/paired-pins";
import type { DesiredRun, ObservedRun } from "../primitives/requested-actual";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { formTakesFile } from "../service/service-events";
import { stampOf } from "../shell/attention-popover";
import { useOverlayStore } from "../workspace";
import {
	SEPARATOR,
	blockGateKind,
	blockShort,
	heldText,
	newerParts,
	servedActual,
	versionLabel,
	versionText,
	whereKind,
} from "./events-copy";
import {
	type EventsDevicesLive,
	type EventsDevicesValue,
	useEventsDevices,
} from "./events-devices";
import {
	type RunsOnElsewhere,
	type RunsOnRow,
	type RunsOnServed,
	type RunsOnUnknown,
	isClaimedRow,
} from "./runs-on-model";

/* APP §4.4: where one event runs on devices, or why it can't. */

const SECTION =
	"flex flex-col gap-1.5 border-t border-hairline px-2 py-1.5 first:border-t-0";
const SUB = "text-xs/4 text-muted-foreground";
const LABEL =
	"text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground";
const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
const ROW =
	"grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 border-t border-hairline py-1.5 first:border-t-0";
const ROW_MAIN = "flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5";
const DEVICE = "font-mono font-semibold";

function Head({
	event,
	stamp,
	sub,
}: Readonly<{ event: IEvent; stamp?: ReactNode; sub: ReactNode }>) {
	const { renderTile } = useEventsDevices();
	return (
		<>
			<h3 className="flex items-center gap-2 text-ui font-semibold">
				{renderTile?.(event.id)}
				<span className="min-w-0 wrap-anywhere">{event.name}</span>
				<span className="flex-1" />
				{stamp}
			</h3>
			<p className={SUB}>{sub}</p>
		</>
	);
}

/** R11: a fleet can have many devices; a group shows the first few and offers the rest. */
const GROUP_CAP = 5;

function Group<T extends { deviceId: string }>({
	label,
	items,
	render,
	empty,
}: Readonly<{
	label: string;
	items: readonly T[];
	render(item: T): ReactNode;
	/** The line shown instead of rows when the group has none. */
	empty?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const [all, setAll] = useState(false);
	const shown = all ? items : items.slice(0, GROUP_CAP);
	const hidden = items.length - shown.length;
	return (
		<div className={SECTION}>
			<p className={LABEL}>
				{t("events.pop.group", "{{label}} · {{count, number}}", {
					label,
					count: items.length,
				})}
			</p>
			<ul className="flex flex-col">
				{shown.map((item) => (
					<Fragment key={item.deviceId}>{render(item)}</Fragment>
				))}
				{items.length ? null : empty}
			</ul>
			{hidden > 0 ? (
				<DvButton
					variant="link"
					size="xs"
					className="w-fit"
					onClick={() => setAll(true)}
				>
					{t("events.pop.more", "Show {{count, number}} more", {
						count: hidden,
					})}
				</DvButton>
			) : null}
		</div>
	);
}

/* Why it can't. */

type FixableCode = Extract<
	EventIneligibleCode,
	"paused" | "canary" | "variants"
>;

function fixSteps(t: DevicesT, code: EventIneligibleCode): string | null {
	const steps = {
		paused: t(
			"devices:events.fix.paused",
			"Activate it with the play button in its row, then run it on a device.",
		),
		canary: t(
			"devices:events.fix.canary",
			"End the canary in the event's Canary section. A device serves one version and can't split traffic.",
		),
		variants: t(
			"devices:events.fix.variants",
			"Remove the traffic variants in the event's Canary section. A device can't split traffic.",
		),
	} satisfies Record<FixableCode, string>;
	return Object.hasOwn(steps, code) ? steps[code as FixableCode] : null;
}

function WhyBody({
	event,
	rule,
	onClose,
}: Readonly<{ event: IEvent; rule: EventEligibility; onClose(): void }>) {
	const { t } = useTranslation("devices");
	const { eligibility, openEvent, status, link, live } = useEventsDevices();
	const code = rule.code ?? "type";
	const reason = eligibilityCopy(
		t,
		eligibilityInput({ ...rule, code }, event.event_type),
	);
	const steps = fixSteps(t, code);
	// The reason itself says what to change; the button opens where to change it.
	const opens =
		openEvent && code !== "paused" && reason.fix === "open_events"
			? openEvent
			: null;
	const still = live?.rows.get(event.id)?.served ?? [];
	const canRun = [...eligibility.values()].filter(
		(other) => other.eligible,
	).length;
	return (
		<>
			<div className={SECTION}>
				<Head
					event={event}
					sub={eventTypeLabel(
						t,
						event.event_type,
						Boolean(event.default_page_id),
					)}
				/>
			</div>
			<div className={SECTION}>
				<p className="flex items-center gap-1.5 text-ui font-semibold">
					<CircleSlash aria-hidden className="size-4 text-muted-foreground" />
					{code === "paused"
						? t("events.pop.paused", "Paused, so it can't run on devices")
						: t("events.cell.cantRun", "Can't run on devices")}
				</p>
				<p className="text-ui">{reason.long}</p>
				{steps || opens ? (
					<div className="flex flex-col gap-1.5 rounded-lg border border-border bg-surface-sunken px-2.5 py-2">
						{steps ? (
							<p className="text-[12.5px]/[17px]">
								<b className="font-semibold">
									{t("events.fix.lead", "To fix it:")}
								</b>{" "}
								{steps}
							</p>
						) : null}
						{opens ? (
							<DvButton
								size="sm"
								icon={Settings}
								className="w-fit"
								onClick={() => {
									onClose();
									opens(event.id);
								}}
							>
								{t("events.fix.open", "Open the event…")}
							</DvButton>
						) : null}
					</div>
				) : null}
				{still.map((served) => (
					<StillServed
						key={served.deviceId}
						served={served}
						paused={code === "paused"}
					/>
				))}
				<p className={SUB}>
					{t("events.pop.others", {
						count: canRun,
						defaultValue_one:
							"Deploy doesn't offer it. The app's other events aren't affected: {{count, number}} event can run on a device.",
						defaultValue_other:
							"Deploy doesn't offer it. The app's other events aren't affected: {{count, number}} events can run on a device.",
					})}
				</p>
			</div>
			{status === "ready" ? (
				<div className={SECTION}>
					<div className="flex flex-wrap items-center gap-2">
						<OpenInDevices eventId={event.id} link={link} />
					</div>
				</div>
			) : null}
		</>
	);
}

/** Pausing or changing an event acts on the hub and this computer at once; a device keeps its deployed copy. */
function StillServed({
	served,
	paused,
}: Readonly<{ served: RunsOnServed; paused: boolean }>) {
	const { t } = useTranslation("devices");
	const { link } = useEventsDevices();
	const [serviceId = ""] = served.cell.serviceIds;
	const values = { device: served.device, service: serviceId };
	return (
		<p
			data-still-served={served.deviceId}
			className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px]/[17px] text-warning"
		>
			<span>
				{paused
					? t(
							"events.pop.schedulePaused",
							"Paused in Events. {{device}} still runs it until you stop or update {{service}}.",
							values,
						)
					: t(
							"events.pop.stillServed",
							"{{device}} still runs it until you stop or update {{service}}.",
							values,
						)}
			</span>
			{serviceId ? (
				<a
					{...link({
						screen: "service",
						deviceId: served.deviceId,
						serviceId,
					})}
					className={LINK}
				>
					{t("events.pop.openService", "Open {{service}}", {
						service: serviceId,
					})}
				</a>
			) : null}
		</p>
	);
}

function OpenInDevices({
	eventId,
	link,
}: Readonly<{ eventId: string; link: EventsDevicesValue["link"] }>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton variant="ghost" size="sm" asChild>
			<a {...link({ screen: "app-devices", by: "event", eventId })}>
				{t("events.openInDevices", "Open in Devices")}
			</a>
		</DvButton>
	);
}

/* Where it runs. */

function headStamp(row: RunsOnRow): FreshnessStampProps | null {
	const stamps = row.served.flatMap((served) =>
		served.service ? [stampOf(served.service.view.freshness)] : [],
	);
	const [first] = stamps;
	if (first)
		return !row.unknown.length &&
			stamps.length === row.served.length &&
			stamps.every((stamp) => sameSource(stamp, first))
			? first
			: null;
	return row.unknown.length && row.lockedOnly
		? { source: "snap", age: "locked" }
		: null;
}

function Drift({
	served,
	row,
}: Readonly<{ served: RunsOnServed; row: RunsOnRow }>) {
	const { t } = useTranslation("devices");
	const { cell } = served;
	if (cell.state === "staged")
		return (
			<span className="text-info">
				{t("events.pop.staged", "an update is staged")}
			</span>
		);
	// A served event that follows Latest is never "newest" while its flow has edits or its state is unknown.
	const note = appCopy(t).servedNote(cell.drift);
	if (note)
		return note.tone === "info" ? (
			<span className="inline-flex items-center gap-1 text-info">
				<CircleArrowUp aria-hidden className="size-3" />
				{note.text}
			</span>
		) : (
			<span className="text-muted-foreground">{note.text}</span>
		);
	const parts = newerParts(served, row);
	if (!parts.length)
		return (
			<span className="text-good">{t("events.pop.newest", "newest")}</span>
		);
	const newer = parts
		.map((part) => versionLabel(t, part.kind, part.to))
		.join(SEPARATOR(t));
	return (
		<span
			className="inline-flex items-center gap-1 text-info"
			title={t(
				"events.pop.behindTitle",
				"Runs an older version. The newest version has {{newer}}.",
				{ newer },
			)}
		>
			<CircleArrowUp aria-hidden className="size-3" />
			{t("events.pop.behind", "newest has {{newer}}", { newer })}
		</span>
	);
}

/** What the serving service does with the event's kind: when its schedule runs (either kind), or its bot's state. */
function servedLines(
	t: DevicesT,
	served: RunsOnServed,
	row: RunsOnRow,
	live: EventsDevicesLive | null,
	time: AreaTime,
): string[] {
	const view = served.service?.view;
	if (!view) return [];
	if (row.kind === "bot") return [botLine(t, botRun(view, row.eventId))];
	if (!row.schedule && !row.once) return [];
	return scheduleRunLines(
		t,
		scheduleRun(view, row.eventId, time.nowS),
		scheduleRunNames(t, {
			deviceId: served.deviceId,
			serviceId: view.serviceId,
			device: served.device,
			eventId: row.eventId,
			...(row.where ? { where: row.where } : {}),
			deviceName: (id) => live?.names.get(id) ?? id,
			siblings: live?.view.groups
				.find((group) => group.deviceId === served.deviceId)
				?.services.map((entry) => entry.view),
		}),
		time,
	);
}

/**
 * The device runs the schedule of the event version it was deployed with,
 * not the one in Events: a one-time schedule whose time changed since, or any
 * schedule whose event version did. Null when nothing waits for an update.
 */
function changedText(
	t: DevicesT,
	served: RunsOnServed,
	row: RunsOnRow,
): string | null {
	const { cell } = served;
	const [serviceId = ""] = cell.serviceIds;
	const schedules = served.service?.view.schedules;
	const deployed = Array.isArray(schedules)
		? schedules.find((entry) => entry.event_id === row.eventId)?.once_at
		: undefined;
	if (row.once && deployed !== undefined)
		return deployed === row.once.at
			? null
			: t(
					"devices:events.pop.onceChanged",
					"A new time was set in Events. Update {{service}} to apply it; until then nothing runs it.",
					{ service: serviceId },
				);
	if (!(row.schedule || row.once) || !cell.pin || !row.pin) return null;
	if (versionText(cell.pin.eventVersion) === versionText(row.pin.eventVersion))
		return null;
	return t(
		"devices:events.pop.scheduleChanged",
		"{{device}} runs the schedule of event {{version}}. Update {{service}} to apply the change.",
		{
			device: served.device,
			version: versionText(cell.pin.eventVersion),
			service: serviceId,
		},
	);
}

/** Run now… for a person-started event that this service runs: the reason it is off, from the device's live facts. */
function useServedRunNow(served: RunsOnServed, row: RunsOnRow) {
	const { t } = useTranslation("devices");
	const { live } = useEventsDevices();
	const time = useAreaTime();
	const view = served.service?.view;
	if (row.kind !== "on_demand" || !view || !live) return null;
	const result = live.runGate(served.deviceId, view);
	if (!result.ok && result.hide) return null;
	const file = formTakesFile(view, row.eventId);
	return {
		gate: file
			? {
					kind: "unsupported" as const,
					reason: t(
						"serviceStatus.actions.fileOnly",
						"This form takes a file. Open it on the service page.",
					),
				}
			: result.ok
				? null
				: { kind: result.kind, reason: gateCopy(t, result, time).inline },
		open: () =>
			openRunNow({
				deviceId: served.deviceId,
				serviceId: view.serviceId,
				eventId: row.eventId,
			}),
	};
}

function ServedRow({
	served,
	row,
	stamped,
}: Readonly<{ served: RunsOnServed; row: RunsOnRow; stamped: boolean }>) {
	const { t } = useTranslation("devices");
	const { link, live } = useEventsDevices();
	const time = useAreaTime();
	const reasonId = useId();
	const { cell, service } = served;
	const [serviceId] = cell.serviceIds;
	const href = serviceId
		? link({ screen: "service", deviceId: served.deviceId, serviceId })
		: null;
	const view = service?.view;
	const runs = servedLines(t, served, row, live, time);
	const changed = changedText(t, served, row);
	const runNow = useServedRunNow(served, row);
	return (
		<li data-on-device={served.deviceId} className={ROW}>
			<div className={ROW_MAIN}>
				<span className={DEVICE}>{served.device}</span>
				{href ? (
					<a {...href} className={cx("font-mono", LINK)}>
						{serviceId}
					</a>
				) : null}
				{view ? (
					<span className="inline-flex items-center gap-1 text-ink-2">
						<PairedPins
							desired={view.desired as DesiredRun}
							observed={view.observed as ObservedRun}
							conv={view.conv}
						/>
						{view.conv === "stopped_by_user"
							? t("events.pop.stoppedAsAsked", "Stopped, as you asked")
							: servedActual(t, served)}
					</span>
				) : null}
			</div>
			{href || runNow ? (
				<div className="row-span-2 flex flex-wrap items-start justify-end gap-1.5">
					{runNow ? (
						<DvButton
							size="xs"
							icon={CirclePlay}
							data-run-now=""
							{...(runNow.gate
								? {
										"aria-disabled": true,
										"aria-describedby": reasonId,
										"data-gated": runNow.gate.kind,
									}
								: { onClick: runNow.open })}
						>
							{t("events.pop.runNow", "Run now…")}
						</DvButton>
					) : null}
					{href ? (
						<DvButton size="xs" asChild>
							<a {...href}>{t("events.pop.open", "Open")}</a>
						</DvButton>
					) : null}
				</div>
			) : null}
			<p className={cx(SUB, "col-start-1 flex flex-wrap items-center gap-1.5")}>
				{cell.pin ? (
					<span className="font-mono text-[11.5px]">
						{t("events.pop.pin", "event {{event}} · flow {{flow}}", {
							event: versionText(cell.pin.eventVersion),
							flow: versionText(cell.pin.boardVersion),
						})}
					</span>
				) : null}
				<Drift served={served} row={row} />
				{row.followsLatest ? <LatestTag /> : null}
				{stamped || !view ? null : (
					<FreshnessStamp {...stampOf(view.freshness)} compact />
				)}
			</p>
			{runs.map((line) => (
				<p key={line} data-schedule-run="" className={cx(SUB, "col-start-1")}>
					{line}
				</p>
			))}
			{changed ? (
				<p
					data-schedule-changed=""
					className={cx(SUB, "col-start-1 text-warning")}
				>
					{changed}
				</p>
			) : null}
			{runNow?.gate ? (
				<div className="col-start-1">
					<GateInline
						kind={runNow.gate.kind}
						id={reasonId}
						className="max-w-none"
					>
						{runNow.gate.reason}
					</GateInline>
				</div>
			) : null}
		</li>
	);
}

function UnknownRow({ entry }: Readonly<{ entry: RunsOnUnknown }>) {
	const { t } = useTranslation("devices");
	const { link } = useEventsDevices();
	const { unknown, deviceId, serviceId } = entry;
	const why =
		unknown.kind === "never" || unknown.kind === "noaccess"
			? "notloaded"
			: unknown.kind;
	return (
		<li data-on-device={deviceId} data-why={unknown.kind} className={ROW}>
			<div className={ROW_MAIN}>
				<span className={DEVICE}>{entry.device}</span>
				<MatrixCell
					state="unknown"
					why={why}
					{...("since" in unknown && unknown.since !== undefined
						? { since: unknown.since }
						: {})}
				/>
			</div>
			{unknown.kind === "locked" ? (
				<DvButton
					size="xs"
					icon={LockOpen}
					className="row-span-2"
					onClick={() => useOverlayStore.getState().openUnlock(deviceId)}
				>
					{t("events.pop.unlock", "Unlock…")}
				</DvButton>
			) : unknown.kind === "nokeys" ? (
				<DvButton size="xs" asChild className="row-span-2">
					<a
						{...link(
							{ screen: "keys", focusDeviceId: deviceId },
							ACCOUNT_SCOPE,
						)}
					>
						{t("events.pop.restoreKeys", "Restore keys…")}
					</a>
				</DvButton>
			) : null}
			{unknown.kind === "locked" ? (
				<p className={cx(SUB, "col-start-1")}>
					{t(
						"events.pop.lockedWhy",
						"Only keys on this computer can read which services it runs.",
					)}
				</p>
			) : unknown.kind === "snapshot" ? (
				<p className={cx(SUB, "col-start-1")}>
					{serviceId
						? t(
								"events.pop.snapshotWhy",
								"It runs {{service}}, but its status doesn't list the events it serves. An agent update on the device adds them.",
								{ service: serviceId },
							)
						: t(
								"events.pop.snapshotWhyPlain",
								"Its status doesn't list the events it serves. An agent update on the device adds them.",
							)}
				</p>
			) : null}
		</li>
	);
}

function ElsewhereRow({
	entry,
	eventId,
	live,
}: Readonly<{
	entry: RunsOnElsewhere;
	eventId: string;
	live: EventsDevicesLive;
}>) {
	const { t } = useTranslation("devices");
	const { link, go, block } = useEventsDevices();
	const { gate, deviceId } = entry;
	const copy = gate ? gateCopy(t, gate) : null;
	const fix = gate?.fix;
	const runFix = () => {
		if (!fix) return;
		const outcome = live.runFix(fix);
		if (outcome.kind === "navigate") go(outcome.route, ACCOUNT_SCOPE);
	};
	return (
		<li data-on-device={deviceId} data-state={entry.state} className={ROW}>
			<div className={ROW_MAIN}>
				<span className={DEVICE}>{entry.device}</span>
				{entry.state === "not_served" ? (
					<span className="text-muted-foreground">
						{t("events.pop.notOnIt", "Not on it")}
					</span>
				) : entry.state === "cant_here" ? (
					<MatrixCell
						state="cant_here"
						reason={cantHereCopy(t, entry, entry.device)}
					/>
				) : (
					<MatrixCell state="no_access" />
				)}
			</div>
			{entry.why === "agent" ? (
				<DvButton size="xs" asChild className="row-span-2">
					<a {...link({ screen: "device", deviceId, tab: "settings" })}>
						{agentTooOldCopy(t, entry.device, entry.feature).fix}
					</a>
				</DvButton>
			) : null}
			{entry.never ? (
				<>
					<DvButton size="xs" asChild className="row-span-2">
						<a
							{...link({
								screen: "device",
								deviceId,
								tab: "overview",
							})}
						>
							{t("events.pop.startInstructions", "Start instructions")}
						</a>
					</DvButton>
					<div className="col-start-1">
						<GateInline kind="live" className="max-w-none">
							{t(
								"events.pop.never",
								"{{device}} hasn't checked in yet. Deploying needs it online first.",
								{ device: entry.device },
							)}
						</GateInline>
					</div>
				</>
			) : entry.state === "cant_here" ? null : gate && copy ? (
				<>
					{fix && copy.fix ? (
						<DvButton size="xs" className="row-span-2" onClick={runFix}>
							{copy.fix}
						</DvButton>
					) : null}
					<div className="col-start-1">
						<GateInline kind={gate.kind} className="max-w-none">
							{copy.inline}
						</GateInline>
					</div>
				</>
			) : block || entry.state !== "not_served" ? null : (
				<DvButton size="xs" asChild>
					<a
						{...link({
							screen: "deploy",
							deviceIds: [deviceId],
							mode: "new",
							eventId,
							from: "events",
						})}
					>
						{t("events.pop.deployHere", "Deploy here")}
					</a>
				</DvButton>
			)}
		</li>
	);
}

/** Connected devices learn of a give-back at their next check; one that is not connected may keep running from its cache. */
function isConnected(view: AppView, deviceId: string | undefined): boolean {
	if (!deviceId) return false;
	const presence =
		view.groups.find((group) => group.deviceId === deviceId)?.presence ??
		Object.values(view.everywhereElse)
			.flat()
			.find((row) => row.deviceId === deviceId)?.presence;
	return presence?.kind === "online" || presence?.kind === "late";
}

type GiveBackResult = Awaited<
	ReturnType<EventsDevicesLive["schedules"]["giveBack"]>
>;

/** Why the hub did not take the schedule or bot back, in the reader's words. */
function giveBackRefusal(
	t: DevicesT,
	result: GiveBackResult,
	kind: WhereKind,
): string {
	if (result.kind === "schedule_role")
		return t(
			"devices:events.pop.runOnHub.needsRole",
			"Only someone who can edit this app's events can move it.",
		);
	if (result.kind === "missing_on_hub")
		return kind === "bot"
			? t(
					"devices:eligibility.hubType.bot.long",
					"This hub can't hand bots to devices yet. Update the hub.",
				)
			: t(
					"devices:eligibility.hubSchedules.long",
					"This hub can't run schedules on devices yet. Update the hub.",
				);
	return t(
		"devices:events.pop.runOnHub.refused",
		"The hub didn't take it back. Read where it runs again, then retry.",
	);
}

/** The hub's two hand-back grace periods: nothing of the service runs any more, or something still may (`api/instances/schedules.rs`). */
const CONFIRMED_GRACE_S = 300;
const UNCONFIRMED_GRACE_S = 3_900;

/** When the hub would take a schedule back now, as far as this page can tell: soon when its service runs nothing, else after the longer grace. */
function resumeEstimate(where: ScheduleWhere, nowS: number): number {
	const idle =
		where.fact === "device_idle" &&
		(where.why === "stopped" || where.why === "removed");
	return nowS + (idle ? CONFIRMED_GRACE_S : UNCONFIRMED_GRACE_S);
}

/** Where a give-back leaves a one-time schedule: its time is still ahead or not, and whether the hub has it back before then. */
function onceBack(
	t: DevicesT,
	once: OnceSchedule,
	resumesAt: number,
	time: Pick<AreaTime, "at" | "now">,
): string {
	return onceAhead(once, time.now)
		? t(
				"devices:events.pop.runOnHub.once",
				"It goes back to the hub from {{time}}. If its time is before that, nobody runs it.",
				{ time: time.at(resumesAt) },
			)
		: t(
				"devices:events.pop.runOnHub.onceDone",
				"It goes back to the hub. Its time has passed, so nothing runs it again.",
			);
}

/** Who can still answer as a bot after it is taken back: a device keeps the token until it is replaced at the provider. */
function botTokenText(
	t: DevicesT,
	values: { device: string; event: string; provider: string },
	endsAt: number | undefined,
	time: Pick<AreaTime, "at">,
) {
	const { device, event, provider } = values;
	return endsAt === undefined
		? t(
				"devices:events.pop.botTakeBack.tokenNoDate",
				"{{device}} disconnects {{events}} at its next check with the hub, within 30 minutes. If it can't reach the hub, it can stay connected until its cloud access ends. The bot token stays on {{device}}: to cut the bot off for certain, replace the token with {{provider}}.",
				{ device, events: event, provider },
			)
		: t(
				"devices:events.pop.botTakeBack.token",
				"{{device}} disconnects {{events}} at its next check with the hub, within 30 minutes. If it can't reach the hub, it can stay connected until its cloud access ends on {{date}}. The bot token stays on {{device}}: to cut the bot off for certain, replace the token with {{provider}}.",
				{ device, events: event, provider, date: time.at(endsAt) },
			);
}

/** Where a bot's token is replaced. */
function botProviderPlace(t: DevicesT, rule: EventEligibility) {
	return rule.bot?.provider === "discord"
		? t(
				"devices:events.pop.botTakeBack.provider.discord",
				"the Discord Developer Portal",
			)
		: t("devices:events.pop.botTakeBack.provider.telegram", "BotFather");
}

/** When the cloud access of the service a claim names ends, when this page can see it. */
function accessEndsAt(view: AppView, where: ScheduleWhere): number | undefined {
	if (!("deviceId" in where) || !where.deviceId || !where.serviceId)
		return undefined;
	const row = view.services.find(
		(entry) =>
			entry.deviceId === where.deviceId && entry.serviceId === where.serviceId,
	);
	return row?.cloud.state === "approved"
		? row.cloud.placement.grant.effective_expires_at
		: undefined;
}

interface WhereContext {
	t: DevicesT;
	event: IEvent;
	rule: EventEligibility;
	where: ScheduleWhere;
	live: EventsDevicesLive;
	time: AreaTime;
	names: WhereNames;
}

const valuesOf = ({ event, where, names }: WhereContext) => ({
	event: event.name,
	device: names.device("deviceId" in where ? where.deviceId : undefined),
});

/** Taking a bot back: the hub forgets the claim, the device disconnects at its next check and keeps the token. */
function botRows(context: WhereContext): ConsequenceRows {
	const { t, rule, where, live, time } = context;
	const values = valuesOf(context);
	const released = where.fact === "released";
	return {
		what: released
			? t(
					"devices:events.pop.botTakeBack.released",
					"{{device}} can no longer take it over. Nothing runs it until you deploy it again.",
					values,
				)
			: t(
					"devices:events.pop.botTakeBack.what",
					"{{device}} disconnects it at its next check, within 30 minutes. Nothing else runs it afterwards.",
					values,
				),
		who: released
			? null
			: t(
					"devices:events.pop.botTakeBack.who",
					"People who message {{event}} get no answer once it disconnects.",
					values,
				),
		stays: botTokenText(
			t,
			{ ...values, provider: botProviderPlace(t, rule) },
			accessEndsAt(live.view, where),
			time,
		),
		when: t("devices:action.service.when.now", "Immediately."),
		undo: {
			reversible: true,
			text: t(
				"devices:events.pop.botTakeBack.undo",
				"Deploy it to a device again, or run it in the desktop app.",
			),
		},
	};
}

/** Handing a schedule of either kind back to the hub. */
function scheduleRows(context: WhereContext): ConsequenceRows {
	const { t, rule, where, live, time } = context;
	const values = valuesOf(context);
	const deviceId = "deviceId" in where ? where.deviceId : undefined;
	const what =
		where.fact === "released"
			? t(
					"devices:events.pop.runOnHub.conseqReleased",
					"The hub keeps running {{event}}. {{device}} can no longer take it over.",
					values,
				)
			: rule.once
				? onceBack(t, rule.once, resumeEstimate(where, time.nowS), time)
				: t(
						"devices:events.pop.runOnHub.conseq",
						"The hub runs {{event}} again in about 5 minutes, or in about an hour while {{device}} is still running its service. {{device}} stops running it at its next check, within 30 minutes.",
						values,
					);
	return {
		what,
		who:
			where.fact === "released" || isConnected(live.view, deviceId)
				? t(
						"devices:events.pop.runOnHub.who",
						"Runs that are due before the hub takes over are skipped, never run twice.",
					)
				: t(
						"devices:events.pop.runOnHub.offline",
						"{{device}} is not connected. If it is still running, it keeps running {{event}} from its cache until it reconnects, so it can run in both places.",
						values,
					),
		when: t("devices:action.service.when.now", "Immediately."),
		undo: {
			reversible: true,
			text: t(
				"devices:events.pop.runOnHub.undo",
				"Deploy it to a device again once the hub has taken it back.",
			),
		},
	};
}

/** What the hub answered to a give-back, in the kind's words. */
function givenBackText(
	context: WhereContext,
	kind: WhereKind,
	resumes: number | null,
): string {
	const { t, rule, names, time } = context;
	const values = valuesOf(context);
	if (kind === "bot")
		return resumes === null
			? t(
					"devices:events.pop.botTakeBack.doneReleased",
					"Taken back. No device runs it.",
				)
			: t(
					"devices:events.pop.botTakeBack.done",
					"Taken back. {{device}} disconnects it within 30 minutes; nothing runs it afterwards.",
					values,
				);
	if (resumes === null)
		return t(
			"devices:events.pop.runOnHub.doneNow",
			"The hub runs {{event}}.",
			values,
		);
	return rule.once
		? onceBack(t, rule.once, resumes, time)
		: t(
				"devices:events.pop.runOnHub.done",
				"The hub runs {{event}} again from {{time}}.",
				{ ...values, time: names.at(resumes) },
			);
}

/** A schedule or bot of an online app: where it runs, and the way back for whoever may edit the app's events. */
function WhereSection({
	event,
	rule,
	where,
	kind,
	live,
}: Readonly<{
	event: IEvent;
	rule: EventEligibility;
	where: ScheduleWhere;
	kind: WhereKind;
	live: EventsDevicesLive;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [asking, setAsking] = useState(false);
	const [resumes, setResumes] = useState<number | null | undefined>(undefined);
	const names = whereNames(t, live.names, time);
	const context: WhereContext = { t, event, rule, where, live, time, names };
	const label =
		kind === "bot"
			? t("events.pop.botTakeBack.label", "Take it back")
			: t("events.pop.runOnHub.label", "Run it on the hub again");
	const confirm = async () => {
		const result = await live.schedules.giveBack(event.id);
		if (result.kind !== "ok") throw new Error(giveBackRefusal(t, result, kind));
		setResumes(result.data.hub_resumes_at);
		setAsking(false);
	};
	return (
		<div
			className={SECTION}
			data-schedule-where={where.fact}
			data-where-kind={kind}
		>
			<p className={LABEL}>{t("events.pop.whereLabel", "Where it runs")}</p>
			<p className="text-ui">{scheduleWhereText(t, where, names, kind)}</p>
			{resumes !== undefined ? (
				<InlineResult tone="good" onDismiss={() => setResumes(undefined)}>
					{givenBackText(context, kind, resumes)}
				</InlineResult>
			) : null}
			{!canRunOnHubAgain(where) ? null : asking ? (
				<InlineConfirm
					label={label}
					title={label}
					confirmLabel={label}
					tone="default"
					rows={kind === "bot" ? botRows(context) : scheduleRows(context)}
					onConfirm={confirm}
					onCancel={() => setAsking(false)}
				/>
			) : (
				<div className="flex flex-wrap items-center gap-2">
					{live.schedules.canEdit === false ? (
						<>
							<DvButton size="sm" icon={Undo2} aria-disabled data-gated="">
								{label}
							</DvButton>
							<GateInline kind="role">
								{t(
									"events.pop.runOnHub.needsRole",
									"Only someone who can edit this app's events can move it.",
								)}
							</GateInline>
						</>
					) : (
						<DvButton size="sm" icon={Undo2} onClick={() => setAsking(true)}>
							{label}
						</DvButton>
					)}
				</div>
			)}
		</div>
	);
}

/** The line under the event's name: its kind in its own words. */
function whereHead(t: DevicesT, event: IEvent, rule: EventEligibility): string {
	if (rule.once) return scheduleOnceHead(t, rule.once);
	if (rule.schedule) return scheduleHead(t, rule.schedule);
	if (rule.route)
		return t("devices:events.pop.endpoint", "Endpoint · {{method}} {{path}}", {
			method: rule.route.method,
			path: rule.route.path,
		});
	if (rule.kind === "on_demand")
		return event.event_type === "quick_action"
			? t("devices:events.pop.action", "Quick action · started by a person")
			: t("devices:events.pop.form", "Form · started by a person");
	return t("devices:events.pop.typeAndHow", "{{type}} · {{how}}", {
		type: eventTypeLabel(t, event.event_type, Boolean(event.default_page_id)),
		how: howItRunsCopy(t, rule),
	});
}

function WhereBody({
	event,
	rule,
	row,
	live,
}: Readonly<{
	event: IEvent;
	rule: EventEligibility;
	row: RunsOnRow;
	live: EventsDevicesLive;
}>) {
	const { t } = useTranslation("devices");
	const { block, link, explainMode } = useEventsDevices();
	const { name, mode, localOnly } = live.view.app;
	const stamp = headStamp(row);
	const runLabel = row.served.length
		? t("events.pop.runAnother", "Run on another device…")
		: t("events.cell.run", "Run on a device…");
	// A schedule or bot runs in one place: while a service holds it, another device can't take it.
	const held = holdsSchedule(row.where);
	return (
		<>
			<div className={SECTION}>
				<Head
					event={event}
					stamp={
						stamp ? (
							<FreshnessStamp {...stamp} compact />
						) : (
							<MixedSourcesStamp compact />
						)
					}
					sub={whereHead(t, event, rule)}
				/>
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px]/[17px] text-ink-2">
					<ModeChip mode={mode} app={name} />
					<span>
						{localOnly
							? t(
									"events.pop.modeOffline",
									"Devices get an offline copy of {{app}} from this computer. Its data lives only on the device.",
									{ app: name },
								)
							: t(
									"events.pop.modeOnline",
									"Devices run {{app}} online. Its data stays in the cloud.",
									{ app: name },
								)}
					</span>
					<DvButton variant="link" size="xs" onClick={explainMode}>
						{appCopy(t).explainButton()}
					</DvButton>
				</p>
				{localOnly && isClaimedRow(row) ? (
					<p className={SUB} data-schedule-where="local">
						{row.kind === "bot"
							? t(
									"events.pop.botLocal",
									"Runs in the desktop app while it is open. On a device it stays connected without a computer.",
								)
							: t(
									"deploy.what.scheduleLocal",
									"Runs in the desktop app while it is open. On a device it runs without a computer.",
								)}
					</p>
				) : null}
			</div>
			{row.where ? (
				<WhereSection
					event={event}
					rule={rule}
					where={row.where}
					kind={whereKind(row)}
					live={live}
				/>
			) : null}
			<Group
				label={t("events.pop.serves", "Serves it")}
				items={row.served}
				render={(served) => (
					<ServedRow served={served} row={row} stamped={stamp !== null} />
				)}
				empty={
					<li className="py-1.5 text-muted-foreground">
						{row.newIn
							? t(
									"events.pop.newIn",
									"New in {{version}}, so no device runs it yet.",
									{ version: row.newIn },
								)
							: row.unknown.length
								? t("events.pop.noneVisible", "Not on a device you can see.")
								: t("events.pop.none", "Not on any device yet.")}
					</li>
				}
			/>
			{row.unknown.length ? (
				<Group
					label={t("events.pop.unknown", "Status unknown")}
					items={row.unknown}
					render={(entry) => <UnknownRow entry={entry} />}
				/>
			) : null}
			{row.elsewhere.length ? (
				<Group
					label={t("events.pop.elsewhere", "Doesn't serve it")}
					items={row.elsewhere}
					render={(entry) => (
						<ElsewhereRow entry={entry} eventId={event.id} live={live} />
					)}
				/>
			) : null}
			<div className={SECTION}>
				<div className="flex flex-wrap items-center gap-2">
					{block ? (
						<>
							<DvButton size="sm" icon={Rocket} aria-disabled data-gated="">
								{runLabel}
							</DvButton>
							<GateInline kind={blockGateKind(block)}>
								{blockShort(t, block)}
							</GateInline>
						</>
					) : held ? (
						<>
							<DvButton size="sm" icon={Rocket} aria-disabled data-gated="">
								{runLabel}
							</DvButton>
							<GateInline kind="busy">{heldText(t, row)}</GateInline>
						</>
					) : (
						<DvButton size="sm" icon={Rocket} asChild>
							<a
								{...link({
									screen: "deploy",
									deviceIds: [],
									mode: "new",
									eventId: event.id,
									from: "events",
								})}
							>
								{runLabel}
							</a>
						</DvButton>
					)}
					<OpenInDevices eventId={event.id} link={link} />
				</div>
			</div>
		</>
	);
}

/** The Devices popover of one event: where it runs, or why it can't run on devices. */
export function OnDevicesPopover({
	eventId,
	onClose,
}: Readonly<{ eventId: string; onClose(): void }>) {
	const devices = useEventsDevices();
	const event = devices.events.get(eventId);
	const rule = devices.eligibility.get(eventId);
	if (!event || !rule) return null;
	if (!rule.eligible)
		return <WhyBody event={event} rule={rule} onClose={onClose} />;
	const row = devices.live?.rows.get(eventId);
	if (!devices.live || !row) return null;
	return <WhereBody event={event} rule={rule} row={row} live={devices.live} />;
}
