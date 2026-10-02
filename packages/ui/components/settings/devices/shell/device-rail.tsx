"use client";

import { useTranslation } from "@flow-like/locales";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
	ChevronDown,
	Gauge,
	LayoutGrid,
	Lock,
	LockOpen,
	Search,
	X,
} from "lucide-react";
import {
	type MouseEvent,
	type ReactNode,
	type Ref,
	useCallback,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useSearch } from "../../../../hooks/use-search-index";
import { formatMoment, formatRelativeTime } from "../../../../lib/date";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	AttentionItem,
	DeviceViewModel,
	DevicesRoute,
	DevicesScope,
	FleetFilter,
	HealthLevel,
	PendingSetup,
	PresenceKind,
	Relationship,
} from "../../../../lib/device-management/model/types";
import { Input } from "../../../ui/input";
import { Sheet, SheetContent, SheetTitle } from "../../../ui/sheet";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { Kbd } from "../primitives/kbd";
import { FilterChip } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import type { KeyChipState } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { devicesHref } from "../routing/devices-href";
import {
	hubErrorCopy,
	useAttentionCounts,
	useAttentionState,
	useDeviceViews,
	useKeyChip,
	useOverlayStore,
} from "../workspace";
import { useAppNames } from "./attention-popover";
import { keyChipOf } from "./keys-popover";
import {
	type ChromeNavigate,
	type RailCounts,
	type RailKeyState,
	RailRow,
	attentionShort,
	plainClick,
} from "./rail-row";

export type DeviceRailMode = "docked" | "overlay";

export interface DeviceRailProps {
	scope: DevicesScope;
	route: DevicesRoute;
	onNavigate: ChromeNavigate;
	/** `docked`: an in-flow 280 px column. `overlay`: a left sheet over a scrim. */
	mode: DeviceRailMode;
	/** Overlay mode only. */
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	/** The filter field, for the `/` hotkey. */
	filterRef?: Ref<HTMLInputElement>;
	className?: string;
}

/* Model (pure): one row per device, grouped in SPEC §3.4 order. */

export const RAIL_GROUPS = [
	"attention",
	"setup",
	"offline",
	"shared",
	"unknown",
	"healthy",
	"revoked",
] as const;
export type RailGroupId = (typeof RAIL_GROUPS)[number];
export type RailChip = "all" | "mine" | "shared" | "locked";

/** R11: groups show 8 rows, then "Show N more" in steps of 50. */
export const RAIL_GROUP_CAP = 8;
export const RAIL_MORE_STEP = 50;
/** From this many devices on, only the visible rows are rendered. */
export const RAIL_WINDOW_AT = 200;
/** Healthy starts collapsed above this many devices. */
export const RAIL_HEALTHY_OPEN_MAX = 12;

export interface RailDevice {
	id: string;
	name: string;
	presence: PresenceKind;
	/** "Offline since 11:00". */
	presenceLabel?: string;
	relationship: Relationship;
	health: HealthLevel;
	counts: RailCounts;
	keyState: RailKeyState;
	/** Top reason, short state, agent version. */
	sub?: string;
	/** Service ids and app names, for the filter. */
	services?: string;
}

/** An unused setup package, listed under "Setting up". */
export interface RailPackage {
	id: string;
	name: string;
	sub: string;
	expired: boolean;
}

const NO_PACKAGES: readonly RailPackage[] = [];

const notOwned = (device: RailDevice) =>
	device.relationship === "shared" || device.relationship === "unknown";

const CHIP_TEST: Record<RailChip, (device: RailDevice) => boolean> = {
	all: () => true,
	mine: (device) => device.relationship === "owner",
	shared: notOwned,
	locked: (device) => device.keyState === "locked",
};

const FILTER_TEST: Record<FleetFilter, (device: RailDevice) => boolean> = {
	critical: (device) => device.health === "critical",
	attention: (device) => device.health === "attention",
	unknown: (device) => device.health === "unknown",
	healthy: (device) => device.health === "healthy",
	revoked: (device) => device.health === "revoked",
	offline: (device) => device.presence === "offline",
	locked: (device) => device.keyState === "locked",
};

const needsAttention = (device: RailDevice) =>
	device.health === "critical" || device.health === "attention";

/** Each device lands in the first group that fits. */
const GROUP_TEST: Record<RailGroupId, (device: RailDevice) => boolean> = {
	attention: needsAttention,
	setup: (device) => device.presence === "never",
	offline: (device) => device.presence === "offline",
	shared: (device) => notOwned(device) && device.health !== "revoked",
	unknown: (device) => device.health === "unknown",
	healthy: (device) => device.health === "healthy",
	revoked: () => true,
};

const openItems = (device: RailDevice) =>
	(device.counts.critical ?? 0) +
	(device.counts.warning ?? 0) +
	(device.counts.notice ?? 0);

