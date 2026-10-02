"use client";

import { useTranslation } from "@flow-like/locales";
import {
	FileBadge,
	KeyRound,
	LockOpen,
	type LucideIcon,
	RefreshCw,
	Server,
	Stethoscope,
} from "lucide-react";
import { type MouseEvent, type ReactNode, useMemo } from "react";
import {
	type DeviceFacts,
	fleetFacts,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import type {
	AttentionInput,
	DevicesRoute,
	DevicesScope,
	MyAccess,
} from "../../../../lib/device-management/model/types";
import type { LiveState } from "../../../../lib/device-management/workspace/types";
import { enumLabel } from "../copy/enum-labels";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import { useAreaTime, useHubFreshness } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import {
	ageLabel,
	planeText,
	sourceLabel,
} from "../primitives/freshness-stamp";
import { SOURCE_ICON } from "../primitives/icons";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, cx } from "../primitives/tone";
import { devicesHref } from "../routing/devices-href";
import {
	type PlaneSegment,
	type PlaneSegmentId,
	type PlaneStatus,
	useAttentionState,
	useHubSupport,
	useOverlayStore,
	usePlaneStatus,
} from "../workspace";
import { type ChromeNavigate, plainClick } from "./rail-row";

/** The six status-bar segments (SPEC §3.5): the five planes a page reads plus the certificate reports. */
export const PLANE_SEGMENTS = [
	"hub",
	"status",
	"live",
	"local",
	"device",
	"certificates",
] as const satisfies readonly PlaneSegmentId[];
export type { PlaneSegmentId };

type Close = () => void;
type OpenDevice = (deviceId: string) => void;

export interface PlanePopoverProps {
	plane: PlaneSegmentId;
	scope: DevicesScope;
	onNavigate: ChromeNavigate;
	/** Called after a link or action was chosen so the popover closes. */
	onClose?: Close;
}

/* Model (pure): what the page knows about each plane, and the sentences made of it. */

export type PlaneState = "ok" | "warn" | "err" | "off" | "busy";

/** Times are unix seconds, hub-corrected. */
export interface PlaneFacts {
	hub: {
		state: "checking" | "on" | "off" | "unreachable";
		/** The last refresh failed; data on screen is from `dataFrom`. */
		failing: boolean;
		checkedAt?: number;
		dataFrom?: number;
		retryAt?: number;
		/** "hub.example.com". */
		host?: string;
	};
	status: {
		/** Non-revoked devices the viewer can see. */
		active: number;
		readable: number;
		locked: number;
		newestAt?: number;
		/** Devices whose last refresh failed. */
		failing: number;
	};
	live: { connections: number; reconnecting: number };
	local: { platform: "desktop" | "web"; persisted: boolean; keys: number };
	/** Devices only a shell on the device can explain (offline, never checked in). */
	device: { toCheck: number };
	certificates: {
		total: number;
		reported: number;
		never: number;
		noAccess: number;
	};
}

export interface PlaneLine {
	id: PlaneSegmentId;
	state: PlaneState;
	/** The state in a word, for the chip ("Current"). */
	word: string;
	/** The status-bar text ("3 of 5 readable"). */
	short: string;
	/** The full line ("3 of 5 readable · newest 41 s ago · 2 locked"). */
	text: string;
}

const PLANE_TONE: Record<PlaneState, ChipTone> = {
	ok: "good",
	warn: "warning",
	err: "critical",
	off: "unknown",
	busy: "info",
};

const PLANE_ICON: Record<PlaneSegmentId, LucideIcon> = {
	hub: SOURCE_ICON.hub,
	status: SOURCE_ICON.snap,
	live: SOURCE_ICON.live,
	local: SOURCE_ICON.local,
	device: SOURCE_ICON.device,
	certificates: FileBadge,
};

export function planeIcon(id: PlaneSegmentId) {
	return PLANE_ICON[id];
}

export function planeName(t: DevicesT, id: PlaneSegmentId) {
	const names = {
		hub: sourceLabel(t, "hub"),
		status: t("devices:chrome.planes.name.status", "Encrypted status"),
		live: sourceLabel(t, "live"),
		local: sourceLabel(t, "local"),
		device: sourceLabel(t, "device"),
		certificates: t("devices:chrome.planes.name.certificates", "Certificates"),
	} satisfies Record<PlaneSegmentId, string>;
	return names[id];
}

/** SPEC §6.1: what the plane is, in one sentence. */
export function planeExplanation(t: DevicesT, id: PlaneSegmentId) {
	const texts = {
		hub: planeText(t, "hub", 30),
		status: planeText(t, "snap", 60),
		live: planeText(t, "live", 15),
		local: planeText(t, "local", 0),
		device: planeText(t, "device", 0),
		certificates: t(
			"devices:chrome.planes.explain.certificates",
			"Each device reports its certificate IDs and expiry to the hub, on change and at least hourly.",
		),
	} satisfies Record<PlaneSegmentId, string>;
	return texts[id];
}

