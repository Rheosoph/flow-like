"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Box, CirclePause, Gauge } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { classify } from "../../../../lib/device-management/model/freshness";
import type {
	DevicesScope,
	Freshness,
} from "../../../../lib/device-management/model/types";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvTable, Td, Th, Tr } from "../primitives/dv-table";
import {
	FreshnessStamp,
	type FreshnessStampProps,
} from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { LOCKED_DATA_CLASS, StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useFleetDeviceStates, useGate, useLiveStream } from "../workspace";
import { LiveDataState, livePhase, liveWanted } from "./live-state";
import {
	type MetricSample,
	OBSERVE_STACK,
	TABLE_RESET,
	amount,
	asRecord,
	bytesText,
	mergeSamples,
	metricSamples,
	spanOf,
	whole,
	word,
} from "./observe-data";
import { DeviceResourceCells, ServiceResourceCells } from "./resource-cells";
import { RetainedHistory } from "./retained-history";
import { SharedLiveMetrics } from "./shared-live-metrics";
import { UsageBlocks } from "./usage-block";
import { useHistoryPanel } from "./use-history-panel";
import { useMetricsHistory } from "./use-metrics-history";
import {
	type ObserveTarget,
	readRefusal,
	useObserveTarget,
} from "./use-observe-target";

const KEEP_LIVE = 360;
const LINK =
	"text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-foreground";

/** Live reads return the newest sample only: the ones seen while the tab is open make a trend. */
function useSeen(live: readonly MetricSample[], key: string): MetricSample[] {
	const [seen, setSeen] = useState<{ key: string; rows: MetricSample[] }>({
		key,
		rows: [],
	});
	useEffect(() => {
		setSeen((known) => {
			const same = known.key === key;
			const rows = same ? known.rows : [];
			if (!live.length) return same && !rows.length ? known : { key, rows: [] };
			const newest = rows.at(-1)?.at ?? 0;
			const fresh = live.filter((sample) => sample.at > newest);
			if (!fresh.length && same) return known;
			return { key, rows: [...rows, ...fresh].slice(-KEEP_LIVE) };
		});
	}, [live, key]);
	return seen.key === key && seen.rows.length ? seen.rows : [...live];
}

interface Resources {
	samples: MetricSample[];
	history: MetricSample[] | undefined;
	historySupported: boolean;
	/** Stamps of the block head: the live sample and the trend's source. */
	stamps: FreshnessStampProps[];
	/** The stamp of the newest value, for the usage blocks. */
	latestStamp: FreshnessStampProps | undefined;
	lastKnown: boolean;
	locked: boolean;
	noAccess: ReactNode;
	rejected: boolean;
	loading: boolean;
}

/** The stamp says "sample 3 s ago" / "trend 28 s ago" only while its age is the expected one; any other age keeps its own words. */
const withText = (
	stamp: FreshnessStampProps,
	age: FreshnessStampProps["age"],
	text: string,
): FreshnessStampProps => (stamp.age === age ? { ...stamp, text } : stamp);

/** The newest sample over the live connection; nothing is asked without access. */
function useLiveSamples(target: ObserveTarget) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { deviceId, serviceId } = target;
	const projectId = target.service?.projectId;
	const gate = useGate(
		"live_metrics",
		deviceId,
		serviceId ? { placementId: serviceId, projectId } : undefined,
	);
	const refusal = readRefusal(
		t,
		target,
		gate,
		"metrics",
		serviceId ? { serviceId, projectId } : null,
		time,
	);
	const stream = useLiveStream<unknown>(
		deviceId,
		liveWanted(target) && !refusal
			? { kind: "metrics", placementId: serviceId }
			: null,
	);
	const live = useMemo(() => metricSamples(stream.data), [stream.data]);
	return { refusal, stream, live };
}

/** The trend the device publishes in its encrypted status, and how old it is. */
function useTrend(target: ObserveTarget) {
	const time = useAreaTime();
	const { deviceId, serviceId } = target;
	const fleet = useFleetDeviceStates()[deviceId];
	const snapshot = fleet?.metrics?.find((entry) =>
		serviceId
			? entry.scope.kind === "placement" &&
				entry.scope.placement_id === serviceId
			: entry.scope.kind === "device",
	);
	const samples = useMemo(
		() => metricSamples(snapshot?.sample, snapshot?.observedAt),
		[snapshot],
	);
	const failing = fleet?.freshness.metrics.error;
	const freshness: Freshness | undefined = snapshot
		? classify("fleet_metrics", {
				now: time.nowS,
				at: snapshot.observedAt,
				loaded: true,
				...(failing ? { error: failing } : {}),
			})
		: undefined;
	return { samples, freshness, at: snapshot?.observedAt };
}

