"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	ChevronRight,
	Funnel,
	Search,
	TriangleAlert,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	fleetFacts,
	keysLocked,
	subjectDevice,
} from "../../../../lib/device-management/model/device-view";
import type {
	AttentionInput,
	AttentionItem,
	DeviceViewModel,
	FleetFilter,
	FleetRoute,
	HealthLevel,
} from "../../../../lib/device-management/model/types";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "../primitives/area-context";
import type { AttentionEntry } from "../primitives/attention-list";
import { DvButton } from "../primitives/dv-button";
import { DvTable, GroupRow, SortHeader, Th } from "../primitives/dv-table";
import { DvInput } from "../primitives/form-fields";
import { FilterChip } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import { healthLabel } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useAppNames, useAttentionEntries } from "../shell/attention-popover";
import {
	useAttentionState,
	useDeviceViews,
	useFleetCertificateInventory,
	useKeyChip,
	useResourceSummary,
} from "../workspace";
import {
	FLEET_DEVICE_COLS,
	type FleetDeviceEntry,
	FleetDeviceRow,
	type FleetRowAction,
	ROW_ACTION_KEYS,
	fleetColumnLabels,
	platformLabel,
	shareOfGrants,
} from "./fleet-device-row";

/** R11: the N1 tables show 50 rows per page. */
export const FLEET_PAGE_SIZE = 50;

/**
 * The app's base layer gives every `table` a margin and every cell a full
 * border; inside a block only the row hairlines of the table primitive remain.
 */
export const FLEET_TABLE_CLASS =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0";

/** How long typing rests before the search lands in the URL. */
const SEARCH_URL_DELAY_MS = 300;

const HEALTH_RANK: Record<HealthLevel, number> = {
	critical: 0,
	attention: 1,
	unknown: 2,
	healthy: 3,
	revoked: 4,
};

export type FleetSort = "severity" | "name" | "checkin" | "registered";
export type FleetGroup = "none" | "health" | "relationship";

/* Toolbar pieces shared with the Services view. */

export interface ToolbarOption<T extends string> {
	value: T;
	label: string;
}