export function planeCadence(t: DevicesT, id: PlaneSegmentId) {
	const cadences = {
		hub: t("devices:chrome.planes.cadence.hub", "every 30 s"),
		status: t(
			"devices:chrome.planes.cadence.status",
			"on change, at least every 60 s",
		),
		live: t("devices:chrome.planes.cadence.live", "every 15 s while connected"),
		local: t(
			"devices:chrome.planes.cadence.local",
			"whenever keys on this computer change",
		),
		device: t(
			"devices:chrome.planes.cadence.device",
			"only with a shell on the device",
		),
		certificates: t(
			"devices:chrome.planes.cadence.certificates",
			"on change, at least hourly",
		),
	} satisfies Record<PlaneSegmentId, string>;
	return cadences[id];
}

export function planeReaders(t: DevicesT, id: PlaneSegmentId) {
	const readers = {
		hub: t(
			"devices:chrome.planes.who.hub",
			"Any signed-in account with access, without a device password.",
		),
		status: t(
			"devices:chrome.planes.who.status",
			"Only this computer, with the device's keys unlocked.",
		),
		live: t(
			"devices:chrome.planes.who.live",
			"This window, while a live connection is open.",
		),
		local: t(
			"devices:chrome.planes.who.local",
			"Only this app profile on this computer.",
		),
		device: t(
			"devices:chrome.planes.who.device",
			"Someone with a shell on the device.",
		),
		certificates: t(
			"devices:chrome.planes.who.certificates",
			"The owner and people with whole-device View status.",
		),
	} satisfies Record<PlaneSegmentId, string>;
	return readers[id];
}

type Segment = Omit<PlaneLine, "id">;

function segment(state: PlaneState, word: string, short: string, text = short) {
	const built: Segment = { state, word, short, text };
	return built;
}

function retryText(t: DevicesT, time: AreaTime, hub: PlaneFacts["hub"]) {
	if (hub.retryAt === undefined) return "";
	const countdown = time.countdown(hub.retryAt);
	return t("devices:chrome.planes.hub.retryIn", " · retry in {{time}}", {
		time: countdown,
	});
}

function hubFailing(t: DevicesT, time: AreaTime, hub: PlaneFacts["hub"]) {
	const word = t("devices:chrome.planes.hub.failingWord", "Couldn't refresh");
	const retry = retryText(t, time, hub);
	if (hub.dataFrom === undefined) {
		const bare = t("devices:chrome.planes.hub.failingBare", "couldn't refresh");
		return segment("err", word, bare, `${bare}${retry}`);
	}
	const from = time.clock(hub.dataFrom);
	const short = t(
		"devices:chrome.planes.hub.failingShort",
		"couldn't refresh since {{time}}",
		{ time: from },
	);
	const text = t(
		"devices:chrome.planes.hub.failing",
		"couldn't refresh · data from {{time}}",
		{ time: from },
	);
	return segment("err", word, short, `${text}${retry}`);
}

function hubChecking(t: DevicesT, hub: PlaneFacts["hub"]) {
	const word = t("devices:chrome.planes.hub.checkingWord", "Checking");
	const short = t("devices:chrome.planes.hub.checkingShort", "checking…");
	if (!hub.host) return segment("busy", word, short);
	const text = t("devices:chrome.planes.hub.checking", "checking {{host}}…", {
		host: hub.host,
	});
	return segment("busy", word, short, text);
}

function hubDown(t: DevicesT, hub: PlaneFacts["hub"]) {
	if (hub.state === "unreachable") {
		const word = t("devices:chrome.planes.hub.unreachableWord", "Unreachable");
		const short = t(
			"devices:chrome.planes.hub.unreachableShort",
			"unreachable",
		);
		const text = t(
			"devices:chrome.planes.hub.unreachable",
			"unreachable · retrying",
		);
		return segment("err", word, short, text);
	}
	if (hub.state !== "off") return hubChecking(t, hub);
	const word = t("devices:chrome.planes.hub.offWord", "Devices off");
	const short = t("devices:chrome.planes.hub.offShort", "devices off");
	const text = t(
		"devices:chrome.planes.hub.off",
		"devices are off on this hub",
	);
	return segment("err", word, short, text);
}

function hubSegment(t: DevicesT, time: AreaTime, hub: PlaneFacts["hub"]) {
	if (hub.state !== "on") return hubDown(t, hub);
	if (hub.failing) return hubFailing(t, time, hub);
	const word = t("devices:chrome.planes.hub.currentWord", "Current");
	if (hub.checkedAt === undefined) {
		const current = t("devices:chrome.planes.hub.current", "current");
		return segment("ok", word, current);
	}
	const ago = time.ago(hub.checkedAt);
	const checked = t("devices:chrome.planes.hub.checked", "checked {{ago}}", {
		ago,
	});
	return segment("ok", word, checked);
}

