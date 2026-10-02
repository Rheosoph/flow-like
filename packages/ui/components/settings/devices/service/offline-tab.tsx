"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	CircleCheck,
	CircleHelp,
	CirclePause,
	Cloud,
	Database,
	Hourglass,
	ListChecks,
	LoaderCircle,
	type LucideIcon,
	RefreshCw,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import {
	type OfflineOperationSummary,
	agentSupports,
	readOfflineOperations,
} from "../../../../lib/device-management/agent-reads";
import type {
	AgentFeatures,
	AttentionInput,
	Freshness,
} from "../../../../lib/device-management/model/types";
import {
	type OfflineQueueStatus,
	retryOfflineQueue,
	skipOfflineQueue,
} from "../../../../lib/device-management/offline-queue";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import type {
	ManagementRejection,
	ManagementResponse,
} from "../../../../lib/device-management/types";
import { humanFileSize } from "../../../../lib/utils";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline, GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { KvRow } from "../primitives/key-value-list";
import { Meter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, TONE_LINE, TONE_TEXT, cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import type { ServiceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import {
	type GateTarget,
	type LiveStreamView,
	deviceCall,
	useAttentionState,
	useDeviceAction,
	useGates,
	useLiveStream,
} from "../workspace";
import { ageText } from "./config-diff";
import { HEAD_NEEDS_YOU, type QueueTarget, queueTarget } from "./config-model";
import {
	ActionResults,
	ConfigStamp,
	ConfigUnavailable,
	FactList,
	Mono,
	type Note,
	TABLE_RESET,
	gateLine,
} from "./config-parts";
import {
	ACCESS_REFUSED,
	type ServiceConfigRead,
	useServiceConfig,
} from "./use-service-config";

const QUEUES_EVERY_MS = 15_000;
const FINISHED_SHOWN = 5;

type Head = NonNullable<OfflineQueueStatus["head"]>;
type Budgets = { maxBytes: number; maxAgeS: number } | undefined;

interface QueuesRead {
	queues: OfflineQueueStatus[] | undefined;
	freshness: Freshness;
	live: boolean;
	/** The device turned the read down: without View status on this service, or for a reason of its own. */
	refused: ManagementRejection | undefined;
	/** The read failed and no earlier read is left to show. */
	failed: boolean;
	loading: boolean;
	refresh(): Promise<void>;
}

type Queues = OfflineQueueStatus[];
type QueueStream = LiveStreamView<Queues>;

/** What the stream read, or what an earlier read of this session left behind. */
function queuesRead(
	stream: QueueStream,
	known: Queues | undefined,
	live: boolean,
) {
	const queues = stream.data ?? known;
	const refused = stream.rejected;
	const failed =
		!queues && !refused && stream.started && stream.freshness.age === "error";
	const read: QueuesRead = {
		queues,
		freshness: stream.started ? stream.freshness : keptFreshness(!!queues),
		live,
		refused,
		failed,
		loading: live && !queues && !refused && !failed,
		refresh: stream.refresh,
	};
	return read;
}

/** Queues load on connect and refresh while this tab is open; after a disconnect the last read stays as last known. */
function useQueues(deviceId: string, serviceId: string) {
	const { input } = useAttentionState();
	const facts = input.live[deviceId];
	const state = facts?.state.kind;
	const live = state === "live" || state === "renewing";
	const spec = live ? queueSpec(serviceId) : null;
	const stream: QueueStream = useLiveStream(deviceId, spec);
	const known = facts?.offlineQueues?.[serviceId];
	return useMemo(() => queuesRead(stream, known, live), [stream, known, live]);
}

const queueSpec = (serviceId: string) => ({
	kind: "offline_queues" as const,
	placementId: serviceId,
	everyMs: QUEUES_EVERY_MS,
});

/** Queues read earlier in this session stay as last known; without any, nothing is loaded. */
function keptFreshness(known: boolean) {
	const freshness: Freshness = known
		? { src: "live", age: "lastknown" }
		: { src: "live", age: "notloaded" };
	return freshness;
}

/* BG18: what kind of change waits, and what happened to the ones before it. */

interface QueueDetail {
	head?: OfflineOperationSummary;
	finished: OfflineOperationSummary[];
}

interface DetailsRead {
	/** False on an agent without the lookup: the interim line renders instead. */
	supported: boolean;
	byScope: Record<string, QueueDetail>;
}

/** The change at the head of a queue and its last finished ones; `null` when the agent doesn't know the lookup. */
async function readQueueDetail(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	serviceId: string,
	queue: OfflineQueueStatus,
): Promise<QueueDetail | null> {
	const scope = { placementId: serviceId, scope: queue.scope };
	const headId = queue.head?.operation_id;
	const open = headId
		? await readOfflineOperations(call, features, { ...scope, limit: 1 })
		: undefined;
	if (open?.kind === "unsupported") return null;
	const done = await readOfflineOperations(call, features, {
		...scope,
		limit: FINISHED_SHOWN,
		terminal: true,
	});
	if (done.kind === "unsupported") return null;
	const head = open?.data.operations.find(
		(operation) => operation.operation_id === headId,
	);
	return { ...(head ? { head } : {}), finished: done.data.operations };
}

function useQueueDetails(
	deviceId: string,
	serviceId: string,
	queues: readonly OfflineQueueStatus[] | undefined,
	live: boolean,
): DetailsRead {
	const { workspace, input } = useAttentionState();
	const features = input.live[deviceId]?.inspection?.value.features;
	const supported = agentSupports(features, "offline_lookup");
	const heads = (queues ?? [])
		.map((queue) => `${queue.scope}:${queue.head?.operation_id ?? ""}`)
		.join("|");
	const query = useQuery({
		queryKey: [
			"devices",
			workspace.scopeKey,
			"queue-detail",
			deviceId,
			serviceId,
			heads,
		],
		queryFn: async (): Promise<Record<string, QueueDetail> | null> => {
			const call = deviceCall(workspace, deviceId, "poll");
			const byScope: Record<string, QueueDetail> = {};
			for (const queue of queues ?? []) {
				const detail = await readQueueDetail(call, features, serviceId, queue);
				if (!detail) return null;
				byScope[queue.scope] = detail;
			}
			return byScope;
		},
		enabled: live && supported && !!queues?.length,
		staleTime: QUEUES_EVERY_MS,
		// Read with the device's keys: it goes when the tab closes, like the settings.
		gcTime: 0,
		retry: false,
		meta: { persist: false },
	});
	return useMemo(
		() => ({
			supported: supported && query.data !== null,
			byScope: query.data ?? {},
		}),
		[supported, query.data],
	);
}

/* Copy. */

const PURPOSE = {
	storage: (t) =>
		t("devices:serviceConfig.buffer.purpose.storage", "Project storage"),
	user: (t) => t("devices:serviceConfig.buffer.purpose.user", "User files"),
	files: (t) =>
		t("devices:serviceConfig.buffer.purpose.files", "Project files"),
	temporary: (t) =>
		t("devices:serviceConfig.buffer.purpose.temporary", "Temporary files"),
} satisfies Record<string, (t: DevicesT) => string>;

const TABLE_PURPOSE = {
	storage: PURPOSE.storage,
	user: (t) => t("devices:serviceConfig.buffer.purpose.userData", "User data"),
} satisfies Record<string, (t: DevicesT) => string>;

/** The same stores inside a sentence ("table records in project storage"). */
const PURPOSE_INLINE = {
	storage: (t) =>
		t("devices:serviceConfig.buffer.purposeInline.storage", "project storage"),
	user: (t) =>
		t("devices:serviceConfig.buffer.purposeInline.user", "user files"),
	files: (t) =>
		t("devices:serviceConfig.buffer.purposeInline.files", "project files"),
	temporary: (t) =>
		t(
			"devices:serviceConfig.buffer.purposeInline.temporary",
			"temporary files",
		),
} satisfies Record<string, (t: DevicesT) => string>;

const TABLE_PURPOSE_INLINE = {
	storage: PURPOSE_INLINE.storage,
	user: (t) =>
		t("devices:serviceConfig.buffer.purposeInline.userData", "user data"),
} satisfies Record<string, (t: DevicesT) => string>;

type PurposeLabels = Record<string, (t: DevicesT) => string>;

/** Where a buffered table or folder lives, in the app's words. */
function purposeLabel(
	t: DevicesT,
	table: boolean,
	purpose: string,
	inline: boolean,
): string {
	const names: PurposeLabels = table ? TABLE_PURPOSE : PURPOSE;
	const inSentence: PurposeLabels = table
		? TABLE_PURPOSE_INLINE
		: PURPOSE_INLINE;
	return (
		(inline ? inSentence : names)[purpose]?.(t) ??
		t("devices:serviceConfig.buffer.purpose.other", "another store")
	);
}

/** "table notes in Project storage" / "folder exports in project files": what buffered changes write to. */
export function BufferTarget({
	kind,
	name,
	purpose,
	inline = false,
}: Readonly<{
	kind: "table" | "file" | "folder";
	name: string;
	purpose: string;
	inline?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const values = {
		where: purposeLabel(t, kind === "table", purpose, inline),
	};
	const components = { 1: <Mono>{name}</Mono> };
	if (kind === "table")
		return (
			<Trans
				t={t}
				i18nKey="serviceConfig.buffer.targetTable"
				defaults="table <1/> in {{where}}"
				values={values}
				components={components}
			/>
		);
	return kind === "folder" ? (
		<Trans
			t={t}
			i18nKey="serviceConfig.buffer.targetFolder"
			defaults="folder <1/> in {{where}}"
			values={values}
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="serviceConfig.buffer.targetFile"
			defaults="file <1/> in {{where}}"
			values={values}
			components={components}
		/>
	);
}

function TargetText({ target }: Readonly<{ target: QueueTarget }>) {
	const { t } = useTranslation("devices");
	if (!target)
		return (
			<span className="text-muted-foreground">
				{t("serviceConfig.buffer.targetUnknown", "Not named by the device")}
			</span>
		);
	return (
		<BufferTarget
			kind={target.kind}
			name={target.name}
			purpose={target.purpose}
		/>
	);
}

interface Look {
	tone: ChipTone;
	icon: LucideIcon;
	spin?: boolean;
	text: string;
}

function headLook(t: DevicesT, state: Head["state"]): Look {
	const looks: Record<Head["state"], Look> = {
		pending: {
			tone: "outline",
			icon: Hourglass,
			text: t("devices:serviceConfig.buffer.head.pending", "Waiting"),
		},
		attempting: {
			tone: "info",
			icon: LoaderCircle,
			spin: true,
			text: t("devices:serviceConfig.buffer.head.attempting", "Sending"),
		},
		conflict: {
			tone: "warning",
			icon: TriangleAlert,
			text: t(
				"devices:serviceConfig.buffer.head.conflict",
				"Conflicts with newer cloud data",
			),
		},
		blocked: {
			tone: "warning",
			icon: TriangleAlert,
			text: t("devices:serviceConfig.buffer.head.blocked", "Stuck"),
		},
		outcome_unknown: {
			tone: "warning",
			icon: CircleHelp,
			text: t(
				"devices:serviceConfig.buffer.head.outcomeUnknown",
				"Might already be saved",
			),
		},
	};
	return looks[state];
}

function queueLook(t: DevicesT, queue: OfflineQueueStatus): Look {
	if (queue.quarantined)
		return {
			tone: "warning",
			icon: CirclePause,
			text: t(
				"devices:serviceConfig.buffer.queue.paused",
				"Paused: cloud access changed",
			),
		};
	const needs: Partial<Record<Head["state"], string>> = {
		conflict: t(
			"devices:serviceConfig.buffer.queue.conflict",
			"Needs you: conflict",
		),
		blocked: t(
			"devices:serviceConfig.buffer.queue.blocked",
			"Needs you: stuck",
		),
		outcome_unknown: t(
			"devices:serviceConfig.buffer.queue.outcomeUnknown",
			"Needs you: might already be saved",
		),
	};
	const need = queue.head ? needs[queue.head.state] : undefined;
	if (need) return { tone: "warning", icon: TriangleAlert, text: need };
	if (queue.pending_count)
		return {
			tone: "info",
			icon: LoaderCircle,
			spin: true,
			text: t("devices:serviceConfig.buffer.queue.sending", "Sending"),
		};
	return {
		tone: "good",
		icon: CircleCheck,
		text: t("devices:serviceConfig.buffer.queue.upToDate", "Up to date"),
	};
}

function Chip({ look }: Readonly<{ look: Look }>) {
	return (
		<StatusChip tone={look.tone} icon={look.icon} spin={look.spin}>
			{look.text}
		</StatusChip>
	);
}

const needsYou = (queue: OfflineQueueStatus) =>
	queue.quarantined || (!!queue.head && HEAD_NEEDS_YOU.has(queue.head.state));

function changeKind(t: DevicesT, kind: string | null | undefined): string {
	const kinds: Record<string, string> = {
		table_insert: t("devices:serviceConfig.buffer.kind.insert", "Adds rows"),
		table_upsert: t(
			"devices:serviceConfig.buffer.kind.upsert",
			"Adds or updates rows",
		),
		table_update: t("devices:serviceConfig.buffer.kind.update", "Updates rows"),
		table_delete: t("devices:serviceConfig.buffer.kind.delete", "Deletes rows"),
		file_put: t("devices:serviceConfig.buffer.kind.filePut", "Writes a file"),
		file_delete: t(
			"devices:serviceConfig.buffer.kind.fileDelete",
			"Deletes a file",
		),
	};
	return (
		(kind ? kinds[kind] : undefined) ??
		t("devices:serviceConfig.buffer.kind.other", "Another kind of change")
	);
}

function finishedState(t: DevicesT, state: string): string {
	const states: Record<string, string> = {
		applied: t("devices:serviceConfig.buffer.finished.applied", "Sent"),
		skipped: t("devices:serviceConfig.buffer.finished.skipped", "Discarded"),
		superseded: t(
			"devices:serviceConfig.buffer.finished.superseded",
			"Replaced by a newer change",
		),
	};
	return (
		states[state] ??
		t("devices:serviceConfig.buffer.finished.other", "Finished")
	);
}

/* Whose approval a queue replays under (the scope is a hash; the name is derived). */

interface Approval {
	mine: boolean;
	replacedAt?: number;
}

function approvalOf(
	input: AttentionInput,
	deviceId: string,
	serviceId: string,
): Approval {
	const current = input.resources[deviceId]?.grants.find(
		(grant) => grant.placement_id === serviceId && grant.status === "active",
	);
	return {
		mine: !current || current.delegating_user_id === input.me,
		...(current?.created_at ? { replacedAt: current.created_at } : {}),
	};
}

function queueName(
	t: DevicesT,
	time: AreaTime,
	queue: OfflineQueueStatus,
	approval: Approval,
): string {
	if (!queue.quarantined)
		return approval.mine
			? t(
					"devices:serviceConfig.buffer.queue.current",
					"For your current approval",
				)
			: t(
					"devices:serviceConfig.buffer.queue.currentOther",
					"For the current approval",
				);
	const earlier = approval.mine
		? t(
				"devices:serviceConfig.buffer.queue.earlier",
				"For your earlier approval",
			)
		: t(
				"devices:serviceConfig.buffer.queue.earlierOther",
				"For an earlier approval",
			);
	return approval.replacedAt
		? t(
				"devices:serviceConfig.buffer.queue.replaced",
				"{{name}} (replaced {{at}})",
				{
					name: earlier,
					at: time.at(approval.replacedAt),
				},
			)
		: earlier;
}

/* Queues table. */

function SizeCell({
	queue,
	budgets,
}: Readonly<{ queue: OfflineQueueStatus; budgets: Budgets }>) {
	const { t } = useTranslation("devices");
	const used = humanFileSize(queue.pending_bytes);
	if (!budgets) return <span className="tabular-nums">{used}</span>;
	const text = t("serviceConfig.buffer.sizeOf", "{{used}} of {{max}}", {
		used,
		max: humanFileSize(budgets.maxBytes, false, 0),
	});
	const share = (queue.pending_bytes / budgets.maxBytes) * 100;
	return (
		<Meter
			label={text}
			caption={text}
			segments={[
				{
					value: queue.pending_bytes ? Math.max(0.8, share) : 0,
					tone: share > 80 ? "warning" : "neutral",
				},
			]}
		/>
	);
}

function OldestCell({
	queue,
	budgets,
}: Readonly<{ queue: OfflineQueueStatus; budgets: Budgets }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (queue.oldest_at === null) return <>–</>;
	const sub = queue.quarantined
		? t("serviceConfig.buffer.keptPaused", "kept while paused")
		: budgets
			? t(
					"serviceConfig.buffer.keptUntil",
					"of {{limit}} max · kept until {{until}}",
					{
						limit: ageText(t, budgets.maxAgeS),
						until: time.at(queue.oldest_at + budgets.maxAgeS),
					},
				)
			: null;
	return (
		<>
			<span title={time.abs(queue.oldest_at)}>{time.at(queue.oldest_at)}</span>
			{sub ? <CellSub>{sub}</CellSub> : null}
		</>
	);
}

function QueueRow({
	queue,
	budgets,
	approval,
}: Readonly<{
	queue: OfflineQueueStatus;
	budgets: Budgets;
	approval: Approval;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const col = {
		queue: t("serviceConfig.buffer.col.queue", "Queue"),
		state: t("serviceConfig.buffer.col.state", "State"),
		waiting: t("serviceConfig.buffer.col.waiting", "Waiting"),
		size: t("serviceConfig.buffer.col.size", "Size"),
		oldest: t("serviceConfig.buffer.col.oldest", "Oldest"),
		copy: t("serviceConfig.buffer.col.copy", "Table copy"),
	};
	return (
		<Tr data-queue={queue.quarantined ? "paused" : "open"}>
			<Td label={col.queue}>
				{queueName(t, time, queue, approval)}
				<CellSub>
					<IdRef
						id={queue.scope}
						copyLabel={t("serviceConfig.buffer.copyQueueId", "Copy queue ID")}
					/>
				</CellSub>
			</Td>
			<Td label={col.state}>
				<Chip look={queueLook(t, queue)} />
			</Td>
			<Td label={col.waiting} className="tabular-nums">
				{t("serviceConfig.buffer.count", "{{count, number}}", {
					count: queue.pending_count,
				})}
			</Td>
			<Td label={col.size}>
				<SizeCell queue={queue} budgets={budgets} />
			</Td>
			<Td label={col.oldest}>
				<OldestCell queue={queue} budgets={budgets} />
			</Td>
			<Td label={col.copy}>
				{queue.mirror_error ? (
					<span className={TONE_TEXT.warning}>{queue.mirror_error}</span>
				) : (
					<span
						className={cx("inline-flex items-center gap-1", TONE_TEXT.good)}
					>
						<CircleCheck aria-hidden className="size-3.25" />
						{t("serviceConfig.buffer.copyOk", "OK")}
					</span>
				)}
			</Td>
		</Tr>
	);
}

function queueSummary(t: DevicesT, queues: readonly OfflineQueueStatus[]) {
	const waiting = queues.reduce((sum, queue) => sum + queue.pending_count, 0);
	const need = queues.filter(needsYou).length;
	if (need)
		return t("devices:serviceConfig.buffer.summaryNeed", {
			count: need,
			waiting,
			defaultValue_one:
				"{{waiting, number}} waiting · {{count, number}} needs you",
			defaultValue_other:
				"{{waiting, number}} waiting · {{count, number}} need you",
		});
	return waiting
		? t(
				"devices:serviceConfig.buffer.summarySending",
				"{{waiting, number}} waiting · sending",
				{ waiting },
			)
		: t("devices:serviceConfig.buffer.summaryIdle", "Up to date · 0 waiting");
}

function FinishedChanges({
	queues,
	details,
}: Readonly<{ queues: readonly OfflineQueueStatus[]; details: DetailsRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const rows = queues.flatMap(
		(queue) => details.byScope[queue.scope]?.finished ?? [],
	);
	if (!details.supported || !rows.length) return null;
	return (
		<details data-finished="" className="border-t border-hairline px-4 py-2.5">
			<summary className="cursor-pointer text-xs text-muted-foreground">
				{t("serviceConfig.buffer.finished.title", {
					count: rows.length,
					defaultValue_one: "Last {{count, number}} finished change",
					defaultValue_other: "Last {{count, number}} finished changes",
				})}
			</summary>
			<ul className="mt-2 flex flex-col gap-1 text-ui">
				{rows.map((row) => (
					<li
						key={row.operation_id}
						className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5"
					>
						<span className="font-medium">{finishedState(t, row.state)}</span>
						<span className="min-w-0">
							<TargetText target={queueTarget(row.resource)} />
						</span>
						<span className="text-xs text-muted-foreground">
							{t("serviceConfig.buffer.finished.queued", "queued {{at}}", {
								at: time.at(row.created_at),
							})}
						</span>
					</li>
				))}
			</ul>
		</details>
	);
}

function QueuesBlock({
	serviceId,
	deviceId,
	read,
	budgets,
	details,
	summaries,
}: Readonly<{
	serviceId: string;
	deviceId: string;
	read: QueuesRead;
	budgets: Budgets;
	details: DetailsRead;
	/** The device's agent reports queue summaries in its encrypted status (BG11). */
	summaries: boolean;
}>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const queues = read.queues ?? [];
	const approval = approvalOf(input, deviceId, serviceId);
	const waiting = queues.reduce((sum, queue) => sum + queue.pending_count, 0);
	const head = (
		<tr>
			<Th>{t("serviceConfig.buffer.col.queue", "Queue")}</Th>
			<Th>{t("serviceConfig.buffer.col.state", "State")}</Th>
			<Th>{t("serviceConfig.buffer.col.waiting", "Waiting")}</Th>
			<Th>{t("serviceConfig.buffer.col.size", "Size")}</Th>
			<Th>{t("serviceConfig.buffer.col.oldest", "Oldest")}</Th>
			<Th>{t("serviceConfig.buffer.col.copy", "Table copy")}</Th>
		</tr>
	);
	return (
		<Block
			id="svc-queues"
			icon={Database}
			title={t("serviceConfig.buffer.queues", "Queues")}
			count={queues.length}
			summary={<span data-queue-summary="">{queueSummary(t, queues)}</span>}
			stamp={<FreshnessStamp {...stampOf(read.freshness)} />}
			flush
			foot={
				<span>
					{t(
						"serviceConfig.buffer.foot",
						"Changes {{service}} made while it couldn't reach the cloud. Each queue replays under the approval it was queued with.",
						{ service: serviceId },
					)}{" "}
					{summaries
						? null
						: t(
								"serviceConfig.buffer.footLive",
								"Badges on other pages update while a live connection is open.",
							)}{" "}
					{waiting
						? t("serviceConfig.buffer.footOff", {
								count: waiting,
								defaultValue_one:
									"Write buffering can be turned off once nothing waits: {{count, number}} change still waits.",
								defaultValue_other:
									"Write buffering can be turned off once nothing waits: {{count, number}} changes still wait.",
							})
						: null}
				</span>
			}
		>
			{queues.length ? (
				<DvTable
					label={t("serviceConfig.buffer.queues", "Queues")}
					cols={["23%", "26%", "8%", "17%", "15%", "11%"]}
					className={TABLE_RESET}
					head={head}
				>
					{queues.map((queue) => (
						<QueueRow
							key={queue.scope}
							queue={queue}
							budgets={budgets}
							approval={approval}
						/>
					))}
				</DvTable>
			) : (
				<div className="px-4 py-3">
					<StateView
						kind="empty"
						icon={Database}
						title={t("serviceConfig.buffer.none", "Up to date · 0 waiting")}
						text={t(
							"serviceConfig.buffer.noneText",
							"{{service}} has no buffered change on the device.",
							{ service: serviceId },
						)}
					/>
				</div>
			)}
			<FinishedChanges queues={queues} details={details} />
		</Block>
	);
}

/* Next change in line. */

const DISCARD_REASONS = [
	"cloud_newer",
	"not_needed",
	"blocks_others",
	"other",
] as const;
type DiscardReason = (typeof DISCARD_REASONS)[number];

function reasonLabel(t: DevicesT, reason: DiscardReason): string {
	const labels: Record<DiscardReason, string> = {
		cloud_newer: t(
			"devices:serviceConfig.buffer.reason.cloudNewer",
			"The cloud data is newer and correct",
		),
		not_needed: t(
			"devices:serviceConfig.buffer.reason.notNeeded",
			"The change is wrong or no longer needed",
		),
		blocks_others: t(
			"devices:serviceConfig.buffer.reason.blocksOthers",
			"It blocks the changes behind it",
		),
		other: t("devices:serviceConfig.buffer.reason.other", "Other"),
	};
	return labels[reason];
}

const DISCARD_REASON_BYTES = 512;

/** The reason the device files with the discard; at most 512 bytes (the device adds who did it). */
function discardText(t: DevicesT, reason: string | undefined, note?: string) {
	const label = DISCARD_REASONS.includes(reason as DiscardReason)
		? reasonLabel(t, reason as DiscardReason)
		: "";
	const text = [label, note].filter(Boolean).join(": ");
	const encoder = new TextEncoder();
	if (encoder.encode(text).length <= DISCARD_REASON_BYTES) return text;
	let clipped = "";
	for (const character of text) {
		const next = `${clipped}${character}`;
		if (encoder.encode(`${next}…`).length > DISCARD_REASON_BYTES) break;
		clipped = next;
	}
	return `${clipped}…`;
}

interface QueueActions {
	retry(queue: OfflineQueueStatus): Promise<void>;
	discard(queue: OfflineQueueStatus): Promise<void>;
	pending: boolean;
	resultKey: string;
	note: Note | null;
	dismiss(): void;
}

interface Retried {
	queue: OfflineQueueStatus;
	time: string;
}

const stillNeedsYou = (
	queues: readonly OfflineQueueStatus[] | undefined,
	before: OfflineQueueStatus,
) => {
	const now = queues?.find((queue) => queue.scope === before.scope)?.head;
	return (
		!!now &&
		now.operation_id === before.head?.operation_id &&
		HEAD_NEEDS_YOU.has(now.state)
	);
};

function useQueueActions(
	deviceId: string,
	serviceId: string,
	read: ServiceConfigRead,
	queues: QueuesRead,
): QueueActions {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { workspace } = useAttentionState();
	const actions = useDeviceAction();
	const [note, setNote] = useState<Note | null>(null);
	const [retried, setRetried] = useState<Retried | null>(null);
	const resultKey = `service-queue:${deviceId}/${serviceId}`;
	const projectId = read.service?.projectId;
	const { deviceLabel } = read;
	const refresh = queues.refresh;

	const target = useCallback(
		(queue: OfflineQueueStatus): GateTarget => ({
			placementId: serviceId,
			...(projectId ? { projectId } : {}),
			labels: { service: serviceId },
			extra: { queueQuarantined: queue.quarantined },
		}),
		[serviceId, projectId],
	);
	const activity = useMemo(
		() => ({
			kind: "offline_write_retry" as const,
			deviceName: deviceLabel,
			serviceId,
			...(projectId ? { projectId } : {}),
			href: {
				screen: "service" as const,
				deviceId,
				serviceId,
				tab: "offline" as const,
			},
		}),
		[deviceId, serviceId, projectId, deviceLabel],
	);
	const now = useCallback(
		() => time.clock(workspace.clock.now() / 1000),
		[time, workspace],
	);

	const retry = useCallback(
		async (queue: OfflineQueueStatus) => {
			setNote(null);
			setRetried(null);
			const outcome = await actions.run<ManagementResponse>({
				action: "offline_queue_retry",
				deviceId,
				target: target(queue),
				label: t("serviceConfig.buffer.retryLabel", "Try the change again"),
				resultKey,
				call: (context) => context.request(retryOfflineQueue(serviceId, queue)),
				activity,
			});
			if (outcome.status !== "done") return;
			await refresh();
			setRetried({ queue, time: now() });
		},
		[
			actions,
			deviceId,
			serviceId,
			target,
			resultKey,
			activity,
			refresh,
			now,
			t,
		],
	);

	const discard = useCallback(
		async (queue: OfflineQueueStatus) => {
			const head = queue.head;
			if (!head) return;
			setNote(null);
			setRetried(null);
			const behind = Math.max(0, queue.pending_count - 1);
			const rows: ConsequenceRows = {
				what: t(
					"serviceConfig.buffer.discard.what",
					"This change will never be applied to the cloud.",
				),
				stays: t("serviceConfig.buffer.discard.stays", {
					count: behind,
					defaultValue_one: "The {{count, number}} change behind it continues.",
					defaultValue_other:
						"The {{count, number}} changes behind it continue.",
				}),
				who: t(
					"serviceConfig.buffer.discard.who",
					"Nobody is told. The table copy on the device keeps its own version until the next sync.",
				),
				when: t("serviceConfig.buffer.discard.when", "Immediately."),
				undo: {
					reversible: false,
					text: t(
						"serviceConfig.buffer.discard.undo",
						"It can't be queued again.",
					),
				},
			};
			const attempted = head.attempts > 0;
			const outcome = await actions.run<ManagementResponse>({
				action: "offline_queue_skip",
				deviceId,
				target: target(queue),
				label: t("serviceConfig.buffer.discard.label", "Discard change"),
				consequence: rows,
				strength: "reason",
				confirm: {
					title: t(
						"serviceConfig.buffer.discard.title",
						"Discard this queued change?",
					),
					sub: t(
						"serviceConfig.buffer.discard.subtitle",
						"{{service}} · {{device}} · queued {{at}}",
						{
							service: serviceId,
							device: deviceLabel,
							at: time.at(head.created_at),
						},
					),
					tone: "danger",
					reasons: DISCARD_REASONS.map((value) => ({
						value,
						label: reasonLabel(t, value),
					})),
					requireCheck: attempted,
					...(attempted
						? {
								checkLabel: t(
									"serviceConfig.buffer.discard.ack",
									"It may already have reached the cloud. I still want to discard it.",
								),
							}
						: {}),
				},
				resultKey,
				call: (context) =>
					context.request(
						skipOfflineQueue(
							serviceId,
							queue,
							discardText(t, context.confirm.reason, context.confirm.note),
							attempted,
						),
					),
				activity,
			});
			if (outcome.status !== "done") return;
			await refresh();
			setNote({
				tone: "good",
				text: t(
					"serviceConfig.buffer.discard.done",
					"Discarded at {{time}}. The changes behind it continue.",
					{ time: now() },
				),
			});
		},
		[
			actions,
			deviceId,
			serviceId,
			target,
			resultKey,
			activity,
			refresh,
			deviceLabel,
			time,
			now,
			t,
		],
	);

	const current = queues.queues;
	return useMemo(() => {
		// What a retry did shows in the queue's next read: the sentence follows it.
		const retryNote: Note | null = !retried
			? null
			: stillNeedsYou(current, retried.queue)
				? {
						tone: "warning",
						text: t(
							"serviceConfig.buffer.retryStill",
							"Tried again at {{time}}: the change still needs you. Discard it, or change the cloud data so it no longer conflicts.",
							{ time: retried.time },
						),
					}
				: {
						tone: "good",
						text: t(
							"serviceConfig.buffer.retrySent",
							"Tried again at {{time}}. The device is sending the change; the queue shows the result.",
							{ time: retried.time },
						),
					};
		return {
			retry,
			discard,
			pending: actions.pending(resultKey),
			resultKey,
			note: note ?? retryNote,
			dismiss: () => {
				setNote(null);
				setRetried(null);
			},
		};
	}, [retry, discard, actions, resultKey, note, retried, current, t]);
}

const PLAIN_ERROR: Partial<Record<Head["state"], (t: DevicesT) => string>> = {
	conflict: (t) =>
		t(
			"devices:serviceConfig.buffer.plain.conflict",
			"The cloud copy of this record changed after this change was queued, so replaying it would overwrite newer data.",
		),
	blocked: (t) =>
		t(
			"devices:serviceConfig.buffer.plain.blocked",
			"The cloud refused this change.",
		),
	outcome_unknown: (t) =>
		t(
			"devices:serviceConfig.buffer.plain.outcomeUnknown",
			"The connection dropped while this change was being sent, so the device can't tell whether the cloud saved it.",
		),
};

function HeadError({
	queue,
	head,
}: Readonly<{ queue: OfflineQueueStatus; head: Head }>) {
	const { t } = useTranslation("devices");
	const plain = queue.quarantined
		? t(
				"serviceConfig.buffer.plain.paused",
				"The cloud access this change was queued under was replaced, so the device won't send it.",
			)
		: PLAIN_ERROR[head.state]?.(t);
	if (!plain && !head.error) return null;
	return (
		<div data-head-error="" className="flex flex-col gap-1 text-ui">
			{plain ? <p className="text-ui">{plain}</p> : null}
			{head.error ? (
				<details>
					<summary className="cursor-pointer text-xs text-muted-foreground">
						{t("serviceConfig.buffer.details", "Details")}
					</summary>
					<code className="mt-1 block rounded-sm border border-hairline bg-surface-sunken px-2 py-1.5 font-mono text-xs wrap-anywhere text-ink-2">
						{head.error}
					</code>
				</details>
			) : null}
		</div>
	);
}

function HeadFacts({
	queue,
	head,
	details,
}: Readonly<{ queue: OfflineQueueStatus; head: Head; details: DetailsRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const detail = details.byScope[queue.scope]?.head;
	return (
		<FactList>
			<KvRow label={t("serviceConfig.buffer.fact.target", "Target")}>
				<TargetText target={queueTarget(head.resource)} />
			</KvRow>
			<KvRow label={t("serviceConfig.buffer.fact.kind", "Change kind")}>
				{details.supported && detail ? (
					<span data-change-kind="">
						{changeKind(t, detail.mutation_kind)}
						<span className="text-muted-foreground">
							{" · "}
							{humanFileSize(detail.bytes)}
						</span>
					</span>
				) : details.supported ? (
					<span data-change-kind="unknown" className="text-muted-foreground">
						{t(
							"serviceConfig.buffer.kindUnknown",
							"Not reported by the device",
						)}
					</span>
				) : (
					<span
						data-change-kind="unsupported"
						className="text-muted-foreground"
					>
						{t(
							"serviceConfig.buffer.kindUnsupported",
							"Update the device agent to see the change kind.",
						)}
					</span>
				)}
			</KvRow>
			<KvRow label={t("serviceConfig.buffer.fact.attempts", "Attempts")}>
				<span className="tabular-nums">
					{t("serviceConfig.buffer.count", "{{count, number}}", {
						count: head.attempts,
					})}
				</span>
			</KvRow>
			<KvRow label={t("serviceConfig.buffer.fact.queued", "Queued")}>
				<span title={time.abs(head.created_at)}>
					{time.at(head.created_at)} · {time.ago(head.created_at, "long")}
				</span>
			</KvRow>
			<KvRow label={t("serviceConfig.buffer.fact.behind", "Behind it")}>
				{t("serviceConfig.buffer.behind", {
					count: Math.max(0, queue.pending_count - 1),
					defaultValue_one: "{{count, number}} change waits",
					defaultValue_other: "{{count, number}} changes wait",
				})}
			</KvRow>
			<KvRow label={t("serviceConfig.buffer.fact.id", "Change ID")}>
				<IdRef
					id={head.operation_id}
					copyLabel={t("serviceConfig.buffer.copyChangeId", "Copy change ID")}
				/>
			</KvRow>
		</FactList>
	);
}

function HeadButtons({
	deviceId,
	serviceId,
	queue,
	head,
	actions,
	gates,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	queue: OfflineQueueStatus;
	head: Head;
	actions: QueueActions;
	gates: {
		retry: ReturnType<typeof gateLine>;
		discard: ReturnType<typeof gateLine>;
	};
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (queue.quarantined)
		return (
			<div className="flex flex-col gap-1.5">
				<div className="flex flex-wrap items-center gap-2">
					<DvButton asChild size="sm" icon={Cloud}>
						<a
							data-act="fix-cloud"
							{...link({
								screen: "service",
								deviceId,
								serviceId,
								tab: "cloud",
							})}
						>
							{t("serviceConfig.buffer.fixCloud", "Fix cloud access")}
						</a>
					</DvButton>
				</div>
				<GateInline kind="busy">
					{t(
						"serviceConfig.buffer.pausedGate",
						"Paused until the queue uses current cloud access. Nothing is sent or discarded before that.",
					)}
				</GateInline>
			</div>
		);
	if (!HEAD_NEEDS_YOU.has(head.state))
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.buffer.automatic",
					"The device sends it on its own as soon as the cloud is reachable.",
				)}
			</p>
		);
	return (
		<div className="flex flex-wrap items-start gap-2">
			<GatedAction gate={gates.retry}>
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={actions.pending}
					data-act="queue-retry"
					onClick={() => void actions.retry(queue)}
				>
					{t("serviceConfig.buffer.retry", "Try again")}
				</DvButton>
			</GatedAction>
			<GatedAction gate={gates.discard}>
				<DvButton
					size="sm"
					variant="danger-ghost"
					data-act="queue-discard"
					onClick={() => void actions.discard(queue)}
				>
					{t("serviceConfig.buffer.discardButton", "Discard this change…")}
				</DvButton>
			</GatedAction>
		</div>
	);
}

function HeadCard(
	props: Readonly<{
		deviceId: string;
		serviceId: string;
		queue: OfflineQueueStatus;
		head: Head;
		details: DetailsRead;
		actions: QueueActions;
		gates: {
			retry: ReturnType<typeof gateLine>;
			discard: ReturnType<typeof gateLine>;
		};
	}>,
) {
	const { t } = useTranslation("devices");
	const { queue, head } = props;
	const warning = queue.quarantined || HEAD_NEEDS_YOU.has(head.state);
	return (
		<article
			data-queue-head={head.state}
			className={cx(
				"flex min-w-0 flex-col gap-3 rounded-lg border bg-card p-3.5",
				warning ? TONE_LINE.warning : "border-border",
			)}
		>
			<div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
				<Chip look={headLook(t, head.state)} />
				<span className="text-xs text-muted-foreground">
					{queue.quarantined
						? t("serviceConfig.buffer.earlierApproval", "earlier approval")
						: t("serviceConfig.buffer.currentApproval", "current approval")}
				</span>
			</div>
			<HeadFacts queue={queue} head={head} details={props.details} />
			<HeadError queue={queue} head={head} />
			<HeadButtons {...props} />
		</article>
	);
}

function NextChanges({
	deviceId,
	serviceId,
	read,
	queues,
	details,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	read: ServiceConfigRead;
	queues: QueuesRead;
	details: DetailsRead;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useQueueActions(deviceId, serviceId, read, queues);
	const target = useMemo<GateTarget>(
		() => ({
			placementId: serviceId,
			...(read.service ? { projectId: read.service.projectId } : {}),
			labels: { service: serviceId },
		}),
		[serviceId, read.service],
	);
	const gates = useGates(
		["offline_queue_retry", "offline_queue_skip"],
		deviceId,
		target,
	);
	const lines = {
		retry: gateLine(t, time, gates.offline_queue_retry),
		discard: gateLine(t, time, gates.offline_queue_skip),
	};
	const heads = (queues.queues ?? []).flatMap((queue) =>
		queue.head ? [{ queue, head: queue.head }] : [],
	);
	if (!heads.length) return null;
	return (
		<Block
			id="svc-queue-heads"
			icon={ListChecks}
			title={t("serviceConfig.buffer.next", "Next change in line")}
			stamp={<FreshnessStamp {...stampOf(queues.freshness)} />}
		>
			<div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,340px),1fr))] gap-3">
				{heads.map(({ queue, head }) => (
					<HeadCard
						key={queue.scope}
						deviceId={deviceId}
						serviceId={serviceId}
						queue={queue}
						head={head}
						details={details}
						actions={actions}
						gates={lines}
					/>
				))}
			</div>
			<ActionResults
				resultKey={actions.resultKey}
				note={actions.note}
				onDismiss={actions.dismiss}
			/>
		</Block>
	);
}

/* States without queues. */

function NotUsed({
	read,
	offlineCopy,
}: Readonly<{ read: ServiceConfigRead; offlineCopy: boolean }>) {
	const { t } = useTranslation("devices");
	return (
		<Titled stamp={<ConfigStamp read={read} />}>
			<StateView
				kind="empty"
				icon={Database}
				title={
					offlineCopy
						? t(
								"serviceConfig.buffer.notUsedCopy",
								"Not used: this service runs an offline copy",
							)
						: t(
								"serviceConfig.buffer.notUsedOff",
								"Not used: write buffering is off",
							)
				}
				text={
					offlineCopy
						? t(
								"serviceConfig.buffer.notUsedCopyText",
								"An offline copy keeps its data only on {{device}} and never syncs back, so nothing waits for the cloud.",
								{ device: read.deviceLabel },
							)
						: t(
								"serviceConfig.buffer.notUsedOffText",
								"Changes go straight to the cloud; nothing is queued on the device. Turn it on with Update… to keep accepting changes when the internet drops.",
							)
				}
			/>
		</Titled>
	);
}

function Titled({
	stamp,
	children,
}: Readonly<{ stamp?: ReactNode; children: ReactNode }>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			id="svc-buffering"
			icon={Database}
			title={t("serviceConfig.buffer.title", "Write buffering")}
			stamp={stamp}
		>
			{children}
		</Block>
	);
}

