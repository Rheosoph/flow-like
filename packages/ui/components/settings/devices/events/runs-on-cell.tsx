"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	CircleArrowUp,
	CircleDashed,
	CircleSlash,
	Lock,
	type LucideIcon,
	Server,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import {
	Popover,
	PopoverAnchor,
	PopoverContent,
	PopoverTrigger,
} from "../../../ui/popover";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { CONVERGENCE_LOOK } from "../primitives/status-chip";
import { TONE_SOLID, type Tone, cx } from "../primitives/tone";
import { CHROME_POPOVER } from "../shell/attention-button";
import {
	type CantRun,
	SEPARATOR,
	cantRun,
	newerParts,
	servedActual,
	versionLabel,
} from "./events-copy";
import {
	AreaEventsDevices,
	type EventsDevicesStatus,
	type EventsDevicesValue,
	useEventsDevices,
	useOptionalEventsDevices,
} from "./events-devices";
import { OnDevicesPopover } from "./on-devices-popover";
import type { RunsOnRow, RunsOnServed } from "./runs-on-model";

/* APP §4.4: where one event runs on devices, as a table cell or a one-line summary. */

/** Id of the cell's visible text, for `aria-describedby` on the row's Run on a device… button. */
export function runsOnReasonId(eventId: string, compact = false): string {
	return `${compact ? "runs-on-summary" : "runs-on"}-${eventId}`;
}

const CHIPS_SHOWN = 2;
const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
const CHIP =
	"inline-flex h-5.5 max-w-full min-w-0 items-center gap-1.25 rounded-full border border-border bg-card px-1.75 text-xs font-medium whitespace-nowrap text-foreground no-underline hover:border-border-strong";
const SUB = "block text-xs/4 text-muted-foreground";

const DOT_SHAPE: Partial<Record<Tone, string>> = {
	paused: "rounded-[1px]",
	critical: "scale-85 rotate-45 rounded-[1px]",
	unknown: "border-[1.5px] border-unknown bg-transparent",
};

/** The service's convergence as a shaped dot (R14: never colour alone). */
function ConvergenceDot({ tone }: Readonly<{ tone: Tone }>) {
	return (
		<span
			aria-hidden
			data-tone={tone}
			className={cx(
				"size-2 shrink-0 rounded-full",
				tone === "unknown" ? undefined : TONE_SOLID[tone],
				DOT_SHAPE[tone],
			)}
		/>
	);
}

const servedTone = (served: RunsOnServed): Tone => {
	const { tone } = CONVERGENCE_LOOK[served.cell.conv ?? "unknown"];
	return tone === "outline" ? "unknown" : tone;
};

/** "invoice-extractor · Running · 1.4.0 → 1.5.0 available". */
function chipTitle(t: DevicesT, served: RunsOnServed, row: RunsOnRow) {
	const [service = ""] = served.cell.serviceIds;
	const actual = servedActual(t, served);
	const [drift] = newerParts(served, row);
	const version = (value: string) =>
		drift?.kind === "flow" ? versionLabel(t, "flow", value) : value;
	return drift
		? t(
				"devices:events.chip.titleBehind",
				"{{service}} · {{actual}} · {{from}} → {{to}} available",
				{
					service,
					actual,
					from: version(drift.from),
					to: version(drift.to),
				},
			)
		: t("devices:events.chip.title", "{{service}} · {{actual}}", {
				service,
				actual,
			});
}

function unknownLabel(t: DevicesT, row: RunsOnRow) {
	const count = row.unknown.length;
	return row.lockedOnly
		? t("devices:events.cell.locked", "{{count, number}} locked", { count })
		: t("devices:events.cell.unknown", "{{count, number}} unknown", { count });
}

/** Unknown is never "not deployed": with a device it can't read, the cell only speaks for the ones it can. */
function notOnText(t: DevicesT, row: RunsOnRow) {
	return row.unknown.length
		? t("devices:events.cell.notOnVisible", "Not on a device you can see")
		: t("devices:events.cell.notOn", "Not on a device");
}