const byName = (a: RailDevice, b: RailDevice) => a.name.localeCompare(b.name);

function byUrgency(a: RailDevice, b: RailDevice) {
	const rank = (device: RailDevice) => (device.health === "critical" ? 0 : 1);
	return rank(a) - rank(b) || openItems(b) - openItems(a) || byName(a, b);
}

export type RailGroups = Record<RailGroupId, RailDevice[]>;

export function matchesRail(
	device: RailDevice,
	chip: RailChip,
	filter?: FleetFilter,
) {
	return CHIP_TEST[chip](device) && (!filter || FILTER_TEST[filter](device));
}

function emptyGroups() {
	const groups = {} as RailGroups;
	for (const id of RAIL_GROUPS) groups[id] = [];
	return groups;
}

function groupOf(device: RailDevice) {
	for (const id of RAIL_GROUPS) if (GROUP_TEST[id](device)) return id;
	return "revoked";
}

export function groupRailDevices(devices: readonly RailDevice[]) {
	const groups = emptyGroups();
	for (const device of devices) groups[groupOf(device)].push(device);
	for (const id of RAIL_GROUPS)
		groups[id].sort(id === "attention" ? byUrgency : byName);
	return groups;
}

export type RailItem =
	| { kind: "head"; group: RailGroupId; count: number; open: boolean }
	| { kind: "device"; device: RailDevice }
	| { kind: "package"; pkg: RailPackage }
	| { kind: "note"; group: RailGroupId }
	| { kind: "more"; group: RailGroupId; hidden: number };

export interface RailLayout {
	/** The viewer's explicit choice per group; absent = the default. */
	open: Partial<Record<RailGroupId, boolean>>;
	caps: Partial<Record<RailGroupId, number>>;
	packages: readonly RailPackage[];
	currentId?: string;
}

const startsOpen = (id: RailGroupId, count: number) =>
	id === "revoked" ? false : id !== "healthy" || count <= RAIL_HEALTHY_OPEN_MAX;

/** An empty group keeps its head and says why; Status unknown simply isn't there. */
function emptyGroupItems(id: RailGroupId) {
	const items: RailItem[] = [];
	if (id === "unknown") return items;
	items.push({ kind: "head", group: id, count: 0, open: true });
	items.push({ kind: "note", group: id });
	return items;
}

function isGroupOpen(
	id: RailGroupId,
	devices: readonly RailDevice[],
	layout: RailLayout,
) {
	for (const device of devices) if (device.id === layout.currentId) return true;
	const count = devices.length + packagesOf(id, layout).length;
	return layout.open[id] ?? startsOpen(id, count);
}

const packagesOf = (id: RailGroupId, layout: RailLayout) =>
	id === "setup" ? layout.packages : NO_PACKAGES;

/** The rows of an open group: devices up to the cap, setup packages, then "Show N more". */
function pushRows(
	items: RailItem[],
	id: RailGroupId,
	devices: readonly RailDevice[],
	layout: RailLayout,
) {
	const cap = layout.caps[id] ?? RAIL_GROUP_CAP;
	for (const device of devices.slice(0, cap))
		items.push({ kind: "device", device });
	for (const pkg of packagesOf(id, layout))
		items.push({ kind: "package", pkg });
	const hidden = Math.min(RAIL_MORE_STEP, devices.length - cap);
	if (hidden > 0) items.push({ kind: "more", group: id, hidden });
}

function groupItems(
	id: RailGroupId,
	devices: readonly RailDevice[],
	layout: RailLayout,
) {
	const count = devices.length + packagesOf(id, layout).length;
	if (count === 0) return emptyGroupItems(id);
	const open = isGroupOpen(id, devices, layout);
	const items: RailItem[] = [{ kind: "head", group: id, count, open }];
	if (open) pushRows(items, id, devices, layout);
	return items;
}

/** The rail body as one flat list, so long fleets can be windowed. */
export function railItems(groups: RailGroups, layout: RailLayout) {
	const items: RailItem[] = [];
	for (const id of RAIL_GROUPS)
		items.push(...groupItems(id, groups[id], layout));
	return items;
}

const itemKey = (item: RailItem) => {
	if (item.kind === "device") return `device:${item.device.id}`;
	if (item.kind === "package") return `package:${item.pkg.id}`;
	return `${item.kind}:${item.group}`;
};

/* View. */

const GROUPS_KEY = "flow-like.devices.rail-groups";

function readOpenGroups(): RailLayout["open"] {
	try {
		const raw = globalThis.localStorage?.getItem(GROUPS_KEY);
		const parsed: unknown = raw ? JSON.parse(raw) : null;
		return parsed && typeof parsed === "object"
			? (parsed as RailLayout["open"])
			: {};
	} catch {
		return {};
	}
}

function saveOpenGroups(open: RailLayout["open"]) {
	try {
		globalThis.localStorage?.setItem(GROUPS_KEY, JSON.stringify(open));
	} catch {
		// The choice only lasts for this visit.
	}
}