/** Why no queue is on screen yet: still reading, refused, or not connected. */
function QueuesState({
	queues,
	device,
	settingsClosed,
}: Readonly<{ queues: QueuesRead; device: string; settingsClosed: boolean }>) {
	const { t } = useTranslation("devices");
	if (queues.loading)
		return (
			<StateView
				kind="loading"
				rows={3}
				title={t("serviceConfig.buffer.loading", "Reading the queues…")}
			/>
		);
	if (queues.refused?.code === ACCESS_REFUSED)
		return (
			<StateView
				kind="noaccess"
				title={t(
					"serviceConfig.buffer.refused",
					"Needs View status on this service to read its queues.",
				)}
			/>
		);
	if (queues.refused || queues.failed)
		return (
			<StateView
				kind="error"
				title={t(
					"serviceConfig.buffer.failed",
					"The queues couldn't be read from {{device}}",
					{ device },
				)}
				text={
					queues.refused
						? t(
								"serviceConfig.buffer.failedReason",
								"{{device}} answered: “{{reason}}” Buffered changes stay on the device.",
								{ device, reason: queues.refused.error },
							)
						: t(
								"serviceConfig.buffer.failedText",
								"The device turned the read down or the connection dropped. Buffered changes stay on the device.",
							)
				}
				actions={
					<DvButton
						size="sm"
						icon={RefreshCw}
						data-act="queues-retry"
						onClick={() => void queues.refresh()}
					>
						{t("serviceConfig.state.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	if (settingsClosed) return null;
	return (
		<StateView
			kind="notloaded"
			title={t(
				"serviceConfig.buffer.onConnect",
				"The queues are read when you connect live",
			)}
			text={t(
				"serviceConfig.buffer.onConnectText",
				"Buffered changes are kept on {{device}}; nothing is lost while you aren't connected.",
				{ device },
			)}
		/>
	);
}

const budgetsOf = (read: ServiceConfigRead): Budgets => {
	const writes = read.configuration?.config.offline_writes;
	return writes
		? { maxBytes: writes.max_queue_bytes, maxAgeS: writes.max_age_seconds }
		: undefined;
};

/** The queues of a service that buffers changes, or of one whose settings this account may not read. */
function Buffered({
	deviceId,
	serviceId,
	read,
	settingsClosed,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	read: ServiceConfigRead;
	settingsClosed: boolean;
}>) {
	const { input } = useAttentionState();
	const queues = useQueues(deviceId, serviceId);
	const details = useQueueDetails(
		deviceId,
		serviceId,
		queues.queues,
		queues.live,
	);
	const summaries = agentSupports(
		input.live[deviceId]?.inspection?.value.features,
		"offline_summary",
	);
	if (!queues.queues)
		return (
			<Titled stamp={<FreshnessStamp {...stampOf(queues.freshness)} />}>
				{settingsClosed ? (
					<ConfigUnavailable read={read} serviceId={serviceId} />
				) : null}
				<QueuesState
					queues={queues}
					device={read.deviceLabel}
					settingsClosed={settingsClosed}
				/>
			</Titled>
		);
	return (
		<div data-service-buffering="" className="flex min-w-0 flex-col gap-4">
			<QueuesBlock
				deviceId={deviceId}
				serviceId={serviceId}
				read={queues}
				budgets={budgetsOf(read)}
				details={details}
				summaries={summaries}
			/>
			<NextChanges
				deviceId={deviceId}
				serviceId={serviceId}
				read={read}
				queues={queues}
				details={details}
			/>
		</div>
	);
}

/** Without Deploy & configure the settings stay closed, but the queues only need View status. */
const settingsClosedOf = (read: ServiceConfigRead) =>
	!read.configuration && (read.gate?.kind === "noaccess" || !!read.refused);

/** SPEC §5.3 Offline writes, labelled Write buffering (APP A2): queues, the next change of each and what to do about it. */
export function ServiceOfflineTab({
	deviceId,
	serviceId,
}: Readonly<ServiceTabProps>) {
	const read = useServiceConfig(deviceId, serviceId);
	const config = read.configuration?.config;
	const settingsClosed = settingsClosedOf(read);
	if (!config && !settingsClosed)
		return (
			<Titled>
				<ConfigUnavailable read={read} serviceId={serviceId} />
			</Titled>
		);
	if (config && !config.offline_writes)
		return <NotUsed read={read} offlineCopy={config.source === "offline"} />;
	return (
		<Buffered
			deviceId={deviceId}
			serviceId={serviceId}
			read={read}
			settingsClosed={settingsClosed}
		/>
	);
}