/** What a cell says when it has no devices to show: never "Not on a device" (R6). */
function statusText(t: DevicesT, status: EventsDevicesStatus) {
	if (status === "signed_out" || status === "token")
		return t("devices:events.cell.signIn", "Sign in to see devices");
	if (status === "hub_off")
		return t("devices:events.cell.hubOff", "Device status off on this hub");
	if (status === "blind")
		return t(
			"devices:events.cell.blind",
			"Unknown: you can't read this app's flows",
		);
	if (status === "error")
		return t("devices:events.cell.unavailable", "Device status unavailable");
	return t("devices:events.cell.checking", "Checking devices…");
}

/** States that say the same thing in every row, whatever the event. */
const PAGE_STATUSES: readonly EventsDevicesStatus[] = [
	"signed_out",
	"token",
	"hub_off",
];

/** APP §4.4, exactly one per event: the page's state first, then the event's own rule, then its devices. */
function cellKind(devices: EventsDevicesValue, eventId: string) {
	if (PAGE_STATUSES.includes(devices.status)) return "page";
	if (devices.eligibility.get(eventId)?.code) return "cant";
	return devices.status === "ready" && devices.live?.rows.has(eventId)
		? "row"
		: "pending";
}

/** The cell of this event opens a popover: where it runs, or why it can't. */
export function runsOnExplains(
	devices: EventsDevicesValue,
	eventId: string,
): boolean {
	const kind = cellKind(devices, eventId);
	return kind === "cant" || kind === "row";
}

type CellCase =
	| { kind: "plain"; text: string }
	| { kind: "cant"; reason: CantRun }
	| { kind: "row"; row: RunsOnRow };

function cellCase(
	t: DevicesT,
	devices: EventsDevicesValue,
	eventId: string,
): CellCase {
	const kind = cellKind(devices, eventId);
	const reason = kind === "cant" ? cantRun(t, devices, eventId) : null;
	if (reason) return { kind: "cant", reason };
	const row = kind === "row" ? devices.live?.rows.get(eventId) : undefined;
	return row
		? { kind: "row", row }
		: { kind: "plain", text: statusText(t, devices.status) };
}

interface TriggerProps {
	open: boolean;
	toggle(): void;
}

/** A device that serves the event: links to the service in app scope; the dot is its convergence. */
function ServedChip({
	served,
	row,
}: Readonly<{ served: RunsOnServed; row: RunsOnRow }>) {
	const { t } = useTranslation("devices");
	const { link } = useEventsDevices();
	const [serviceId] = served.cell.serviceIds;
	const content = (
		<>
			<ConvergenceDot tone={servedTone(served)} />
			<span className="min-w-0 truncate font-mono text-[11.5px]">
				{served.device}
			</span>
			{served.cell.behind ? (
				<CircleArrowUp aria-hidden className="size-3 shrink-0 text-info" />
			) : null}
		</>
	);
	if (!serviceId) return <span className={CHIP}>{content}</span>;
	return (
		<a
			{...link({ screen: "service", deviceId: served.deviceId, serviceId })}
			title={chipTitle(t, served, row)}
			data-runs-on-chip={served.deviceId}
			className={CHIP}
		>
			{content}
		</a>
	);
}

function ServedChips({
	row,
	open,
	toggle,
}: Readonly<TriggerProps & { row: RunsOnRow }>) {
	const { t } = useTranslation("devices");
	const more = row.served.length - CHIPS_SHOWN;
	return (
		<>
			{row.served.slice(0, CHIPS_SHOWN).map((served) => (
				<ServedChip key={served.deviceId} served={served} row={row} />
			))}
			{more > 0 ? (
				<button
					type="button"
					aria-haspopup="dialog"
					aria-expanded={open}
					onClick={toggle}
					className={cx(CHIP, "cursor-pointer text-ink-2")}
				>
					{t("events.cell.more", "+{{count, number}}", { count: more })}
				</button>
			) : null}
		</>
	);
}