function groupLabel(t: DevicesT, id: RailGroupId, legacy: boolean) {
	const labels = {
		attention: t("devices:chrome.rail.group.attention", "Needs attention"),
		setup: t("devices:chrome.rail.group.setup", "Setting up"),
		offline: enumLabel(t, "presence", "offline"),
		shared: legacy
			? enumLabel(t, "relationship", "unknown")
			: t("devices:chrome.rail.group.shared", "Shared with me"),
		unknown: enumLabel(t, "health", "unknown"),
		healthy: enumLabel(t, "health", "healthy"),
		revoked: enumLabel(t, "health", "revoked"),
	} satisfies Record<RailGroupId, string>;
	return labels[id];
}

function offlineNote(
	t: DevicesT,
	elsewhere: readonly string[],
	locale: string,
) {
	if (!elsewhere.length)
		return t(
			"devices:chrome.rail.note.offline",
			"Every device checked in within the last 10 minutes.",
		);
	return t("devices:chrome.rail.note.offlineElsewhere", {
		count: elsewhere.length,
		names: new Intl.ListFormat(locale, { type: "conjunction" }).format(
			elsewhere.slice(0, 3),
		),
		defaultValue_one:
			"{{names}} is offline too. It's listed under Needs attention because of open items.",
		defaultValue_other:
			"{{names}} are offline too. They're listed under Needs attention because of open items.",
	});
}

function groupNote(t: DevicesT, id: RailGroupId, legacy: boolean) {
	const notes = {
		attention: t(
			"devices:chrome.rail.note.attention",
			"Nothing needs attention right now.",
		),
		setup: t("devices:chrome.rail.note.setup", "No setups are waiting."),
		offline: "",
		shared: legacy
			? t(
					"devices:chrome.rail.note.sharedLegacy",
					"No device here belongs to someone else.",
				)
			: t(
					"devices:chrome.rail.note.shared",
					"Nobody shares a device with you right now.",
				),
		unknown: "",
		healthy: t(
			"devices:chrome.rail.note.healthy",
			"No device of yours is free of attention items right now.",
		),
		revoked: t("devices:chrome.rail.note.revoked", "No revoked devices."),
	} satisfies Record<RailGroupId, string>;
	return notes[id];
}

function filterLabel(t: DevicesT, filter: FleetFilter) {
	const labels = {
		critical: enumLabel(t, "health", "critical"),
		attention: enumLabel(t, "health", "attention"),
		unknown: enumLabel(t, "health", "unknown"),
		healthy: enumLabel(t, "health", "healthy"),
		revoked: enumLabel(t, "health", "revoked"),
		offline: enumLabel(t, "presence", "offline"),
		locked: enumLabel(t, "vaultState", "locked"),
	} satisfies Record<FleetFilter, string>;
	return labels[filter];
}

interface RailRender {
	scope: DevicesScope;
	currentId?: string;
	legacy: boolean;
	offlineElsewhere: readonly string[];
	go: (route: DevicesRoute, event: MouseEvent<HTMLAnchorElement>) => void;
	toggle: (group: RailGroupId, open: boolean) => void;
	more: (group: RailGroupId) => void;
}

interface ItemProps<K extends RailItem["kind"]> {
	item: Extract<RailItem, { kind: K }>;
	rail: RailRender;
}

function HeadItem({ item, rail }: Readonly<ItemProps<"head">>) {
	const { t } = useTranslation("devices");
	return (
		<button
			type="button"
			aria-expanded={item.open}
			data-rail-group={item.group}
			onClick={() => rail.toggle(item.group, !item.open)}
			className="mt-1.5 flex h-7.5 w-full items-center gap-1.5 rounded-lg px-1.5 text-left text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
		>
			<ChevronDown
				aria-hidden
				className={cx(
					"size-3 transition-transform",
					!item.open && "-rotate-90",
				)}
			/>
			<span className="min-w-0 truncate">
				{groupLabel(t, item.group, rail.legacy)}
			</span>
			<span className="ml-auto font-mono text-xs font-medium tracking-normal tabular-nums">
				{item.count}
			</span>
		</button>
	);
}

function DeviceItem({ item, rail }: Readonly<ItemProps<"device">>) {
	const { device } = item;
	const route: DevicesRoute = {
		screen: "device",
		deviceId: device.id,
		tab: "overview",
	};
	return (
		<RailRow
			name={device.name}
			href={devicesHref(route, rail.scope)}
			onSelect={(event) => rail.go(route, event)}
			presence={device.presence}
			presenceLabel={device.presenceLabel}
			counts={device.counts}
			keyState={device.keyState}
			sub={device.sub}
			current={device.id === rail.currentId}
		/>
	);
}

