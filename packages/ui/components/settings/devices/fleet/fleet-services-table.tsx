"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Cloud,
	Database,
	Funnel,
	LayoutGrid,
	Link2,
	LoaderCircle,
	LockOpen,
	type LucideIcon,
	Play,
	Rocket,
	RotateCw,
	Square,
	X,
} from "lucide-react";
import { Fragment, type ReactNode, useId, useMemo, useState } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type AppMode,
	appMode,
} from "../../../../lib/device-management/model/app-plan";
import { isLastKnown } from "../../../../lib/device-management/model/device-view";
import type {
	AttentionInput,
	Convergence,
	DevicesScope,
	FleetRoute,
	LiveDeviceInput,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { useBackend } from "../../../../state/backend-state";
import type { IAppState } from "../../../../state/backend-state/app-state";
import { appCopy } from "../copy/app-copy";
import { enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import { MODE_ICON } from "../primitives/app-chips";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import {
	FreshnessStamp,
	type StampSpec,
	baseSource,
	sameSource,
} from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineConfirmRow } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { PairedPins } from "../primitives/paired-pins";
import {
	type DesiredRun,
	type ObservedRun,
	RequestedActual,
} from "../primitives/requested-actual";
import { FilterChip } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import { StatusChip, convergenceLabel } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	type ServiceCommand,
	useAppView,
	useAttentionInput,
	useFixAction,
	useInlineResults,
	useOverlayStore,
	useServiceCommands,
} from "../workspace";
import { CELL_CHIP, type FleetDeviceEntry } from "./fleet-device-row";
import {
	FLEET_PAGE_SIZE,
	FLEET_TABLE_CLASS,
	FleetSearch,
	ToolbarSelect,
	useFleetSearch,
} from "./fleet-devices-table";

/** SPEC §5.1 column plan: Service · Device · Requested → actual · Instances · Versions · Update & badges · Actions. */
const COLS = ["19%", "12%", "16%", "9%", "12%", "17%", "15%"] as const;

/** Severity order of the Services view: crashing first, stopped by you last. */
const CONV_RANK: Record<Convergence, number> = {
	crash_looping: 0,
	failed_stopped: 1,
	unknown: 2,
	update_in_progress: 3,
	converging: 4,
	converged: 5,
	stopped_by_user: 6,
};

const ACTIVE_ROLLOUT = new Set([
	"staged",
	"validating",
	"activating",
	"rolling_back",
]);

const OBSERVED = new Set<ObservedRun>([
	"unknown",
	"starting",
	"running",
	"stopping",
	"stopped",
	"backoff",
	"failed",
	"removed",
]);

const desiredOf = (service: ServiceView): DesiredRun =>
	service.desired === "stopped" ? "stopped" : "running";
const observedOf = (service: ServiceView): ObservedRun =>
	OBSERVED.has(service.observed as ObservedRun)
		? (service.observed as ObservedRun)
		: "unknown";

/** One service of a device whose status this computer can read. */
export interface FleetService {
	entry: FleetDeviceEntry;
	service: ServiceView;
	stamp: StampSpec;
}

export interface FleetServices {
	services: FleetService[];
	/** The stamp most rows share (R5): the block head states it once. */
	base: StampSpec | null;
	/** Active devices whose services can't be read here. */
	hidden: FleetDeviceEntry[];
}

/** Every readable service across devices, most severe first; computed once for the block head and the table. */
export function useFleetServices(
	entries: readonly FleetDeviceEntry[],
): FleetServices {
	return useMemo(() => {
		const services: FleetService[] = [];
		const hidden: FleetDeviceEntry[] = [];
		for (const entry of entries) {
			const { view } = entry;
			if (view.row.status !== "active") continue;
			if (!Array.isArray(view.services)) {
				if (view.relationship !== "cloud_approval") hidden.push(entry);
				continue;
			}
			for (const service of view.services)
				services.push({ entry, service, stamp: stampOf(service.freshness) });
		}
		services.sort(
			(a, b) =>
				CONV_RANK[a.service.conv] - CONV_RANK[b.service.conv] ||
				a.entry.name.localeCompare(b.entry.name) ||
				a.service.serviceId.localeCompare(b.service.serviceId),
		);
		return {
			services,
			base: baseSource(services.map((row) => row.stamp)),
			hidden,
		};
	}, [entries]);
}

export type ServiceChip =
	| "all"
	| "notreq"
	| "crash"
	| "updating"
	| "stopped"
	| "writes"
	| "cloud";

const CHIPS: readonly ServiceChip[] = [
	"all",
	"notreq",
	"crash",
	"updating",
	"stopped",
	"writes",
	"cloud",
];

function chipLabel(t: DevicesT, chip: ServiceChip): string {
	const labels = {
		all: t("devices:fleet.svc.filter.all", "All"),
		notreq: t("devices:fleet.svc.filter.notRequested", "Not as requested"),
		crash: t("devices:fleet.svc.filter.crashing", "Crashing"),
		updating: t("devices:fleet.svc.filter.updating", "Updating"),
		stopped: t("devices:fleet.svc.filter.stopped", "Stopped"),
		writes: t("devices:fleet.svc.filter.writes", "Has write buffering"),
		cloud: t("devices:fleet.svc.filter.cloud", "Uses cloud access"),
	} satisfies Record<ServiceChip, string>;
	return labels[chip];
}