function UnknownChip({
	row,
	open,
	toggle,
}: Readonly<TriggerProps & { row: RunsOnRow }>) {
	const { t } = useTranslation("devices");
	if (!row.unknown.length) return null;
	const anyLocked = row.unknown.some(
		(entry) => entry.unknown.kind === "locked",
	);
	const Icon: LucideIcon = anyLocked ? Lock : CircleDashed;
	return (
		<button
			type="button"
			aria-haspopup="dialog"
			aria-expanded={open}
			onClick={toggle}
			title={t("events.cell.unknownTitle", {
				count: row.unknown.length,
				defaultValue_one: "Status unknown on {{count, number}} device",
				defaultValue_other: "Status unknown on {{count, number}} devices",
			})}
			data-runs-on-unknown={row.lockedOnly ? "locked" : "unknown"}
			className={cx(CHIP, "cursor-pointer border-dashed text-ink-2")}
		>
			<Icon aria-hidden className="size-3" />
			{unknownLabel(t, row)}
		</button>
	);
}

function DetailsButton({ label }: Readonly<{ label: string }>) {
	return (
		<PopoverTrigger asChild>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={ChevronDown}
				aria-label={label}
				data-runs-on-trigger=""
				className="size-5.5"
			/>
		</PopoverTrigger>
	);
}

/** The table cell: chips, counts and the details button on line 1, one fact on line 2. */
function FullCell({
	eventId,
	name,
	state,
	open,
	toggle,
}: Readonly<
	TriggerProps & { eventId: string; name: string; state: CellCase }
>) {
	const { t } = useTranslation("devices");
	const { link, block } = useEventsDevices();
	const line = "flex min-w-0 flex-wrap items-center gap-1";
	if (state.kind === "plain")
		return <span className="text-muted-foreground">{state.text}</span>;
	if (state.kind === "cant")
		return (
			<>
				<span className={line}>
					<span className="text-muted-foreground">
						{state.reason.paused
							? state.reason.short
							: t("events.cell.cantRun", "Can't run on devices")}
					</span>
					<DetailsButton
						label={t("events.cell.why", "Why {{event}} can't run on devices", {
							event: name,
						})}
					/>
				</span>
				{state.reason.paused ? null : (
					<span className={SUB}>{state.reason.short}</span>
				)}
			</>
		);
	const { row } = state;
	const details = (
		<DetailsButton
			label={t("events.cell.where", "Where {{event}} runs", { event: name })}
		/>
	);
	if (!row.served.length)
		return (
			<>
				<span className={line}>
					<span className="text-muted-foreground">{notOnText(t, row)}</span>
					<UnknownChip row={row} open={open} toggle={toggle} />
					{details}
				</span>
				{block ? null : (
					<a
						{...link({
							screen: "deploy",
							deviceIds: [],
							mode: "new",
							eventId,
							from: "events",
						})}
						className={cx(SUB, LINK, "w-fit text-foreground")}
					>
						{t("events.cell.run", "Run on a device…")}
					</a>
				)}
			</>
		);
	return (
		<>
			<span className={line}>
				<ServedChips row={row} open={open} toggle={toggle} />
				<UnknownChip row={row} open={open} toggle={toggle} />
				{details}
			</span>
			{row.older ? (
				<span className={SUB}>
					{t("events.cell.older", {
						count: row.older,
						defaultValue_one: "{{count, number}} runs an older version",
						defaultValue_other: "{{count, number}} run an older version",
					})}
				</span>
			) : null}
		</>
	);
}

function summaryText(t: DevicesT, row: RunsOnRow) {
	const names = row.served
		.slice(0, CHIPS_SHOWN)
		.map((served) => served.device)
		.join(t("devices:events.summary.comma", ", "));
	const more = row.served.length - CHIPS_SHOWN;
	const where = !row.served.length
		? notOnText(t, row)
		: more > 0
			? t(
					"devices:events.summary.onMore",
					"On {{devices}} +{{count, number}}",
					{ devices: names, count: more },
				)
			: t("devices:events.summary.on", "On {{devices}}", { devices: names });
	return [
		where,
		row.older
			? t("devices:events.summary.older", "{{count, number}} older", {
					count: row.older,
				})
			: "",
		row.unknown.length ? unknownLabel(t, row) : "",
	]
		.filter(Boolean)
		.join(SEPARATOR(t));
}

