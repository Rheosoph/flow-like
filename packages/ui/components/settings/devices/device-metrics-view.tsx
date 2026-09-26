"use client";

import { useEffect, useState } from "react";

function object(value: unknown): Record<string, unknown> | undefined {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function amount(value: unknown): number | undefined {
	return typeof value === "number" && Number.isFinite(value) && value >= 0
		? value
		: undefined;
}

function count(value: unknown): string {
	const number = amount(value);
	return number !== undefined && Number.isSafeInteger(number)
		? number.toLocaleString()
		: "Unavailable";
}

function percent(value: unknown): string {
	const number = amount(value);
	return number === undefined ? "Unavailable" : `${number.toFixed(1)}%`;
}

function bytes(value: unknown): string {
	const number = amount(value);
	if (number === undefined || !Number.isSafeInteger(number))
		return "Unavailable";
	const units = ["B", "KiB", "MiB", "GiB", "TiB"];
	const unit = Math.min(Math.floor(Math.log2(Math.max(number, 1)) / 10), 4);
	return `${(number / 1024 ** unit).toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function Metric({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded border p-3">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="mt-1 font-medium tabular-nums">{value}</dd>
		</div>
	);
}

export function DeviceMetricsView({
	sample,
	placement,
	connected,
}: {
	sample: Record<string, unknown> | undefined;
	placement: boolean;
	connected: boolean;
}) {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 5000);
		return () => clearInterval(timer);
	}, []);
	const records = Array.isArray(sample?.records) ? sample.records : [];
	const record = object(records.at(-1));
	const data = object(record?.data);
	const timestamp = amount(record?.timestamp);
	if (!data || timestamp === undefined)
		return (
			<p className="text-sm text-muted-foreground">
				Waiting for a device sample…
			</p>
		);
	const age = now / 1000 - timestamp;
	const stale = !connected || age > 15 || age < -30;
	const usage = object(data.usage);
	const retained = object(data.usage_retained);
	const retainedCounters = object(retained?.counters);
	const coverage = object(retained?.coverage);
	const metricCoverage = object(data.metric_coverage);
	const io = object(data.io);
	const disk = object(data.disk_quota);
	const volume = object(data.storage_volume);
	const network = object(data.network);
	const replicas = Array.isArray(data.replicas)
		? data.replicas.slice(0, 32)
		: [];
	return (
		<div className="space-y-3">
			<p className="text-xs text-muted-foreground">
				{stale ? "Last known sample" : "Latest sample"}:{" "}
				{new Date(timestamp * 1000).toLocaleTimeString()}.
				{stale &&
					" Reconnect or wait for a fresh sample before treating these values as current."}
			</p>
			<dl className="grid grid-cols-2 gap-2 sm:grid-cols-3">
				<Metric
					label={placement ? "CPU (100% per core)" : "Device CPU"}
					value={percent(data.cpu_percent)}
				/>
				<Metric
					label={placement ? "Process memory" : "Device memory used"}
					value={bytes(placement ? data.memory_bytes : data.memory_used_bytes)}
				/>
				<Metric
					label="Ready / requested replicas"
					value={`${count(data.ready_replicas)} / ${count(data.desired_replicas)}`}
				/>
				{placement && (
					<Metric
						label="Processes measured / running"
						value={`${count(data.processes_observed)} / ${count(data.running_replicas)}`}
					/>
				)}
				{!placement && (
					<Metric label="Agent memory" value={bytes(data.agent_memory_bytes)} />
				)}
				{!placement && (
					<Metric
						label="Agent CPU (100% per core)"
						value={percent(data.agent_cpu_percent_of_one_core)}
					/>
				)}
				{!placement && (
					<Metric
						label="Device memory capacity"
						value={bytes(data.memory_total_bytes)}
					/>
				)}
				{io && (
					<>
						<Metric
							label="Storage read / second"
							value={
								amount(io.read_bytes_per_second) === undefined
									? "Unavailable"
									: `${bytes(Math.round(Number(io.read_bytes_per_second)))}/s`
							}
						/>
						<Metric
							label="Storage written / second"
							value={
								amount(io.written_bytes_per_second) === undefined
									? "Unavailable"
									: `${bytes(Math.round(Number(io.written_bytes_per_second)))}/s`
							}
						/>
					</>
				)}
				{disk && (
					<Metric
						label="Placement disk used / limit"
						value={`${bytes(disk.used_bytes)} / ${bytes(disk.limit_bytes)}`}
					/>
				)}
				{volume && (
					<Metric
						label="State volume available / capacity"
						value={`${bytes(volume.available_bytes)} / ${bytes(volume.total_bytes)}`}
					/>
				)}
				{network && (
					<>
						<Metric
							label="Host interfaces received / sample"
							value={bytes(network.received_bytes)}
						/>
						<Metric
							label="Host interfaces sent / sample"
							value={bytes(network.transmitted_bytes)}
						/>
					</>
				)}
			</dl>
			{network && (
				<p className="text-xs text-muted-foreground">
					Network counters cover all host interfaces, including virtual
					interfaces. They can count the same traffic more than once and are not
					project traffic or billing.
				</p>
			)}
			{io && (
				<p className="text-xs text-muted-foreground">
					Storage I/O covers{" "}
					{io.basis === "cgroup_block_io"
						? "the isolated process group"
						: "observed worker processes"}
					. A restart or missing counter leaves the rate unavailable.
				</p>
			)}
			{metricCoverage && (
				<p className="text-xs text-muted-foreground">
					Project process measurements include{" "}
					{count(metricCoverage.sampled_placements)} placements with fresh
					samples. {count(metricCoverage.missing_or_stale_placements)} running
					placements have missing or stale samples. CPU and memory remain
					unavailable when their aggregate coverage is incomplete.
				</p>
			)}
			{usage && (
				<div className="space-y-2">
					<p className="text-xs text-muted-foreground">
						Service lifetimes and invocations from{" "}
						{count(usage.reported_replicas)} of {count(usage.expected_replicas)}{" "}
						running replicas. Totals cover the current process lifetimes and
						reset when those processes restart.
					</p>
					<dl className="grid grid-cols-2 gap-2 sm:grid-cols-3">
						<Metric
							label="Lifetimes / invocations started"
							value={count(usage.invocations_started)}
						/>
						<Metric
							label="Lifetimes / invocations active"
							value={count(usage.in_flight)}
						/>
						<Metric
							label="Runtime messages"
							value={count(usage.runtime_messages)}
						/>
						<Metric
							label="Lifetimes / invocations completed"
							value={count(usage.invocations_succeeded)}
						/>
						<Metric
							label="Lifetimes / invocations failed"
							value={count(usage.invocations_failed)}
						/>
						<Metric
							label="Lifetimes / invocations cancelled"
							value={count(usage.invocations_cancelled)}
						/>
					</dl>
				</div>
			)}
			{retained && (
				<div className="space-y-2 rounded border p-3">
					<p className="text-sm font-medium">Retained service usage</p>
					<p className="text-xs text-muted-foreground">
						Observed service counters survive process restarts. They cover
						reports received by this device and are separate from billing.
						{typeof retained.since === "number" &&
							` Recording since ${new Date(retained.since * 1000).toLocaleString()}.`}
					</p>
					<dl className="grid grid-cols-2 gap-2 sm:grid-cols-3">
						<Metric
							label="Lifetimes / invocations started"
							value={count(retainedCounters?.invocations_started)}
						/>
						<Metric
							label="Lifetimes / invocations completed"
							value={count(retainedCounters?.invocations_succeeded)}
						/>
						<Metric
							label="Lifetimes / invocations failed"
							value={count(retainedCounters?.invocations_failed)}
						/>
						<Metric
							label="Lifetimes / invocations cancelled"
							value={count(retainedCounters?.invocations_cancelled)}
						/>
						<Metric
							label="Runtime messages"
							value={count(retainedCounters?.runtime_messages)}
						/>
						<Metric
							label="Concurrency rejections"
							value={count(retainedCounters?.concurrency_rejections)}
						/>
						<Metric
							label="Request payload"
							value={bytes(retainedCounters?.request_payload_bytes)}
						/>
						<Metric
							label="Response payload"
							value={bytes(retainedCounters?.response_payload_bytes)}
						/>
					</dl>
					<p className="text-xs text-muted-foreground">
						{count(coverage?.reported_runs)} of{" "}
						{count(coverage?.registered_runs)} registered process runs reported
						counters. {count(coverage?.finalized_runs)} have a final checkpoint;{" "}
						{count(coverage?.incomplete_runs)} ended without one.{" "}
						{count(coverage?.unreported_runs)} ended before their first report.{" "}
						{count(coverage?.stale_active_runs)} active reports are stale and{" "}
						{count(coverage?.awaiting_first_report_runs)} are awaiting their
						first report.
						{retained.tail_loss_possible === true &&
							" Some activity may be missing after the last accepted checkpoint."}
					</p>
				</div>
			)}
			{replicas.length > 0 && (
				<details className="rounded border p-3 text-sm">
					<summary className="cursor-pointer">Replica samples</summary>
					<table className="mt-2 w-full text-left tabular-nums">
						<thead>
							<tr>
								<th>Slot</th>
								<th>CPU (100% per core)</th>
								<th>Memory</th>
							</tr>
						</thead>
						<tbody>
							{replicas.map((value) => {
								const replica = object(value);
								const slot = replica?.slot;
								if (
									typeof slot !== "number" ||
									!Number.isInteger(slot) ||
									slot < 0 ||
									slot > 31
								)
									return null;
								return (
									<tr key={slot}>
										<td>{slot}</td>
										<td>{percent(replica?.cpu_percent)}</td>
										<td>{bytes(replica?.memory_bytes)}</td>
									</tr>
								);
							})}
						</tbody>
					</table>
				</details>
			)}
		</div>
	);
}
