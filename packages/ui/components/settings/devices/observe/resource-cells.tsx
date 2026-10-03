"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import type { ServiceView } from "../../../../lib/device-management/model/types";
import { enumLabel } from "../copy/enum-labels";
import { type AreaTime, useAreaTime } from "../primitives/area-context";
import { Metric, MetricGrid, type SparklineProps } from "../primitives/metric";
import {
	GIB,
	type Json,
	type MetricSample,
	amount,
	asRecord,
	bytesParts,
	bytesText,
	seriesOf,
	spanOf,
	stepOf,
	whole,
} from "./observe-data";

const DASH = "–";
/** A service has six values: three wide columns with room for the trends, two and one as the area narrows. */
const SERVICE_GRID =
	"grid-cols-1 @min-[560px]/devices:grid-cols-2 @min-[1100px]/devices:grid-cols-3";

/** The device's own history when it reaches further back than the samples at hand. */
const longer = (
	history: readonly MetricSample[] | undefined,
	samples: readonly MetricSample[],
) => (history && spanOf(history) > spanOf(samples) ? history : samples);

/** A value the device didn't report: the cell stays, it never reads zero. */
export function Unavailable({ label }: Readonly<{ label: ReactNode }>) {
	const { t } = useTranslation("devices");
	return (
		<Metric
			label={label}
			value={DASH}
			note={t("observe.metrics.unavailable", "Unavailable right now")}
			className="text-muted-foreground"
		/>
	);
}

interface Trend {
	spark(
		samples: readonly MetricSample[],
		label: string,
		read: (data: Json) => number | undefined,
		format: (value: number) => string,
		range?: { min?: number; max?: number },
	): SparklineProps | undefined;
	axis(
		samples: readonly MetricSample[],
		middle: string,
	): readonly ReactNode[] | undefined;
}

function trendOf(time: AreaTime, lastKnown: boolean, nowLabel: string): Trend {
	return {
		spark(samples, label, read, format, range = {}) {
			if (samples.length < 2) return undefined;
			return {
				series: seriesOf(samples, read),
				label,
				format,
				startAt: samples[0]?.at,
				stepSec: stepOf(samples),
				...range,
			};
		},
		axis(samples, middle) {
			const first = samples[0];
			const last = samples.at(-1);
			if (!first || !last || samples.length < 2) return undefined;
			return [
				time.clock(first.at, false),
				middle,
				lastKnown ? time.clock(last.at, false) : nowLabel,
			];
		},
	};
}

function useFormats() {
	const time = useAreaTime();
	const one = new Intl.NumberFormat(time.locale, {
		minimumFractionDigits: 1,
		maximumFractionDigits: 1,
	});
	const count = new Intl.NumberFormat(time.locale);
	return { time, one, count };
}

export interface DeviceResourceCellsProps {
	/** Trend and live samples, oldest first; the last one is the value shown. */
	samples: readonly MetricSample[];
	/** The device's own metric history (BG16): a denser trend for CPU and memory. */
	history?: readonly MetricSample[];
	/** The newest value isn't current: the axis ends at its time, not at "now". */
	lastKnown: boolean;
	services: readonly ServiceView[] | null;
}