/** One line under the event's name on a phone: the same facts as a sentence that opens the popover. */
function SummaryCell({ state }: Readonly<{ state: CellCase }>) {
	const { t } = useTranslation("devices");
	const shell = "inline-flex max-w-full items-center gap-1.5 text-left";
	if (state.kind === "plain")
		return (
			<span className={cx(shell, "text-muted-foreground")}>
				<Server aria-hidden className="size-3 shrink-0" />
				<span className="min-w-0">{state.text}</span>
			</span>
		);
	const [first] = state.kind === "row" ? state.row.served : [];
	const text =
		state.kind === "row" ? summaryText(t, state.row) : state.reason.sentence;
	const glyph: ReactNode = first ? (
		<ConvergenceDot tone={servedTone(first)} />
	) : state.kind === "cant" ? (
		<CircleSlash aria-hidden className="size-3 shrink-0" />
	) : (
		<Server aria-hidden className="size-3 shrink-0" />
	);
	return (
		<PopoverTrigger asChild>
			<button
				type="button"
				data-runs-on-trigger=""
				className={cx(shell, LINK, "cursor-pointer text-foreground")}
			>
				{glyph}
				<span className="min-w-0 wrap-anywhere">{text}</span>
			</button>
		</PopoverTrigger>
	);
}

interface RunsOnCellProps {
	appId: string;
	eventId: string;
	/** The one-line summary of a phone row. */
	compact?: boolean;
}

/**
 * The Devices cell of one event (APP §4.4), from the same matrix as
 * App › Devices By event. On the Events page the list's
 * `EventsDevicesProvider` feeds every cell; inside the Devices area a cell
 * reads the area's workspace by itself.
 */
export function RunsOnCell(props: Readonly<RunsOnCellProps>) {
	const shared = useOptionalEventsDevices();
	if (shared) return <Cell {...props} />;
	return (
		<AreaEventsDevices appId={props.appId}>
			<Cell {...props} />
		</AreaEventsDevices>
	);
}

function Cell({ appId, eventId, compact = false }: Readonly<RunsOnCellProps>) {
	const { t } = useTranslation("devices");
	const devices = useEventsDevices();
	const [open, setOpen] = useState(false);
	const anchor = useRef<HTMLSpanElement>(null);
	const state = cellCase(t, devices, eventId);
	const name = devices.events.get(eventId)?.name ?? "";
	const hasPopover = state.kind !== "plain";
	const { focus, claimFocus } = devices;

	useEffect(() => {
		if (!hasPopover || focus?.eventId !== eventId) return;
		if (!anchor.current?.getClientRects().length) return;
		if (claimFocus(focus.token)) setOpen(true);
	}, [hasPopover, focus, eventId, claimFocus]);

	const toggle = () => setOpen((value) => !value);
	return (
		<Popover open={hasPopover && open} onOpenChange={setOpen}>
			<PopoverAnchor asChild>
				<span
					ref={anchor}
					id={runsOnReasonId(eventId, compact)}
					data-app={appId}
					data-runs-on={state.kind}
					className={cx(
						"min-w-0 text-[12.5px]/[17px]",
						compact ? "block" : "flex flex-col gap-0.5",
					)}
				>
					{compact ? (
						<SummaryCell state={state} />
					) : (
						<FullCell
							eventId={eventId}
							name={name}
							state={state}
							open={open}
							toggle={toggle}
						/>
					)}
				</span>
			</PopoverAnchor>
			{hasPopover ? (
				<PopoverContent
					align="start"
					collisionPadding={8}
					aria-label={
						state.kind === "cant"
							? t("events.cell.why", "Why {{event}} can't run on devices", {
									event: name,
								})
							: t("events.cell.where", "Where {{event}} runs", { event: name })
					}
					onInteractOutside={(event) => {
						if (anchor.current?.contains(event.target as Node))
							event.preventDefault();
					}}
					className={cx(CHROME_POPOVER, "w-[min(560px,calc(100vw-16px))] p-2")}
				>
					<OnDevicesPopover eventId={eventId} onClose={() => setOpen(false)} />
				</PopoverContent>
			) : null}
		</Popover>
	);
}

export function RunsOnColumnHeader(_props: Readonly<Record<string, never>>) {
	const { t } = useTranslation("devices");
	return <span>{t("events.column.devices", "Devices")}</span>;
}