function PackageItem({ item, rail }: Readonly<ItemProps<"package">>) {
	const { t } = useTranslation("devices");
	const { pkg } = item;
	const route: DevicesRoute = { screen: "setup", enrollmentId: pkg.id };
	return (
		<RailRow
			name={pkg.name}
			tag={t("chrome.rail.packageTag", "package")}
			href={devicesHref(route, rail.scope)}
			onSelect={(event) => rail.go(route, event)}
			presence="pending"
			counts={pkg.expired ? { notice: 1 } : undefined}
			sub={pkg.sub}
		/>
	);
}

function NoteItem({ item, rail }: Readonly<ItemProps<"note">>) {
	const { t, i18n } = useTranslation("devices");
	const text =
		item.group === "offline"
			? offlineNote(t, rail.offlineElsewhere, i18n?.language ?? "en")
			: groupNote(t, item.group, rail.legacy);
	return (
		<p
			data-rail-note={item.group}
			className="pt-0.5 pr-2 pb-1.5 pl-6 text-xs text-muted-foreground"
		>
			{text}
		</p>
	);
}

function MoreItem({ item, rail }: Readonly<ItemProps<"more">>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			variant="link"
			size="xs"
			className="mt-0.5 mb-1 ml-6.5 text-ink-2"
			onClick={() => rail.more(item.group)}
		>
			{t("chrome.rail.more", "Show {{count, number}} more", {
				count: item.hidden,
			})}
		</DvButton>
	);
}

interface RailItemViewProps {
	item: RailItem;
	rail: RailRender;
}

function RailItemView({ item, rail }: Readonly<RailItemViewProps>) {
	switch (item.kind) {
		case "head":
			return <HeadItem item={item} rail={rail} />;
		case "device":
			return <DeviceItem item={item} rail={rail} />;
		case "package":
			return <PackageItem item={item} rail={rail} />;
		case "note":
			return <NoteItem item={item} rail={rail} />;
		default:
			return <MoreItem item={item} rail={rail} />;
	}
}

const ITEM_HEIGHT: Record<RailItem["kind"], number> = {
	head: 36,
	device: 45,
	package: 45,
	note: 40,
	more: 30,
};

const BODY =
	"min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-2 pt-1 pb-3";

interface RailBodyProps {
	items: readonly RailItem[];
	rail: RailRender;
}

function WindowedBody({ items, rail }: Readonly<RailBodyProps>) {
	const scrollRef = useRef<HTMLDivElement>(null);
	const virtualizer = useVirtualizer({
		count: items.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: (index) => ITEM_HEIGHT[items[index]?.kind ?? "device"],
		getItemKey: (index) => {
			const item = items[index];
			return item ? itemKey(item) : index;
		},
		overscan: 8,
	});
	return (
		<div ref={scrollRef} data-rail-body="" data-windowed="" className={BODY}>
			<div
				className="relative w-full"
				style={{ height: virtualizer.getTotalSize() }}
			>
				{virtualizer.getVirtualItems().map((row) => {
					const item = items[row.index];
					if (!item) return null;
					return (
						<div
							key={row.key}
							ref={virtualizer.measureElement}
							data-index={row.index}
							className="absolute inset-x-0 pb-px"
							style={{ top: row.start }}
						>
							<RailItemView item={item} rail={rail} />
						</div>
					);
				})}
			</div>
		</div>
	);
}

function PlainBody({ items, rail }: Readonly<RailBodyProps>) {
	return (
		<div data-rail-body="" className={cx(BODY, "flex flex-col gap-px")}>
			{items.map((item) => {
				return <RailItemView key={itemKey(item)} item={item} rail={rail} />;
			})}
		</div>
	);
}

const CHIPS: readonly RailChip[] = ["all", "mine", "shared", "locked"];

function chipLabel(t: DevicesT, chip: RailChip, legacy: boolean) {
	const labels = {
		all: t("devices:chrome.rail.chip.all", "All"),
		mine: t("devices:chrome.rail.chip.mine", "Mine"),
		shared: legacy
			? enumLabel(t, "relationship", "unknown")
			: t("devices:chrome.rail.chip.shared", "Shared"),
		locked: enumLabel(t, "vaultState", "locked"),
	} satisfies Record<RailChip, string>;
	return labels[chip];
}

interface RailHeadProps {
	query: string;
	onQuery: (query: string) => void;
	chip: RailChip;
	onChip: (chip: RailChip) => void;
	legacy: boolean;
	meta: string;
	filter?: FleetFilter;
	onClearFilter: () => void;
	filterRef?: Ref<HTMLInputElement>;
	onClose?: () => void;
}