function useResources(target: ObserveTarget): Resources {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { deviceId, serviceId, locked } = target;
	const { refusal, stream, live } = useLiveSamples(target);
	const trend = useTrend(target);
	const history = useMetricsHistory(target);
	const seen = useSeen(live, `${deviceId}|${serviceId ?? ""}`);
	const samples = useMemo(
		() => mergeSamples(trend.samples, seen),
		[trend.samples, seen],
	);

	const hasLive = live.length > 0;
	const liveStamp = hasLive ? stampOf(stream.freshness) : undefined;
	const trendStamp = trend.freshness
		? stampOf(locked ? { ...trend.freshness, age: "locked" } : trend.freshness)
		: undefined;
	const ago = (at: number) => time.ago(Math.min(at, time.nowS));
	const stamps: FreshnessStampProps[] = [];
	if (liveStamp)
		stamps.push(
			withText(
				liveStamp,
				"live",
				t("observe.metrics.sampleAgo", "sample {{ago}}", {
					ago: ago(live.at(-1)?.at ?? time.nowS),
				}),
			),
		);
	if (trendStamp && trend.at !== undefined)
		stamps.push(
			withText(
				trendStamp,
				"current",
				t("observe.metrics.trendAgo", "trend {{ago}}", {
					ago: ago(trend.at),
				}),
			),
		);
	return {
		samples,
		history: history.samples,
		historySupported: history.supported,
		stamps,
		latestStamp: liveStamp ?? trendStamp,
		lastKnown: !hasLive && trend.freshness?.age !== "current",
		locked,
		noAccess: refusal ? (
			<GateNotice
				kind="noaccess"
				title={t("observe.metrics.noAccess", "No access to metrics.")}
				text={refusal}
			/>
		) : null,
		rejected: !!stream.rejected && !hasLive,
		loading:
			livePhase(target) === "open" &&
			stream.data === undefined &&
			!stream.rejected,
	};
}

function Stamps({
	stamps,
}: Readonly<{ stamps: readonly FreshnessStampProps[] }>) {
	if (!stamps.length) return null;
	return (
		<>
			{stamps.map((stamp) => (
				<FreshnessStamp key={`${stamp.source}-${stamp.age}`} {...stamp} />
			))}
		</>
	);
}

function DeviceResources({
	target,
	resources,
}: Readonly<{ target: ObserveTarget; resources: Resources }>) {
	const { t } = useTranslation("devices");
	const { samples } = resources;
	const phase = livePhase(target);
	const body =
		resources.noAccess ??
		(samples.length ? (
			<div className={resources.locked ? LOCKED_DATA_CLASS : undefined}>
				<DeviceResourceCells
					samples={samples}
					history={resources.history}
					lastKnown={resources.lastKnown}
					services={target.services}
				/>
			</div>
		) : resources.rejected ? (
			<StateView
				kind="noaccess"
				title={t(
					"observe.metrics.rejected",
					"{{device}} didn't return its resources",
					{ device: target.name },
				)}
				text={t(
					"observe.metrics.rejectedText",
					"Reading resources needs Read metrics on this device.",
				)}
			/>
		) : phase !== "open" ? (
			<LiveDataState target={target} what="metrics" />
		) : resources.loading ? (
			<StateView
				kind="loading"
				title={t("observe.metrics.reading", "Reading resources…")}
			/>
		) : (
			<StateView
				kind="notloaded"
				title={t("observe.metrics.notSampled", "Not sampled yet")}
				text={t(
					"observe.metrics.notSampledDevice",
					"{{device}} hasn't sent resource samples yet.",
					{ device: target.name },
				)}
			/>
		));
	const flush = !resources.noAccess && samples.length > 0;
	return (
		<Block
			id="observe-resources"
			icon={Gauge}
			title={t("observe.metrics.deviceTitle", "Device resources")}
			stamp={flush ? <Stamps stamps={resources.stamps} /> : null}
			flush={flush}
			foot={
				flush
					? resources.history && spanOf(resources.history) > spanOf(samples)
						? t(
								"observe.metrics.deviceFootHistory",
								"CPU counts all logical CPUs: 100 % means every CPU is busy. Network is summed over all interfaces and isn't a billing figure. CPU and memory trends come from the device's own history.",
							)
						: t(
								"observe.metrics.deviceFoot",
								"CPU counts all logical CPUs: 100 % means every CPU is busy. Network is summed over all interfaces and isn't a billing figure. Trends come from the encrypted status, every 30 s.",
							)
					: null
			}
		>
			{body}
		</Block>
	);
}

