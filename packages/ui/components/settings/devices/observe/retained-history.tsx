"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Archive,
	ChartLine,
	CircleSlash,
	SlidersHorizontal,
} from "lucide-react";
import {
	type ReactElement,
	type ReactNode,
	useEffect,
	useRef,
	useState,
} from "react";
import { classifyDeviceError } from "../../../../lib/device-management/workspace/errors";
import { enumLabel } from "../copy/enum-labels";
import { errorCopy } from "../copy/error-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GateNotice, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import {
	useArchiveUsage,
	useDeviceWorkspace,
	useGate,
	useHubSupport,
} from "../workspace";
import type { ReadersEdit, ReadersMode } from "./history-readers-sheet";
import {
	type MetricSample,
	amount,
	asRecord,
	bytesText,
	dayLabel,
	logRecords,
	metricSamples,
	recordsOf,
} from "./observe-data";
import { DeviceResourceCells, ServiceResourceCells } from "./resource-cells";
import { pauseReason } from "./timeline-sentences";
import {
	type ArchiveContent,
	type ArchiveList,
	type ArchiveRow,
	DEVICE_SCOPE,
	type HistoryKind,
	type HistoryRead,
	type HistoryStream,
	type Recording,
	interimDigest,
	policyAppliedOf,
	readArchive,
	recordingOf,
	useArchiveList,
} from "./use-history";
import { type ObserveTarget, gateLine } from "./use-observe-target";

const PAGE = 5;
const DAY_S = 86_400;

function LogLines({ content }: Readonly<{ content: ArchiveContent }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const lines = logRecords(recordsOf({ records: content.records }));
	return (
		<div
			role="log"
			aria-label={t("observe.retained.linesLabel", "Retained log lines")}
			className="max-h-36 overflow-auto rounded-lg bg-surface-sunken py-1 font-mono text-xs/5"
		>
			{lines.length ? (
				lines.map((line) =>
					line.kind === "gap" ? (
						<div
							key={line.id}
							className="flex items-center gap-1.5 border-y border-dashed border-unknown-line px-3 font-sans whitespace-nowrap text-muted-foreground"
						>
							<CircleSlash aria-hidden className="size-3.25 shrink-0" />
							{t("observe.retained.droppedLines", {
								count: line.count ?? 0,
								defaultValue_one: "{{count, number}} line dropped here.",
								defaultValue_other: "{{count, number}} lines dropped here.",
							})}
						</div>
					) : (
						<div
							key={line.id}
							className="grid grid-cols-[64px_52px_max-content] gap-3 px-3 whitespace-pre"
						>
							<span className="text-muted-foreground tabular-nums">
								{time.clock(line.at)}
							</span>
							<span
								className={
									line.stream === "stderr"
										? "text-warning"
										: "text-muted-foreground"
								}
							>
								{enumLabel(t, "logStream", line.stream)}
							</span>
							<span
								className={
									line.stream === "stderr" ? "text-warning" : undefined
								}
							>
								{line.message}
							</span>
						</div>
					),
				)
			) : (
				<p className="px-3 font-sans text-muted-foreground">
					{t("observe.retained.noLines", "This chunk holds no lines.")}
				</p>
			)}
		</div>
	);
}

function MetricSamples({
	target,
	samples,
}: Readonly<{ target: ObserveTarget; samples: readonly MetricSample[] }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const first = samples[0];
	const last = samples.at(-1);
	if (!first || !last)
		return (
			<p className="text-xs text-muted-foreground">
				{t("observe.retained.noSamples", "This chunk holds no samples.")}
			</p>
		);
	return (
		<div className="overflow-hidden rounded-lg border border-hairline">
			<p className="border-b border-hairline bg-surface-sunken px-4 py-1.5 text-xs text-muted-foreground">
				{t("observe.retained.samples", {
					count: samples.length,
					from: time.clock(first.at),
					to: time.clock(last.at),
					defaultValue_one: "{{count, number}} sample at {{from}}",
					defaultValue_other:
						"{{count, number}} samples from {{from}} to {{to}}",
				})}
			</p>
			{target.serviceId === null ? (
				<DeviceResourceCells samples={samples} lastKnown services={null} />
			) : target.service ? (
				<ServiceResourceCells
					samples={samples}
					lastKnown
					service={target.service}
				/>
			) : null}
		</div>
	);
}

