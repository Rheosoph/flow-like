"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleDashed,
	CirclePause,
	Gauge,
	LockOpen,
	Stethoscope,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type DeviceFacts,
	fleetFacts,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type {
	GateContext,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { StreamState } from "../../../../lib/device-management/workspace/types";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import { useBackend } from "../../../../state/backend-state";
import type { IOwnRole } from "../../../../state/backend-state/types";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { Metric, MetricGrid } from "../primitives/metric";
import { StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { useAppNames } from "../shell/attention-popover";
import {
	buildGateContext,
	useAttentionState,
	useDeviceWorkspace,
	useOverlay,
} from "../workspace";
import { ConnectButton } from "./live-state";
import {
	type Json,
	PLAIN_PARAGRAPHS,
	amount,
	asRecord,
	bytesParts,
	bytesText,
	dayLabel,
	metricSamples,
	whole,
} from "./observe-data";
import { Unavailable } from "./resource-cells";

const ROWS = 8;
const NAMES = 4;
const LINK = "font-mono font-semibold text-foreground hover:underline";
/* Six cells sit three to a row, five in one row when the block is wide enough (APP §2.14). */
const SIX_CELLS = "grid-cols-[repeat(auto-fit,minmax(260px,1fr))]";
const FIVE_CELLS = "grid-cols-[repeat(auto-fit,minmax(170px,1fr))]";
const TWO_LINE_HEAD = "whitespace-normal";
const SUB = "font-sans whitespace-nowrap";

type ProjectRole = NonNullable<GateContext["projectRole"]>;

async function noRole(): Promise<IOwnRole | null> {
	return null;
}

interface ProjectRoleRead {
	/** Undefined for an app without a role table (local-only): it is open to its user. */
	role: ProjectRole | undefined;
	/** The role is still being read: nothing is asked of a device until it is known. */
	loading: boolean;
}

function useProjectRole(appId: string): ProjectRoleRead {
	const backend = useBackend();
	const roleState = backend.roleState as
		| { getOwnRole?: (appId: string) => Promise<IOwnRole> }
		| undefined;
	const getOwnRole =
		typeof roleState?.getOwnRole === "function" ? roleState.getOwnRole : null;
	const role = useInvoke<IOwnRole | null, [string]>(
		getOwnRole ?? noRole,
		roleState,
		[appId],
		!!getOwnRole && !!appId,
	);
	const loading = !!getOwnRole && role.isLoading;
	return useMemo(() => {
		const data = Number.isInteger(role.data?.permissions)
			? role.data
			: undefined;
		if (!data) return { role: undefined, loading };
		const permissions = new RolePermissions(BigInt(data.permissions));
		return {
			role: {
				readBoards: permissions.hasPermission(RolePermissions.ReadBoards),
				admin: permissions.hasPermission(RolePermissions.Admin),
				executeBoards: permissions.hasPermission(RolePermissions.ExecuteBoards),
				// `is_owner` also holds for an Admin; only the Owner permission names the app's owner.
				owner: permissions.contains(RolePermissions.Owner),
			},
			loading,
		};
	}, [role.data, loading]);
}

type NoLive = "offline" | "locked" | "nokeys" | "idle" | "noaccess";

interface Totals {
	cpu: number | undefined;
	memory: number | undefined;
	read: number | undefined;
	written: number | undefined;
	started: number | undefined;
	succeeded: number;
	failed: number;
	inFlight: number;
	reported: number;
	expected: number;
	retainedStarted: number | undefined;
	retainedSucceeded: number;
	retainedFailed: number;
	since: number | undefined;
	tailLoss: boolean;
}

interface Row {
	device: DeviceFacts;
	/** The app's services there: read now, or as last read before the keys were locked. */
	services: Pick<ServiceView, "serviceId">[];
	running: Pick<ServiceView, "serviceId">[];
	ready: number;
	wanted: number;
	/** Why no live sample can exist; undefined when a session is up. */
	noLive?: NoLive;
	sample?: Json;
	sampled: number;
}

/** A total stays unavailable once one device's value is: a partial sum would read as the whole. */
const sum = (a: number | undefined, b: number | undefined) =>
	a === undefined || b === undefined ? undefined : a + b;

const counted = (value: unknown) => whole(value) ?? 0;

const earlier = (a: number | undefined, b: number | undefined) =>
	a === undefined || b === undefined ? (a ?? b) : Math.min(a, b);

const NO_TOTALS: Totals = {
	cpu: 0,
	memory: 0,
	read: 0,
	written: 0,
	started: 0,
	succeeded: 0,
	failed: 0,
	inFlight: 0,
	reported: 0,
	expected: 0,
	retainedStarted: undefined,
	retainedSucceeded: 0,
	retainedFailed: 0,
	since: undefined,
	tailLoss: false,
};

function addResources(total: Totals, sample: Json): Totals {
	const io = asRecord(sample.io) ?? {};
	return {
		...total,
		cpu: sum(total.cpu, amount(sample.cpu_percent)),
		memory: sum(total.memory, amount(sample.memory_bytes)),
		read: sum(total.read, amount(io.read_bytes_per_second)),
		written: sum(total.written, amount(io.written_bytes_per_second)),
	};
}

function addUsage(total: Totals, sample: Json): Totals {
	const usage = asRecord(sample.usage) ?? {};
	return {
		...total,
		started: sum(total.started, whole(usage.invocations_started)),
		succeeded: total.succeeded + counted(usage.invocations_succeeded),
		failed: total.failed + counted(usage.invocations_failed),
		inFlight: total.inFlight + counted(usage.in_flight),
		reported: total.reported + counted(usage.reported_replicas),
		expected: total.expected + counted(usage.expected_replicas),
	};
}

function addRetained(total: Totals, sample: Json): Totals {
	const retained = asRecord(sample.usage_retained);
	if (!retained) return total;
	const counters = asRecord(retained.counters) ?? {};
	return {
		...total,
		retainedStarted:
			counted(total.retainedStarted) + counted(counters.invocations_started),
		retainedSucceeded:
			total.retainedSucceeded + counted(counters.invocations_succeeded),
		retainedFailed: total.retainedFailed + counted(counters.invocations_failed),
		since: earlier(total.since, amount(retained.since)),
		tailLoss: total.tailLoss || retained.tail_loss_possible === true,
	};
}

function totalsOf(samples: readonly Json[]): Totals {
	return samples.reduce<Totals>(
		(total, sample) =>
			addRetained(addUsage(addResources(total, sample), sample), sample),
		NO_TOTALS,
	);
}

/** Subscribes to the app's samples on every device that already has a live session wanted. */
function useProjectSamples(
	appId: string,
	deviceIds: readonly string[],
): Record<string, StreamState<unknown>> {
	const { streams } = useDeviceWorkspace();
	const key = deviceIds.join("|");
	const [states, setStates] = useState<Record<string, StreamState<unknown>>>(
		{},
	);
	useEffect(() => {
		setStates({});
		if (!key) return;
		const stops = key
			.split("|")
			.map((deviceId) =>
				streams.subscribe<unknown>(
					deviceId,
					{ kind: "project_metrics", projectId: appId },
					(state) => setStates((known) => ({ ...known, [deviceId]: state })),
				),
			);
		return () => {
			for (const stop of stops) stop();
		};
	}, [streams, appId, key]);
	return states;
}

/** Session states in which a sample can arrive (or is on its way). */
const SESSION_UP = new Set(["live", "renewing", "connecting", "reconnecting"]);
const AWAY = new Set(["offline", "never"]);

function keysProblem(device: DeviceFacts): NoLive | undefined {
	if (!device.vault) return "nokeys";
	return keysLocked(device.keys) ? "locked" : undefined;
}

function noLiveOf(
	device: DeviceFacts,
	gate: ReturnType<typeof evaluateGate>,
): NoLive | undefined {
	const keys = keysProblem(device);
	if (keys) return keys;
	if (!gate.ok && gate.kind === "noaccess") return "noaccess";
	const kind = device.live.kind;
	if (kind === "live" || kind === "renewing") return undefined;
	if (AWAY.has(device.presence.kind)) return "offline";
	return SESSION_UP.has(kind) ? undefined : "idle";
}

function noLiveText(t: DevicesT, reason: NoLive, device: string): string {
	const texts: Record<NoLive, string> = {
		offline: t(
			"devices:observe.app.offline",
			"{{device}} is offline. App metrics need a live connection.",
			{ device },
		),
		locked: t(
			"devices:observe.app.locked",
			"Locked. App metrics need the keys and a live connection to {{device}}.",
			{ device },
		),
		nokeys: t(
			"devices:observe.app.noKeys",
			"This computer has no keys for {{device}}.",
			{ device },
		),
		idle: t(
			"devices:observe.app.idle",
			"Encrypted status only. App metrics need a live connection to {{device}}.",
			{ device },
		),
		noaccess: t(
			"devices:observe.app.noAccess",
			"Your access to {{device}} doesn't include Read metrics.",
			{ device },
		),
	};
	return texts[reason];
}

function RowAction({ row }: Readonly<{ row: Row }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const id = row.device.id;
	if (row.noLive === "offline")
		return (
			<DvButton
				size="xs"
				icon={Stethoscope}
				onClick={() => overlay.openDiagnose(id)}
			>
				{t("devices:observe.app.diagnose", "Diagnose")}
			</DvButton>
		);
	if (row.noLive === "locked")
		return (
			<DvButton
				size="xs"
				icon={LockOpen}
				onClick={() => overlay.openUnlock(id, { connectLive: true })}
			>
				{t("devices:observe.app.unlock", "Unlock…")}
			</DvButton>
		);
	if (row.noLive === "idle") return <ConnectButton deviceId={id} size="xs" />;
	return null;
}

/** APP §2.14 "App metrics": the app's services sampled live on each device, and totalled. */
export function AppMetricsBlock({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const overlay = useOverlay();
	const sources = useAttentionState();
	const { role, loading: roleLoading } = useProjectRole(appId);
	const hidden = role !== undefined && !role.readBoards;
	const appName =
		useAppNames()(appId) ?? t("devices:observe.app.thisApp", "this app");
	const [all, setAll] = useState(false);
	const { input } = sources;

	const base = useMemo(
		() =>
			fleetFacts(input).devices.flatMap((device): Row[] => {
				if (!device.active) return [];
				const readable = Array.isArray(device.services) ? device.services : [];
				const own = readable.filter((row) => row.projectId === appId);
				const remembered = (
					keysLocked(device.keys)
						? (device.keys.lockedSummary?.services ?? [])
						: []
				).filter((row) => row.projectId === appId);
				const listed = own.length ? own : remembered;
				if (!listed.length) return [];
				const running = listed.filter((row) => row.desired === "running");
				const gate = evaluateGate(
					"project_metrics",
					buildGateContext(sources, device.id, {
						projectId: appId,
						...(role ? { projectRole: role } : {}),
						extra: { hadPlacementInApp: true },
					}),
				);
				return [
					{
						device,
						services: listed,
						running,
						ready: own.reduce((count, row) => count + row.instances.ready, 0),
						wanted: own
							.filter((row) => row.desired === "running")
							.reduce((count, row) => count + row.instances.requested, 0),
						noLive: noLiveOf(device, gate),
						sampled: 0,
					},
				];
			}),
		[input, sources, appId, role],
	);
	const liveIds =
		hidden || roleLoading
			? []
			: base
					.filter((row) => row.running.length && !row.noLive)
					.map((row) => row.device.id);
	const states = useProjectSamples(appId, liveIds);

	if (hidden) return null;

	const rows = base.map((row): Row => {
		const state = states[row.device.id];
		const sample = row.noLive
			? undefined
			: metricSamples(state?.data).at(-1)?.data;
		const coverage = asRecord(sample?.metric_coverage);
		return {
			...row,
			...(sample ? { sample } : {}),
			sampled: Math.min(
				row.running.length,
				whole(coverage?.sampled_placements) ?? 0,
			),
		};
	});
	const running = rows.reduce((count, row) => count + row.running.length, 0);
	const sampled = rows.reduce((count, row) => count + row.sampled, 0);
	const services = rows.reduce((count, row) => count + row.services.length, 0);
	const complete = running > 0 && sampled === running;
	const totals = totalsOf(
		rows.flatMap((row) => (row.sample && row.sampled ? [row.sample] : [])),
	);
	const ready = rows.reduce((count, row) => count + row.ready, 0);
	const wanted = rows.reduce((count, row) => count + row.wanted, 0);
	const allLocked =
		rows.length > 0 &&
		rows.every((row) => row.noLive === "locked" || row.noLive === "nokeys");
	const one = new Intl.NumberFormat(time.locale, {
		minimumFractionDigits: 1,
		maximumFractionDigits: 1,
	});
	const count = new Intl.NumberFormat(time.locale);
	const rate = (value: number) =>
		t("devices:observe.app.rate", "{{size}}/s", { size: bytesText(value) });
	const cover =
		sampled > 0 && !complete
			? t(
					"devices:observe.app.coverNote",
					" · from {{sampled, number}} of {{running, number}} services",
					{ sampled, running },
				)
			: "";

	const labels = {
		device: t("devices:observe.app.col.device", "Device"),
		sampled: t("devices:observe.app.col.sampled", "Services sampled"),
		cpu: t("devices:observe.app.col.cpu", "CPU"),
		memory: t("devices:observe.app.col.memory", "Memory"),
		instances: t("devices:observe.app.col.instances", "Instances"),
		disk: t("devices:observe.app.col.disk", "Disk read / written"),
		runs: t("devices:observe.app.col.runs", "Runs"),
		io: t("devices:observe.app.io", "Disk I/O"),
		runsCurrent: t(
			"devices:observe.app.runsCurrent",
			"Runs since instances started",
		),
	};
	const dash = <span className="text-muted-foreground">–</span>;
	const instancesCell = (
		<Metric
			label={labels.instances}
			value={count.format(ready)}
			unit={t("devices:observe.app.ofReady", "of {{wanted, number}} ready", {
				wanted,
			})}
			note={t(
				"devices:observe.app.instancesNote",
				"Requested instances of running services, from each device's status",
			)}
		/>
	);
	const [memorySize, memoryUnit] =
		totals.memory === undefined ? ["", ""] : bytesParts(totals.memory);
	const [writtenSize, writtenUnit] =
		totals.written === undefined ? ["", ""] : bytesParts(totals.written);

	const retainedKnown =
		totals.retainedStarted !== undefined && totals.since !== undefined;
	const cells: ReactNode =
		running === 0 ? null : sampled === 0 ? (
			<MetricGrid className={FIVE_CELLS}>
				<Unavailable label={labels.cpu} />
				<Unavailable label={labels.memory} />
				{instancesCell}
				<Unavailable label={labels.io} />
				<Unavailable label={labels.runsCurrent} />
			</MetricGrid>
		) : (
			<MetricGrid className={retainedKnown ? SIX_CELLS : FIVE_CELLS}>
				{totals.cpu === undefined ? (
					<Unavailable label={labels.cpu} />
				) : (
					<Metric
						label={labels.cpu}
						value={one.format(totals.cpu)}
						unit={enumLabel(t, "cpuBasis", "one_logical_cpu")}
						note={t(
							"devices:observe.app.cpuNote",
							"Sum of the app's services · can exceed 100 % on multi-core devices{{cover}}",
							{ cover },
						)}
					/>
				)}
				{totals.memory === undefined ? (
					<Unavailable label={labels.memory} />
				) : (
					<Metric
						label={labels.memory}
						value={memorySize}
						unit={memoryUnit}
						note={t(
							"devices:observe.app.memoryNote",
							"Summed over the app's instances{{cover}}",
							{ cover },
						)}
					/>
				)}
				{instancesCell}
				{totals.written === undefined || totals.read === undefined ? (
					<Unavailable label={labels.io} />
				) : (
					<Metric
						label={labels.io}
						value={writtenSize}
						unit={t("devices:observe.app.writtenUnit", "{{unit}}/s written", {
							unit: writtenUnit,
						})}
						note={t("devices:observe.app.ioNote", "{{read}} read{{cover}}", {
							read: rate(totals.read),
							cover,
						})}
					/>
				)}
				{totals.started === undefined ? (
					<Unavailable label={labels.runsCurrent} />
				) : (
					<Metric
						label={labels.runsCurrent}
						value={count.format(totals.started)}
						note={t(
							"devices:observe.app.runsNote",
							"{{succeeded, number}} succeeded · {{failed, number}} failed · {{inFlight, number}} in flight · from {{reported, number}} of {{expected, number}} running instances · Not a billing record",
							{
								succeeded: totals.succeeded,
								failed: totals.failed,
								inFlight: totals.inFlight,
								reported: totals.reported,
								expected: totals.expected,
							},
						)}
					/>
				)}
				{totals.retainedStarted === undefined ||
				totals.since === undefined ? null : (
					<Metric
						label={t("devices:observe.app.runsSince", "Runs since {{date}}", {
							date: dayLabel(time, totals.since),
						})}
						value={count.format(totals.retainedStarted)}
						note={
							totals.tailLoss
								? t(
										"devices:observe.app.retainedNoteLoss",
										"{{succeeded, number}} succeeded · {{failed, number}} failed · kept across restarts · Not a billing record · Some activity may be missing",
										{
											succeeded: totals.retainedSucceeded,
											failed: totals.retainedFailed,
										},
									)
								: t(
										"devices:observe.app.retainedNote",
										"{{succeeded, number}} succeeded · {{failed, number}} failed · kept across restarts · Not a billing record",
										{
											succeeded: totals.retainedSucceeded,
											failed: totals.retainedFailed,
										},
									)
						}
					/>
				)}
			</MetricGrid>
		);

	const missing = rows.flatMap((row): string[] => {
		if (!row.running.length) return [];
		if (row.noLive)
			return row.running.map((service) =>
				row.noLive === "offline"
					? t(
							"devices:observe.app.missingOffline",
							"{{service}} on {{device}} (offline)",
							{
								service: service.serviceId,
								device: row.device.name,
							},
						)
					: t(
							"devices:observe.app.missingNoLive",
							"{{service}} on {{device}} (no live connection)",
							{ service: service.serviceId, device: row.device.name },
						),
			);
		const lacking = row.running.length - row.sampled;
		return lacking > 0
			? [
					t("devices:observe.app.missingSample", {
						count: lacking,
						device: row.device.name,
						defaultValue_one:
							"{{count, number}} service on {{device}} (no fresh sample)",
						defaultValue_other:
							"{{count, number}} services on {{device}} (no fresh sample)",
					}),
				]
			: [];
	});
	const list = new Intl.ListFormat(time.locale, { type: "conjunction" });
	const missingText =
		missing.length <= NAMES
			? list.format(missing)
			: t(
					"devices:observe.app.missingMore",
					"{{names}} and {{count, number}} more",
					{
						names: missing.slice(0, NAMES - 1).join(", "),
						count: missing.length - (NAMES - 1),
					},
				);

	const shown = all ? rows : rows.slice(0, ROWS);
	const cellOr = (row: Row, value: ReactNode) =>
		row.sampled === row.running.length && row.sample ? value : dash;
	const table = rows.length ? (
		<DvTable
			label={t("devices:observe.app.tableLabel", "App metrics per device")}
			cols={["20%", "20%", "11%", "12%", "11%", "15%", "11%"]}
			head={
				<tr>
					<Th>{labels.device}</Th>
					<Th>{labels.sampled}</Th>
					<Th numeric>{labels.cpu}</Th>
					<Th numeric>{labels.memory}</Th>
					<Th numeric>{labels.instances}</Th>
					<Th numeric className={TWO_LINE_HEAD}>
						{labels.disk}
					</Th>
					<Th numeric>{labels.runs}</Th>
				</tr>
			}
		>
			{shown.map((row) => {
				const { device } = row;
				const name = (
					<a
						className={LINK}
						{...link({
							screen: "device",
							deviceId: device.id,
							tab: "metrics",
						})}
					>
						{device.name}
					</a>
				);
				if (!row.running.length)
					return (
						<Tr key={device.id} data-app-row="none">
							<Td label={labels.device} kind="name">
								{name}
							</Td>
							<Td label={labels.sampled} colSpan={6}>
								<span className="text-muted-foreground">
									{t("devices:observe.app.noneRunning", {
										count: row.services.length,
										services: list.format(
											row.services.map((service) => service.serviceId),
										),
										defaultValue_one:
											"No running services here: {{services}} is stopped.",
										defaultValue_other:
											"No running services here: {{services}} are stopped.",
									})}
								</span>
							</Td>
						</Tr>
					);
				if (row.noLive)
					return (
						<Tr key={device.id} data-app-row="nolive">
							<Td label={labels.device} kind="name">
								{name}
							</Td>
							<Td label={labels.sampled} colSpan={6}>
								<span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1.5">
									<GateInline
										kind={
											row.noLive === "locked"
												? "locked"
												: row.noLive === "nokeys"
													? "nokeys"
													: row.noLive === "noaccess"
														? "noaccess"
														: "live"
										}
										className="max-w-[60ch]"
									>
										{noLiveText(t, row.noLive, device.name)}
									</GateInline>
									<RowAction row={row} />
								</span>
							</Td>
						</Tr>
					);
				const sample = row.sample ?? {};
				const io = asRecord(sample.io);
				const cpu = amount(sample.cpu_percent);
				const memory = amount(sample.memory_bytes);
				const read = amount(io?.read_bytes_per_second);
				const written = amount(io?.written_bytes_per_second);
				const started = whole(asRecord(sample.usage)?.invocations_started);
				return (
					<Tr key={device.id} data-app-row="live">
						<Td label={labels.device} kind="name">
							{name}
						</Td>
						<Td label={labels.sampled}>
							{t(
								"devices:observe.app.sampledOf",
								"{{sampled, number}} of {{running, number}}",
								{ sampled: row.sampled, running: row.running.length },
							)}
							{row.sampled < row.running.length ? (
								<CellSub className="text-warning">
									{row.sample
										? t(
												"devices:observe.app.noFreshSample",
												"no fresh sample for the rest",
											)
										: t(
												"devices:observe.app.readingSample",
												"reading the first sample…",
											)}
								</CellSub>
							) : null}
						</Td>
						<Td label={labels.cpu} kind="num">
							{cellOr(row, cpu === undefined ? dash : `${one.format(cpu)} %`)}
						</Td>
						<Td label={labels.memory} kind="num">
							{cellOr(row, memory === undefined ? dash : bytesText(memory))}
						</Td>
						<Td label={labels.instances} kind="num">
							{t(
								"devices:observe.app.readyOf",
								"{{ready, number}} of {{wanted, number}}",
								{ ready: row.ready, wanted: row.wanted },
							)}
						</Td>
						<Td label={labels.disk} kind="num">
							{cellOr(
								row,
								read === undefined || written === undefined ? (
									dash
								) : (
									<>
										{rate(read)}
										<CellSub className={SUB}>
											{t("devices:observe.app.written", "{{rate}} written", {
												rate: rate(written),
											})}
										</CellSub>
									</>
								),
							)}
						</Td>
						<Td label={labels.runs} kind="num">
							{cellOr(
								row,
								started === undefined ? dash : count.format(started),
							)}
						</Td>
					</Tr>
				);
			})}
			{rows.length > shown.length ? (
				<tr data-app-row="more">
					<td colSpan={7} className="border-t border-hairline px-4 py-2">
						<span className="inline-flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
							{t(
								"devices:observe.app.showing",
								"Showing {{shown, number}} of {{count, number}} devices. The total covers all of them.",
								{ shown: shown.length, count: rows.length },
							)}
							<DvButton size="xs" variant="ghost" onClick={() => setAll(true)}>
								{t("devices:observe.app.showAll", "Show all devices")}
							</DvButton>
						</span>
					</td>
				</tr>
			) : null}
			{rows.length > 1 && sampled > 0 ? (
				<Tr data-app-row="total" className="bg-surface-sunken font-medium">
					<Td label={labels.device}>
						{t("devices:observe.app.total", "Total")}
					</Td>
					<Td label={labels.sampled}>
						{t(
							"devices:observe.app.sampledOf",
							"{{sampled, number}} of {{running, number}}",
							{ sampled, running },
						)}
					</Td>
					<Td label={labels.cpu} kind="num">
						{totals.cpu === undefined ? dash : `${one.format(totals.cpu)} %`}
					</Td>
					<Td label={labels.memory} kind="num">
						{totals.memory === undefined ? dash : bytesText(totals.memory)}
					</Td>
					<Td label={labels.instances} kind="num">
						{t(
							"devices:observe.app.readyOf",
							"{{ready, number}} of {{wanted, number}}",
							{
								ready,
								wanted,
							},
						)}
					</Td>
					<Td label={labels.disk} kind="num">
						{totals.read === undefined || totals.written === undefined ? (
							dash
						) : (
							<>
								{rate(totals.read)}
								<CellSub className={SUB}>
									{t("devices:observe.app.written", "{{rate}} written", {
										rate: rate(totals.written),
									})}
								</CellSub>
							</>
						)}
					</Td>
					<Td label={labels.runs} kind="num">
						{totals.started === undefined ? dash : count.format(totals.started)}
					</Td>
				</Tr>
			) : null}
		</DvTable>
	) : null;

	const padded = (node: ReactNode) => <div className="px-4 py-3">{node}</div>;
	let body: ReactNode;
	if (allLocked)
		body = padded(
			<StateView
				kind="locked"
				icon={Gauge}
				title={t(
					"devices:observe.app.lockedTitle",
					"Metrics need unlocked devices",
				)}
				text={t(
					"devices:observe.app.lockedText",
					"Samples come from each device over a live connection. Unlock the devices that run {{app}} first.",
					{ app: appName },
				)}
				actions={
					<DvButton
						size="sm"
						icon={LockOpen}
						onClick={() => overlay.openUnlockSeveral()}
					>
						{t("devices:observe.app.unlockSeveral", "Unlock several…")}
					</DvButton>
				}
			/>,
		);
	else if (!services)
		body = padded(
			<StateView
				kind="empty"
				icon={Gauge}
				title={t("devices:observe.app.nothing", "Nothing to measure")}
				text={t(
					"devices:observe.app.nothingText",
					"{{app}} doesn't run on any device you can read.",
					{ app: appName },
				)}
			/>,
		);
	else if (!running)
		body = (
			<>
				{padded(
					<StateView
						kind="empty"
						icon={CirclePause}
						title={t(
							"devices:observe.app.noRunning",
							"No running services to sample",
						)}
						text={t(
							"devices:observe.app.noRunningText",
							"Every service of {{app}} is stopped, as you asked. Metrics appear once one runs.",
							{ app: appName },
						)}
					/>,
				)}
				{table}
			</>
		);
	else
		body = (
			<>
				{missing.length ? (
					<p
						data-app-coverage=""
						className="flex items-start gap-1.5 border-b border-hairline px-4 py-2.5 text-xs text-muted-foreground"
					>
						<CircleDashed aria-hidden className="mt-0.5 size-3.25 shrink-0" />
						<span>
							{sampled
								? t(
										"devices:observe.app.notSampledSome",
										"Not sampled: {{missing}}. Totals cover the sampled services only.",
										{ missing: missingText },
									)
								: t(
										"devices:observe.app.notSampledAll",
										"Not sampled: {{missing}}. CPU and memory stay unavailable until every running service has a fresh sample.",
										{ missing: missingText },
									)}
						</span>
					</p>
				) : null}
				{cells}
				{table}
			</>
		);

	const stampText = t("devices:observe.app.stamp", {
		count: running,
		sampled,
		defaultValue_one:
			"{{sampled, number}} of {{count, number}} running service sampled",
		defaultValue_other:
			"{{sampled, number}} of {{count, number}} running services sampled",
	});
	return (
		<Block
			id="observe-app-metrics"
			className={PLAIN_PARAGRAPHS}
			icon={Gauge}
			title={t("devices:observe.app.title", "App metrics")}
			stamp={
				allLocked ? (
					<FreshnessStamp source="live" age="locked" />
				) : !running ? (
					<FreshnessStamp
						source="live"
						age="notloaded"
						text={
							services
								? t("devices:observe.app.stampNoRunning", "no running services")
								: t(
										"devices:observe.app.stampNothing",
										"nothing deployed to sample",
									)
						}
					/>
				) : (
					<FreshnessStamp
						source="live"
						age={complete ? "live" : sampled ? "delayed" : "notloaded"}
						text={stampText}
					/>
				)
			}
			flush
			foot={t(
				"devices:observe.app.foot",
				"CPU counts one logical CPU as 100 %. Values come from each device's latest live sample; this block shows no trends.",
			)}
		>
			{body}
		</Block>
	);
}