function statusParts(t: DevicesT, time: AreaTime, snap: PlaneFacts["status"]) {
	const parts: string[] = [];
	if (snap.readable > 0 && snap.newestAt !== undefined) {
		const ago = time.ago(snap.newestAt);
		const newest = t("devices:chrome.planes.status.newest", "newest {{ago}}", {
			ago,
		});
		parts.push(newest);
	}
	if (snap.locked > 0) {
		const locked = t(
			"devices:chrome.planes.status.locked",
			"{{count, number}} locked",
			{ count: snap.locked },
		);
		parts.push(locked);
	}
	if (snap.failing > 0) {
		const failing = t(
			"devices:chrome.planes.status.failing",
			"{{count, number}} couldn't refresh",
			{ count: snap.failing },
		);
		parts.push(failing);
	}
	return parts;
}

function statusState(snap: PlaneFacts["status"]) {
	if (snap.failing > 0) return "warn";
	return snap.readable > 0 ? "ok" : "off";
}

function statusSegment(
	t: DevicesT,
	time: AreaTime,
	snap: PlaneFacts["status"],
) {
	const short = t(
		"devices:chrome.planes.status.short",
		"{{readable, number}} of {{active, number}} readable",
		{ readable: snap.readable, active: snap.active },
	);
	const word =
		snap.readable > 0
			? t("devices:chrome.planes.status.readableWord", "Readable")
			: t("devices:chrome.planes.status.noneWord", "Nothing readable");
	const parts = statusParts(t, time, snap);
	const text = [short, ...parts].join(" · ");
	return segment(statusState(snap), word, short, text);
}

function liveParts(t: DevicesT, live: PlaneFacts["live"]) {
	const parts: string[] = [];
	if (live.connections > 0) {
		const connections = t("devices:chrome.planes.live.connections", {
			count: live.connections,
			defaultValue_one: "{{count, number}} connection",
			defaultValue_other: "{{count, number}} connections",
		});
		parts.push(connections);
	}
	if (live.reconnecting > 0) {
		const reconnecting = t(
			"devices:chrome.planes.live.reconnecting",
			"{{count, number}} reconnecting",
			{ count: live.reconnecting },
		);
		parts.push(reconnecting);
	}
	return parts;
}

function liveSegment(t: DevicesT, live: PlaneFacts["live"]) {
	const parts = liveParts(t, live);
	if (parts.length === 0) {
		const word = t("devices:chrome.planes.live.noneWord", "Not connected");
		const none = t("devices:chrome.planes.live.none", "not connected");
		return segment("off", word, none);
	}
	const short = parts.join(" · ");
	const text = t(
		"devices:chrome.planes.live.text",
		"{{summary}} · refreshed every 15 s",
		{ summary: short },
	);
	if (live.reconnecting > 0) {
		const word = t(
			"devices:chrome.planes.live.reconnectingWord",
			"Reconnecting",
		);
		return segment("warn", word, short, text);
	}
	const word = t("devices:chrome.planes.live.connectedWord", "Connected");
	return segment("ok", word, short, text);
}

function localSegment(t: DevicesT, local: PlaneFacts["local"]) {
	const keys = t("devices:chrome.planes.local.keys", {
		count: local.keys,
		defaultValue_one: "keys for {{count, number}} device",
		defaultValue_other: "keys for {{count, number}} devices",
	});
	const desktop = local.platform === "desktop";
	const platform = desktop
		? t("devices:chrome.planes.local.desktop", "Desktop app")
		: t("devices:chrome.planes.local.web", "Web");
	if (desktop || local.persisted) {
		const word = t("devices:chrome.planes.local.safeWord", "Kept safely");
		const safe = t("devices:chrome.planes.local.safe", "kept safely");
		return segment("ok", word, keys, `${platform} · ${keys} · ${safe}`);
	}
	const word = t("devices:chrome.planes.local.atRiskWord", "Keys at risk");
	const short = t(
		"devices:chrome.planes.local.atRiskShort",
		"keys may be deleted",
	);
	const risk = t(
		"devices:chrome.planes.local.atRisk",
		"the browser may delete them",
	);
	return segment("warn", word, short, `${platform} · ${keys} · ${risk}`);
}

function deviceSegment(t: DevicesT, device: PlaneFacts["device"]) {
	const count = device.toCheck;
	if (count === 0) {
		const word = t("devices:chrome.planes.device.noneWord", "Nothing to check");
		const none = t("devices:chrome.planes.device.none", "nothing to check");
		return segment("off", word, none);
	}
	const word = t("devices:chrome.planes.device.word", "Needs a shell");
	const short = t(
		"devices:chrome.planes.device.short",
		"{{count, number}} to check",
		{ count },
	);
	const text = t("devices:chrome.planes.device.text", {
		count,
		defaultValue_one: "{{count, number}} device needs a local check",
		defaultValue_other: "{{count, number}} devices need a local check",
	});
	return segment("off", word, short, text);
}