const serviceKey = (deviceId: string, serviceId: string) =>
	`${deviceId}/${serviceId}`;

/** Approvals in the hub's summary that still run. */
function summaryApprovals(summary: AttentionInput["resourceSummary"]) {
	const keys: string[] = [];
	if (!summary) return keys;
	for (const device of summary.devices)
		for (const approval of device.approvals)
			if (approval.status === "active")
				keys.push(serviceKey(device.device_id, approval.placement_id));
	return keys;
}

/** Approvals from the per-device lists this computer has opened (older hubs). */
function deviceApprovals(resources: AttentionInput["resources"]) {
	const keys: string[] = [];
	for (const [deviceId, device] of Object.entries(resources)) {
		if (!device) continue;
		for (const grant of device.grants)
			if (grant.status === "active")
				keys.push(serviceKey(deviceId, grant.placement_id));
	}
	return keys;
}

/** Services whose live configuration names an approval. */
function liveApprovals(live: AttentionInput["live"]) {
	const keys: string[] = [];
	for (const [deviceId, device] of Object.entries(live)) {
		if (!device.placements) continue;
		for (const [placementId, facts] of Object.entries(device.placements))
			if (facts.resourceGrantId) keys.push(serviceKey(deviceId, placementId));
	}
	return keys;
}

/** Services with a cloud approval the viewer can see: the hub summary, the device's own list, or the last live read. */
function cloudServices(input: AttentionInput): Set<string> {
	return new Set([
		...summaryApprovals(input.resourceSummary),
		...deviceApprovals(input.resources),
		...liveApprovals(input.live),
	]);
}

interface Buffering {
	pending?: number;
	needsYou: boolean;
}

/** What a row shows beyond the service's own state. */
interface ServiceExtras {
	usesCloud: boolean;
	/** Set only when the service is known to buffer writes; `pending` when a count was read. */
	buffering?: Buffering;
	/** `host:port` from the last live configuration read. */
	endpoint?: string;
}

/** The status rows of a device: the live read, then the encrypted snapshot. */
function statusRows(input: AttentionInput, deviceId: string) {
	const live = input.live[deviceId]?.inspection;
	const rows = live ? [...live.value.placements] : [];
	const snapshot = input.fleet[deviceId]?.status;
	if (!snapshot) return rows;
	for (const observation of snapshot.observations)
		rows.push(...observation.placements);
	return rows;
}

type OfflineWrites = ServiceView["offlineWrites"];

/** The pending count of a queue read or status summary, when one was read. */
function countedWrites(read: OfflineWrites) {
	if (!read || read === "not_loaded") return undefined;
	const counted: Buffering = {
		pending: read.pending,
		needsYou: read.quarantined,
	};
	return counted;
}

/** What the status summary says, when it counts at least one buffering scope. */
function summaryWrites(
	input: AttentionInput,
	deviceId: string,
	serviceId: string,
) {
	for (const row of statusRows(input, deviceId)) {
		if (row.id !== serviceId) continue;
		const summary = row.offline_writes;
		if (!summary || summary.scopes === 0) return undefined;
		const counted: Buffering = {
			pending: summary.pending_count,
			needsYou: summary.quarantined_scopes + summary.needs_attention > 0,
		};
		return counted;
	}
	return undefined;
}

const BUFFERING_NOT_COUNTED: Buffering = { needsYou: false };

/** What the live connection knows: queues were read, buffering is configured, or neither. */
function liveBuffering(live: LiveDeviceInput | undefined, serviceId: string) {
	if (!live) return undefined;
	const { offlineQueues, placements } = live;
	if (offlineQueues && hasRows(offlineQueues[serviceId])) return "queues";
	if (!placements) return undefined;
	return placements[serviceId]?.offlineWrites ? "configured" : undefined;
}

const hasRows = (rows: readonly unknown[] | undefined) =>
	rows !== undefined && rows.length > 0;

/**
 * A service buffers writes when its queues were read live, its status summary
 * counts a buffering scope, or its configuration says so. An empty queue list
 * says only that nothing is buffered, not that buffering exists.
 */
function bufferingOf(
	input: AttentionInput,
	deviceId: string,
	service: ServiceView,
) {
	const id = service.serviceId;
	const counted = countedWrites(service.offlineWrites);
	const live = liveBuffering(input.live[deviceId], id);
	if (live === "queues") return counted;
	const summary = summaryWrites(input, deviceId, id);
	if (summary) return summary;
	if (live !== "configured") return undefined;
	return counted ?? BUFFERING_NOT_COUNTED;
}

const NO_EXTRAS: ServiceExtras = { usesCloud: false };

