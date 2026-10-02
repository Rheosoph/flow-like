"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleArrowUp,
	CircleSlash,
	LockOpen,
	Rocket,
	Settings,
} from "lucide-react";
import { Fragment, type ReactNode, useState } from "react";
import type {
	EventEligibility,
	EventIneligibleCode,
} from "../../../../lib/device-management/deployment";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { appCopy } from "../copy/app-copy";
import {
	eligibilityCopy,
	eventTypeLabel,
	howItRunsCopy,
} from "../copy/eligibility-copy";
import { gateCopy } from "../copy/gate-copy";
import { ModeChip } from "../primitives/app-chips";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	type FreshnessStampProps,
	MixedSourcesStamp,
	sameSource,
} from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { MatrixCell } from "../primitives/matrix-cell";
import { PairedPins } from "../primitives/paired-pins";
import type { DesiredRun, ObservedRun } from "../primitives/requested-actual";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { stampOf } from "../shell/attention-popover";
import { useOverlayStore } from "../workspace";
import {
	SEPARATOR,
	blockGateKind,
	blockShort,
	newerParts,
	servedActual,
	versionLabel,
	versionText,
} from "./events-copy";
import {
	type EventsDevicesLive,
	type EventsDevicesValue,
	useEventsDevices,
} from "./events-devices";
import type {
	RunsOnElsewhere,
	RunsOnRow,
	RunsOnServed,
	RunsOnUnknown,
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
	"paused" | "latest_flow" | "canary" | "variants"
>;

function fixSteps(t: DevicesT, code: EventIneligibleCode): string | null {
	const steps = {
		paused: t(
			"devices:events.fix.paused",
			"Activate it with the play button in its row, then run it on a device.",
		),
		latest_flow: t(
			"devices:events.fix.latestFlow",
			"In the event's Flow & target section, pick a published flow version instead of Latest. Devices only run pinned versions.",
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
	const { eligibility, openEvent, status, link } = useEventsDevices();
	const code = rule.code ?? "type";
	const reason = eligibilityCopy(t, {
		code,
		eventType: event.event_type,
		...(rule.detail ? { detail: rule.detail } : {}),
	});
	const steps = fixSteps(t, code);
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
				{steps ? (
					<div className="flex flex-col gap-1.5 rounded-lg border border-border bg-surface-sunken px-2.5 py-2">
						<p className="text-[12.5px]/[17px]">
							<b className="font-semibold">
								{t("events.fix.lead", "To fix it:")}
							</b>{" "}
							{steps}
						</p>
						{openEvent && code !== "paused" ? (
							<DvButton
								size="sm"
								icon={Settings}
								className="w-fit"
								onClick={() => {
									onClose();
									openEvent(event.id);
								}}
							>
								{t("events.fix.open", "Open the event…")}
							</DvButton>
						) : null}
					</div>
				) : null}
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

function ServedRow({
	served,
	row,
	stamped,
}: Readonly<{ served: RunsOnServed; row: RunsOnRow; stamped: boolean }>) {
	const { t } = useTranslation("devices");
	const { link } = useEventsDevices();
	const { cell, service } = served;
	const [serviceId] = cell.serviceIds;
	const href = serviceId
		? link({ screen: "service", deviceId: served.deviceId, serviceId })
		: null;
	const view = service?.view;
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
			{href ? (
				<DvButton size="xs" asChild className="row-span-2">
					<a {...href}>{t("events.pop.open", "Open")}</a>
				</DvButton>
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
				{stamped || !view ? null : (
					<FreshnessStamp {...stampOf(view.freshness)} compact />
				)}
			</p>
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
					<MatrixCell state="cant_here" reason={entry.reason ?? ""} />
				) : (
					<MatrixCell state="no_access" />
				)}
			</div>
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
					sub={t("events.pop.typeAndHow", "{{type}} · {{how}}", {
						type: eventTypeLabel(
							t,
							event.event_type,
							Boolean(event.default_page_id),
						),
						how: howItRunsCopy(t, rule),
					})}
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
			</div>
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