function certificateParts(t: DevicesT, certs: PlaneFacts["certificates"]) {
	const reported = t("devices:chrome.planes.certificates.reported", {
		count: certs.reported,
		defaultValue_one: "{{count, number}} device reported",
		defaultValue_other: "{{count, number}} devices reported",
	});
	const parts = [reported];
	if (certs.never > 0) {
		const never = t(
			"devices:chrome.planes.certificates.never",
			"{{count, number}} not yet",
			{ count: certs.never },
		);
		parts.push(never);
	}
	if (certs.noAccess > 0) {
		const noAccess = t(
			"devices:chrome.planes.certificates.noAccess",
			"{{count, number}} no access",
			{ count: certs.noAccess },
		);
		parts.push(noAccess);
	}
	return parts;
}

function certificatesSegment(t: DevicesT, certs: PlaneFacts["certificates"]) {
	const short = t(
		"devices:chrome.planes.certificates.short",
		"{{reported, number}} of {{total, number}} reported",
		{ reported: certs.reported, total: certs.total },
	);
	const text = certificateParts(t, certs).join(" · ");
	if (certs.reported > 0) {
		const word = t(
			"devices:chrome.planes.certificates.reportedWord",
			"Reported",
		);
		return segment("ok", word, short, text);
	}
	const word = t(
		"devices:chrome.planes.certificates.noneWord",
		"Nothing reported",
	);
	return segment("off", word, short, text);
}

/** The six segments, in status-bar order. */
export function planeSegments(t: DevicesT, time: AreaTime, facts: PlaneFacts) {
	const lines: PlaneLine[] = [];
	const add = (id: PlaneSegmentId, line: Segment) => {
		lines.push({ id, ...line });
	};
	add("hub", hubSegment(t, time, facts.hub));
	add("status", statusSegment(t, time, facts.status));
	add("live", liveSegment(t, facts.live));
	add("local", localSegment(t, facts.local));
	add("device", deviceSegment(t, facts.device));
	add("certificates", certificatesSegment(t, facts.certificates));
	return lines;
}

const WORST: readonly PlaneState[] = ["err", "warn", "busy"];

function firstInState(lines: readonly PlaneLine[], state: PlaneState) {
	for (const line of lines) if (line.state === state) return line;
	return undefined;
}

/** The plane the compact bar names: failing before warning before busy; none = all current. */
export function worstPlane(lines: readonly PlaneLine[]) {
	for (const state of WORST) {
		const found = firstInState(lines, state);
		if (found) return found;
	}
	return undefined;
}

/* View. */

const DOT: Record<PlaneState, string> = {
	ok: "rounded-full bg-good-solid",
	warn: "rounded-[1px] bg-warning-solid",
	err: "scale-85 rotate-45 rounded-[1px] bg-critical-solid",
	off: "rounded-full border-[1.5px] border-unknown",
	busy: "animate-spin rounded-full border-[1.5px] border-info-solid border-r-transparent",
};

interface PlaneDotProps {
	state: PlaneState;
}

/** A shape per state, never colour alone. */
export function PlaneDot({ state }: Readonly<PlaneDotProps>) {
	const shape = cx("inline-block size-2 shrink-0", DOT[state]);
	return <span aria-hidden data-dot={state} className={shape} />;
}

/** One device's state on a plane. */
export interface PlaneDeviceRow {
	deviceId: string;
	name: string;
	state: string;
	/** Unix seconds; rendered as an age. */
	at?: number;
	/** Replaces the age (e.g. a Diagnose button). */
	action?: ReactNode;
}

const HEAD_CELL =
	"border-b border-hairline px-1.5 py-1 text-left text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase";
const CELL = "px-1.5 py-1.5 align-top";
const NAME_CELL = "max-w-[18ch] truncate px-1.5 py-1.5 align-top font-mono";
const AGE_CELL = "px-1.5 py-1.5 align-top whitespace-nowrap tabular-nums";
const DEVICE_LINK =
	"text-foreground underline decoration-border-strong underline-offset-2";

interface PlaneTableRowProps {
	row: PlaneDeviceRow;
	scope: DevicesScope;
	onOpenDevice?: OpenDevice;
}

function PlaneTableRow({
	row,
	scope,
	onOpenDevice,
}: Readonly<PlaneTableRowProps>) {
	const time = useAreaTime();
	const route: DevicesRoute = {
		screen: "device",
		deviceId: row.deviceId,
		tab: "overview",
	};
	const open = (event: MouseEvent) => {
		if (!onOpenDevice || !plainClick(event)) return;
		event.preventDefault();
		onOpenDevice(row.deviceId);
	};
	const age = row.at === undefined ? "–" : time.ago(row.at);
	return (
		<tr className="border-t border-hairline">
			<td className={NAME_CELL}>
				<a
					href={devicesHref(route, scope)}
					onClick={open}
					title={row.name}
					className={DEVICE_LINK}
				>
					{row.name}
				</a>
			</td>
			<td className={CELL}>{row.state}</td>
			<td className={AGE_CELL}>{row.action ?? age}</td>
		</tr>
	);
}