function RailHead(props: Readonly<RailHeadProps>) {
	const { t } = useTranslation("devices");
	const { filter } = props;
	return (
		<div className="flex flex-col gap-2 border-b border-hairline px-3 pt-3 pb-2">
			<div className="flex items-center gap-1.5">
				<div className="flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-lg border border-input bg-card pr-1.5 pl-2 text-muted-foreground focus-within:outline-2 focus-within:outline-ring">
					<Search aria-hidden className="size-4 shrink-0" />
					<Input
						ref={props.filterRef}
						type="search"
						data-rail-filter=""
						value={props.query}
						onChange={(event) => props.onQuery(event.target.value)}
						aria-label={t("chrome.rail.filterLabel", "Filter devices")}
						placeholder={t(
							"chrome.rail.filterPlaceholder",
							"Filter by name, ID, service, app",
						)}
						className="h-auto min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 text-[13px]/[18px] text-foreground shadow-none focus-visible:ring-0 md:text-[13px]/[18px] dark:bg-transparent"
					/>
					<Kbd aria-hidden>/</Kbd>
				</div>
				{props.onClose ? (
					<DvButton
						variant="ghost"
						iconOnly
						icon={X}
						aria-label={t("chrome.rail.close", "Hide devices")}
						onClick={props.onClose}
					/>
				) : null}
			</div>
			<fieldset
				aria-label={t("chrome.rail.show", "Show")}
				className="m-0 flex min-w-0 flex-wrap gap-1.5 border-0 p-0"
			>
				{CHIPS.map((chip) => {
					return (
						<FilterChip
							key={chip}
							pressed={props.chip === chip}
							onPressedChange={() => props.onChip(chip)}
						>
							{chipLabel(t, chip, props.legacy)}
						</FilterChip>
					);
				})}
			</fieldset>
			<div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
				<span data-rail-meta="">{props.meta}</span>
				{filter ? (
					<FilterChip
						pressed
						icon={X}
						onPressedChange={props.onClearFilter}
						className="h-5.5"
					>
						<span className="sr-only">
							{t("chrome.rail.clearFilter", "Clear filter")}{" "}
						</span>
						{filterLabel(t, filter)}
					</FilterChip>
				) : null}
			</div>
		</div>
	);
}

interface RailHomeProps {
	scope: DevicesScope;
	route: DevicesRoute;
	count?: number;
	go: RailRender["go"];
}

const FLEET_HOME: DevicesRoute = { screen: "fleet", view: "devices" };
const APP_HOME: DevicesRoute = { screen: "app-devices", by: "device" };
const HOME_ROW =
	"flex h-9 items-center gap-2 rounded-lg px-2 text-ui font-semibold text-foreground no-underline hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring";

interface HomeCountProps {
	count: number;
}

function HomeCount({ count }: Readonly<HomeCountProps>) {
	const { t } = useTranslation("devices");
	const title = t(
		"chrome.rail.homeCount",
		"{{count, number}} open attention items",
		{ count },
	);
	return (
		<span
			title={title}
			className="font-mono text-xs font-medium text-muted-foreground tabular-nums"
		>
			{count}
		</span>
	);
}

function RailHome({ scope, route, count, go }: Readonly<RailHomeProps>) {
	const { t } = useTranslation("devices");
	const app = scope.kind === "app";
	const home = app ? APP_HOME : FLEET_HOME;
	const Icon = app ? LayoutGrid : Gauge;
	const current = route.screen === home.screen;
	const label = app
		? t("chrome.rail.homeApp", "This app on devices")
		: t("chrome.rail.home", "Fleet overview");
	const open = (event: MouseEvent<HTMLAnchorElement>) => {
		go(home, event);
	};
	return (
		<div className="px-2 pt-2">
			<a
				href={devicesHref(home, scope)}
				onClick={open}
				aria-current={current ? "page" : undefined}
				data-rail-home=""
				className={cx(HOME_ROW, current && "bg-row-selected")}
			>
				<Icon aria-hidden className="size-3.5 text-muted-foreground" />
				<span className="min-w-0 flex-1 truncate">{label}</span>
				{count === undefined ? null : <HomeCount count={count} />}
			</a>
		</div>
	);
}

interface RailFootProps {
	onUnlockSeveral?: () => void;
	onLockAll?: () => void;
	/** Devices whose keys are open: none leaves nothing to lock. */
	unlockedCount?: number;
}

function RailFoot(props: Readonly<RailFootProps>) {
	const { t } = useTranslation("devices");
	const nothingOpen = !props.unlockedCount;
	return (
		<div className="flex flex-wrap gap-2 border-t border-hairline px-3 py-2">
			<DvButton size="sm" icon={LockOpen} onClick={props.onUnlockSeveral}>
				{t("chrome.keys.unlockSeveral", "Unlock several…")}
			</DvButton>
			<DvButton
				size="sm"
				variant="ghost"
				icon={Lock}
				aria-disabled={nothingOpen || undefined}
				onClick={props.onLockAll}
			>
				{t("chrome.keys.lockAll", "Lock all")}
			</DvButton>
		</div>
	);
}

