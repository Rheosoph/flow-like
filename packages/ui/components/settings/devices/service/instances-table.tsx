"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Box,
	CircleCheck,
	CircleDashed,
	CirclePause,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
} from "lucide-react";
import { useMemo } from "react";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import type {
	AttentionInput,
	PlacementStatusPlus,
	ReplicaStatusPlus,
	ServiceView,
	SourcePlane,
} from "../../../../lib/device-management/model/types";
import { humanFileSize } from "../../../../lib/utils";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { LOCKED_DATA_CLASS, StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { stampOf } from "../shell/attention-popover";
import {
	useAttentionState,
	useDeviceResources,
	useLiveStream,
} from "../workspace";

type ObservedState = Parameters<typeof enumLabel<"observed">>[2];

const OBSERVED_LOOK: Record<
	string,
	{ tone: ChipTone; icon: LucideIcon; spin?: boolean }
> = {
	running: { tone: "good", icon: CircleCheck },
	starting: { tone: "info", icon: LoaderCircle, spin: true },
	stopping: { tone: "info", icon: LoaderCircle, spin: true },
	backoff: { tone: "critical", icon: OctagonX },
	failed: { tone: "critical", icon: OctagonX },
	stopped: { tone: "paused", icon: CirclePause },
};
const UNKNOWN_LOOK = { tone: "unknown" as ChipTone, icon: CircleDashed };

interface Observation {
	observed_at?: number;
	placements: PlacementStatusPlus[];
}

/** The newest row of the service across a plane's observations. */
function latestRow(
	observations: readonly Observation[] | undefined,
	serviceId: string,
) {
	return (observations ?? [])
		.flatMap((observation) =>
			observation.placements
				.filter((row) => row.id === serviceId)
				.map((row) => ({ row, at: observation.observed_at ?? 0 })),
		)
		.sort((a, b) => b.at - a.at)[0]?.row;
}

type FleetState = AttentionInput["fleet"][string];

/** Where the rows of the two snapshot planes live; every other plane reads the live inspection. */
const SNAPSHOT_ROWS: Partial<
	Record<SourcePlane, (fleet: FleetState) => readonly Observation[] | undefined>
> = {
	snap: (fleet) => fleet.status?.observations,
	saved: (fleet) => fleet.saved?.observations,
};

/** The device's own row for a service, from the plane its `ServiceView` was built from. */
export function placementRow(
	input: AttentionInput,
	deviceId: string,
	serviceId: string,
	source: SourcePlane | undefined,
): PlacementStatusPlus | undefined {
	const read = source ? SNAPSHOT_ROWS[source] : undefined;
	const fleet = input.fleet[deviceId];
	if (read) return fleet ? latestRow(read(fleet), serviceId) : undefined;
	const live = input.live[deviceId]?.inspection;
	return live?.value.placements.find((row) => row.id === serviceId);
}

export function usePlacementRow(
	deviceId: string,
	service: ServiceView | undefined,
): PlacementStatusPlus | undefined {
	const { input } = useAttentionState();
	const serviceId = service?.serviceId;
	const source = service?.freshness.src;
	return useMemo(
		() =>
			serviceId ? placementRow(input, deviceId, serviceId, source) : undefined,
		[input, deviceId, serviceId, source],
	);
}

interface InstanceSample {
	cpu?: number;
	memory?: number;
	processes?: number;
}

function finite(value: unknown) {
	return typeof value === "number" && Number.isFinite(value)
		? value
		: undefined;
}

const listOf = (value: unknown): unknown[] =>
	Array.isArray(value) ? value : [];

function sampleOf(entry: unknown): [number, InstanceSample] | undefined {
	const row = (entry ?? {}) as Record<string, unknown>;
	const slot = finite(row.slot);
	if (slot === undefined) return undefined;
	return [
		slot,
		{
			cpu: finite(row.cpu_percent),
			memory: finite(row.memory_bytes),
			processes: finite(row.processes_and_threads),
		},
	];
}

/** Per-instance values of the newest sample; anything malformed reads as absent. */
function instanceSamples(data: unknown): Map<number, InstanceSample> {
	const records = listOf((data as { records?: unknown } | undefined)?.records);
	const newest = records.at(-1) as
		| { data?: { replicas?: unknown } }
		| undefined;
	return new Map(
		listOf(newest?.data?.replicas)
			.slice(0, 32)
			.flatMap((entry) => {
				const sample = sampleOf(entry);
				return sample ? [sample] : [];
			}),
	);
}

interface InstanceRow {
	slot: number;
	observed: string;
	applied: number | null;
	replica?: ReplicaStatusPlus;
}

function instanceRows(
	service: ServiceView,
	placement: PlacementStatusPlus | undefined,
): InstanceRow[] {
	const replicas = placement?.replicas;
	if (replicas?.length)
		return [...replicas]
			.sort((a, b) => a.slot - b.slot)
			.map((replica) => ({
				slot: replica.slot,
				observed: replica.observed_state,
				applied: replica.applied_revision,
				replica,
			}));
	const count = Math.max(
		service.instances.requested,
		service.instances.running,
		1,
	);
	return Array.from({ length: count }, (_, slot) => ({
		slot,
		observed: service.observed,
		applied: service.settings.applied,
	}));
}

const ABSENT = "–";

/**
 * Last errors and restart counts (BG7): `reported` by the agent, missing on an
 * `old_agent`, or unknown while `not_live` (a snapshot never carries the text).
 */
export type InstanceDiagnostics = "reported" | "old_agent" | "not_live";

/** SPEC §5.3 Status › Instances: one row per instance; stacked into cards on a phone. */
export function InstancesTable({
	deviceId,
	service,
	placement,
	rollout,
	diagnostics,
	lockedData = false,
}: Readonly<{
	deviceId: string;
	service: ServiceView;
	placement: PlacementStatusPlus | undefined;
	/** The running update, when one switches versions now. */
	rollout?: DeploymentRolloutStatus;
	diagnostics: InstanceDiagnostics;
	lockedData?: boolean;
}>) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const live = service.freshness.src === "live";
	const current =
		service.freshness.age === "live" || service.freshness.age === "current";
	const metrics = useLiveStream<unknown>(
		deviceId,
		live && current
			? { kind: "metrics", placementId: service.serviceId }
			: null,
	);
	const samples = useMemo(() => instanceSamples(metrics.data), [metrics.data]);
	const resources = useDeviceResources(deviceId);
	const grant = resources.data?.grants.find(
		(row) => row.placement_id === service.serviceId && row.status === "active",
	);
	const leases = useMemo(
		() =>
			(resources.data?.instances ?? [])
				.filter(
					(row) =>
						row.grant_id === grant?.grant_id && row.purpose === "workload",
				)
				.sort((a, b) => a.registered_at - b.registered_at),
		[resources.data, grant?.grant_id],
	);
	const rows = instanceRows(service, placement);
	const { requested, ready, max } = service.instances;
	const number = new Intl.NumberFormat(i18n?.language ?? "en", {
		maximumFractionDigits: 1,
		minimumFractionDigits: 1,
	});
	const switching = rollout?.state === "activating" ? rollout : undefined;
	const summary = t(
		"service.instances.ready",
		"{{ready}} of {{requested}} ready",
		{
			ready,
			requested,
		},
	);
	const stamps = (
		<>
			<FreshnessStamp {...stampOf(service.freshness)} />
			{leases.length ? (
				<FreshnessStamp
					{...stampOf(resources.freshness)}
					text={
						resources.freshness.at === undefined
							? undefined
							: t("service.instances.leasesChecked", "leases checked {{ago}}", {
									ago: time.ago(resources.freshness.at),
								})
					}
				/>
			) : null}
		</>
	);
	const foot = t(
		"service.instances.foot",
		"CPU is % of one CPU and can exceed 100 % on multi-core devices. A cloud lease lets an instance get cloud credentials for 10 minutes at a time; it isn't proof the instance is healthy.",
	);

	if (service.observed === "stopped" && !lockedData) {
		const why =
			service.conv === "failed_stopped"
				? t("service.instances.stoppedFailed", {
						count: requested,
						settings: service.settings.latest,
						defaultValue_one:
							"The service stopped after the failed update. Start runs {{count, number}} instance with settings v{{settings}}.",
						defaultValue_other:
							"The service stopped after the failed update. Start runs {{count, number}} instances with settings v{{settings}}.",
					})
				: t("service.instances.stoppedAsked", {
						count: requested,
						settings: service.settings.latest,
						defaultValue_one:
							"It's stopped, as you asked. Start runs {{count, number}} instance with settings v{{settings}}.",
						defaultValue_other:
							"It's stopped, as you asked. Start runs {{count, number}} instances with settings v{{settings}}.",
					});
		return (
			<Block
				icon={Box}
				title={t("service.instances.title", "Instances")}
				summary={summary}
				stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
				foot={foot}
			>
				<StateView
					kind="empty"
					icon={CirclePause}
					title={t("service.instances.none", "No instances are running")}
					text={why}
				/>
			</Block>
		);
	}

	const labels = {
		slot: t("service.instances.col.slot", "Instance"),
		state: t("service.instances.col.state", "State"),
		settings: t("service.instances.col.settings", "Settings"),
		ready: t("service.instances.col.ready", "Ready"),
		cpu: t("service.instances.col.cpu", "CPU"),
		memory: t("service.instances.col.memory", "Memory"),
		processes: t("service.instances.col.processes", "Processes"),
		lease: t("service.instances.col.lease", "Cloud lease"),
		error: t("service.instances.col.error", "Last error"),
	};
	let readyLeft = ready;

	const leaseCell = (index: number) => {
		if (!resources.data)
			return <span className="text-muted-foreground">{ABSENT}</span>;
		const lease = leases[index];
		if (!lease)
			return (
				<span className="text-muted-foreground">
					{grant
						? t("service.instances.noLease", "No lease")
						: t("service.instances.noCloud", "No cloud access")}
				</span>
			);
		return (
			<>
				<span title={time.abs(lease.lease_expires_at)}>
					{t("service.instances.leaseUntil", "until {{time}}", {
						time: time.clock(lease.lease_expires_at),
					})}
				</span>
				{lease.lease_expires_at < time.nowS ? (
					<CellSub className="text-warning">
						{t("service.instances.leaseExpired", "may have expired")}
					</CellSub>
				) : null}
			</>
		);
	};

	const errorCell = (row: InstanceRow) => {
		if (diagnostics !== "reported")
			return (
				<FreshnessStamp
					compact
					source="device"
					age="notloaded"
					text={t("service.instances.onDevice", "only on the device")}
				/>
			);
		const error = row.replica?.last_error ?? placement?.last_error;
		const restarts = row.replica?.restarts ?? placement?.restarts;
		const hasError = row.replica?.has_error ?? placement?.has_error ?? !!error;
		return (
			<>
				{error ? (
					<span className="line-clamp-2 font-mono text-xs" title={error}>
						{error}
					</span>
				) : hasError ? (
					<span>
						{t(
							"service.instances.errorHidden",
							"An error was recorded. Its text shows live with Deploy & configure.",
						)}
					</span>
				) : (
					<span className="text-muted-foreground">
						{t("service.instances.noError", "None")}
					</span>
				)}
				{restarts?.failures ? (
					<CellSub>
						{t("service.instances.restarts", {
							count: restarts.failures,
							max: restarts.max_restarts,
							defaultValue_one:
								"Restarted {{count, number}} time of {{max, number}} allowed",
							defaultValue_other:
								"Restarted {{count, number}} times of {{max, number}} allowed",
						})}
					</CellSub>
				) : null}
			</>
		);
	};

	return (
		<Block
			icon={Box}
			title={t("service.instances.title", "Instances")}
			summary={summary}
			stamp={stamps}
			flush
			foot={foot}
		>
			<DvTable
				label={t("service.instances.caption", "Instances of {{service}}", {
					service: service.serviceId,
				})}
				cols={["5%", "21%", "9%", "9%", "8%", "9%", "10%", "13%", "16%"]}
				className={lockedData ? LOCKED_DATA_CLASS : undefined}
				head={
					<tr>
						<Th>#</Th>
						<Th>{labels.state}</Th>
						<Th>{labels.settings}</Th>
						<Th>{labels.ready}</Th>
						<Th>{labels.cpu}</Th>
						<Th>{labels.memory}</Th>
						<Th>{labels.processes}</Th>
						<Th>{labels.lease}</Th>
						<Th>{labels.error}</Th>
					</tr>
				}
			>
				{rows.map((row, index) => {
					const look = OBSERVED_LOOK[row.observed] ?? UNKNOWN_LOOK;
					const isReady = row.observed === "running" && readyLeft > 0;
					if (isReady) readyLeft -= 1;
					const sample = samples.get(row.slot);
					const newVersion =
						switching &&
						row.applied !== null &&
						row.applied === switching.active_revision;
					return (
						<Tr key={row.slot}>
							<Td label={labels.slot} kind="mono">
								#{row.slot}
							</Td>
							<Td label={labels.state}>
								<StatusChip
									tone={look.tone}
									icon={look.icon}
									spin={look.spin}
									data-observed={row.observed}
									title={enumLabel(
										t,
										"observed",
										row.observed as ObservedState,
									)}
								>
									{enumLabel(t, "observed", row.observed as ObservedState)}
								</StatusChip>
							</Td>
							<Td label={labels.settings} className="font-mono">
								{row.applied === null ? ABSENT : `v${row.applied}`}
								{switching && row.applied !== null ? (
									<CellSub className="font-sans">
										{newVersion
											? t("service.instances.newVersion", "new version")
											: t("service.instances.oldVersion", "old version")}
									</CellSub>
								) : null}
							</Td>
							<Td label={labels.ready}>
								{isReady ? (
									<span className="inline-flex items-center gap-1 text-good">
										<CircleCheck aria-hidden className="size-3.25" />
										{t("service.instances.isReady", "Ready")}
									</span>
								) : (
									<span className="text-muted-foreground">
										{t("service.instances.notReady", "Not ready")}
									</span>
								)}
							</Td>
							<Td label={labels.cpu} className="font-mono tabular-nums">
								{sample?.cpu === undefined
									? ABSENT
									: t("service.instances.cpuValue", "{{value}} %", {
											value: number.format(sample.cpu),
										})}
							</Td>
							<Td label={labels.memory} className="font-mono tabular-nums">
								{sample?.memory === undefined
									? ABSENT
									: humanFileSize(sample.memory)}
							</Td>
							<Td label={labels.processes} className="font-mono tabular-nums">
								{sample?.processes ?? ABSENT}
							</Td>
							<Td label={labels.lease}>{leaseCell(index)}</Td>
							<Td label={labels.error}>{errorCell(row)}</Td>
						</Tr>
					);
				})}
			</DvTable>
			{max <= 1 && switching && switching.updated_at !== undefined ? (
				<p className="border-t border-hairline px-4 py-3 text-ui text-ink-2">
					{t(
						"service.instances.oneSlot",
						"This service has one slot, so the safe update stopped the previous instance (settings v{{from}}) at {{time}} before starting the new one. If the new one doesn't become healthy, the device starts v{{from}} again.",
						{
							from: switching.base_revision ?? service.settings.applied ?? 0,
							time: time.clock(switching.updated_at),
						},
					)}
				</p>
			) : null}
		</Block>
	);
}