type ArchiveUsage = ReturnType<typeof useArchiveUsage>;

function ownPlanLine(
	t: DevicesT,
	plan: NonNullable<ArchiveUsage["data"]>,
): string {
	if (plan.max_bytes <= 0)
		return t(
			"observe.retained.planNone",
			"Your plan ({{tier}}) doesn't store history on the hub.",
			{ tier: plan.tier },
		);
	return t("observe.retained.plan", {
		count: Math.round(plan.retention_seconds / DAY_S),
		tier: plan.tier,
		max: bytesText(plan.max_bytes),
		used: bytesText(plan.used_bytes),
		defaultValue_one:
			"{{tier}} plan · kept {{count, number}} day · up to {{max}} · {{used}} used",
		defaultValue_other:
			"{{tier}} plan · kept {{count, number}} days · up to {{max}} · {{used}} used",
	});
}

/** The tiers of a hub that stores history, as its capability answer lists them. */
function tierLimits(tiers: unknown): { max: number; keep: number }[] {
	return Object.values(asRecord(tiers) ?? {}).flatMap((entry) => {
		const tier = asRecord(entry);
		const max = amount(tier?.max_bytes);
		const keep = amount(tier?.retention_seconds);
		return max && keep !== undefined ? [{ max, keep }] : [];
	});
}

/** An older hub can't say what this account uses: the line names the most any plan keeps. */
function interimPlanLine(t: DevicesT, tiers: unknown): string {
	const limits = tierLimits(tiers);
	if (!limits.length)
		return t(
			"observe.retained.planUnknown",
			"This hub doesn't report how much history your plan stores.",
		);
	return t("observe.retained.planInterim", {
		count: Math.round(Math.max(...limits.map((tier) => tier.keep)) / DAY_S),
		max: bytesText(Math.max(...limits.map((tier) => tier.max))),
		defaultValue_one:
			"Depending on your plan, the hub keeps history up to {{count, number}} day and up to {{max}}. This hub doesn't report how much you use.",
		defaultValue_other:
			"Depending on your plan, the hub keeps history up to {{count, number}} days and up to {{max}}. This hub doesn't report how much you use.",
	});
}

function planLine(
	t: DevicesT,
	usage: ArchiveUsage,
	tiers: unknown,
): string | null {
	if (usage.data) return ownPlanLine(t, usage.data);
	return usage.missingOnHub ? interimPlanLine(t, tiers) : null;
}

type Paused = Extract<Recording, { state: "paused" }>;

/** "Recording paused on 29 Sept because the readers list expired." */
function pausedTitle(
	t: DevicesT,
	recording: Paused,
	day: (atS: number) => string,
): string {
	const { reason, since } = recording;
	if (reason === null)
		return since === undefined
			? t("observe.retained.pausedPlain", "Recording is paused.")
			: t(
					"observe.retained.pausedPlainSince",
					"Recording paused on {{date}}.",
					{
						date: day(since),
					},
				);
	const because = pauseReason(t, reason);
	return since === undefined
		? t("observe.retained.paused", "Recording is paused because {{reason}}.", {
				reason: because,
			})
		: t(
				"observe.retained.pausedSince",
				"Recording paused on {{date}} because {{reason}}.",
				{ date: day(since), reason: because },
			);
}

function PausedNotice({
	recording,
	metrics,
	resume,
}: Readonly<{
	recording: Paused;
	metrics: boolean;
	/** Owner only. */
	resume: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<GateNotice
			kind="busy"
			title={pausedTitle(t, recording, (at) => dayLabel(time, at))}
			text={
				metrics
					? t(
							"observe.retained.pausedMetrics",
							"No metrics were retained since then. Live values above still work.",
						)
					: t(
							"observe.retained.pausedLogs",
							"No logs were retained since then. Live logs above still work.",
						)
			}
			actions={resume}
		/>
	);
}

type Opened =
	| { id: string; content: ArchiveContent }
	| { id: string; failure: string };