function extrasOf(
	input: AttentionInput,
	cloud: ReadonlySet<string>,
	{ entry, service }: FleetService,
): ServiceExtras {
	const deviceId = entry.view.row.device_id;
	const facts = input.live[deviceId]?.placements?.[service.serviceId];
	const buffering = bufferingOf(input, deviceId, service);
	return {
		usesCloud: cloud.has(serviceKey(deviceId, service.serviceId)),
		...(buffering ? { buffering } : {}),
		...(facts?.host && facts.port
			? { endpoint: `${facts.host}:${facts.port}` }
			: {}),
	};
}

const SETTLED = new Set<Convergence>(["converged", "stopped_by_user"]);
const CRASHED = new Set<Convergence>(["crash_looping", "failed_stopped"]);

const CHIP_MATCH: Record<
	ServiceChip,
	(service: ServiceView, extras: ServiceExtras) => boolean
> = {
	all: () => true,
	notreq: (service) => !SETTLED.has(service.conv),
	crash: (service) => CRASHED.has(service.conv),
	updating: (service) => service.conv === "update_in_progress",
	stopped: (service) => service.conv === "stopped_by_user",
	writes: (_service, extras) => extras.buffering !== undefined,
	cloud: (_service, extras) => extras.usesCloud,
};

interface RowFilter {
	appId?: string;
	chip: ServiceChip;
	extras: ServiceExtras;
	/** Lower-case search text. */
	needle: string;
	appName: string;
}

function matchesFilter({ service, entry }: FleetService, filter: RowFilter) {
	if (filter.appId && service.projectId !== filter.appId) return false;
	if (!CHIP_MATCH[filter.chip](service, filter.extras)) return false;
	if (!filter.needle) return true;
	return `${service.serviceId} ${entry.name} ${filter.appName}`
		.toLowerCase()
		.includes(filter.needle);
}

/* Row. */

interface ServiceRowProps {
	row: FleetService;
	base: StampSpec | null;
	appName: string;
	mode: AppMode | null;
	extras: ServiceExtras;
	titled: boolean;
}

function AppCell({
	row,
	appName,
	mode,
}: Readonly<Pick<ServiceRowProps, "row" | "appName" | "mode">>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { service, entry } = row;
	const Icon = mode ? MODE_ICON[mode] : LayoutGrid;
	return (
		<CellSub className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
			<Icon aria-hidden className="size-3 shrink-0" />
			<a
				{...link(
					{
						screen: "app-devices",
						by: "device",
						focusDeviceId: entry.view.row.device_id,
					},
					{ scope: { kind: "app", appId: service.projectId } },
				)}
				title={t(
					"fleet.svc.appTitle",
					"Where {{app}} runs, in its Devices settings",
					{ app: appName },
				)}
				className="underline decoration-border-strong underline-offset-2 hover:text-foreground hover:decoration-current"
			>
				{appName}
			</a>
			{mode ? <span>· {appCopy(t).mode(mode, appName).chip}</span> : null}
		</CellSub>
	);
}

function VersionsCell({ service }: Readonly<{ service: ServiceView }>) {
	const { t } = useTranslation("devices");
	const { appVersion, settings } = service;
	return (
		<>
			{appVersion?.label ? (
				<span className="font-mono">{appVersion.label}</span>
			) : appVersion?.hash ? (
				<span className="inline-flex flex-wrap items-center gap-x-1">
					<span className="text-muted-foreground">
						{t("fleet.svc.appVersion", "App version")}
					</span>
					<IdRef
						id={appVersion.hash}
						copyLabel={t("fleet.svc.copyVersion", "Copy app version hash")}
					/>
				</span>
			) : (
				<span className="text-muted-foreground">
					{t("fleet.svc.versionUnknown", "Version unknown")}
				</span>
			)}
			<CellSub>
				{settings.applied !== null && settings.applied !== settings.latest
					? t(
							"fleet.svc.settingsApplying",
							"Settings v{{latest}} · running v{{applied}}, applying",
							{ latest: settings.latest, applied: settings.applied },
						)
					: t("fleet.svc.settings", "Settings v{{latest}}", {
							latest: settings.latest,
						})}
			</CellSub>
		</>
	);
}

function Badges({ usesCloud, buffering, endpoint }: Readonly<ServiceExtras>) {
	const { t } = useTranslation("devices");
	if (!usesCloud && !buffering && !endpoint) return null;
	return (
		<span data-badges="" className="mt-1.5 flex flex-wrap gap-1">
			{usesCloud ? (
				<StatusChip tone="outline" icon={Cloud} className={BADGE}>
					{t("fleet.svc.badge.cloud", "Cloud access")}
				</StatusChip>
			) : null}
			{buffering ? (
				<StatusChip
					tone={buffering.needsYou ? "warning" : "outline"}
					icon={Database}
					data-badge="writes"
					className={BADGE}
				>
					{bufferingText(t, buffering)}
				</StatusChip>
			) : null}
			{endpoint ? (
				<StatusChip
					tone="outline"
					icon={Link2}
					title={endpoint}
					className={cx(BADGE, "font-mono")}
				>
					{endpoint}
				</StatusChip>
			) : null}
		</span>
	);
}