interface PlaneTableProps {
	rows: readonly PlaneDeviceRow[];
	scope: DevicesScope;
	onOpenDevice?: OpenDevice;
}

function PlaneTable({ rows, scope, onOpenDevice }: Readonly<PlaneTableProps>) {
	const { t } = useTranslation("devices");
	const empty = rows.length ? null : (
		<tr>
			<td colSpan={3} className="px-1.5 py-1.5 text-muted-foreground">
				{t("chrome.planes.table.empty", "No devices to show here.")}
			</td>
		</tr>
	);
	return (
		<table data-plane-table="" className="w-full border-collapse text-xs">
			<thead>
				<tr>
					<th scope="col" className={HEAD_CELL}>
						{t("chrome.planes.table.device", "Device")}
					</th>
					<th scope="col" className={HEAD_CELL}>
						{t("chrome.planes.table.state", "State")}
					</th>
					<th scope="col" className={HEAD_CELL}>
						{t("chrome.planes.table.age", "Age")}
					</th>
				</tr>
			</thead>
			<tbody>
				{empty}
				{rows.map((row) => {
					return (
						<PlaneTableRow
							key={row.deviceId}
							row={row}
							scope={scope}
							onOpenDevice={onOpenDevice}
						/>
					);
				})}
			</tbody>
		</table>
	);
}

export interface PlaneDetailProps {
	segment: PlaneLine;
	rows: readonly PlaneDeviceRow[];
	scope: DevicesScope;
	/** Retry, Hub status, Unlock several…, Keys & recovery, Certificates. */
	actions?: ReactNode;
	onOpenDevice?: OpenDevice;
	/** Heading level of the plane's name. */
	heading?: "h3" | "h4";
	className?: string;
}

const ACTION_ROW = "flex flex-wrap items-center gap-2 pt-0.5";

/** One plane explained: state, what it is, cadence, readers, per-device table. Shared with the plane sheet. */
export function PlaneDetail(props: Readonly<PlaneDetailProps>) {
	const { t } = useTranslation("devices");
	const { segment: line, actions } = props;
	const Icon = planeIcon(line.id);
	const Heading = props.heading ?? "h3";
	const name = planeName(t, line.id);
	const explanation = planeExplanation(t, line.id);
	const cadence = planeCadence(t, line.id);
	const who = planeReaders(t, line.id);
	const hint = t(
		"chrome.planes.hint",
		"Refreshed {{cadence}} · Who can read it: {{who}}",
		{ cadence, who },
	);
	const root = cx("flex min-w-0 flex-col", props.className);
	const actionRow = actions ? (
		<div className={ACTION_ROW}>{actions}</div>
	) : null;
	return (
		<div
			data-chrome="plane-detail"
			data-plane={line.id}
			data-state={line.state}
			className={root}
		>
			<div className="flex flex-col gap-1.5 px-4 pt-3 pb-2.5">
				<Heading className="flex items-center gap-2 text-ui font-semibold">
					<PlaneDot state={line.state} />
					<Icon aria-hidden className="size-4 text-muted-foreground" />
					<span className="min-w-0 flex-1 truncate">{name}</span>
					<StatusChip tone={PLANE_TONE[line.state]}>{line.word}</StatusChip>
				</Heading>
				<p className="font-medium text-foreground">{line.text}</p>
				<p className="text-ink-2">{explanation}</p>
				<p className="text-xs text-muted-foreground">{hint}</p>
				{actionRow}
			</div>
			<div className="border-t border-hairline px-2.5 pt-1.5 pb-2.5">
				<PlaneTable
					rows={props.rows}
					scope={props.scope}
					onOpenDevice={props.onOpenDevice}
				/>
			</div>
		</div>
	);
}

/* Binding: workspace state → facts, lines and per-device rows. */

function segmentOf(status: PlaneStatus, id: PlaneSegmentId) {
	for (const entry of status.segments) if (entry.id === id) return entry;
	return undefined;
}

const countOf = (entry: PlaneSegment | undefined) => (entry ? entry.count : 0);
const failingOf = (entry: PlaneSegment | undefined) =>
	entry ? entry.failing : 0;

const isActive = (device: DeviceFacts) => device.active;
const hasLockedKeys = (device: DeviceFacts) =>
	device.active && keysLocked(device.keys);
const needsLocalCheck = (device: DeviceFacts) =>
	device.active &&
	(device.presence.kind === "offline" || device.presence.kind === "never");

const seesWholeDevice = (grant: MyAccess["grants"][number]) =>
	grant.scope.kind === "device" && grant.capabilities.includes("status");

/** A shared device whose access rules don't include whole-device View status can't read the report. */
function certificatesHidden(input: AttentionInput, device: DeviceFacts) {
	if (device.relationship !== "shared" || !input.myAccess) return false;
	const access = input.myAccess[device.id];
	if (!access) return false;
	return !access.grants.some(seesWholeDevice);
}