/** A chunk sealed before this computer became a reader can't be opened here. */
const readFailure = (t: DevicesT, error: unknown): string =>
	error instanceof Error &&
	/recipient|wrapped|not a reader/i.test(error.message)
		? t(
				"observe.retained.notForYou",
				"This chunk wasn't encrypted for this computer. Ask the owner to add it as a reader.",
			)
		: errorCopy(t, classifyDeviceError(error).code);

/** One chunk open at a time; decrypted content never outlives the keys it was read with. */
function useChunkReader(
	target: ObserveTarget,
	scope: string,
	kind: HistoryKind,
) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const { deviceId, locked } = target;
	const [opened, setOpened] = useState<Opened | null>(null);
	const [busy, setBusy] = useState<string | null>(null);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	useEffect(() => {
		if (locked) setOpened(null);
	}, [locked]);

	const toggle = async (row: ArchiveRow) => {
		const id = row.archive_id;
		if (opened?.id === id) {
			setOpened(null);
			return;
		}
		setBusy(id);
		const result: Opened = await readArchive(workspace, {
			deviceId,
			scope,
			kind,
			archiveId: id,
		}).then(
			(content) => ({ id, content }),
			(error: unknown) => ({ id, failure: readFailure(t, error) }),
		);
		if (!alive.current) return;
		setOpened(result);
		setBusy(null);
	};
	return { opened, busy, toggle };
}

/** Only a reader of the list can open what the hub stores. */
function useReadGate(target: ObserveTarget, stream: HistoryStream | undefined) {
	const workspace = useDeviceWorkspace();
	const { deviceId, serviceId } = target;
	const controller = target.locked
		? undefined
		: workspace.keys.controller(deviceId);
	const myId = controller?.publicBundle().controller_key.x;
	const member = stream?.roster?.recipients.some(
		(row) => row.recipient_id === myId,
	);
	return useGate("history_read_cloud", deviceId, {
		...(serviceId ? { placementId: serviceId } : {}),
		extra: member === undefined ? {} : { rosterMember: member },
	});
}

function ChunkContent({
	target,
	open,
	metrics,
}: Readonly<{ target: ObserveTarget; open: Opened; metrics: boolean }>) {
	const { t } = useTranslation("devices");
	if ("failure" in open)
		return <InlineResult tone="critical">{open.failure}</InlineResult>;
	const { records, dropped } = open.content;
	return (
		<>
			{dropped > 0 ? (
				<p className="text-xs text-muted-foreground">
					{t("observe.retained.dropped", {
						count: dropped,
						defaultValue_one:
							"{{count, number}} record before this chunk was dropped on the device.",
						defaultValue_other:
							"{{count, number}} records before this chunk were dropped on the device.",
					})}
				</p>
			) : null}
			{metrics ? (
				<MetricSamples target={target} samples={metricSamples({ records })} />
			) : (
				<LogLines content={open.content} />
			)}
		</>
	);
}