export function DeviceResourceCells({
	samples,
	history,
	lastKnown,
	services,
}: Readonly<DeviceResourceCellsProps>) {
	const { t } = useTranslation("devices");
	const { time, one, count } = useFormats();
	const latest = samples.at(-1);
	if (!latest) return null;
	const data = latest.data;
	const trend = trendOf(time, lastKnown, t("observe.metrics.now", "now"));
	const dense = longer(history, samples);

	const cpu = amount(data.cpu_percent);
	const cpus = whole(data.logical_cpus);
	const memUsed = amount(data.memory_used_bytes);
	const memTotal = amount(data.memory_total_bytes);
	const volume = asRecord(data.storage_volume);
	const free = amount(volume?.available_bytes);
	const total = amount(volume?.total_bytes);
	const network = asRecord(data.network);
	const received = amount(network?.received_bytes);
	const sent = amount(network?.transmitted_bytes);
	const seconds = amount(data.sample_seconds);
	const agentCpu = amount(data.agent_cpu_percent_of_one_core);
	const agentMemory = amount(data.agent_memory_bytes);
	const placements = whole(data.placements);
	const ready = whole(data.ready_replicas);
	const wanted = whole(data.desired_replicas);
	const stopped = (services ?? [])
		.filter((service) => service.desired === "stopped")
		.map((service) => service.serviceId);

	const labels = {
		cpu: t("observe.metrics.cpu", "CPU"),
		memory: t("observe.metrics.memory", "Memory"),
		disk: enumLabel(t, "resourceBasis", "agent_state_volume"),
		network: t("observe.metrics.network", "Network"),
		agentCpu: t("observe.metrics.agentCpu", "Agent CPU"),
		agentMemory: t("observe.metrics.agentMemory", "Agent memory"),
		services: t("observe.metrics.services", "Services"),
		instances: t("observe.metrics.instances", "Instances"),
		sample: t("observe.metrics.sampleLength", "Sample length"),
	};
	const [receivedSize, receivedUnit] =
		received === undefined ? ["", ""] : bytesParts(received);
	const [agentSize, agentUnit] =
		agentMemory === undefined ? ["", ""] : bytesParts(agentMemory);

	return (
		<MetricGrid>
			{cpu === undefined ? (
				<Unavailable label={labels.cpu} />
			) : (
				<Metric
					label={labels.cpu}
					value={one.format(cpu)}
					unit="%"
					note={
						cpus === undefined
							? t("observe.metrics.cpuNoteAll", "100 % = every CPU is busy")
							: t(
									"observe.metrics.cpuNote",
									"{{count, number}} logical CPUs · 100 % = all of them",
									{ count: cpus },
								)
					}
					spark={trend.spark(
						dense,
						labels.cpu,
						(row) => amount(row.cpu_percent),
						(value) => `${one.format(value)} %`,
						{ min: 0, max: 100 },
					)}
					axis={trend.axis(dense, t("observe.metrics.cpuMax", "max 100 %"))}
				/>
			)}
			{memUsed === undefined || memTotal === undefined ? (
				<Unavailable label={labels.memory} />
			) : (
				<Metric
					label={labels.memory}
					value={one.format(memUsed / GIB)}
					unit={t("observe.metrics.memoryOf", "of {{total}} GiB", {
						total: one.format(memTotal / GIB),
					})}
					note={
						memTotal > 0
							? t("observe.metrics.percentUsed", "{{percent, number}} % used", {
									percent: Math.round((memUsed / memTotal) * 100),
								})
							: undefined
					}
					spark={trend.spark(
						dense,
						labels.memory,
						(row) => {
							const used = amount(row.memory_used_bytes);
							return used === undefined ? undefined : used / GIB;
						},
						(value) => `${one.format(value)} GiB`,
						{ min: 0, max: memTotal / GIB },
					)}
					axis={trend.axis(
						dense,
						t("observe.metrics.memoryMax", "max {{total}} GiB", {
							total: one.format(memTotal / GIB),
						}),
					)}
				/>
			)}
			{free === undefined || total === undefined ? (
				<Unavailable label={labels.disk} />
			) : (
				<Metric
					label={labels.disk}
					value={count.format(Math.round(free / GIB))}
					unit={t("observe.metrics.diskOf", "GiB free of {{total, number}}", {
						total: Math.round(total / GIB),
					})}
					note={
						total > 0
							? t("observe.metrics.percentUsed", "{{percent, number}} % used", {
									percent: Math.round((1 - free / total) * 100),
								})
							: undefined
					}
					spark={trend.spark(
						samples,
						labels.disk,
						(row) => {
							const value = amount(
								asRecord(row.storage_volume)?.available_bytes,
							);
							return value === undefined ? undefined : value / GIB;
						},
						(value) =>
							t("observe.metrics.diskFree", "{{value}} GiB free", {
								value: one.format(value),
							}),
					)}
					axis={trend.axis(
						samples,
						t("observe.metrics.diskAxis", "free space"),
					)}
				/>
			)}
			{received === undefined || sent === undefined ? (
				<Unavailable label={labels.network} />
			) : (
				<Metric
					label={labels.network}
					value={receivedSize}
					unit={t(
						"observe.metrics.networkUnit",
						"{{unit}} received · {{sent}} sent",
						{ unit: receivedUnit, sent: bytesText(sent) },
					)}
					note={
						seconds === undefined
							? t(
									"observe.metrics.networkNote",
									"per sample · all network interfaces · not a billing figure",
								)
							: t(
									"observe.metrics.networkNoteSeconds",
									"per {{seconds, number}} s sample · all network interfaces · not a billing figure",
									{ seconds: Math.round(seconds) },
								)
					}
					spark={trend.spark(
						samples,
						labels.network,
						(row) => amount(asRecord(row.network)?.received_bytes),
						(value) => bytesText(value),
					)}
					axis={trend.axis(
						samples,
						t("observe.metrics.networkAxis", "received per sample"),
					)}
				/>
			)}
			{agentCpu === undefined ? (
				<Unavailable label={labels.agentCpu} />
			) : (
				<Metric
					label={labels.agentCpu}
					value={one.format(agentCpu)}
					unit={t("observe.metrics.ofOneCore", "% of one core")}
					note={t("observe.metrics.agentItself", "The Flow-Like agent itself")}
				/>
			)}
			{agentMemory === undefined ? (
				<Unavailable label={labels.agentMemory} />
			) : (
				<Metric
					label={labels.agentMemory}
					value={agentSize}
					unit={agentUnit}
					note={t(
						"observe.metrics.agentMemoryNote",
						"Process memory of the agent",
					)}
				/>
			)}
			{placements === undefined ? (
				<Unavailable label={labels.services} />
			) : (
				<Metric
					label={labels.services}
					value={count.format(placements)}
					unit={t("observe.metrics.onDevice", "on the device")}
					note={
						services === null
							? undefined
							: stopped.length
								? t(
										"observe.metrics.stoppedList",
										"{{list}} stopped as requested",
										{
											list: new Intl.ListFormat(time.locale, {
												type: "conjunction",
											}).format(stopped),
										},
									)
								: t("observe.metrics.noneStopped", "none stopped")
					}
				/>
			)}
			{ready === undefined || wanted === undefined ? (
				<Unavailable label={labels.instances} />
			) : (
				<Metric
					label={labels.instances}
					value={t(
						"observe.metrics.readyOf",
						"{{ready, number}} of {{wanted, number}}",
						{ ready, wanted },
					)}
					unit={t("observe.metrics.ready", "ready")}
					note={t(
						"observe.metrics.instancesNote",
						"requested instances of every service",
					)}
				/>
			)}
			{seconds === undefined ? null : (
				<Metric
					label={labels.sample}
					value={count.format(Math.round(seconds))}
					unit={t("observe.metrics.secondsUnit", "s")}
					note={t(
						"observe.metrics.sampleNote",
						"The time the latest sample covers",
					)}
				/>
			)}
		</MetricGrid>
	);
}