function reportedAt(input: AttentionInput, deviceId: string) {
	const inventory = input.certInventory[deviceId];
	return inventory ? inventory.updated_at : null;
}

function statusFreshness(input: AttentionInput, deviceId: string) {
	const state = input.fleet[deviceId];
	return state ? state.freshness.status : undefined;
}

function newestStatus(input: AttentionInput, devices: readonly DeviceFacts[]) {
	let newest = Number.NEGATIVE_INFINITY;
	for (const device of devices) {
		const freshness = statusFreshness(input, device.id);
		if (freshness && freshness.at !== undefined)
			newest = Math.max(newest, freshness.at);
	}
	return Number.isFinite(newest) ? newest : undefined;
}

function hiddenReports(input: AttentionInput, devices: readonly DeviceFacts[]) {
	let hidden = 0;
	for (const device of devices) {
		if (reportedAt(input, device.id) != null) continue;
		if (certificatesHidden(input, device)) hidden += 1;
	}
	return hidden;
}

export interface PlaneHubState {
	failing: boolean;
	dataFrom?: number;
	retryAt?: number;
	host?: string;
}

function hubFacts(
	input: AttentionInput,
	status: PlaneStatus,
	hub: PlaneHubState,
) {
	const entry = segmentOf(status, "hub");
	const checkedAt = entry ? entry.freshness.at : undefined;
	const facts: PlaneFacts["hub"] = {
		...hub,
		state: input.hub.state,
		checkedAt,
	};
	return facts;
}

function statusFacts(
	input: AttentionInput,
	status: PlaneStatus,
	active: readonly DeviceFacts[],
) {
	const entry = segmentOf(status, "status");
	const readable = countOf(entry);
	const failing = failingOf(entry);
	const locked = active.filter(hasLockedKeys).length;
	const newestAt = newestStatus(input, active);
	const facts: PlaneFacts["status"] = {
		active: active.length,
		readable,
		locked,
		newestAt,
		failing,
	};
	return facts;
}

function certificateFacts(
	input: AttentionInput,
	status: PlaneStatus,
	active: readonly DeviceFacts[],
) {
	const total = active.length;
	const reported = countOf(segmentOf(status, "certificates"));
	const noAccess = hiddenReports(input, active);
	const never = Math.max(0, total - reported - noAccess);
	const facts: PlaneFacts["certificates"] = {
		total,
		reported,
		never,
		noAccess,
	};
	return facts;
}

function localFacts(input: AttentionInput) {
	const { platform, persistence, vaults } = input.local;
	const persisted = persistence === "persisted";
	const facts: PlaneFacts["local"] = {
		platform,
		persisted,
		keys: vaults.length,
	};
	return facts;
}

/** What the six planes can say right now, from the one attention input. */
export function planeFactsOf(
	input: AttentionInput,
	status: PlaneStatus,
	hub: PlaneHubState,
) {
	const active = fleetFacts(input).devices.filter(isActive);
	const liveEntry = segmentOf(status, "live");
	const connections = countOf(liveEntry);
	const reconnecting = failingOf(liveEntry);
	const toCheck = active.filter(needsLocalCheck).length;
	const hubLine = hubFacts(input, status, hub);
	const statusLine = statusFacts(input, status, active);
	const local = localFacts(input);
	const certificates = certificateFacts(input, status, active);
	const facts: PlaneFacts = {
		hub: hubLine,
		status: statusLine,
		live: { connections, reconnecting },
		local,
		device: { toCheck },
		certificates,
	};
	return facts;
}

/** The six status-bar lines of the area, re-read on the area clock. */
export function usePlaneLines() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const status = usePlaneStatus();
	const { failing, dataFrom, retryAt } = useHubFreshness();
	const { host } = useHubSupport();
	const facts = useMemo(() => {
		return planeFactsOf(input, status, { failing, dataFrom, retryAt, host });
	}, [input, status, failing, dataFrom, retryAt, host]);
	return planeSegments(t, time, facts);
}

interface RowContext {
	t: DevicesT;
	time: AreaTime;
	input: AttentionInput;
	onDiagnose: OpenDevice;
}

type RowCells = Pick<PlaneDeviceRow, "state" | "at" | "action">;

function cell(state: string, at?: number, action?: ReactNode) {
	const cells: RowCells = { state, at, action };
	return cells;
}

function presenceCell(ctx: RowContext, device: DeviceFacts) {
	const { t, time } = ctx;
	const { kind, since } = device.presence;
	if (kind !== "offline" || since === undefined)
		return cell(enumLabel(t, "presence", kind), since);
	const at = time.at(since);
	const state = t(
		"devices:chrome.planes.row.offlineSince",
		"Offline since {{time}}",
		{ time: at },
	);
	return cell(state, since);
}

/** Why the encrypted status can't be read, from the keys alone; nothing when they are open. */
function closedKeysText(t: DevicesT, device: DeviceFacts) {
	if (device.keys.state === "none") return enumLabel(t, "vaultState", "absent");
	if (device.keys.state === "stale")
		return t("devices:chrome.planes.row.staleKeys", "Unusable keys");
	return keysLocked(device.keys)
		? enumLabel(t, "vaultState", "locked")
		: undefined;
}