export interface DeviceRailViewProps extends DeviceRailProps {
	devices: readonly RailDevice[];
	packages?: readonly RailPackage[];
	/** Counted open attention items, shown on the home row. */
	attentionTotal?: number;
	/** Replaces the groups while the device list has never loaded (R6). */
	unavailable?: ReactNode;
	unlockedCount?: number;
	onUnlockSeveral?: () => void;
	onLockAll?: () => void;
}

function moreRows(caps: RailLayout["caps"], group: RailGroupId) {
	const shown = caps[group] ?? RAIL_GROUP_CAP;
	return { ...caps, [group]: shown + RAIL_MORE_STEP };
}

function useRailLayout() {
	const [open, setOpen] = useState(readOpenGroups);
	const [caps, setCaps] = useState<RailLayout["caps"]>({});
	const toggle = useCallback((group: RailGroupId, next: boolean) => {
		setOpen((current) => {
			const updated = { ...current, [group]: next };
			saveOpenGroups(updated);
			return updated;
		});
	}, []);
	const more = useCallback((group: RailGroupId) => {
		setCaps((current) => {
			return moreRows(current, group);
		});
	}, []);
	return { open, caps, toggle, more };
}

const isLocked = (device: RailDevice) => device.keyState === "locked";
const isOffline = (device: RailDevice) => device.presence === "offline";
const isLegacy = (device: RailDevice) => device.relationship === "unknown";
const nameOf = (device: RailDevice) => device.name;

function metaText(
	t: DevicesT,
	devices: readonly RailDevice[],
	packages: number,
) {
	const all = t("devices:chrome.rail.metaDevices", {
		count: devices.length,
		defaultValue_one: "{{count, number}} device",
		defaultValue_other: "{{count, number}} devices",
	});
	const locked = t(
		"devices:chrome.rail.metaLocked",
		"{{count, number}} locked",
		{ count: devices.filter(isLocked).length },
	);
	const waiting = t("devices:chrome.rail.metaPackages", {
		count: packages,
		defaultValue_one: "{{count, number}} setup package",
		defaultValue_other: "{{count, number}} setup packages",
	});
	return `${all} · ${locked} · ${waiting}`;
}

function currentDeviceId(route: DevicesRoute) {
	if (route.screen === "device") return route.deviceId;
	return route.screen === "service" ? route.deviceId : undefined;
}

interface RailFilters {
	query: string;
	chip: RailChip;
	filter?: FleetFilter;
}

/** The rows left after the filter field, the chips and the annunciator filter. */
function useRailGroups(devices: readonly RailDevice[], filters: RailFilters) {
	const { query, chip, filter } = filters;
	const found = useSearch(devices, query, {
		fields: ["name", "id", "services"],
	});
	return useMemo(() => {
		const shown: RailDevice[] = [];
		for (const device of found)
			if (matchesRail(device, chip, filter)) shown.push(device);
		return groupRailDevices(shown);
	}, [found, chip, filter]);
}

function useRailFilters(route: DevicesRoute) {
	const [query, setQuery] = useState("");
	const [chip, setChip] = useState<RailChip>("all");
	const fleet = route.screen === "fleet" ? route : undefined;
	const filter = fleet ? fleet.filter : undefined;
	const search = query.trim();
	const narrowed = search !== "" || chip !== "all" || filter !== undefined;
	return { query, setQuery, chip, setChip, fleet, filter, search, narrowed };
}

function useRailNavigation(props: Readonly<DeviceRailViewProps>) {
	const { onNavigate, onOpenChange } = props;
	const close = useCallback(() => {
		if (onOpenChange) onOpenChange(false);
	}, [onOpenChange]);
	const go = useCallback<RailRender["go"]>(
		(target, event) => {
			if (!plainClick(event)) return;
			event.preventDefault();
			onNavigate(target);
			close();
		},
		[onNavigate, close],
	);
	return { go, close };
}

interface RailBodySlotProps extends RailBodyProps {
	windowed: boolean;
	unavailable?: ReactNode;
}

function RailBodySlot(props: Readonly<RailBodySlotProps>) {
	if (props.unavailable) return <div className={BODY}>{props.unavailable}</div>;
	const Body = props.windowed ? WindowedBody : PlainBody;
	return <Body items={props.items} rail={props.rail} />;
}

