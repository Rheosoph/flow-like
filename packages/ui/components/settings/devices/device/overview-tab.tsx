"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	CircleCheck,
	FileBadge,
	Gauge,
	History,
	Hourglass,
	IdCard,
	Stethoscope,
	Terminal,
	TriangleAlert,
	Users,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { groupFingerprint } from "../../../../lib/device-management/fingerprint";
import { keysLocked } from "../../../../lib/device-management/model/device-view";
import { classify } from "../../../../lib/device-management/model/freshness";
import { presetOf } from "../../../../lib/device-management/model/permissions";
import type {
	DeviceAuthRejection,
	DeviceTab,
	Freshness,
} from "../../../../lib/device-management/model/types";
import type { PolicyView } from "../../../../lib/device-management/types";
import type { ServiceSummary } from "../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../lib/utils";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { AttentionList } from "../primitives/attention-list";
import { Block } from "../primitives/block";
import { CommandBlock } from "../primitives/command-block";
import { DvButton } from "../primitives/dv-button";
import { ExpiryRail } from "../primitives/expiry-rail";
import {
	FreshnessStamp,
	MixedSourcesStamp,
	baseSource,
} from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { Metric, MetricGrid } from "../primitives/metric";
import { PairedPins } from "../primitives/paired-pins";
import { PersonChip } from "../primitives/person-chip";
import { RequestedActual } from "../primitives/requested-actual";
import { LOCKED_DATA_CLASS, StateView } from "../primitives/state-view";
import { Timeline, type TimelineEntry } from "../primitives/timeline";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import {
	stampOf,
	useAppNames,
	useAttentionEntries,
} from "../shell/attention-popover";
import {
	type AppViewRead,
	type PolicyRead,
	useAttentionState,
	useCertificateInventory,
	useDeviceRows,
	useFleetDeviceStates,
	useLiveStream,
	useOverlay,
	usePolicy,
} from "../workspace";
import { DeviceDataState, desiredRun, observedRun } from "./services-tab";
import { TrustPanel, fingerprintOf } from "./trust-panel";
import {
	type DevicePage,
	capabilitiesFor,
	servicesOfApp,
	sharedPeople,
	usePersonName,
} from "./use-device-page";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
/* The side column is narrower than the facts list's own breakpoint; its facts stay in two columns there, as designed. */
const SIDE_LIST =
	"@max-[520px]/kv:grid-cols-[minmax(104px,max-content)_minmax(0,1fr)] @max-[520px]/kv:gap-y-0";
const SIDE_ROW = "@max-[520px]/kv:mt-1.5";
/** R11: open items are capped; the rest sits behind "Show all". */
const ATTENTION_CAP = 7;
const SUMMARY_ROWS = 5;
const RECENT_ROWS = 5;

type OpenTab = (tab: DeviceTab) => void;

interface BlockProps {
	page: DevicePage;
	onTab: OpenTab;
}

/* 1. On this device. */