function SampleDetails({
	target,
	sample,
	historySupported,
}: Readonly<{
	target: ObserveTarget;
	sample: MetricSample;
	historySupported: boolean;
}>) {
	const { t } = useTranslation("devices");
	const cohort = word(sample.data.process_cohort);
	const settings = whole(sample.data.config_revision);
	const seconds = amount(sample.data.sample_seconds);
	return (
		<details className="border-t border-hairline px-4 py-3 text-ui">
			<summary className="cursor-pointer text-xs font-medium text-ink-2">
				{t("observe.metrics.advanced", "Advanced: sample details")}
			</summary>
			<KeyValueList className="mt-3">
				{cohort ? (
					<KvRow
						label={t("observe.metrics.cohort", "Process group")}
						provenance={t(
							"observe.metrics.cohortNote",
							"changes when instances restart",
						)}
					>
						<IdRef
							id={cohort}
							copyLabel={t(
								"observe.metrics.copyCohort",
								"Copy process group fingerprint",
							)}
						/>
					</KvRow>
				) : null}
				{settings === undefined ? null : (
					<KvRow label={t("observe.metrics.sampledWith", "Sampled with")}>
						{seconds === undefined
							? t(
									"observe.metrics.sampledSettings",
									"settings v{{version, number}}",
									{ version: settings },
								)
							: t(
									"observe.metrics.sampledSettingsEvery",
									"settings v{{version, number}} · every {{seconds, number}} s",
									{ version: settings, seconds: Math.round(seconds) },
								)}
					</KvRow>
				)}
				<KvRow label={t("observe.metrics.trendHistory", "Trend history")}>
					{historySupported
						? t(
								"observe.metrics.trendFromDevice",
								"Read from {{device}}'s own metric history, up to 30 minutes back.",
								{ device: target.name },
							)
						: t(
								"observe.metrics.trendFromTab",
								"Built from the samples read while this tab is open. Update the device agent to read its own history.",
							)}
				</KvRow>
			</KeyValueList>
		</details>
	);
}

function ServiceResources({
	target,
	resources,
}: Readonly<{ target: ObserveTarget; resources: Resources }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { samples } = resources;
	const { service, deviceId, name } = target;
	const phase = livePhase(target);
	const latest = samples.at(-1);
	const stopped =
		service !== undefined &&
		service.desired === "stopped" &&
		service.instances.running === 0;
	const offline =
		target.presence?.kind === "offline" || target.presence?.kind === "never";
	const deviceMetrics = (
		<a
			className={LINK}
			{...link({ screen: "device", deviceId, tab: "metrics" })}
		/>
	);

	let body: ReactNode;
	if (resources.noAccess) body = resources.noAccess;
	else if (latest && service)
		body = (
			<>
				<div className={resources.locked ? LOCKED_DATA_CLASS : undefined}>
					<ServiceResourceCells
						samples={samples}
						history={resources.history}
						lastKnown={resources.lastKnown}
						service={service}
					/>
				</div>
				<SampleDetails
					target={target}
					sample={latest}
					historySupported={resources.historySupported}
				/>
			</>
		);
	else if (resources.rejected)
		body = (
			<StateView
				kind="noaccess"
				title={t(
					"observe.metrics.rejectedService",
					"{{device}} didn't return this service's resources",
					{ device: name },
				)}
				text={t(
					"observe.metrics.rejectedServiceText",
					"Reading resources needs Read metrics on this service.",
				)}
			/>
		);
	else if (phase === "idle" && offline)
		body = (
			<StateView
				kind="notloaded"
				title={t("observe.metrics.noLive", "No live values")}
				text={
					<Trans
						t={t}
						i18nKey="observe.metrics.noLiveText"
						defaults="{{device}} is offline. Its last known resources are device-wide only: <1>{{device}} › Metrics</1>."
						values={{ device: name }}
						components={{ 1: deviceMetrics }}
					/>
				}
			/>
		);
	else if (phase !== "open")
		body = <LiveDataState target={target} what="metrics" />;
	else if (resources.loading)
		body = (
			<StateView
				kind="loading"
				title={t("observe.metrics.reading", "Reading resources…")}
			/>
		);
	else if (stopped)
		body = (
			<StateView
				kind="empty"
				icon={CirclePause}
				title={t(
					"observe.metrics.stoppedTitle",
					"No running instances to measure",
				)}
				text={t(
					"observe.metrics.stoppedText",
					"{{service}} is stopped. Resource values appear once it runs.",
					{ service: target.serviceId ?? "" },
				)}
			/>
		);
	else
		body = (
			<StateView
				kind="notloaded"
				title={t("observe.metrics.notSampled", "Not sampled yet")}
				text={t(
					"observe.metrics.notSampledService",
					"The device samples service resources every 5 s.",
				)}
			/>
		);

	const flush = !resources.noAccess && !!latest && !!service;
	return (
		<Block
			id="observe-resources"
			icon={Gauge}
			title={t("observe.metrics.serviceTitle", "Service resources")}
			stamp={flush ? <Stamps stamps={resources.stamps} /> : null}
			flush={flush}
			foot={
				flush
					? t(
							"observe.metrics.serviceFoot",
							"Live values arrive every 5 s over the live connection. Hover a trend to read a value.",
						)
					: null
			}
		>
			{body}
		</Block>
	);
}