function RailPanel(props: Readonly<DeviceRailViewProps>) {
	const { t } = useTranslation("devices");
	const { devices, route, scope, onNavigate } = props;
	const filters = useRailFilters(route);
	const layout = useRailLayout();
	const { go, close } = useRailNavigation(props);
	const { chip, filter, fleet, search } = filters;
	const groups = useRailGroups(devices, { query: search, chip, filter });
	const known = props.packages ?? NO_PACKAGES;
	const packages = filters.narrowed ? NO_PACKAGES : known;
	const currentId = currentDeviceId(route);
	const { open, caps, toggle, more } = layout;
	const items = useMemo(() => {
		return railItems(groups, { open, caps, packages, currentId });
	}, [groups, open, caps, packages, currentId]);
	const legacy = devices.some(isLegacy);
	const offlineElsewhere = groups.attention.filter(isOffline).map(nameOf);
	const rail = { scope, currentId, legacy, offlineElsewhere, go, toggle, more };
	const meta = metaText(t, devices, known.length);
	const clearFilter = () => {
		if (fleet) onNavigate({ ...fleet, filter: undefined }, { replace: true });
	};
	return (
		<div
			data-chrome="rail"
			className="flex h-full min-h-0 w-full flex-col text-ui"
		>
			<RailHead
				query={filters.query}
				onQuery={filters.setQuery}
				chip={chip}
				onChip={filters.setChip}
				legacy={legacy}
				meta={meta}
				filter={filter}
				onClearFilter={clearFilter}
				filterRef={props.filterRef}
				onClose={props.mode === "overlay" ? close : undefined}
			/>
			<RailHome
				scope={scope}
				route={route}
				count={props.attentionTotal}
				go={go}
			/>
			<RailBodySlot
				items={items}
				rail={rail}
				windowed={devices.length >= RAIL_WINDOW_AT}
				unavailable={props.unavailable}
			/>
			<RailFoot
				onUnlockSeveral={props.onUnlockSeveral}
				onLockAll={props.onLockAll}
				unlockedCount={props.unlockedCount}
			/>
		</div>
	);
}

const RAIL_DOCKED =
	"flex w-70 shrink-0 flex-col border-r border-hairline bg-sidebar";
const RAIL_OVERLAY =
	"w-[min(320px,100vw)] gap-0 border-border-strong bg-sidebar p-0 shadow-none backdrop-blur-none max-[720px]:w-full max-[720px]:max-w-full sm:max-w-[320px] [&>button]:hidden [&>div.pointer-events-none]:hidden";

/** The sheet takes the focus itself: focusing the filter would raise a phone's keyboard. */
function focusSheet(event: Event) {
	event.preventDefault();
	const sheet = event.currentTarget;
	if (sheet instanceof HTMLElement) sheet.focus();
}

/** The rail over plain data: docked column or left overlay. */
export function DeviceRailView(props: Readonly<DeviceRailViewProps>) {
	const { t } = useTranslation("devices");
	const label = t("chrome.rail.label", "Devices");
	if (props.mode === "docked") {
		const docked = cx(RAIL_DOCKED, props.className);
		return (
			<aside aria-label={label} className={docked}>
				<RailPanel {...props} />
			</aside>
		);
	}
	const overlay = cx(RAIL_OVERLAY, props.className);
	return (
		<Sheet open={props.open === true} onOpenChange={props.onOpenChange}>
			<SheetContent
				side="left"
				overlayClassName="bg-scrim backdrop-blur-none"
				aria-describedby={undefined}
				onOpenAutoFocus={focusSheet}
				className={overlay}
			>
				<SheetTitle className="sr-only">{label}</SheetTitle>
				<RailPanel {...props} />
			</SheetContent>
		</Sheet>
	);
}

/* Binding: device views → rail rows. */

interface RailCopy {
	t: DevicesT;
	/** Hub-corrected epoch milliseconds of the last recompute: rows don't tick every second. */
	now: number;
	locale: string;
	appName: (appId: string) => string | undefined;
}

const KEY_GLYPH: Partial<Record<KeyChipState, RailKeyState>> = {
	live: "live",
	unlocked: "unlocked",
	reconnecting: "unlocked",
	stale: "stale",
	none: "none",
};

function countsOf(items: readonly AttentionItem[]): RailCounts {
	const counts = { critical: 0, warning: 0, notice: 0 };
	for (const item of items)
		if (item.severity !== "info") counts[item.severity]++;
	return counts;
}

function stateWord({ t }: RailCopy, view: DeviceViewModel) {
	const words: Partial<Record<DeviceViewModel["presence"]["kind"], string>> = {
		offline: t("devices:chrome.rail.state.offline", "offline"),
		never: t("devices:chrome.rail.state.never", "no check-in"),
		late: t("devices:chrome.rail.state.late", "late"),
		revoked: t("devices:chrome.rail.state.revoked", "revoked"),
	};
	const word = words[view.presence.kind];
	if (word) return word;
	if (view.health !== "unknown")
		return t("devices:chrome.rail.state.online", "online");
	return view.keys.state === "none"
		? t("devices:chrome.rail.state.noKeys", "no keys here")
		: t("devices:chrome.rail.state.locked", "locked");
}

function reasonOf(copy: RailCopy, view: DeviceViewModel) {
	const top = view.attention.find((item) => item.severity !== "info");
	if (top && view.health !== "unknown") return attentionShort(copy.t, top.key);
	const ends = view.row.access_expires_at;
	if (view.relationship !== "shared" || ends == null) return "";
	return copy.t("devices:chrome.rail.accessEnds", "access ends {{when}}", {
		when: formatRelativeTime(ends * 1000, "narrow", "", {
			now: copy.now,
			locale: copy.locale,
		}),
	});
}