function statusCell(ctx: RowContext, device: DeviceFacts) {
	const { t, input } = ctx;
	if (!device.active)
		return cell(t("devices:chrome.planes.row.notRead", "Not read"));
	const closed = closedKeysText(t, device);
	if (closed) return cell(closed);
	const freshness = statusFreshness(input, device.id);
	if (!freshness) return cell(ageLabel(t, "notloaded"));
	return cell(ageLabel(t, freshness.age), freshness.at);
}

function idleLiveText(t: DevicesT, live: LiveState) {
	const states: Partial<Record<LiveState["kind"], string>> = {
		connecting: t("devices:chrome.planes.row.connecting", "Connecting…"),
		reconnecting: t("devices:chrome.planes.row.reconnecting", "Reconnecting…"),
		unreachable: t(
			"devices:chrome.planes.row.unreachable",
			"Couldn't reach the device",
		),
		failed: t("devices:chrome.planes.row.failed", "Couldn't connect"),
	};
	const known = states[live.kind];
	if (known) return known;
	return t("devices:chrome.planes.row.notConnected", "Not connected");
}

function liveCell(ctx: RowContext, device: DeviceFacts) {
	const { t } = ctx;
	const { live } = device;
	if (live.kind !== "live" && live.kind !== "renewing")
		return cell(idleLiveText(t, live));
	const state =
		live.transport === "websocket"
			? t("devices:chrome.planes.row.liveRelayed", "Live · relayed")
			: t("devices:chrome.planes.row.liveDirect", "Live · direct");
	return cell(state, live.kind === "live" ? live.connectedAt : undefined);
}

function backupText(ctx: RowContext, deviceId: string) {
	const { t, input } = ctx;
	const local = input.local.backups[deviceId];
	if (local?.pending)
		return t("devices:chrome.planes.row.backupPending", "upload pending");
	const backup = input.accountBackups[deviceId];
	if (!backup)
		return t("devices:chrome.planes.row.notBackedUp", "not backed up");
	return t("devices:chrome.planes.row.backedUp", "backed up v{{revision}}", {
		revision: backup.revision,
	});
}

function localCell(ctx: RowContext, device: DeviceFacts) {
	const { t } = ctx;
	if (!device.vault)
		return cell(t("devices:chrome.planes.row.noKeys", "No keys"));
	if (device.keys.state === "stale")
		return cell(t("devices:chrome.planes.row.staleKeys", "Unusable keys"));
	const kind =
		device.vault.role === "shared"
			? t("devices:chrome.planes.row.sharedKeys", "Shared-access keys")
			: t("devices:chrome.planes.row.ownerKeys", "Owner keys");
	const backup = backupText(ctx, device.id);
	return cell(`${kind} · ${backup}`);
}

interface DiagnoseButtonProps {
	deviceId: string;
	onDiagnose: OpenDevice;
}

function DiagnoseButton({
	deviceId,
	onDiagnose,
}: Readonly<DiagnoseButtonProps>) {
	const { t } = useTranslation("devices");
	const diagnose = () => {
		onDiagnose(deviceId);
	};
	return (
		<DvButton size="xs" icon={Stethoscope} onClick={diagnose}>
			{t("chrome.planes.row.diagnose", "Diagnose")}
		</DvButton>
	);
}

function deviceCell(ctx: RowContext, device: DeviceFacts) {
	if (!needsLocalCheck(device)) return undefined;
	const state = enumLabel(ctx.t, "presence", device.presence.kind);
	const action = (
		<DiagnoseButton deviceId={device.id} onDiagnose={ctx.onDiagnose} />
	);
	return cell(state, undefined, action);
}

function certificatesCell(ctx: RowContext, device: DeviceFacts) {
	const { t, input } = ctx;
	if (!device.active)
		return cell(t("devices:chrome.planes.row.notRead", "Not read"));
	const at = reportedAt(input, device.id);
	if (at != null)
		return cell(t("devices:chrome.planes.row.reported", "Reported"), at);
	if (certificatesHidden(input, device)) return cell(ageLabel(t, "noaccess"));
	return cell(t("devices:chrome.planes.row.notReported", "Not reported yet"));
}

type ReadCells = (ctx: RowContext, device: DeviceFacts) => RowCells | undefined;

const ROW_CELLS: Record<PlaneSegmentId, ReadCells> = {
	hub: presenceCell,
	status: statusCell,
	live: liveCell,
	local: localCell,
	device: deviceCell,
	certificates: certificatesCell,
};

/** One plane's state per device, in hub list order. */
export function planeRowsOf(plane: PlaneSegmentId, ctx: RowContext) {
	const rows: PlaneDeviceRow[] = [];
	const read = ROW_CELLS[plane];
	for (const device of fleetFacts(ctx.input).devices) {
		const cells = read(ctx, device);
		if (cells) rows.push({ deviceId: device.id, name: device.name, ...cells });
	}
	return rows;
}