function OnThisDevice({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const { route, navigate } = useDevicesRoute();
	const items = page.app ? page.appAttention : page.attention;
	const entries = useAttentionEntries(items, { onNavigate: navigate });
	const counted = items.filter((item) => item.severity !== "info").length;
	const hidden = page.app
		? page.attention.filter((item) => item.severity !== "info").length - counted
		: 0;
	const base = baseSource(entries.map((entry) => entry.stamp));
	const rows = useDeviceRows();
	const [expanded, setExpanded] = useState(false);
	return (
		<Block
			id="device-attention"
			icon={TriangleAlert}
			title={
				page.app
					? t("device.overview.attentionApp", "{{app}} on this device", {
							app: page.app.name,
						})
					: t("device.overview.attention", "On this device")
			}
			count={counted}
			stamp={
				base ? (
					<FreshnessStamp {...base} />
				) : entries.length ? (
					<MixedSourcesStamp />
				) : (
					<FreshnessStamp {...stampOf(rows.freshness)} />
				)
			}
			flush
			foot={
				hidden > 0 ? (
					<>
						<span>
							{t("device.overview.attentionHidden", {
								count: hidden,
								device: page.name,
								defaultValue_one:
									"{{count, number}} item about the rest of {{device}} isn't shown.",
								defaultValue_other:
									"{{count, number}} items about the rest of {{device}} aren't shown.",
							})}
						</span>
						<DvButton
							variant="link"
							onClick={() => navigate(route, { scope: ACCOUNT_SCOPE })}
						>
							{t("device.overview.showWhole", "Show whole device")}
						</DvButton>
					</>
				) : null
			}
		>
			<AttentionList
				className="text-ui"
				items={entries}
				compact
				cap={expanded ? undefined : ATTENTION_CAP}
				expanded={expanded}
				onShowAll={() => setExpanded(true)}
				onShowFewer={() => setExpanded(false)}
				base={base ?? null}
				emptyText={
					page.app
						? t(
								"device.overview.attentionEmptyApp",
								"Nothing about {{app}} on {{device}} needs you right now.",
								{ app: page.app.name, device: page.name },
							)
						: t(
								"device.overview.attentionEmpty",
								"Nothing on {{device}} needs you right now.",
								{ device: page.name },
							)
				}
			/>
		</Block>
	);
}

/* 2. Services summary. */

type SummaryService = Pick<
	ServiceSummary,
	"serviceId" | "projectId" | "desired" | "observed" | "conv"
>;

function SummaryRow({
	page,
	service,
	locked,
	titled,
}: Readonly<{
	page: DevicePage;
	service: SummaryService;
	locked: boolean;
	titled: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	const full = page.services?.find(
		(row) => row.serviceId === service.serviceId,
	);
	const name = page.app ? undefined : appName(service.projectId);
	return (
		<li className="grid grid-cols-[minmax(150px,1.1fr)_minmax(0,1.6fr)_auto] items-center gap-x-3 gap-y-1.5 border-t border-hairline px-4 py-2 first:border-t-0 @max-[460px]/summary:grid-cols-[minmax(0,1fr)_auto] @max-[460px]/summary:px-3">
			<span className="flex min-w-0 flex-col gap-0.5">
				<span className="flex min-w-0 items-center gap-2">
					<PairedPins
						desired={desiredRun(service.desired)}
						observed={observedRun(service.observed)}
						conv={service.conv}
						title={titled}
					/>
					<a
						{...link({
							screen: "service",
							deviceId: page.deviceId,
							serviceId: service.serviceId,
							tab: "status",
						})}
						title={service.serviceId}
						className="truncate font-mono text-[12.5px] font-semibold hover:underline"
					>
						{service.serviceId}
					</a>
				</span>
				{name ? (
					<a
						{...link(
							{
								screen: "app-devices",
								by: "device",
								focusDeviceId: page.deviceId,
							},
							{ scope: { kind: "app", appId: service.projectId } },
						)}
						className="truncate text-xs font-medium hover:underline"
					>
						{name}
					</a>
				) : null}
			</span>
			<span className="min-w-0 @max-[460px]/summary:col-span-full @max-[460px]/summary:row-start-2">
				<RequestedActual
					desired={desiredRun(service.desired)}
					observed={observedRun(service.observed)}
					conv={service.conv}
					lastKnown={locked}
				/>
			</span>
			<span className="whitespace-nowrap text-right text-xs tabular-nums text-muted-foreground">
				{full
					? t(
							"device.overview.ready",
							"{{ready, number}} of {{requested, number}} ready",
							{
								ready: full.instances.ready,
								requested: full.instances.requested,
							},
						)
					: null}
			</span>
		</li>
	);
}

function SummaryEmpty({
	page,
	others,
}: Readonly<{ page: DevicePage; others: number }>) {
	const { t } = useTranslation("devices");
	if (!page.app)
		return (
			<StateView
				kind="empty"
				title={t(
					"device.overview.servicesEmpty",
					"No services on this device yet",
				)}
				text={t("device.overview.deployAnApp", "Deploy an app to run it here.")}
			/>
		);
	const deploy = t(
		"device.overview.deployAppHere",
		"Deploy {{app}} to run it here.",
		{ app: page.app.name },
	);
	const running =
		others > 0
			? t("device.overview.othersRun", {
					count: others,
					defaultValue_one:
						"{{count, number}} service of other apps runs here.",
					defaultValue_other:
						"{{count, number}} services of other apps run here.",
				})
			: null;
	return (
		<StateView
			kind="empty"
			title={t(
				"device.overview.servicesEmptyApp",
				"{{app}} doesn't run on {{device}}",
				{ app: page.app.name, device: page.name },
			)}
			text={running ? `${running} ${deploy}` : deploy}
		/>
	);
}

function SummaryFoot({
	page,
	count,
	others,
	onTab,
}: Readonly<{
	page: DevicePage;
	count: number;
	others: number;
	onTab: OpenTab;
}>) {
	const { t } = useTranslation("devices");
	const hidden =
		page.app && others > 0
			? `${t("device.overview.othersHidden", {
					count: others,
					defaultValue_one:
						"{{count, number}} service of other apps not shown.",
					defaultValue_other:
						"{{count, number}} services of other apps not shown.",
				})} `
			: null;
	return (
		<>
			<DvButton variant="link" onClick={() => onTab("services")}>
				{page.app
					? t("device.overview.appOnDevice", "{{app}} on {{device}}", {
							app: page.app.name,
							device: page.name,
						})
					: t("device.overview.allServices", {
							count,
							defaultValue_one: "All {{count, number}} service",
							defaultValue_other: "All {{count, number}} services",
						})}
			</DvButton>
			<span>
				{hidden}
				{t(
					"device.overview.servicesFoot",
					"Requested is what you asked for; actual is what the device reports.",
				)}
			</span>
		</>
	);
}

/** Live rows carry their own stamp; rows kept from before the lock say when they were read. */
function SummaryStamp({ page }: Readonly<{ page: DevicePage }>) {
	const first = page.services?.[0];
	if (first) return <FreshnessStamp {...stampOf(first.freshness)} />;
	if (page.services || !page.lockedRows) return null;
	return (
		<FreshnessStamp
			source="live"
			age="locked"
			observedAt={page.lockedRows.readAt}
		/>
	);
}

function SummaryList({
	page,
	list,
	others,
	locked,
}: Readonly<{
	page: DevicePage;
	list: readonly ServiceSummary[];
	others: number;
	locked: boolean;
}>) {
	if (list.length === 0) return <SummaryEmpty page={page} others={others} />;
	return (
		<ul
			className={cx(
				"@container/summary flex flex-col",
				locked && LOCKED_DATA_CLASS,
			)}
		>
			{list.slice(0, SUMMARY_ROWS).map((service, index) => (
				<SummaryRow
					key={service.serviceId}
					page={page}
					service={service}
					locked={locked}
					titled={index === 0}
				/>
			))}
		</ul>
	);
}

function ServicesSummary({ page, onTab }: Readonly<BlockProps>) {
	const { t } = useTranslation("devices");
	const all = page.services ?? page.lockedRows?.services ?? [];
	const list = servicesOfApp(all, page.app?.id);
	const others = all.length - list.length;
	const head = {
		id: "device-services-summary",
		icon: Boxes,
		title: page.app
			? t("device.overview.servicesApp", "{{app}} services", {
					app: page.app.name,
				})
			: t("device.overview.services", "Services"),
	};
	if (page.revoked || (!!page.unavailable && !page.lockedRows))
		return (
			<Block {...head} stamp={<SummaryStamp page={page} />}>
				<DeviceDataState page={page} what="services" />
			</Block>
		);
	return (
		<Block
			{...head}
			count={list.length}
			stamp={<SummaryStamp page={page} />}
			flush={list.length > 0}
			foot={
				<SummaryFoot
					page={page}
					count={list.length}
					others={others}
					onTab={onTab}
				/>
			}
		>
			<SummaryList
				page={page}
				list={list}
				others={others}
				locked={!page.services && !!page.lockedRows}
			/>
		</Block>
	);
}

/* 3. Resources. */

interface Sample {
	at: number;
	data: Record<string, unknown>;
}

const record = (value: unknown): Record<string, unknown> | undefined =>
	value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;

const amount = (value: unknown): number | undefined =>
	typeof value === "number" && Number.isFinite(value) && value >= 0
		? value
		: undefined;

/** Samples of a metrics read, oldest first. A read without records is one sample taken at `fallbackAt`. */
export function resourceSamples(
	source: Record<string, unknown> | undefined,
	fallbackAt?: number,
): Sample[] {
	if (!source) return [];
	const rows = Array.isArray(source.records) ? source.records : [];
	const samples = rows.flatMap((row) => {
		const entry = record(row);
		const data = record(entry?.data);
		const at = amount(entry?.timestamp);
		return data && at !== undefined ? [{ at, data }] : [];
	});
	if (samples.length) return samples.sort((a, b) => a.at - b.at);
	return amount(source.cpu_percent) !== undefined && fallbackAt !== undefined
		? [{ at: fallbackAt, data: source }]
		: [];
}

const GIB = 1024 ** 3;

function series(
	samples: readonly Sample[],
	read: (data: Sample["data"]) => number | undefined,
) {
	return samples.map((sample) => read(sample.data) ?? Number.NaN);
}

function ResourceCells({
	samples,
	lastKnown,
}: Readonly<{ samples: readonly Sample[]; lastKnown: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const latest = samples.at(-1);
	if (!latest) return null;
	const data = latest.data;
	const start = samples[0]?.at ?? latest.at;
	const trend = samples.length > 1;
	const number = new Intl.NumberFormat(time.locale, {
		minimumFractionDigits: 1,
		maximumFractionDigits: 1,
	});
	const axis = (middle: string) =>
		trend
			? [
					time.clock(start, false),
					middle,
					lastKnown
						? time.clock(latest.at, false)
						: t("device.overview.now", "now"),
				]
			: undefined;
	const cpu = amount(data.cpu_percent);
	const cpus = amount(data.logical_cpus);
	const memUsed = amount(data.memory_used_bytes);
	const memTotal = amount(data.memory_total_bytes);
	const volume = record(data.storage_volume);
	const free = amount(volume?.available_bytes);
	const total = amount(volume?.total_bytes);
	const network = record(data.network);
	const received = amount(network?.received_bytes);
	const sent = amount(network?.transmitted_bytes);
	const seconds = amount(data.sample_seconds);
	const step = trend
		? Math.max(1, Math.round((latest.at - start) / (samples.length - 1)))
		: undefined;
	return (
		<MetricGrid>
			{cpu === undefined ? null : (
				<Metric
					label={t("device.overview.cpu", "CPU")}
					value={number.format(cpu)}
					unit="%"
					note={
						cpus === undefined
							? undefined
							: t(
									"device.overview.cpuNote",
									"{{count, number}} logical CPUs · 100 % = all of them",
									{ count: cpus },
								)
					}
					spark={
						trend
							? {
									series: series(samples, (row) => amount(row.cpu_percent)),
									label: t("device.overview.cpu", "CPU"),
									min: 0,
									max: 100,
									format: (value) => `${number.format(value)} %`,
									startAt: start,
									stepSec: step,
								}
							: undefined
					}
					axis={axis(t("device.overview.cpuMax", "max 100 %"))}
				/>
			)}
			{memUsed === undefined || memTotal === undefined ? null : (
				<Metric
					label={t("device.overview.memory", "Memory")}
					value={number.format(memUsed / GIB)}
					unit={t("device.overview.memoryOf", "of {{total}} GiB", {
						total: number.format(memTotal / GIB),
					})}
					note={
						memTotal > 0
							? t("device.overview.percentUsed", "{{percent, number}} % used", {
									percent: Math.round((memUsed / memTotal) * 100),
								})
							: undefined
					}
					spark={
						trend
							? {
									series: series(samples, (row) => {
										const used = amount(row.memory_used_bytes);
										return used === undefined ? undefined : used / GIB;
									}),
									label: t("device.overview.memory", "Memory"),
									min: 0,
									max: memTotal / GIB,
									format: (value) => `${number.format(value)} GiB`,
									startAt: start,
									stepSec: step,
								}
							: undefined
					}
					axis={axis(
						t("device.overview.memoryMax", "max {{total}} GiB", {
							total: number.format(memTotal / GIB),
						}),
					)}
				/>
			)}
			{free === undefined || total === undefined ? null : (
				<Metric
					label={t("device.overview.disk", "Agent's data disk")}
					value={String(Math.round(free / GIB))}
					unit={t("device.overview.diskOf", "GiB free of {{total, number}}", {
						total: Math.round(total / GIB),
					})}
					note={
						total > 0
							? t("device.overview.percentUsed", "{{percent, number}} % used", {
									percent: Math.round((1 - free / total) * 100),
								})
							: undefined
					}
					spark={
						trend
							? {
									series: series(samples, (row) => {
										const value = amount(
											record(row.storage_volume)?.available_bytes,
										);
										return value === undefined ? undefined : value / GIB;
									}),
									label: t("device.overview.disk", "Agent's data disk"),
									format: (value) =>
										t("device.overview.diskFree", "{{value}} GiB free", {
											value: number.format(value),
										}),
									startAt: start,
									stepSec: step,
								}
							: undefined
					}
					axis={axis(t("device.overview.diskAxis", "free space"))}
				/>
			)}
			{received === undefined || sent === undefined ? null : (
				<Metric
					label={t("device.overview.network", "Network")}
					value={humanFileSize(received)}
					unit={t("device.overview.networkUnit", "received · {{sent}} sent", {
						sent: humanFileSize(sent),
					})}
					note={
						seconds === undefined
							? t(
									"device.overview.networkNote",
									"per sample · not a billing figure",
								)
							: t(
									"device.overview.networkNoteSeconds",
									"per {{seconds, number}} s sample · not a billing figure",
									{ seconds: Math.round(seconds) },
								)
					}
					spark={
						trend
							? {
									series: series(samples, (row) =>
										amount(record(row.network)?.received_bytes),
									),
									label: t("device.overview.network", "Network"),
									format: (value) => humanFileSize(value),
									startAt: start,
									stepSec: step,
								}
							: undefined
					}
					axis={axis(t("device.overview.networkAxis", "received per sample"))}
				/>
			)}
		</MetricGrid>
	);
}

/** While a read is current, its stamp says what the age belongs to ("sample 2s ago"). */
const agoText = (freshness: Freshness | undefined, text: string) =>
	freshness?.age === "current" ? { text } : {};

function Resources({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const fleet = useFleetDeviceStates()[page.deviceId];
	const stream = useLiveStream<Record<string, unknown>>(
		page.deviceId,
		page.liveOpen ? { kind: "metrics", placementId: null } : null,
	);
	const snapshot = fleet?.metrics?.find(
		(entry) => entry.scope.kind === "device",
	);
	const samples = useMemo(() => {
		const trend = resourceSamples(snapshot?.sample, snapshot?.observedAt);
		const live = resourceSamples(stream.data);
		const newestTrend = trend.at(-1)?.at ?? 0;
		return [...trend, ...live.filter((sample) => sample.at > newestTrend)];
	}, [snapshot, stream.data]);
	const liveAt = resourceSamples(stream.data).at(-1)?.at;
	const state =
		page.unavailable && !samples.length ? (
			<DeviceDataState page={page} what="resources" />
		) : null;
	const trendFreshness: Freshness | undefined = snapshot
		? classify("fleet_metrics", {
				now: time.nowS,
				at: snapshot.observedAt,
				loaded: true,
				...(fleet?.freshness.metrics.error
					? { error: fleet.freshness.metrics.error }
					: {}),
			})
		: undefined;
	const live = page.liveOpen && liveAt !== undefined;
	const lastKnown = !live && trendFreshness?.age !== "current";
	const locked = keysLocked(page.view.keys);
	return (
		<Block
			id="device-resources"
			icon={Gauge}
			title={t("device.overview.resources", "Resources")}
			stamp={
				samples.length ? (
					<>
						{live && liveAt !== undefined ? (
							<FreshnessStamp
								{...stampOf(stream.freshness)}
								{...agoText(
									stream.freshness,
									t("device.overview.sampleAgo", "sample {{ago}}", {
										ago: time.ago(liveAt),
									}),
								)}
							/>
						) : null}
						{trendFreshness && snapshot ? (
							<FreshnessStamp
								{...stampOf(
									locked
										? { ...trendFreshness, age: "locked" }
										: trendFreshness,
								)}
								{...agoText(
									locked ? undefined : trendFreshness,
									t("device.overview.trendAgo", "trend {{ago}}", {
										ago: time.ago(snapshot.observedAt),
									}),
								)}
							/>
						) : null}
					</>
				) : null
			}
			flush={!state && samples.length > 0}
			foot={
				samples.length
					? t(
							"device.overview.resourcesFoot",
							"Live values come every 5 s over the live connection. Trends come from the encrypted snapshot, every 30 s. Hover a trend to read a value.",
						)
					: null
			}
		>
			{state ??
				(samples.length ? (
					<div className={locked ? LOCKED_DATA_CLASS : undefined}>
						<ResourceCells samples={samples} lastKnown={lastKnown} />
					</div>
				) : (
					<StateView
						kind="notloaded"
						title={t("device.overview.resourcesNone", "Not loaded")}
						text={t(
							"device.overview.resourcesNoneText",
							"This device hasn't sent resource samples yet.",
						)}
					/>
				))}
		</Block>
	);
}

/* 9. Diagnose on the device (offline, never checked in, or the hub refused its check-ins). */

function RejectionNote({
	page,
	rejection,
}: Readonly<{ page: DevicePage; rejection: DeviceAuthRejection }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const facts = {
		count: rejection.count,
		device: page.name,
		since: time.at(rejection.first_at),
	};
	return (
		<p data-device-rejection={rejection.code} className="text-ui text-ink-2">
			{rejection.code === "revoked_credential"
				? t("device.diagnose.revokedCredential", {
						...facts,
						defaultValue_one:
							"The hub refused {{device}}'s last check-in ({{since}}): it used a credential that was revoked. Run the recovery command on the device.",
						defaultValue_other:
							"The hub refused {{device}}'s last {{count, number}} check-ins since {{since}}: it uses a credential that was revoked. Run the recovery command on the device.",
					})
				: clockSkewText(t, facts, rejection.skew_seconds)}
		</p>
	);
}

/** BG6: which way the device's clock is off, when the hub measured it. */
function clockSkewText(
	t: DevicesT,
	facts: { count: number; device: string; since: string },
	skewSeconds: number | null,
): string {
	if (skewSeconds === null)
		return t("devices:device.diagnose.clockSkew", {
			...facts,
			defaultValue_one:
				"The hub refused {{device}}'s last check-in ({{since}}) because its clock is off. Set the clock on the device.",
			defaultValue_other:
				"The hub refused {{device}}'s last {{count, number}} check-ins since {{since}} because its clock is off. Set the clock on the device.",
		});
	const minutes = Math.max(1, Math.round(Math.abs(skewSeconds) / 60));
	return skewSeconds > 0
		? t("devices:device.diagnose.clockAhead", {
				...facts,
				minutes,
				defaultValue_one:
					"The hub refused {{device}}'s last check-in ({{since}}) because its clock is about {{minutes, number}} min ahead. Set the clock on the device.",
				defaultValue_other:
					"The hub refused {{device}}'s last {{count, number}} check-ins since {{since}} because its clock is about {{minutes, number}} min ahead. Set the clock on the device.",
			})
		: t("devices:device.diagnose.clockBehind", {
				...facts,
				minutes,
				defaultValue_one:
					"The hub refused {{device}}'s last check-in ({{since}}) because its clock is about {{minutes, number}} min behind. Set the clock on the device.",
				defaultValue_other:
					"The hub refused {{device}}'s last {{count, number}} check-ins since {{since}} because its clock is about {{minutes, number}} min behind. Set the clock on the device.",
			});
}

function DiagnoseOnDevice({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const { presence, row } = page.view;
	const rejection = row.auth_rejection ?? null;
	const away = presence.kind === "offline" || presence.kind === "never";
	if (page.revoked || page.consentOnly || (!away && !rejection)) return null;
	const commands: [string, string][] = [
		[
			"flow-like-standalone status",
			t(
				"device.diagnose.status",
				"Shows the connection state, last contact and each service's last error.",
			),
		],
		[
			"flow-like-standalone service-status",
			t(
				"device.diagnose.serviceStatus",
				"Shows whether the agent runs as a system service and restarts at boot.",
			),
		],
		[
			"flow-like-standalone recover-enrollment",
			t(
				"device.diagnose.recover",
				"Use this if status says the hub refused this device.",
			),
		],
		[
			"./flow-like-standalone install-service",
			t(
				"device.diagnose.install",
				"Installs the agent as a system service so it starts at boot.",
			),
		],
	];
	return (
		<Block
			id="device-diagnose"
			icon={Terminal}
			title={t("device.diagnose.title", "Diagnose on the device")}
			stamp={
				<FreshnessStamp
					source="device"
					age="notloaded"
					text={t("device.diagnose.stamp", "only on the device")}
				/>
			}
			foot={
				<DvButton
					size="sm"
					icon={Stethoscope}
					onClick={() => overlay.openDiagnose(page.deviceId)}
				>
					{t("device.diagnose.open", "Open full diagnosis")}
				</DvButton>
			}
		>
			{rejection ? <RejectionNote page={page} rejection={rejection} /> : null}
			<p className="text-xs text-muted-foreground">
				{t(
					"device.diagnose.hint",
					"These show what only the device knows: its connection state, last contact and each service's last error.",
				)}
			</p>
			{commands.map(([command, note]) => (
				<CommandBlock key={command} command={command} note={note} />
			))}
		</Block>
	);
}

/* 5. Registration & identity. */

function Registration({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const rows = useDeviceRows();
	const { row, presence } = page.view;
	const ownerName = usePersonName(row.owner_id, !page.owner);
	const fingerprint = fingerprintOf(row);
	return (
		<Block
			id="device-registration"
			icon={IdCard}
			title={t("device.overview.registration", "Registration & identity")}
			stamp={<FreshnessStamp {...stampOf(rows.freshness)} />}
		>
			<KeyValueList className={SIDE_LIST}>
				<KvRow
					className={SIDE_ROW}
					label={t("device.overview.registered", "Registered")}
				>
					<span title={time.abs(row.registered_at)}>
						{time.at(row.registered_at)}
					</span>
				</KvRow>
				<KvRow
					className={SIDE_ROW}
					label={t("device.overview.lastCheckIn", "Last check-in")}
				>
					{row.last_seen_at === null ? (
						t("device.overview.never", "Never")
					) : (
						<span title={time.abs(row.last_seen_at)}>
							{t("device.overview.lastCheckInValue", "{{time}} · {{ago}}", {
								time: time.clock(row.last_seen_at),
								ago: time.ago(presence.since ?? row.last_seen_at),
							})}
						</span>
					)}
					<p className="mt-0.5 text-xs text-muted-foreground">
						{t(
							"device.overview.checkInHint",
							"A check-in is a signed \"I'm here\" the device sends about once a minute. It doesn't prove a live connection works.",
						)}
					</p>
				</KvRow>
				<KvRow className={SIDE_ROW} label={t("device.overview.owner", "Owner")}>
					{page.owner ? (
						<PersonChip name={t("device.overview.you", "You")} you />
					) : ownerName ? (
						<PersonChip name={ownerName} />
					) : (
						t("device.overview.ownerOther", "Someone else")
					)}
				</KvRow>
				<KvRow
					className={SIDE_ROW}
					label={t("device.overview.deviceId", "Device ID")}
				>
					<IdRef
						id={page.deviceId}
						copyLabel={t("device.overview.copyDeviceId", "Copy device ID")}
					/>
				</KvRow>
			</KeyValueList>
			<details className="text-ui">
				<summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground">
					{t(
						"device.overview.advanced",
						"Advanced: key generation and fingerprints",
					)}
				</summary>
				<KeyValueList className={cx("mt-2", SIDE_LIST)}>
					<KvRow
						className={SIDE_ROW}
						label={t("device.overview.keyGeneration", "Key generation")}
						provenance={t(
							"device.overview.keyGenerationHint",
							"increases only when a device is revoked",
						)}
					>
						<span className="font-mono tabular-nums">{row.auth_epoch}</span>
					</KvRow>
					<KvRow
						className={SIDE_ROW}
						label={t("device.overview.fingerprint", "Identity fingerprint")}
					>
						{fingerprint ? (
							<IdRef
								id={groupFingerprint(fingerprint)}
								copyLabel={t(
									"device.overview.copyFingerprint",
									"Copy fingerprint",
								)}
							/>
						) : (
							t("device.overview.fingerprintUnknown", "Not known")
						)}
					</KvRow>
				</KeyValueList>
			</details>
		</Block>
	);
}

/* 6. Certificates (public inventory). */

function CertificatesMini({ page, onTab }: Readonly<BlockProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const read = useCertificateInventory(
		page.revoked ? undefined : page.deviceId,
	);
	const inventory = read.data ?? page.view.certificates;
	const labels = new Map(
		input.live[page.deviceId]?.certificates?.certificates.map((row) => [
			row.certificate_id,
			row.label,
		]),
	);
	const title = t("device.overview.certificates", "Certificates");
	if (page.revoked)
		return (
			<Block id="device-certificates" icon={FileBadge} title={title}>
				<StateView
					kind="notloaded"
					title={t("device.overview.certificatesNotRead", "Not read")}
					text={t("device.state.revokedText", "Revoked devices aren't read.")}
				/>
			</Block>
		);
	if (read.error?.code === "forbidden")
		return (
			<Block id="device-certificates" icon={FileBadge} title={title}>
				<GateNotice
					kind="noaccess"
					title={t(
						"device.overview.certificatesNoAccess",
						"No access to certificates.",
					)}
					text={t(
						"device.overview.certificatesNoAccessText",
						"Needs whole-device View status or Manage certificates. Your access is scoped to one app.",
					)}
				/>
			</Block>
		);
	const reported = inventory && inventory.updated_at !== null;
	const rows = [...(inventory?.certificates ?? [])].sort(
		(a, b) => a.not_after - b.not_after,
	);
	return (
		<Block
			id="device-certificates"
			icon={FileBadge}
			title={title}
			count={reported ? rows.length : undefined}
			stamp={
				inventory?.updated_at ? (
					<FreshnessStamp
						source="hub"
						age={
							stampOf(read.freshness).age === "error" ? "lastknown" : "current"
						}
						observedAt={inventory.updated_at}
						text={t(
							"device.overview.certificatesConfirmed",
							"device confirmed {{ago}}",
							{
								ago: time.ago(inventory.updated_at),
							},
						)}
						noFail
					/>
				) : null
			}
			foot={
				page.tabs.includes("certificates") && rows.length ? (
					<DvButton variant="link" onClick={() => onTab("certificates")}>
						{t("device.overview.openCertificates", "Open certificates")}
					</DvButton>
				) : null
			}
		>
			{!reported ? (
				read.loading ? (
					<StateView kind="loading" rows={2} />
				) : (
					<StateView
						kind="never"
						title={t(
							"device.overview.certificatesNever",
							"The device hasn't reported certificates",
						)}
						text={t(
							"device.overview.certificatesNeverText",
							"It reports them after a check-in with a current agent.",
						)}
					/>
				)
			) : rows.length === 0 ? (
				<StateView
					kind="empty"
					icon={FileBadge}
					title={t(
						"device.overview.certificatesNone",
						"No certificates on this device",
					)}
					text={t(
						"device.overview.certificatesNoneText",
						"Services here serve plain HTTP or none at all.",
					)}
				/>
			) : (
				<ul className="-my-2 flex flex-col">
					{rows.map((row) => {
						const label = labels.get(row.certificate_id);
						const expired = row.not_after <= time.nowS;
						return (
							<li
								key={row.certificate_id}
								className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 border-t border-hairline py-2 first:border-t-0"
							>
								<span className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 text-ui">
									{label ? (
										<b className="font-semibold">{label}</b>
									) : (
										<>
											<span className="font-mono">
												{row.certificate_id.slice(0, 8)}
											</span>
											<span className="font-mono text-muted-foreground">
												{groupFingerprint(
													row.fingerprint_sha256.slice(0, 8).toUpperCase(),
												)}
											</span>
										</>
									)}
									<span className={expired ? "text-critical" : "text-ink-2"}>
										{expired
											? t(
													"device.overview.certificateExpired",
													"· expired {{date}}",
													{
														date: time.at(row.not_after),
													},
												)
											: t(
													"device.overview.certificateExpires",
													"· expires {{date}}",
													{
														date: time.at(row.not_after),
													},
												)}
									</span>
								</span>
								<ExpiryRail notAfter={row.not_after} />
								{label ? null : (
									<span className="col-span-full text-xs text-muted-foreground">
										{t(
											"device.overview.certificateName",
											"Name shows after a live read.",
										)}
									</span>
								)}
							</li>
						);
					})}
				</ul>
			)}
		</Block>
	);
}

/* 7. Access summary. */

/** A recipient's own access: preset, how many permissions, where, and when it ends. */
function RecipientAccess({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const appName = useAppNames();
	const scope = page.capabilities?.[0]?.scope;
	const appId = scope && scope.kind !== "device" ? scope.project_id : undefined;
	const caps = capabilitiesFor(page, appId) ?? [];
	const ends = page.view.row.access_expires_at;
	if (!caps.length)
		return (
			<>
				{t(
					"device.overview.accessUnknown",
					"Your access: unlock to check your permissions.",
				)}
			</>
		);
	return (
		<>
			{t("device.overview.accessYours", {
				count: caps.length,
				preset: enumLabel(t, "preset", presetOf(caps).preset ?? "custom"),
				scope: appId
					? t("device.overview.accessApp", "App {{app}}", {
							app: appName(appId) ?? appId,
						})
					: t("device.access.scopeDevice", "Whole device"),
				defaultValue_one:
					"Your access: {{preset}} · {{count, number}} permission · {{scope}}",
				defaultValue_other:
					"Your access: {{preset}} · {{count, number}} permissions · {{scope}}",
			})}
			{ends
				? ` ${t("device.overview.accessEnds", "· ends {{ago}}", {
						ago: time.ago(ends, "long"),
					})}`
				: null}
		</>
	);
}

/** Saved against applied access rules, with how many people they cover. */
function SharedRules({
	page,
	policy,
}: Readonly<{ page: DevicePage; policy: PolicyRead & { data: PolicyView } }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const expires = page.view.row.access_rules_expire_at;
	const { version, applied_version: applied } = policy.data;
	const people = sharedPeople(policy.policy, input.me);
	const waiting = applied !== version;
	return (
		<span className="leading-7">
			{people === undefined
				? t("device.overview.accessShared", "Shared")
				: t("device.overview.accessPeople", {
						count: people,
						defaultValue_one: "Shared with {{count, number}} person",
						defaultValue_other: "Shared with {{count, number}} people",
					})}
			<span aria-hidden className="text-muted-foreground">
				{" · "}
			</span>
			<RequestedActual
				className="align-middle"
				versions
				requested={t(
					"device.overview.rulesSaved",
					"Saved v{{version, number}}",
					{ version },
				)}
				actual={t(
					"device.overview.rulesDevice",
					"device has v{{version, number}}",
					{ version: applied },
				)}
				label={t(
					"device.overview.rulesLabel",
					"Access rules: saved v{{saved, number}}, device has v{{applied, number}}",
					{ saved: version, applied },
				)}
				tone={waiting ? "info" : "good"}
				icon={waiting ? Hourglass : CircleCheck}
			/>
			{expires ? (
				<span className="text-muted-foreground">
					{" "}
					{t("device.overview.rulesExpire", "· rules expire {{date}}", {
						date: time.at(expires),
					})}
				</span>
			) : null}
		</span>
	);
}

function OwnerAccess({
	page,
	policy,
}: Readonly<{ page: DevicePage; policy: PolicyRead }>) {
	const { t } = useTranslation("devices");
	if (page.revoked)
		return (
			<>
				{t(
					"device.overview.accessRevoked",
					"Nobody can reach a revoked device. Cloud approvals stay listed so you can close them.",
				)}
			</>
		);
	const { data } = policy;
	if (!data)
		return (
			<>
				{policy.error
					? t(
							"device.overview.accessError",
							"The access rules couldn't be read from the hub.",
						)
					: t("device.overview.accessLoading", "Reading the access rules…")}
			</>
		);
	if (data.version === 0)
		return (
			<>
				{t(
					"device.overview.accessNone",
					"Not shared. Only you can reach this device.",
				)}
			</>
		);
	return <SharedRules page={page} policy={{ ...policy, data }} />;
}

function AccessSummary({ page, onTab }: Readonly<BlockProps>) {
	const { t } = useTranslation("devices");
	const policy = usePolicy(
		page.owner && !page.revoked ? page.deviceId : undefined,
	);
	const rows = useDeviceRows();
	return (
		<Block
			id="device-access"
			icon={Users}
			title={t("device.overview.access", "Access")}
			stamp={
				<FreshnessStamp
					{...stampOf(
						page.owner && policy.data ? policy.freshness : rows.freshness,
					)}
				/>
			}
			foot={
				page.tabs.includes("access") ? (
					<DvButton variant="link" onClick={() => onTab("access")}>
						{t("device.overview.openAccess", "Open access")}
					</DvButton>
				) : null
			}
		>
			<div className="text-ui">
				{page.consentOnly ? (
					t(
						"device.overview.accessConsent",
						"Only cloud approvals you approve or pay for are visible.",
					)
				) : page.owner ? (
					<OwnerAccess page={page} policy={policy} />
				) : (
					<RecipientAccess page={page} />
				)}
			</div>
		</Block>
	);
}

/* 8. Recent activity. */

/** A name inside a <Trans> sentence: a component child, never an interpolated value (which <Trans> parses as markup). */
function ServiceName({ id }: Readonly<{ id: string }>) {
	return <span className="font-mono">{id}</span>;
}

interface MessageRecord {
	sequence: number;
	timestamp: number;
	data: Record<string, unknown>;
}

function messageRecords(data: unknown): MessageRecord[] {
	const rows = record(data)?.records;
	if (!Array.isArray(rows)) return [];
	return rows.flatMap((row) => {
		const entry = record(row);
		const body = record(entry?.data);
		const sequence = amount(entry?.sequence);
		const timestamp = amount(entry?.timestamp);
		return body && sequence !== undefined && timestamp !== undefined
			? [{ sequence, timestamp, data: body }]
			: [];
	});
}

function RecentActivity({ page, onTab }: Readonly<BlockProps>) {
	const { t } = useTranslation("devices");
	const stream = useLiveStream<unknown>(
		page.deviceId,
		page.liveOpen
			? { kind: "messages", placementId: null, projectId: null, limit: 20 }
			: null,
	);
	const entries = useMemo<TimelineEntry[]>(() => {
		const commands: Record<string, (service?: string) => ReactNode> = {
			accepted: (service) =>
				service ? (
					<Trans
						t={t}
						i18nKey="device.overview.activity.acceptedFor"
						defaults="A command for <1/> was accepted"
						components={{ 1: <ServiceName id={service} /> }}
					/>
				) : (
					t("device.overview.activity.accepted", "A command was accepted")
				),
			completed: (service) =>
				service ? (
					<Trans
						t={t}
						i18nKey="device.overview.activity.completedFor"
						defaults="A command for <1/> finished"
						components={{ 1: <ServiceName id={service} /> }}
					/>
				) : (
					t("device.overview.activity.completed", "A command finished")
				),
			failed: (service) =>
				service ? (
					<Trans
						t={t}
						i18nKey="device.overview.activity.failedFor"
						defaults="A command for <1/> failed"
						components={{ 1: <ServiceName id={service} /> }}
					/>
				) : (
					t("device.overview.activity.failed", "A command failed")
				),
			unknown: (service) =>
				service ? (
					<Trans
						t={t}
						i18nKey="device.overview.activity.unknownFor"
						defaults="The result of a command for <1/> isn't known"
						components={{ 1: <ServiceName id={service} /> }}
					/>
				) : (
					t(
						"device.overview.activity.unknown",
						"A command's result isn't known",
					)
				),
		};
		return messageRecords(stream.data)
			.sort((a, b) => b.sequence - a.sequence)
			.flatMap((row): TimelineEntry[] => {
				const { data } = row;
				const state = typeof data.state === "string" ? data.state : "";
				const service =
					typeof data.placement_id === "string" ? data.placement_id : undefined;
				if (data.kind === "operation") {
					const text = commands[state];
					if (!text) return [];
					return [
						{
							id: String(row.sequence),
							at: row.timestamp,
							kind: "command",
							tone:
								state === "failed"
									? "critical"
									: state === "completed"
										? "good"
										: "info",
							text: text(service),
						},
					];
				}
				if (data.kind !== "replica" || !service) return [];
				const slot = amount(data.replica_slot) ?? 0;
				return [
					{
						id: String(row.sequence),
						at: row.timestamp,
						kind: "instance",
						tone:
							state === "failed" || state === "backoff"
								? "critical"
								: state === "running"
									? "good"
									: "info",
						text: (
							<Trans
								t={t}
								i18nKey="device.overview.activity.instance"
								defaults="Instance #{{slot, number}} of <1/>: {{state}}"
								values={{
									slot,
									state: enumLabel(t, "observed", observedRun(state)),
								}}
								components={{ 1: <ServiceName id={service} /> }}
							/>
						),
					},
				];
			})
			.slice(0, RECENT_ROWS);
	}, [stream.data, t]);
	const locked = keysLocked(page.view.keys);
	return (
		<Block
			id="device-recent"
			icon={History}
			title={t("device.overview.recent", "Recent activity")}
			stamp={
				entries.length ? (
					<FreshnessStamp {...stampOf(stream.freshness)} />
				) : null
			}
			foot={
				page.tabs.includes("activity") ? (
					<DvButton variant="link" onClick={() => onTab("activity")}>
						{t("device.overview.allActivity", "All activity")}
					</DvButton>
				) : null
			}
		>
			{entries.length ? (
				<Timeline rows={entries} showFilters={false} />
			) : locked ? (
				<StateView
					kind="locked"
					title={t("device.overview.recentLocked", "Locked")}
					text={t(
						"device.overview.recentLockedText",
						"Activity is read over the live connection. Unlock to see it.",
					)}
				/>
			) : stream.rejected ? (
				<StateView
					kind="noaccess"
					title={t("device.overview.recentNoAccess", "No access to activity")}
					text={t(
						"device.overview.recentNoAccessText",
						"Activity needs View status on this device.",
					)}
				/>
			) : (
				<StateView
					kind="notloaded"
					title={t("device.overview.recentNone", "No activity read yet")}
					text={t(
						"device.overview.recentNoneText",
						"Activity comes from the device over a live connection.",
					)}
				/>
			)}
		</Block>
	);
}

export interface DeviceOverviewTabProps {
	page: DevicePage;
	app: AppViewRead | null;
	onTab: OpenTab;
}

/** SPEC §5.2 Overview: what needs you, what runs, and who and what the device is. */
export function DeviceOverviewTab({
	page,
	onTab,
}: Readonly<DeviceOverviewTabProps>) {
	const minimal = page.consentOnly;
	return (
		<div className="@container/overview min-w-0">
			<div className="grid min-w-0 gap-x-6 gap-y-4 @min-[740px]/overview:grid-cols-[minmax(0,1.6fr)_minmax(320px,1fr)]">
				<div className="flex min-w-0 flex-col gap-4">
					<OnThisDevice page={page} />
					{minimal ? null : <ServicesSummary page={page} onTab={onTab} />}
					{minimal || page.revoked ? null : <Resources page={page} />}
				</div>
				<div className="flex min-w-0 flex-col gap-4">
					<DiagnoseOnDevice page={page} />
					{minimal ? null : <TrustPanel page={page} onTab={onTab} />}
					<Registration page={page} />
					{minimal ? null : <CertificatesMini page={page} onTab={onTab} />}
					<AccessSummary page={page} onTab={onTab} />
					{minimal || page.revoked ? null : (
						<RecentActivity page={page} onTab={onTab} />
					)}
				</div>
			</div>
		</div>
	);
}