function searchText(copy: RailCopy, view: DeviceViewModel) {
	if (!Array.isArray(view.services)) return "";
	return view.services
		.flatMap((service) => [
			service.serviceId,
			copy.appName(service.projectId) ?? service.projectId,
		])
		.join(" ");
}

function presenceLabelOf(copy: RailCopy, view: DeviceViewModel) {
	const { kind, since } = view.presence;
	if (kind !== "offline" || since === undefined) return undefined;
	return copy.t("devices:chrome.rail.offlineSince", "Offline since {{time}}", {
		time: formatMoment(since * 1000, { now: copy.now, locale: copy.locale }),
	});
}

/** One device as a rail row: reason first, then a short state, then the agent version. */
export function railDeviceOf(
	copy: RailCopy,
	view: DeviceViewModel,
): RailDevice {
	const chip = keyChipOf(view.keys, view.live).state;
	const sub = [reasonOf(copy, view), stateWord(copy, view), view.agent?.version]
		.filter(Boolean)
		.join(" · ");
	return {
		id: view.row.device_id,
		name: deviceName(view.row),
		presence: view.presence.kind,
		presenceLabel: presenceLabelOf(copy, view),
		relationship: view.relationship,
		health: view.health,
		counts: countsOf(view.attention),
		keyState: KEY_GLYPH[chip] ?? "locked",
		sub,
		services: searchText(copy, view),
	};
}

const OPEN_SETUPS = new Set<PendingSetup["state"]>(["pending", "expired"]);

/** Setup packages nobody has used yet (a registered device has its own row). */
export function railPackagesOf(
	copy: RailCopy,
	setups: readonly PendingSetup[],
	deviceIds: ReadonlySet<string>,
): RailPackage[] {
	const { t, now, locale } = copy;
	return setups
		.filter(
			(setup) =>
				OPEN_SETUPS.has(setup.state) &&
				!(setup.deviceId && deviceIds.has(setup.deviceId)),
		)
		.map((setup) => {
			const expired =
				setup.state === "expired" || setup.expiresAt * 1000 <= now;
			const at = setup.expiresAt * 1000;
			return {
				id: setup.enrollmentId,
				name: setup.name || setup.enrollmentId.slice(0, 8),
				expired,
				sub: expired
					? t(
							"devices:chrome.rail.packageExpired",
							"Expired {{date}} · unusable",
							{ date: formatMoment(at, { now, locale }) },
						)
					: t(
							"devices:chrome.rail.packageWaiting",
							"Waiting · expires {{when}}",
							{ when: formatRelativeTime(at, "narrow", "", { now, locale }) },
						),
			};
		});
}

function useRailRows() {
	const { t, i18n } = useTranslation("devices");
	const { input } = useAttentionState();
	const views = useDeviceViews();
	const appName = useAppNames();
	const locale = i18n?.language ?? "en";
	return useMemo(() => {
		const copy: RailCopy = { t, now: input.now * 1000, locale, appName };
		const ids = new Set(input.devices.map((row) => row.device_id));
		return {
			devices: views.map((view) => railDeviceOf(copy, view)),
			packages: railPackagesOf(copy, input.pendingSetups ?? [], ids),
		};
	}, [t, locale, appName, input, views]);
}

function useRailUnavailable() {
	const { t } = useTranslation("devices");
	const { rows } = useAttentionState();
	if (rows.data !== undefined) return undefined;
	if (!rows.error)
		return (
			<StateView
				kind="loading"
				title={t("chrome.rail.loading", "Reading the device list…")}
			/>
		);
	return (
		<StateView
			kind="error"
			title={t("chrome.rail.error", "Couldn't read the device list")}
			text={hubErrorCopy(t, toHubError(rows.error).code)}
		/>
	);
}

/** SPEC §3.4: the device rail, docked next to the page or as a left overlay. */
export function DeviceRail(props: Readonly<DeviceRailProps>) {
	const { t } = useTranslation("devices");
	const { scope } = props;
	const { devices, packages } = useRailRows();
	const unavailable = useRailUnavailable();
	const counts = useAttentionCounts(
		scope.kind === "app" ? { appId: scope.appId } : {},
	);
	const chip = useKeyChip();
	return (
		<DeviceRailView
			{...props}
			devices={devices}
			packages={packages}
			attentionTotal={counts.total}
			unavailable={unavailable}
			unlockedCount={chip.unlockedCount}
			onUnlockSeveral={() => {
				props.onOpenChange?.(false);
				useOverlayStore.getState().openUnlockSeveral();
			}}
			onLockAll={() => {
				chip.lockAll();
				toast(
					t(
						"chrome.keys.lockedAllToast",
						"All devices are locked on this computer.",
					),
				);
			}}
		/>
	);
}