const KEYS_PLACE: DevicesRoute = { screen: "keys" };
const HUB_PLACE: DevicesRoute = { screen: "hub" };
const CERTIFICATES_PLACE: DevicesRoute = {
	screen: "certificates",
	tab: "expiry",
};

type GoTo = (route: DevicesRoute) => void;

interface PlaceButtonProps {
	icon: LucideIcon;
	place: DevicesRoute;
	go: GoTo;
	children: ReactNode;
}

function PlaceButton({
	icon,
	place,
	go,
	children,
}: Readonly<PlaceButtonProps>) {
	const open = () => {
		go(place);
	};
	return (
		<DvButton size="sm" variant="ghost" icon={icon} onClick={open}>
			{children}
		</DvButton>
	);
}

interface HubActionsProps {
	failing: boolean;
	go: GoTo;
}

function HubActions({ failing, go }: Readonly<HubActionsProps>) {
	const { t } = useTranslation("devices");
	const { onRetry } = useHubFreshness();
	const retry =
		failing && onRetry ? (
			<DvButton size="sm" icon={RefreshCw} onClick={onRetry}>
				{t("chrome.planes.retry", "Retry now")}
			</DvButton>
		) : null;
	return (
		<>
			{retry}
			<PlaceButton icon={Server} place={HUB_PLACE} go={go}>
				{t("chrome.planes.hubStatus", "Hub status")}
			</PlaceButton>
		</>
	);
}

interface UnlockSeveralButtonProps {
	onClose?: Close;
}

function UnlockSeveralButton({ onClose }: Readonly<UnlockSeveralButtonProps>) {
	const { t } = useTranslation("devices");
	const unlock = () => {
		if (onClose) onClose();
		useOverlayStore.getState().openUnlockSeveral();
	};
	return (
		<DvButton size="sm" icon={LockOpen} onClick={unlock}>
			{t("chrome.keys.unlockSeveral", "Unlock several…")}
		</DvButton>
	);
}

export interface PlaneActionsProps {
	plane: PlaneSegmentId;
	state: PlaneState;
	/** Devices with keys here that are closed. */
	locked: number;
	onNavigate: ChromeNavigate;
	onClose?: Close;
}

/** The plane's own actions: Retry now, Hub status, Unlock several…, Keys & recovery, Certificates. */
export function PlaneActions(props: Readonly<PlaneActionsProps>) {
	const { t } = useTranslation("devices");
	const { plane, onNavigate, onClose } = props;
	const go = (route: DevicesRoute) => {
		if (onClose) onClose();
		onNavigate(route);
	};
	if (plane === "hub")
		return <HubActions failing={props.state === "err"} go={go} />;
	if (plane === "certificates")
		return (
			<PlaceButton icon={FileBadge} place={CERTIFICATES_PLACE} go={go}>
				{t("chrome.planes.name.certificates", "Certificates")}
			</PlaceButton>
		);
	if (plane !== "status" && plane !== "local") return null;
	const unlock =
		plane === "status" && props.locked > 0 ? (
			<UnlockSeveralButton onClose={onClose} />
		) : null;
	return (
		<>
			{unlock}
			<PlaceButton icon={KeyRound} place={KEYS_PLACE} go={go}>
				{t("chrome.keys.keysAndRecovery", "Keys & recovery")}
			</PlaceButton>
		</>
	);
}

/** A plane's rows over the live workspace. */
export function usePlaneRows(plane: PlaneSegmentId, onClose?: Close) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const onDiagnose = (deviceId: string) => {
		if (onClose) onClose();
		useOverlayStore.getState().openDiagnose(deviceId);
	};
	return planeRowsOf(plane, { t, time, input, onDiagnose });
}

function lineOf(lines: readonly PlaneLine[], plane: PlaneSegmentId) {
	for (const line of lines) if (line.id === plane) return line;
	return undefined;
}

/** SPEC §3.5: what a plane is, how often it refreshes, who can read it, and its state per device. */
export function PlanePopover(props: Readonly<PlanePopoverProps>) {
	const { plane, scope, onNavigate, onClose } = props;
	const lines = usePlaneLines();
	const rows = usePlaneRows(plane, onClose);
	const { input } = useAttentionState();
	const line = lineOf(lines, plane);
	if (!line) return null;
	const locked = fleetFacts(input).devices.filter(hasLockedKeys).length;
	const openDevice = (deviceId: string) => {
		if (onClose) onClose();
		onNavigate({ screen: "device", deviceId, tab: "overview" });
	};
	const actions = (
		<PlaneActions
			plane={plane}
			state={line.state}
			locked={locked}
			onNavigate={onNavigate}
			onClose={onClose}
		/>
	);
	return (
		<PlaneDetail
			segment={line}
			rows={rows}
			scope={scope}
			actions={actions}
			onOpenDevice={openDevice}
		/>
	);
}