/** R10: a badge wraps inside its cell instead of clipping. */
const BADGE = `${CELL_CHIP} items-start [&>svg]:mt-0.5 [&>svg]:shrink-0`;

function bufferingText(
	t: DevicesT,
	{ pending, needsYou }: NonNullable<ServiceExtras["buffering"]>,
): string {
	if (pending === undefined)
		return t("devices:fleet.svc.badge.writes", "Write buffering");
	if (needsYou)
		return t(
			"devices:fleet.svc.badge.writesNeedsYou",
			"Write buffering · {{count, number}} waiting · needs you",
			{ count: pending },
		);
	return pending > 0
		? t(
				"devices:fleet.svc.badge.writesWaiting",
				"Write buffering · {{count, number}} waiting",
				{ count: pending },
			)
		: t(
				"devices:fleet.svc.badge.writesCurrent",
				"Write buffering · up to date",
			);
}

function UpdateCell({
	service,
	extras,
}: Readonly<{ service: ServiceView; extras: ServiceExtras }>) {
	const { t } = useTranslation("devices");
	const { rollout, settings } = service;
	const active = rollout && ACTIVE_ROLLOUT.has(rollout.state);
	return (
		<>
			{active ? (
				<>
					<StatusChip
						tone="info"
						icon={LoaderCircle}
						spin={rollout.state !== "staged"}
						className={CELL_CHIP}
					>
						{enumLabel(t, "rollout", rollout.state)}
					</StatusChip>
					{settings.applied !== null && settings.applied !== settings.latest ? (
						<CellSub>
							{t("fleet.svc.safeUpdate", "safe update · v{{from}} → v{{to}}", {
								from: settings.applied,
								to: settings.latest,
							})}
						</CellSub>
					) : null}
				</>
			) : (
				<span className="text-muted-foreground">
					{t("fleet.svc.noUpdate", "No update running")}
				</span>
			)}
			<Badges {...extras} />
		</>
	);
}

interface ActionSpec {
	id: "start" | "stop" | "restart";
	label: string;
	icon: LucideIcon;
	command: ServiceCommand;
}