export interface ServiceResourceCellsProps {
	samples: readonly MetricSample[];
	history?: readonly MetricSample[];
	lastKnown: boolean;
	service: ServiceView;
}

export function ServiceResourceCells({
	samples,
	history,
	lastKnown,
	service,
}: Readonly<ServiceResourceCellsProps>) {
	const { t } = useTranslation("devices");
	const { time, one, count } = useFormats();
	const latest = samples.at(-1);
	if (!latest) return null;
	const data = latest.data;
	const trend = trendOf(time, lastKnown, t("observe.metrics.now", "now"));
	const dense = longer(history, samples);

	const cpu = amount(data.cpu_percent);
	const memory = amount(data.memory_bytes);
	const sandbox = data.memory_basis === "cgroup_current";
	const io = asRecord(data.io);
	const read = amount(io?.read_bytes_per_second);
	const written = amount(io?.written_bytes_per_second);
	const ioSeen = whole(io?.replicas_observed);
	const quota = asRecord(data.disk_quota);
	const quotaUsed = amount(quota?.used_bytes);
	const quotaLimit = amount(quota?.limit_bytes);
	const files = whole(quota?.used_inodes);
	const filesLimit = whole(quota?.limit_inodes);
	const processes = whole(data.processes_observed);
	const replicas = Array.isArray(data.replicas) ? data.replicas : [];
	const threads = replicas.reduce<number | undefined>((sum, row) => {
		const value = whole(asRecord(row)?.processes_and_threads);
		return value === undefined ? sum : (sum ?? 0) + value;
	}, undefined);
	const { requested, ready, running, max } = service.instances;

	const labels = {
		cpu: t("observe.metrics.cpu", "CPU"),
		memory: enumLabel(
			t,
			"memoryBasis",
			sandbox ? "cgroup_current" : "process_rss",
		),
		io: enumLabel(
			t,
			"ioBasis",
			io?.basis === "cgroup_block_io" ? "cgroup_block_io" : "process_io",
		),
		quota: t("observe.metrics.quota", "Disk quota"),
		processes: t("observe.metrics.processes", "Processes"),
		instances: t("observe.metrics.instances", "Instances"),
	};
	const [memorySize, memoryUnit] =
		memory === undefined ? ["", ""] : bytesParts(memory);
	const [readSize, readUnit] = read === undefined ? ["", ""] : bytesParts(read);
	const [quotaSize, quotaUnit] =
		quotaUsed === undefined ? ["", ""] : bytesParts(quotaUsed);

	return (
		<MetricGrid className={SERVICE_GRID}>
			{cpu === undefined ? (
				<Unavailable label={labels.cpu} />
			) : (
				<Metric
					label={labels.cpu}
					value={one.format(cpu)}
					unit={enumLabel(t, "cpuBasis", "one_logical_cpu")}
					note={t(
						"observe.metrics.cpuExceeds",
						"Can exceed 100 % on multi-core devices",
					)}
					spark={trend.spark(
						dense,
						labels.cpu,
						(row) => amount(row.cpu_percent),
						(value) => `${one.format(value)} %`,
						{ min: 0 },
					)}
					axis={trend.axis(dense, enumLabel(t, "cpuBasis", "one_logical_cpu"))}
				/>
			)}
			{memory === undefined ? (
				<Unavailable label={labels.memory} />
			) : (
				<Metric
					label={labels.memory}
					value={memorySize}
					unit={memoryUnit}
					note={
						sandbox
							? t(
									"observe.metrics.memorySandbox",
									"Everything inside the service's sandbox",
								)
							: t(
									"observe.metrics.memoryProcess",
									"Resident memory of its processes · no limit (runs as the agent)",
								)
					}
					spark={trend.spark(
						dense,
						labels.memory,
						(row) => amount(row.memory_bytes),
						(value) => bytesText(value),
						{ min: 0 },
					)}
					axis={trend.axis(dense, labels.memory)}
				/>
			)}
			{read === undefined || written === undefined ? (
				<Unavailable label={labels.io} />
			) : (
				<Metric
					label={labels.io}
					value={readSize}
					unit={t(
						"observe.metrics.ioUnit",
						"{{unit}}/s read · {{written}}/s written",
						{ unit: readUnit, written: bytesText(written) },
					)}
					note={
						ioSeen === undefined
							? t("observe.metrics.ioNote", "per second")
							: t(
									"observe.metrics.ioNoteSeen",
									"per second · {{seen, number}} of {{running, number}} instances reported",
									{ seen: ioSeen, running },
								)
					}
				/>
			)}
			{quotaUsed !== undefined && quotaLimit !== undefined ? (
				<Metric
					label={labels.quota}
					value={quotaSize}
					unit={t("observe.metrics.quotaOf", "{{unit}} of {{limit}}", {
						unit: quotaUnit,
						limit: bytesText(quotaLimit),
					})}
					note={
						files !== undefined && filesLimit !== undefined
							? t(
									"observe.metrics.quotaFiles",
									"{{files, number}} of {{limit, number}} files · {{percent, number}} % used",
									{
										files,
										limit: filesLimit,
										percent:
											quotaLimit > 0
												? Math.round((quotaUsed / quotaLimit) * 100)
												: 0,
									},
								)
							: undefined
					}
				/>
			) : sandbox ? (
				<Unavailable label={labels.quota} />
			) : (
				<Metric
					label={labels.quota}
					value={DASH}
					unit={t("observe.metrics.noLimit", "no limit")}
					note={t(
						"observe.metrics.noQuota",
						"Runs as the agent: no disk quota applies",
					)}
				/>
			)}
			{processes === undefined ? (
				<Unavailable label={labels.processes} />
			) : (
				<Metric
					label={labels.processes}
					value={count.format(processes)}
					unit={t("observe.metrics.observed", "observed")}
					note={
						threads === undefined
							? undefined
							: t(
									"observe.metrics.threads",
									"{{count, number}} processes and threads in total",
									{ count: threads },
								)
					}
				/>
			)}
			<Metric
				label={labels.instances}
				value={count.format(ready)}
				unit={t("observe.metrics.ofReady", "of {{requested, number}} ready", {
					requested,
				})}
				note={t(
					"observe.metrics.runningMax",
					"{{running, number}} running · max {{max, number}}",
					{ running, max },
				)}
			/>
		</MetricGrid>
	);
}