function PerInstance({
	sample,
	stamp,
}: Readonly<{ sample: MetricSample; stamp: FreshnessStampProps | undefined }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const one = new Intl.NumberFormat(time.locale, {
		minimumFractionDigits: 1,
		maximumFractionDigits: 1,
	});
	const rows = (Array.isArray(sample.data.replicas) ? sample.data.replicas : [])
		.slice(0, 32)
		.flatMap((value) => {
			const row = asRecord(value);
			const slot = whole(row?.slot);
			return row && slot !== undefined && slot < 32 ? [{ slot, row }] : [];
		});
	if (!rows.length) return null;
	const labels = {
		instance: t("observe.metrics.instance", "Instance"),
		cpu: t("observe.metrics.cpu", "CPU"),
		memory: t("observe.metrics.memory", "Memory"),
		threads: t("observe.metrics.threadsColumn", "Processes & threads"),
	};
	const dash = <span className="text-muted-foreground">–</span>;
	return (
		<Block
			id="observe-instances"
			icon={Box}
			title={t("observe.metrics.perInstance", "Per instance")}
			stamp={stamp ? <FreshnessStamp {...stamp} /> : null}
			flush
		>
			<DvTable
				className={TABLE_RESET}
				label={t("observe.metrics.perInstanceLabel", "Resources per instance")}
				cols={["16%", "28%", "28%", "28%"]}
				head={
					<tr>
						<Th>{labels.instance}</Th>
						<Th>{labels.cpu}</Th>
						<Th>{labels.memory}</Th>
						<Th>{labels.threads}</Th>
					</tr>
				}
			>
				{rows.map(({ slot, row }) => {
					const cpu = amount(row.cpu_percent);
					const memory = amount(row.memory_bytes);
					const threads = whole(row.processes_and_threads);
					return (
						<Tr key={slot}>
							<Td label={labels.instance} className="font-mono tabular-nums">
								#{slot}
							</Td>
							<Td label={labels.cpu} className="font-mono tabular-nums">
								{cpu === undefined ? dash : `${one.format(cpu)} %`}
							</Td>
							<Td label={labels.memory} className="font-mono tabular-nums">
								{memory === undefined ? dash : bytesText(memory)}
							</Td>
							<Td label={labels.threads} className="font-mono tabular-nums">
								{threads === undefined ? dash : threads}
							</Td>
						</Tr>
					);
				})}
			</DvTable>
		</Block>
	);
}

export interface MetricsViewProps {
	deviceId: string;
	/** Absent: the whole device. */
	serviceId?: string;
	scope: DevicesScope;
}

/** Metrics of one device or one service (SPEC §5.2, §5.3): one implementation for N2 and N3. */
export function MetricsView({
	deviceId,
	serviceId,
	scope,
}: Readonly<MetricsViewProps>) {
	const { t } = useTranslation("devices");
	const target = useObserveTarget(deviceId, serviceId ?? null, scope);
	const resources = useResources(target);
	const panel = useHistoryPanel(target, false);
	if (!target.known)
		return (
			<StateView
				kind="loading"
				title={t("observe.loadingDevice", "Loading the device…")}
			/>
		);
	const latest = resources.noAccess ? undefined : resources.samples.at(-1);
	const usageStamp = resources.latestStamp;
	return (
		<div className={OBSERVE_STACK} data-observe="metrics">
			{serviceId ? (
				<ServiceResources target={target} resources={resources} />
			) : (
				<DeviceResources target={target} resources={resources} />
			)}
			{serviceId && latest && target.service ? (
				<PerInstance sample={latest} stamp={resources.stamps[0]} />
			) : null}
			{latest && usageStamp && !resources.locked ? (
				<UsageBlocks
					sample={latest.data}
					stamp={usageStamp}
					subject={
						serviceId ??
						t("observe.usage.allServices", "All services on {{device}}", {
							device: target.name,
						})
					}
				/>
			) : null}
			{target.revoked || resources.noAccess ? null : (
				<RetainedHistory
					target={target}
					kind="metrics"
					history={panel.history}
					onEdit={panel.onEdit}
				/>
			)}
			{serviceId || target.revoked || resources.noAccess ? null : (
				<SharedLiveMetrics target={target} />
			)}
			{panel.sheet}
		</div>
	);
}