function ServiceActions({
	row,
	onAsk,
}: Readonly<{ row: FleetService; onAsk(action: ActionSpec): void }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { navigate } = useDevicesRoute();
	const fix = useFixAction();
	const { service, entry } = row;
	const deviceId = entry.view.row.device_id;
	const commands = useServiceCommands(deviceId, service.serviceId);
	const results = useInlineResults(commands.resultKey);
	const crashed =
		service.conv === "crash_looping" || service.conv === "failed_stopped";
	const specs: ActionSpec[] =
		service.desired === "stopped" || crashed
			? [
					{
						id: "start",
						label: t("fleet.svc.start", "Start"),
						icon: Play,
						command: commands.start,
					},
				]
			: [
					{
						id: "restart",
						label: t("fleet.svc.restart", "Restart…"),
						icon: RotateCw,
						command: commands.restart,
					},
					{
						id: "stop",
						label: t("fleet.svc.stop", "Stop…"),
						icon: Square,
						command: commands.stop,
					},
				];
	const failing = specs.find((spec) => !spec.command.gate.ok)?.command.gate;
	const gate = failing && !failing.ok ? failing : undefined;
	const copy = gate ? gateCopy(t, gate) : undefined;
	const reasonId = useId();
	const runFix = () => {
		if (!gate?.fix) return;
		const outcome = fix(gate.fix);
		if (outcome.kind === "navigate") navigate(outcome.route);
	};
	return (
		<div className="flex min-w-0 flex-col items-start gap-1.5">
			<div className="flex flex-wrap items-center gap-1.5">
				{specs.map((spec) => {
					const blocked = !spec.command.gate.ok;
					return (
						<DvButton
							key={spec.id}
							size="sm"
							icon={spec.icon}
							data-service-action={spec.id}
							aria-disabled={blocked || undefined}
							aria-describedby={blocked ? reasonId : undefined}
							busy={spec.command.pending}
							onClick={() => onAsk(spec)}
						>
							{spec.label}
						</DvButton>
					);
				})}
				<DvButton size="sm" variant="ghost" asChild>
					<a
						{...link({
							screen: "service",
							deviceId,
							serviceId: service.serviceId,
							tab: "status",
						})}
					>
						{t("fleet.svc.open", "Open")}
					</a>
				</DvButton>
			</div>
			{copy && gate ? (
				<GateInline kind={gate.kind} id={reasonId}>
					{copy.inline}
					{copy.fix && gate.fix ? (
						<>
							{" "}
							<button
								type="button"
								onClick={runFix}
								className="font-medium text-foreground underline underline-offset-2"
							>
								{copy.fix}
							</button>
						</>
					) : null}
				</GateInline>
			) : service.conv === "crash_looping" ? (
				<span className="text-xs text-muted-foreground">
					{t("fleet.svc.startClears", "Start also clears the crash-loop limit")}
				</span>
			) : null}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}

function FleetServiceRow(props: Readonly<ServiceRowProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { row, base } = props;
	const { service, entry } = row;
	const [asked, setAsked] = useState<ActionSpec | null>(null);
	const deviceId = entry.view.row.device_id;
	const lastKnown = isLastKnown(service.freshness);
	const state = {
		desired: desiredOf(service),
		observed: observedOf(service),
		conv: service.conv,
	};
	return (
		<>
			<Tr
				data-service={service.serviceId}
				data-device={deviceId}
				dim={lastKnown}
			>
				<Td label={t("fleet.svc.column.service", "Service")} kind="name">
					<span className="flex min-w-0 items-center gap-2">
						<PairedPins {...state} title={props.titled} />
						<a
							{...link({
								screen: "service",
								deviceId,
								serviceId: service.serviceId,
								tab: "status",
							})}
							title={service.serviceId}
							className="min-w-0 truncate font-mono font-semibold text-foreground hover:underline"
						>
							{service.serviceId}
						</a>
					</span>
					<AppCell row={row} appName={props.appName} mode={props.mode} />
				</Td>
				<Td label={t("fleet.svc.column.device", "Device")} kind="name">
					<a
						{...link({ screen: "device", deviceId, tab: "overview" })}
						title={entry.name}
						className="block truncate font-mono font-semibold text-foreground hover:underline"
					>
						{entry.name}
					</a>
					{sameSource(row.stamp, base) ? null : (
						<CellSub>
							<FreshnessStamp {...row.stamp} compact />
						</CellSub>
					)}
				</Td>
				<Td label={t("fleet.svc.column.state", "Requested → actual")}>
					<RequestedActual
						{...state}
						sub={
							lastKnown && service.freshness.at !== undefined
								? t(
										"fleet.svc.stateLastKnown",
										"{{state}} · {{ago}}, last known",
										{
											state: convergenceLabel(t, service.conv),
											ago: time.ago(service.freshness.at),
										},
									)
								: convergenceLabel(t, service.conv)
						}
					/>
				</Td>
				<Td label={t("fleet.svc.column.instances", "Instances")}>
					<span className="tabular-nums">
						{t(
							"fleet.svc.ready",
							"{{ready, number}} of {{requested, number}} ready",
							{
								ready: service.instances.ready,
								requested: service.instances.requested,
							},
						)}
					</span>
					<CellSub className="tabular-nums">
						{t("fleet.svc.max", "max {{max, number}}", {
							max: service.instances.max,
						})}
					</CellSub>
				</Td>
				<Td label={t("fleet.svc.column.versions", "Versions")}>
					<VersionsCell service={service} />
				</Td>
				<Td label={t("fleet.svc.column.update", "Update & badges")}>
					<UpdateCell service={service} extras={props.extras} />
				</Td>
				<Td label={t("fleet.svc.column.actions", "Actions")} kind="act">
					<ServiceActions
						row={row}
						onAsk={(spec) =>
							setAsked((open) => (open?.id === spec.id ? null : spec))
						}
					/>
				</Td>
			</Tr>
			{asked ? (
				<InlineConfirmRow
					colSpan={COLS.length}
					label={asked.command.title}
					title={asked.command.title}
					sub={asked.command.sub}
					rows={asked.command.rows}
					confirmLabel={asked.command.label}
					tone={asked.command.tone}
					onCancel={() => setAsked(null)}
					onConfirm={async () => {
						await asked.command.run({ confirmed: true });
						setAsked(null);
					}}
				/>
			) : null}
		</>
	);
}

/* App filter. */

interface AppOption {
	id: string;
	name: string;
	mode: AppMode;
	services: number;
}

type AppList = Awaited<ReturnType<IAppState["getApps"]>>;

/** Every app the viewer knows, by name, with how many of its services are readable. */
function appOptionsOf(
	apps: AppList | undefined,
	services: readonly FleetService[],
): AppOption[] {
	const counts = new Map<string, number>();
	for (const { service } of services) {
		const known = counts.get(service.projectId) ?? 0;
		counts.set(service.projectId, known + 1);
	}
	const options: AppOption[] = [];
	for (const [app, meta] of apps ?? [])
		options.push({
			id: app.id,
			name: meta?.name ?? app.id,
			mode: appMode(app.visibility),
			services: counts.get(app.id) ?? 0,
		});
	return options.sort(byName);
}

const byName = (a: AppOption, b: AppOption) => a.name.localeCompare(b.name);

const ALL_APPS = "__all";
const APP_SCOPE = (appId: string): DevicesScope => ({ kind: "app", appId });
const LINK_BUTTON =
	"inline-flex items-center gap-1 font-medium text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-current";

function AppContextBar({
	app,
	onClear,
}: Readonly<{ app: AppOption; onClear(): void }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const scope = APP_SCOPE(app.id);
	const mode = appCopy(t).mode(app.mode, app.name).chip;
	return (
		<output
			data-app-context={app.id}
			className="flex min-w-0 basis-full flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-hairline bg-surface-sunken px-3 py-2 text-ui"
		>
			<LayoutGrid aria-hidden className="size-3.5 shrink-0 text-ink-2" />
			<span>
				{t("fleet.svc.app.label", "App:")}{" "}
				<a
					{...link({ screen: "app-devices", by: "device" }, { scope })}
					className="font-semibold hover:underline"
				>
					{app.name}
				</a>
			</span>
			<span className="text-muted-foreground">
				{app.services > 0
					? t("fleet.svc.app.showing", {
							count: app.services,
							mode,
							defaultValue_one:
								"{{mode}} · showing its {{count, number}} service only",
							defaultValue_other:
								"{{mode}} · showing its {{count, number}} services only",
						})
					: t("fleet.svc.app.none", "{{mode}} · no service you can read", {
							mode,
						})}
			</span>
			<span className="flex-1 @max-[720px]/devices:hidden" />
			<span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1 text-xs @max-[720px]/devices:basis-full">
				<a
					{...link({ screen: "app-devices", by: "device" }, { scope })}
					className={LINK_BUTTON}
				>
					<ArrowRight aria-hidden className="size-3" />
					{t("fleet.svc.app.where", "Where it runs")}
				</a>
				<a
					{...link({ screen: "deploy", deviceIds: [], mode: "new" }, { scope })}
					className={LINK_BUTTON}
				>
					<Rocket aria-hidden className="size-3" />
					{t("fleet.svc.app.deploy", "Deploy {{app}}…", { app: app.name })}
				</a>
				<button type="button" onClick={onClear} className={LINK_BUTTON}>
					<X aria-hidden className="size-3" />
					{t("fleet.svc.app.clear", "Show all apps")}
				</button>
			</span>
		</output>
	);
}

/** APP §6.2: the filtered app has no service this computer can read. */
function AppNotHere({
	app,
	locked,
	onClear,
}: Readonly<{
	app: AppOption;
	locked: readonly FleetDeviceEntry[];
	onClear(): void;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { view } = useAppView(app.id);
	const scope = APP_SCOPE(app.id);
	const copy = appCopy(t).mode(app.mode, app.name);
	const eligible = view?.events.rows.length;
	const total =
		view === undefined
			? undefined
			: view.events.rows.length + view.events.ineligible.length;
	const [first] = locked;
	return (
		<StateView
			kind="empty"
			icon={LayoutGrid}
			title={t(
				"fleet.svc.app.notHere",
				"{{app}} isn't on any device you can see",
				{ app: app.name },
			)}
			text={
				<>
					{t(
						"fleet.svc.app.notHereText",
						"No service of {{app}} shows up in the status of the devices you can read.",
						{ app: app.name },
					)}{" "}
					{locked.length === 1 && first
						? t(
								"fleet.svc.app.lockedOne",
								"{{device}} is locked, so its services aren't read here.",
								{ device: first.name },
							)
						: locked.length > 1
							? t(
									"fleet.svc.app.lockedMany",
									"{{count, number}} devices are locked, so their services aren't read here.",
									{ count: locked.length },
								)
							: null}{" "}
					{copy.sentence}
					{eligible !== undefined && total !== undefined ? (
						<>
							{" "}
							{t(
								"fleet.svc.app.events",
								"{{eligible, number}} of its {{count, number}} events can run on a device. When you deploy, you choose the whole app or one event, and one or more devices.",
								{ eligible, count: total },
							)}
						</>
					) : null}
				</>
			}
			actions={
				<>
					<DvButton size="sm" icon={LayoutGrid} asChild>
						<a {...link({ screen: "app-devices", by: "device" }, { scope })}>
							{t("fleet.svc.app.open", "Open in the app's settings")}
						</a>
					</DvButton>
					<DvButton size="sm" icon={Rocket} asChild>
						<a
							{...link(
								{ screen: "deploy", deviceIds: [], mode: "new" },
								{ scope },
							)}
						>
							{t("fleet.svc.app.deploy", "Deploy {{app}}…", { app: app.name })}
						</a>
					</DvButton>
					<DvButton size="sm" variant="ghost" onClick={onClear}>
						{t("fleet.svc.app.clear", "Show all apps")}
					</DvButton>
				</>
			}
		/>
	);
}

/* Table. */

/** Why a device's services aren't in the table, in two words. */
function hiddenReason(t: DevicesT, entry: FleetDeviceEntry): string {
	const { services } = entry.view;
	const notLoaded = t("devices:fleet.svc.hidden.notLoaded", "not loaded");
	if (Array.isArray(services)) return notLoaded;
	const byState: Partial<Record<string, string>> = {
		locked: t("devices:fleet.svc.hidden.locked", "locked"),
		noaccess: t("devices:fleet.svc.hidden.noAccess", "no access"),
	};
	const byReason: Partial<Record<string, string>> = {
		never_reported: t("devices:fleet.svc.hidden.never", "no status yet"),
		no_keys_here: t("devices:fleet.svc.hidden.noKeys", "no keys here"),
	};
	const reason = services.reason ? byReason[services.reason.code] : undefined;
	return byState[services.state] ?? reason ?? notLoaded;
}

/** Unlocking reads the services only when closed keys are the reason they're missing. */
const unlockHelps = ({ view }: FleetDeviceEntry) =>
	view.keys.state === "locked" &&
	!Array.isArray(view.services) &&
	view.services.state === "locked";

/** R11: the foot names at most this many unreadable devices. */
const HIDDEN_CAP = 5;

function HiddenFoot({
	hidden,
}: Readonly<{ hidden: readonly FleetDeviceEntry[] }>) {
	const { t } = useTranslation("devices");
	if (!hidden.length) return null;
	const more = hidden.length - HIDDEN_CAP;
	return (
		<div
			data-services-hidden=""
			className="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground"
		>
			<span>{t("fleet.svc.hidden.label", "Not shown:")}</span>
			{hidden.slice(0, HIDDEN_CAP).map((entry, index) => (
				<Fragment key={entry.view.row.device_id}>
					{index > 0 ? <span aria-hidden>·</span> : null}
					<span className="inline-flex items-center gap-1.5">
						<span className="font-mono text-ink-2">{entry.name}</span>
						<span>({hiddenReason(t, entry)})</span>
						{unlockHelps(entry) ? (
							<DvButton
								size="xs"
								icon={LockOpen}
								onClick={() =>
									useOverlayStore
										.getState()
										.openUnlock(entry.view.row.device_id)
								}
							>
								{t("fleet.svc.hidden.unlock", "Unlock…")}
							</DvButton>
						) : null}
					</span>
				</Fragment>
			))}
			{more > 0 ? (
				<span>
					{t("fleet.svc.hidden.more", "and {{count, number}} more", {
						count: more,
					})}
				</span>
			) : null}
		</div>
	);
}

export interface FleetServicesTableProps {
	route: FleetRoute;
	fleet: FleetServices;
}

/** SPEC §5.1 Services view: one row per readable service across devices, most severe first. */
export function FleetServicesTable({
	route,
	fleet,
}: Readonly<FleetServicesTableProps>) {
	const { t } = useTranslation("devices");
	const backend = useBackend();
	const input = useAttentionInput();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const [query, setQuery] = useFleetSearch(route);
	const [chip, setChip] = useState<ServiceChip>("all");
	const [appId, setAppId] = useState<string | null>(null);
	const [pages, setPages] = useState(1);
	const { services, base, hidden } = fleet;

	const extras = useMemo(() => {
		const cloud = cloudServices(input);
		return new Map(services.map((row) => [row, extrasOf(input, cloud, row)]));
	}, [input, services]);
	const appOptions = useMemo(
		() => appOptionsOf(apps.data, services),
		[apps.data, services],
	);
	const appById = useMemo(() => {
		const byId = new Map<string, AppOption>();
		for (const option of appOptions) byId.set(option.id, option);
		return byId;
	}, [appOptions]);
	const app = appId ? appById.get(appId) : undefined;

	const matching = useMemo(() => {
		const needle = query.trim().toLowerCase();
		const matches = (row: FleetService) => {
			const name = appById.get(row.service.projectId)?.name;
			return matchesFilter(row, {
				appId: app?.id,
				chip,
				extras: extras.get(row) ?? NO_EXTRAS,
				needle,
				appName: name ?? "",
			});
		};
		return services.filter(matches);
	}, [services, app, chip, extras, query, appById]);

	const cap = FLEET_PAGE_SIZE * pages;
	const shown = matching.slice(0, cap);
	const ofApp = app
		? services.filter(({ service }) => service.projectId === app.id)
		: services;
	const clearApp = () => {
		setAppId(null);
		setPages(1);
	};

	let empty: ReactNode = null;
	if (matching.length === 0) {
		if (app && ofApp.length === 0)
			empty = (
				<AppNotHere
					app={app}
					locked={hidden.filter(
						({ view }) =>
							!Array.isArray(view.services) && view.services.state === "locked",
					)}
					onClear={clearApp}
				/>
			);
		else if (services.length === 0)
			empty = hidden.length ? (
				<StateView
					kind="locked"
					title={t(
						"fleet.svc.noneReadable",
						"No device's services can be read on this computer yet",
					)}
					text={t(
						"fleet.svc.noneReadableText",
						"Services are part of each device's encrypted status. Unlock a device to read them.",
					)}
					actions={
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() => useOverlayStore.getState().openUnlockSeveral()}
						>
							{t("fleet.svc.unlockSeveral", "Unlock several…")}
						</DvButton>
					}
				/>
			) : (
				<StateView
					kind="empty"
					title={t("fleet.svc.none", "No services yet")}
					text={t(
						"fleet.svc.noneText",
						"None of your devices runs a service. Deploy an app to a device to start one.",
					)}
				/>
			);
		else
			empty = (
				<StateView
					kind="empty"
					icon={Funnel}
					title={t("fleet.svc.noMatch", "No service matches this filter")}
					text={
						app
							? t("fleet.svc.noMatchApp", {
									count: ofApp.length,
									app: app.name,
									defaultValue_one:
										"{{count, number}} service of {{app}} is readable right now.",
									defaultValue_other:
										"{{count, number}} services of {{app}} are readable right now.",
								})
							: t("fleet.svc.noMatchAll", {
									count: services.length,
									defaultValue_one:
										"{{count, number}} service is readable right now.",
									defaultValue_other:
										"{{count, number}} services are readable right now.",
								})
					}
					actions={
						<>
							{chip !== "all" ? (
								<DvButton size="sm" onClick={() => setChip("all")}>
									{t("fleet.svc.showAllStates", "Show all states")}
								</DvButton>
							) : null}
							{app ? (
								<DvButton size="sm" onClick={clearApp}>
									{t("fleet.svc.app.clear", "Show all apps")}
								</DvButton>
							) : null}
							{query ? (
								<DvButton size="sm" onClick={() => setQuery("")}>
									{t("fleet.svc.clearSearch", "Clear the search")}
								</DvButton>
							) : null}
						</>
					}
				/>
			);
	}

	return (
		<>
			<div
				data-fleet-toolbar="services"
				className="flex flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-hairline px-4 py-2.5"
			>
				<FleetSearch
					value={query}
					onChange={(value) => {
						setQuery(value);
						setPages(1);
					}}
					placeholder={t(
						"fleet.search.services",
						"Search service, device or app",
					)}
				/>
				<fieldset
					aria-label={t("fleet.svc.filter.label", "Filter services")}
					className="m-0 flex min-w-0 flex-wrap gap-1.5 border-0 p-0"
				>
					{CHIPS.map((value) => (
						<FilterChip
							key={value}
							pressed={chip === value}
							onPressedChange={() => {
								setChip(value);
								setPages(1);
							}}
						>
							{chipLabel(t, value)}
						</FilterChip>
					))}
				</fieldset>
				<ToolbarSelect
					label={t("fleet.svc.app.select", "App")}
					value={app?.id ?? ALL_APPS}
					onChange={(value) => {
						setAppId(value === ALL_APPS ? null : value);
						setPages(1);
					}}
					className="max-w-65"
					options={[
						{ value: ALL_APPS, label: t("fleet.svc.app.all", "All apps") },
						...appOptions.map((option) => ({
							value: option.id,
							label:
								option.services > 0
									? t("fleet.svc.app.option", {
											count: option.services,
											app: option.name,
											defaultValue_one: "{{app}} · {{count, number}} service",
											defaultValue_other:
												"{{app}} · {{count, number}} services",
										})
									: t("fleet.svc.app.optionNone", "{{app}} · none readable", {
											app: option.name,
										}),
						})),
					]}
				/>
				<span
					data-fleet-count=""
					className="ml-auto text-xs whitespace-nowrap text-muted-foreground @max-[900px]/devices:ml-0 @max-[900px]/devices:basis-full"
				>
					{t(
						"fleet.svc.showing",
						"Showing {{shown, number}} of {{total, number}} · most severe first",
						{ shown: shown.length, total: services.length },
					)}
				</span>
				{app ? <AppContextBar app={app} onClear={clearApp} /> : null}
			</div>
			<DvTable
				cols={COLS}
				className={FLEET_TABLE_CLASS}
				label={t(
					"fleet.svc.caption",
					"Services on devices you can read, most severe first",
				)}
				head={
					<tr>
						<Th>{t("fleet.svc.column.service", "Service")}</Th>
						<Th>{t("fleet.svc.column.device", "Device")}</Th>
						<Th>{t("fleet.svc.column.state", "Requested → actual")}</Th>
						<Th>{t("fleet.svc.column.instances", "Instances")}</Th>
						<Th>{t("fleet.svc.column.versions", "Versions")}</Th>
						<Th>{t("fleet.svc.column.update", "Update & badges")}</Th>
						<Th>{t("fleet.svc.column.actions", "Actions")}</Th>
					</tr>
				}
			>
				{empty ? (
					<tr>
						<td colSpan={COLS.length} className="p-4">
							{empty}
						</td>
					</tr>
				) : (
					shown.map((row, index) => {
						const { service } = row;
						const option = appById.get(service.projectId);
						return (
							<FleetServiceRow
								key={serviceKey(
									row.entry.view.row.device_id,
									service.serviceId,
								)}
								row={row}
								base={base}
								appName={
									option?.name ??
									t("fleet.svc.app.unknown", "An app you can't see")
								}
								mode={service.source ?? option?.mode ?? null}
								extras={extras.get(row) ?? NO_EXTRAS}
								titled={index === 0}
							/>
						);
					})
				)}
			</DvTable>
			{matching.length > cap ? (
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					<span>
						{t(
							"fleet.svc.paged",
							"Showing {{shown, number}} of {{total, number}}",
							{ shown: cap, total: matching.length },
						)}
					</span>
					<DvButton size="sm" onClick={() => setPages((value) => value + 1)}>
						{t("fleet.svc.more", "Show {{count, number}} more", {
							count: Math.min(FLEET_PAGE_SIZE, matching.length - cap),
						})}
					</DvButton>
				</div>
			) : null}
			<HiddenFoot hidden={hidden} />
		</>
	);
}