function ChunkRow({
	target,
	row,
	metrics,
	open,
	busy,
	gate,
	onRead,
}: Readonly<{
	target: ObserveTarget;
	row: ArchiveRow;
	metrics: boolean;
	open: Opened | null;
	busy: boolean;
	gate: Gate | null;
	onRead(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<li
			data-chunk={row.archive_id}
			className="flex flex-col gap-2 border-t border-hairline py-2.5 first:border-t-0 first:pt-0"
		>
			<div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
				<span className="font-mono text-ui tabular-nums">
					{t("observe.retained.sealed", "sealed {{at}}", {
						at: time.at(row.created_at),
					})}
				</span>
				<span className="text-ui text-muted-foreground">
					{t(
						"observe.retained.stored",
						"stored on the hub · deleted {{date}}",
						{ date: dayLabel(time, row.expires_at) },
					)}
				</span>
				<span className="flex-1" />
				<GatedAction gate={gate}>
					<DvButton
						size="sm"
						busy={busy}
						aria-expanded={!!open}
						onClick={onRead}
					>
						{open
							? t("observe.retained.hide", "Hide")
							: t("observe.retained.read", "Read")}
					</DvButton>
				</GatedAction>
			</div>
			{open ? (
				<ChunkContent target={target} open={open} metrics={metrics} />
			) : null}
		</li>
	);
}

/** Chunks are listed and read with keys on this computer; without them nothing is asked. */
function keysState(t: DevicesT, target: ObserveTarget): ReactElement | null {
	if (target.revoked)
		return (
			<StateView
				kind="notloaded"
				title={t("observe.retained.revokedTitle", "Not available")}
				text={t("observe.retained.revokedText", "Revoked devices aren't read.")}
			/>
		);
	if (target.hasKeys && !target.locked) return null;
	return (
		<StateView
			kind="locked"
			title={t("observe.retained.lockedTitle", "Locked")}
			text={t(
				"observe.retained.lockedText",
				"Retained history is encrypted for keys on this computer. Unlock {{device}} to list and read it.",
				{ device: target.name },
			)}
		/>
	);
}

function listState(
	t: DevicesT,
	list: ArchiveList,
	kindText: string,
): ReactElement | null {
	if (list.error)
		return (
			<StateView
				kind="error"
				title={t(
					"observe.retained.listFailed",
					"The hub didn't return the list of retained {{kind}}",
					{ kind: kindText },
				)}
				actions={
					<DvButton size="sm" onClick={() => void list.refresh()}>
						{t("observe.retained.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	if (list.loading)
		return (
			<StateView
				kind="loading"
				title={t("observe.retained.loading", "Checking the hub for history…")}
			/>
		);
	return null;
}

/** The hub answered and stores no chunk: says why, as far as this computer knows. */
function NotRetained({
	target,
	recording,
	kindText,
	setUp,
}: Readonly<{
	target: ObserveTarget;
	recording: Recording | undefined;
	kindText: string;
	/** Owner only, once the device said what is set up. */
	setUp: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const { name } = target;
	const subject = target.serviceId ?? name;
	if (recording?.state === "recording")
		return (
			<StateView
				kind="notloaded"
				title={t("observe.retained.nothingYet", "Nothing sealed yet")}
				text={t(
					"observe.retained.nothingYetText",
					"{{device}} records the {{kind}} of {{subject}} and seals them every 30 s. The first chunk appears shortly.",
					{ device: name, kind: kindText, subject },
				)}
			/>
		);
	if (setUp)
		return (
			<StateView
				kind="empty"
				icon={Archive}
				title={t("observe.retained.notRecorded", "Not recorded")}
				text={t(
					"observe.retained.notRecordedText",
					"History isn't set up for {{subject}}, so its {{kind}} aren't kept on the hub.",
					{ subject, kind: kindText },
				)}
				actions={setUp}
			/>
		);
	if (target.owner)
		return (
			<StateView
				kind="notloaded"
				title={t("observe.retained.unknown", "Not loaded")}
				text={t(
					"observe.retained.unknownText",
					"Whether history is set up for {{subject}} is read from {{device}} over a live connection.",
					{ subject, device: name },
				)}
			/>
		);
	return (
		<StateView
			kind="noaccess"
			title={t("observe.retained.noAccess", "No retained history you can read")}
			text={t(
				"observe.retained.noAccessText",
				"History isn't set up for {{subject}}, or this computer isn't one of its readers.",
				{ subject },
			)}
		/>
	);
}

function retainedTitle(
	t: DevicesT,
	metrics: boolean,
	serviceId: string | null,
): string {
	if (metrics)
		return t("observe.retained.titleMetrics", "Retained metrics history");
	return serviceId
		? t("observe.retained.titleService", "Retained history · this service")
		: t("observe.retained.titleLogs", "Retained history · logs");
}

/** The owner of a metrics block is told where its readers are set. */
function retainedFoot(
	t: DevicesT,
	target: ObserveTarget,
	metrics: boolean,
): string | null {
	if (!metrics || !target.owner) return null;
	return target.serviceId
		? t(
				"observe.retained.footService",
				"Retained records are sealed every 30 s and encrypted for the readers you pick. Readers are set in Activity & logs › History for this service.",
			)
		: t(
				"observe.retained.footDevice",
				"Retained records are sealed every 30 s and encrypted for the readers you pick. Readers are set in Activity & logs › History settings.",
			);
}

export interface RetainedHistoryProps {
	target: ObserveTarget;
	kind: HistoryKind;
	history: HistoryRead;
	onEdit(edit: ReadersEdit): void;
}

/** SPEC §5.2 "Retained history": the sealed chunks the hub stores, read with this computer's reader key. */
export function RetainedHistory({
	target,
	kind,
	history,
	onEdit,
}: Readonly<RetainedHistoryProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { deviceId, serviceId } = target;
	const scope = serviceId ?? DEVICE_SCOPE;
	const stream = history.streams?.find(
		(row) => row.scope === scope && row.kind === kind,
	);
	const readGate = useReadGate(target, stream);
	const editGate = useGate("approve_history_readers", deviceId, {
		extra: { policyApplied: policyAppliedOf(target) },
	});
	const usage = useArchiveUsage();
	const hub = useHubSupport();
	const list = useArchiveList(
		deviceId,
		scope,
		kind,
		target.hasKeys && !target.locked && !target.revoked,
	);
	const reader = useChunkReader(target, scope, kind);
	const [shown, setShown] = useState(PAGE);

	const recording = stream
		? recordingOf(
				stream,
				Math.floor(time.nowS / 30) * 30,
				interimDigest(target),
			)
		: undefined;
	const rows = list.rows ?? [];
	const metrics = kind === "metrics";
	const kindText = enumLabel(t, "archiveKind", kind).toLowerCase();
	const readGated = gateLine(t, readGate, time);
	const editGated = gateLine(t, editGate, time);
	const planGated: Gate | null =
		usage.data !== undefined && usage.data.max_bytes <= 0
			? {
					kind: "plan",
					reason: t(
						"observe.retained.planGate",
						"Your plan doesn't store history on the hub.",
					),
				}
			: null;
	const edit = (mode: ReadersMode) => {
		if (stream)
			onEdit({
				scope,
				kinds: [kind],
				projectId: target.service?.projectId ?? null,
				mode,
				streams: [stream],
			});
	};
	const owned = target.owner && stream !== undefined;

	const paused =
		recording?.state === "paused" ? (
			<PausedNotice
				recording={recording}
				metrics={metrics}
				resume={
					owned ? (
						<GatedAction gate={editGated}>
							<DvButton size="sm" onClick={() => edit("resume")}>
								{t("observe.retained.resume", "Resume recording…")}
							</DvButton>
						</GatedAction>
					) : null
				}
			/>
		) : null;
	const setUp = owned ? (
		<GatedAction gate={planGated ?? editGated}>
			<DvButton
				size="sm"
				icon={SlidersHorizontal}
				onClick={() => edit("setup")}
			>
				{t("observe.retained.setUp", "Set up history…")}
			</DvButton>
		</GatedAction>
	) : null;
	const chunks = (
		<>
			{paused}
			<ul className="flex flex-col">
				{rows.slice(0, shown).map((row) => (
					<ChunkRow
						key={row.archive_id}
						target={target}
						row={row}
						metrics={metrics}
						open={reader.opened?.id === row.archive_id ? reader.opened : null}
						busy={reader.busy === row.archive_id}
						gate={readGated}
						onRead={() => void reader.toggle(row)}
					/>
				))}
			</ul>
			{rows.length > shown ? (
				<div>
					<DvButton
						size="sm"
						variant="ghost"
						onClick={() => setShown((count) => count + PAGE)}
					>
						{t("observe.retained.older", "Show older chunks")}
					</DvButton>
				</div>
			) : null}
		</>
	);
	const body =
		keysState(t, target) ??
		(rows.length
			? chunks
			: (listState(t, list, kindText) ??
				paused ?? (
					<NotRetained
						target={target}
						recording={recording}
						kindText={kindText}
						setUp={setUp}
					/>
				)));
	const plan = planLine(t, usage, hub.support.telemetryTiers);

	return (
		<Block
			id={metrics ? "observe-retained-metrics" : "observe-retained-logs"}
			icon={metrics ? ChartLine : Archive}
			title={retainedTitle(t, metrics, serviceId)}
			count={rows.length || undefined}
			stamp={
				list.rows ? (
					<FreshnessStamp
						source="hub"
						age="current"
						observedAt={Math.floor(list.checkedAt / 1000)}
					/>
				) : null
			}
			foot={retainedFoot(t, target, metrics)}
		>
			{body}
			{plan ? (
				<p data-plan-line="" className="text-xs text-muted-foreground">
					{plan}
				</p>
			) : null}
		</Block>
	);
}