/** A compact select of the block toolbar (group, sort, platform, app). */
export function ToolbarSelect<T extends string>({
	label,
	value,
	options,
	onChange,
	className,
}: Readonly<{
	label: string;
	value: T;
	options: readonly ToolbarOption<T>[];
	onChange(value: T): void;
	className?: string;
}>) {
	return (
		<Select value={value} onValueChange={(next) => onChange(next as T)}>
			<SelectTrigger
				size="sm"
				aria-label={label}
				className={cx(
					"h-7 max-w-47.5 min-w-0 rounded-lg border-border bg-card text-ui shadow-none",
					className,
				)}
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

/** The search box of the Devices / Services block. */
export function FleetSearch({
	value,
	onChange,
	placeholder,
}: Readonly<{
	value: string;
	onChange(value: string): void;
	placeholder: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div className="relative max-w-80 min-w-0 flex-[1_1_250px] @max-[900px]/devices:max-w-none @max-[900px]/devices:basis-full">
			<Search
				aria-hidden
				className="pointer-events-none absolute top-2 left-2.5 size-3.5 text-muted-foreground"
			/>
			<DvInput
				type="search"
				aria-label={t("fleet.search.label", "Search")}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				placeholder={placeholder}
				autoComplete="off"
				data-fleet-search=""
				className="h-7.5 pl-8"
			/>
		</div>
	);
}

/** Search text that filters at once and reaches the URL (`q=`) once typing rests. */
export function useFleetSearch(route: FleetRoute) {
	const { navigate } = useDevicesRoute();
	const [query, setQuery] = useState(route.q ?? "");
	const latest = useRef(route);
	latest.current = route;
	const routeQuery = route.q ?? "";
	const typed = useRef(routeQuery);
	useEffect(() => {
		if (routeQuery === typed.current) return;
		typed.current = routeQuery;
		setQuery(routeQuery);
	}, [routeQuery]);
	useEffect(() => {
		if (query === (latest.current.q ?? "")) return;
		const timer = setTimeout(() => {
			typed.current = query;
			const { q: _q, ...rest } = latest.current;
			navigate(query ? { ...rest, q: query } : rest, { replace: true });
		}, SEARCH_URL_DELAY_MS);
		return () => clearTimeout(timer);
	}, [query, navigate]);
	return [query, setQuery] as const;
}

/* Rows. */

/** What this computer already knows about shared access: the hub's answer if one was read, else the verified rules after an unlock. */
function shareOf(input: AttentionInput, deviceId: string) {
	const answered = input.myAccess?.[deviceId];
	if (answered) return shareOfGrants(answered.grants);
	const mine = input.fleet[deviceId]?.policy?.myGrant;
	return mine ? shareOfGrants([mine]) : undefined;
}

type Billing = NonNullable<FleetDeviceEntry["billing"]>;

/** The spending an open "you still pay" item states. */
function itemBilling(item: AttentionItem | undefined) {
	const params = item?.copy.params;
	if (!params) return undefined;
	const { usedMicros, limitMicros } = params;
	if (typeof usedMicros !== "number") return undefined;
	if (typeof limitMicros !== "number") return undefined;
	const billing: Billing = { usedMicros, limitMicros };
	return billing;
}

/** The viewer's own spending limits on one device, from the hub's summary. */
function summaryBilling(input: AttentionInput, deviceId: string) {
	const billing: Billing = { usedMicros: 0, limitMicros: 0 };
	let mine = 0;
	for (const device of input.resourceSummary?.devices ?? []) {
		if (device.device_id !== deviceId) continue;
		for (const row of device.billing) {
			if (!row.payer_is_me) continue;
			mine++;
			billing.usedMicros += row.used_micros + row.reserved_micros;
			billing.limitMicros += row.limit_micros;
		}
	}
	return mine > 0 ? billing : undefined;
}

function billingOf(
	input: AttentionInput,
	view: DeviceViewModel,
	paying: AttentionItem | undefined,
) {
	const stated = itemBilling(paying);
	if (stated) return stated;
	if (view.relationship !== "cloud_approval") return undefined;
	return summaryBilling(input, view.row.device_id);
}

const STILL_PAYING: AttentionItem["key"] = "you_still_pay_for_a_revoked_device";
const NO_ACTIONS: readonly FleetRowAction[] = [];

/** What the open items add to rows: their buttons and the "still billed" item. */
interface RowItemFacts {
	actions: Map<string, FleetRowAction[]>;
	paying: Map<string, AttentionItem>;
}

/** The button an open item puts into its device's row; none while its action is gated. */
function rowActionOf(item: AttentionItem, entry: AttentionEntry | undefined) {
	const action = entry?.action;
	if (!action?.onSelect || action.gate) return undefined;
	const button: FleetRowAction = {
		id: item.id,
		label: action.label,
		danger: item.key === STILL_PAYING,
		onSelect: action.onSelect,
	};
	return button;
}

/** Two items of one device can lead to the same place: one button per label. */
function addRowAction(
	actions: RowItemFacts["actions"],
	deviceId: string,
	button: FleetRowAction,
) {
	const list = actions.get(deviceId) ?? [];
	for (const known of list) if (known.label === button.label) return;
	list.push(button);
	actions.set(deviceId, list);
}

function rowItemFacts(
	items: readonly AttentionItem[],
	entries: readonly AttentionEntry[],
) {
	const facts: RowItemFacts = { actions: new Map(), paying: new Map() };
	for (let index = 0; index < items.length; index++) {
		const item = items[index];
		const deviceId = subjectDevice(item.subject);
		if (!deviceId) continue;
		if (item.key === STILL_PAYING) facts.paying.set(deviceId, item);
		const button = rowActionOf(item, entries[index]);
		if (button) addRowAction(facts.actions, deviceId, button);
	}
	return facts;
}

interface EntryContext {
	input: AttentionInput;
	rowItems: RowItemFacts;
	appName(appId: string): string | undefined;
}

/** Lower-case text the search matches: name, ID, apps and services. */
function haystackOf(view: DeviceViewModel, name: string, ctx: EntryContext) {
	const words = [name, view.row.name, view.row.device_id];
	if (Array.isArray(view.services))
		for (const service of view.services)
			words.push(service.serviceId, ctx.appName(service.projectId) ?? "");
	return words.join(" ").toLowerCase();
}

function entryOf(view: DeviceViewModel, ctx: EntryContext) {
	const { input, rowItems } = ctx;
	const id = view.row.device_id;
	const fact = fleetFacts(input).byId.get(id);
	const name = fact?.name ?? view.row.name;
	const entry: FleetDeviceEntry = {
		view,
		name,
		actions: rowItems.actions.get(id) ?? NO_ACTIONS,
		haystack: haystackOf(view, name, ctx),
	};
	const os = fact?.inspection?.isolation?.platform;
	if (os) entry.os = os;
	if (fact?.inspectionSource) entry.statusSource = fact.inspectionSource;
	const share = view.relationship === "owner" ? undefined : shareOf(input, id);
	if (share) entry.share = share;
	const billing = billingOf(input, view, rowItems.paying.get(id));
	if (billing) entry.billing = billing;
	return entry;
}

const isRowActionItem = (item: AttentionItem) => ROW_ACTION_KEYS.has(item.key);

/** Every device of the hub list as a table entry: one pass per fleet read, no hook per row. */
export function useFleetEntries(): FleetDeviceEntry[] {
	const { navigate } = useDevicesRoute();
	const { input, items } = useAttentionState();
	const views = useDeviceViews({ watch: true });
	const appName = useAppNames();
	const actionItems = useMemo(() => items.filter(isRowActionItem), [items]);
	const actionEntries = useAttentionEntries(actionItems, {
		onNavigate: navigate,
	});
	return useMemo(() => {
		const ctx: EntryContext = {
			input,
			rowItems: rowItemFacts(actionItems, actionEntries),
			appName,
		};
		const entries: FleetDeviceEntry[] = [];
		for (const view of views) entries.push(entryOf(view, ctx));
		return entries;
	}, [input, views, actionItems, actionEntries, appName]);
}

/** The chips above the table: the URL filters (`filter=`), and "All" for none. */
export type DeviceChip = FleetFilter | "all";

const CHIPS: readonly DeviceChip[] = [
	"all",
	"critical",
	"attention",
	"unknown",
	"healthy",
	"offline",
	"locked",
	"shared",
	"revoked",
];

function chipLabel(t: DevicesT, chip: DeviceChip): string {
	switch (chip) {
		case "all":
			return t("devices:fleet.filter.all", "All");
		case "offline":
			return enumLabel(t, "presence", "offline");
		case "locked":
			return t("devices:fleet.filter.locked", "Locked");
		case "shared":
			return t("devices:fleet.filter.shared", "Shared with me");
		default:
			return healthLabel(t, chip);
	}
}

export function matchesChip(view: DeviceViewModel, chip: DeviceChip): boolean {
	switch (chip) {
		case "all":
			return true;
		case "offline":
			return view.presence.kind === "offline";
		case "locked":
			return view.row.status === "active" && keysLocked(view.keys);
		case "shared":
			return view.relationship !== "owner";
		default:
			return view.health === chip;
	}
}

const SORTERS: Record<
	FleetSort,
	(a: FleetDeviceEntry, b: FleetDeviceEntry) => number
> = {
	severity: (a, b) =>
		HEALTH_RANK[a.view.health] - HEALTH_RANK[b.view.health] ||
		b.view.attention.length - a.view.attention.length ||
		a.name.localeCompare(b.name),
	name: (a, b) => a.name.localeCompare(b.name),
	checkin: (a, b) =>
		(b.view.row.last_seen_at ?? 0) - (a.view.row.last_seen_at ?? 0) ||
		a.name.localeCompare(b.name),
	registered: (a, b) =>
		b.view.row.registered_at - a.view.row.registered_at ||
		a.name.localeCompare(b.name),
};

interface TableState {
	chip: DeviceChip;
	platform: string;
	group: FleetGroup;
	sort: FleetSort;
	pages: number;
	showRevoked: boolean;
}

function groupKey(
	t: DevicesT,
	group: Exclude<FleetGroup, "none">,
	view: DeviceViewModel,
): string {
	if (group === "health") return healthLabel(t, view.health);
	return enumLabel(t, "relationship", view.relationship);
}

interface Grouped {
	key: string;
	entries: FleetDeviceEntry[];
}

function grouped(
	t: DevicesT,
	group: FleetGroup,
	entries: readonly FleetDeviceEntry[],
): Grouped[] {
	if (group === "none") return [{ key: "", entries: [...entries] }];
	const groups = new Map<string, FleetDeviceEntry[]>();
	for (const entry of entries) {
		const key = groupKey(t, group, entry.view);
		const list = groups.get(key);
		if (list) list.push(entry);
		else groups.set(key, [entry]);
	}
	return [...groups].map(([key, list]) => ({ key, entries: list }));
}

export interface FleetDevicesTableProps {
	route: FleetRoute;
	entries: readonly FleetDeviceEntry[];
	/** The hub's host, for the table's accessible name. */
	host: string;
}

/** SPEC §5.1 Devices view: toolbar, the eight-column table, the collapsed Revoked group and paging. */
export function FleetDevicesTable({
	route,
	entries,
	host,
}: Readonly<FleetDevicesTableProps>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const appName = useAppNames();
	const keyChip = useKeyChip();
	const inventory = useFleetCertificateInventory();
	const summary = useResourceSummary();
	const [query, setQuery] = useFleetSearch(route);
	const [state, setState] = useState<Omit<TableState, "chip">>({
		platform: "",
		group: "none",
		sort: "severity",
		pages: 1,
		showRevoked: false,
	});
	const chip: DeviceChip = route.filter ?? "all";
	const labels = fleetColumnLabels(t);

	const setChip = useCallback(
		(next: DeviceChip) => {
			setState((previous) => ({
				...previous,
				pages: 1,
				showRevoked: previous.showRevoked || next === "revoked",
			}));
			const { filter: _filter, ...rest } = route;
			navigate(next === "all" ? rest : { ...rest, filter: next }, {
				replace: true,
			});
		},
		[route, navigate],
	);

	const platforms = useMemo(
		() =>
			[...new Set(entries.map((entry) => entry.os ?? ""))].sort((a, b) =>
				a.localeCompare(b),
			),
		[entries],
	);

	const matching = useMemo(() => {
		const needle = query.trim().toLowerCase();
		return entries
			.filter(
				(entry) =>
					matchesChip(entry.view, chip) &&
					(!state.platform ||
						(state.platform === UNKNOWN_PLATFORM
							? !entry.os
							: entry.os === state.platform)) &&
					(!needle || entry.haystack.includes(needle)),
			)
			.sort(SORTERS[state.sort]);
	}, [entries, chip, state.platform, state.sort, query]);

	const revokedInline = chip === "revoked";
	const active = revokedInline
		? matching
		: matching.filter((entry) => entry.view.row.status !== "revoked");
	const revoked = revokedInline
		? []
		: matching.filter((entry) => entry.view.row.status === "revoked");
	const cap = FLEET_PAGE_SIZE * state.pages;
	const shownActive = active.slice(0, cap);
	const billed = revoked.filter((entry) =>
		entry.view.attention.some(
			(item) => item.key === "you_still_pay_for_a_revoked_device",
		),
	).length;

	const certificateRows = useMemo(
		() => new Map(inventory.data?.map((row) => [row.device_id, row]) ?? []),
		[inventory.data],
	);
	const renderRow = (entry: FleetDeviceEntry) => (
		<FleetDeviceRow
			key={entry.view.row.device_id}
			entry={entry}
			certificates={
				certificateRows.get(entry.view.row.device_id) ?? entry.view.certificates
			}
			certificatesPerRow={inventory.missingOnHub}
			appName={appName}
			onLock={keyChip.lock}
		/>
	);
	const sortOf = (sort: FleetSort) =>
		state.sort === sort
			? sort === "name"
				? ("ascending" as const)
				: ("descending" as const)
			: ("none" as const);
	const setSort = (sort: FleetSort) =>
		setState((previous) => ({ ...previous, sort }));

	const clear = () => {
		setQuery("");
		setState((previous) => ({ ...previous, platform: "", pages: 1 }));
		setChip("all");
	};

	let body: ReactNode;
	if (matching.length === 0)
		body = (
			<tr>
				<td colSpan={FLEET_DEVICE_COLS.length} className="p-4">
					<StateView
						kind="empty"
						icon={Funnel}
						title={t(
							"fleet.devices.noMatch",
							"No device matches these filters",
						)}
						text={t(
							"fleet.devices.noMatchText",
							"{{count, number}} devices in total. Clear the search or the filter to see them.",
							{ count: entries.length },
						)}
						actions={
							<DvButton size="sm" onClick={clear}>
								{t("fleet.devices.clear", "Clear filters")}
							</DvButton>
						}
					/>
				</td>
			</tr>
		);
	else
		body = (
			<>
				{grouped(t, state.group, shownActive).map((group) => (
					<GroupRows
						key={group.key}
						title={group.key}
						count={group.entries.length}
					>
						{group.entries.map(renderRow)}
					</GroupRows>
				))}
				{revoked.length ? (
					<>
						<GroupRow colSpan={FLEET_DEVICE_COLS.length}>
							<button
								type="button"
								data-revoked-toggle=""
								aria-expanded={state.showRevoked}
								onClick={() =>
									setState((previous) => ({
										...previous,
										showRevoked: !previous.showRevoked,
									}))
								}
								className="inline-flex items-center gap-1 font-semibold text-foreground focus-visible:outline-2 focus-visible:outline-ring"
							>
								{state.showRevoked ? (
									<ChevronDown aria-hidden className="size-3.5" />
								) : (
									<ChevronRight aria-hidden className="size-3.5" />
								)}
								{t("fleet.revoked.group", "Revoked · {{count, number}}", {
									count: revoked.length,
								})}
							</button>
							{billed > 0 ? (
								<span className="inline-flex items-center gap-1 text-warning">
									<TriangleAlert aria-hidden className="size-3" />
									{summary.missingOnHub
										? t(
												"fleet.revoked.billedOpened",
												"{{count, number}} still billed to you (of the devices opened on this computer)",
												{ count: billed },
											)
										: t(
												"fleet.revoked.billed",
												"{{count, number}} still billed to you",
												{ count: billed },
											)}
								</span>
							) : null}
							<span className="text-muted-foreground">
								{t(
									"fleet.revoked.note",
									"Kept so you can close their cloud approvals and delete leftover keys.",
								)}
							</span>
						</GroupRow>
						{state.showRevoked ? revoked.map(renderRow) : null}
					</>
				) : null}
			</>
		);

	return (
		<>
			<div
				data-fleet-toolbar="devices"
				className="flex flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-hairline px-4 py-2.5"
			>
				<FleetSearch
					value={query}
					onChange={(value) => {
						setQuery(value);
						setState((previous) => ({ ...previous, pages: 1 }));
					}}
					placeholder={t(
						"fleet.search.devices",
						"Search name, ID, app or service",
					)}
				/>
				<fieldset
					aria-label={t(
						"fleet.filter.label",
						"Filter by health, presence or keys",
					)}
					className="m-0 flex min-w-0 flex-wrap gap-1.5 border-0 p-0"
				>
					{CHIPS.map((value) => (
						<FilterChip
							key={value}
							pressed={chip === value}
							onPressedChange={() => setChip(value)}
						>
							{chipLabel(t, value)}
						</FilterChip>
					))}
				</fieldset>
				<ToolbarSelect
					label={t("fleet.platform.label", "Platform")}
					value={state.platform || ALL}
					onChange={(value) =>
						setState((previous) => ({
							...previous,
							platform: value === ALL ? "" : value,
							pages: 1,
						}))
					}
					options={[
						{ value: ALL, label: t("fleet.platform.all", "All platforms") },
						...platforms.map((os) => ({
							value: os || UNKNOWN_PLATFORM,
							label: platformLabel(t, os || undefined),
						})),
					]}
				/>
				<ToolbarSelect<FleetGroup>
					label={t("fleet.group.label", "Group")}
					value={state.group}
					onChange={(group) => setState((previous) => ({ ...previous, group }))}
					options={[
						{ value: "none", label: t("fleet.group.none", "No grouping") },
						{
							value: "health",
							label: t("fleet.group.health", "Group by health"),
						},
						{
							value: "relationship",
							label: t("fleet.group.relationship", "Group by relationship"),
						},
					]}
				/>
				<ToolbarSelect<FleetSort>
					label={t("fleet.sort.label", "Sort")}
					value={state.sort}
					onChange={setSort}
					options={[
						{
							value: "severity",
							label: t("fleet.sort.severity", "Sort by severity"),
						},
						{ value: "name", label: t("fleet.sort.name", "Sort by name") },
						{
							value: "checkin",
							label: t("fleet.sort.checkin", "Sort by last check-in"),
						},
						{
							value: "registered",
							label: t("fleet.sort.registered", "Sort by registered"),
						},
					]}
				/>
				<span
					data-fleet-count=""
					className="ml-auto text-xs whitespace-nowrap text-muted-foreground @max-[900px]/devices:ml-0 @max-[900px]/devices:basis-full"
				>
					{t(
						"fleet.devices.showing",
						"Showing {{shown, number}} of {{total, number}}",
						{
							shown: Math.min(matching.length, cap + revoked.length),
							total: entries.length,
						},
					)}
				</span>
			</div>
			<DvTable
				cols={FLEET_DEVICE_COLS}
				className={FLEET_TABLE_CLASS}
				label={t("fleet.devices.caption", "Devices on {{host}}", { host })}
				head={
					<tr>
						<SortHeader sort={sortOf("name")} onSort={() => setSort("name")}>
							{labels.device}
						</SortHeader>
						<SortHeader
							sort={sortOf("severity")}
							onSort={() => setSort("severity")}
						>
							{labels.health}
						</SortHeader>
						<SortHeader
							sort={sortOf("checkin")}
							onSort={() => setSort("checkin")}
						>
							{labels.checkin}
							<span className="ml-1 font-normal tracking-normal normal-case">
								{t("fleet.column.last24h", "last 24 h")}
							</span>
						</SortHeader>
						<Th>{labels.services}</Th>
						<Th>{labels.certificates}</Th>
						<Th>{labels.access}</Th>
						<Th>{labels.agent}</Th>
						<Th>
							<span className="sr-only">{t("fleet.column.more", "More")}</span>
						</Th>
					</tr>
				}
			>
				{body}
			</DvTable>
			{active.length > cap ? (
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					<span>
						{t(
							"fleet.devices.paged",
							"Showing {{shown, number}} of {{total, number}}",
							{ shown: cap, total: active.length },
						)}
					</span>
					<DvButton
						size="sm"
						onClick={() =>
							setState((previous) => ({
								...previous,
								pages: previous.pages + 1,
							}))
						}
					>
						{t("fleet.devices.more", "Show {{count, number}} more", {
							count: Math.min(FLEET_PAGE_SIZE, active.length - cap),
						})}
					</DvButton>
				</div>
			) : null}
		</>
	);
}

const ALL = "__all";
const UNKNOWN_PLATFORM = "__unknown";

function GroupRows({
	title,
	count,
	children,
}: Readonly<{ title: string; count: number; children: ReactNode }>) {
	if (!title) return children;
	return (
		<>
			<GroupRow colSpan={FLEET_DEVICE_COLS.length}>
				<b className="font-semibold text-foreground">{title}</b>
				<span className="font-mono text-muted-foreground tabular-nums">
					{count}
				</span>
			</GroupRow>
			{children}
		</>
	);
}
