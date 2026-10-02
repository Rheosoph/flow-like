"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { History } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import {
	type AgentOperation,
	agentSupports,
	readOperations,
} from "../../../../lib/device-management/agent-reads";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import type {
	ActivityItem,
	StreamGap,
} from "../../../../lib/device-management/workspace/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline, GateNotice } from "../primitives/gate-notice";
import { StateView } from "../primitives/state-view";
import { Timeline, type TimelineRow } from "../primitives/timeline";
import { useRouteLink } from "../routing/use-devices-route";
import { activityTitle } from "../shell/activity-tray";
import { stampOf, useAppNames } from "../shell/attention-popover";
import {
	type LiveStreamView,
	deviceCall,
	useActivity,
	useDeviceWorkspace,
	useGate,
	useLiveStream,
} from "../workspace";
import { LiveDataState, livePhase, liveWanted } from "./live-state";
import { lastGap, metaOf, recordsOf, sumGaps, whole } from "./observe-data";
import { SmallSelect } from "./small-select";
import { type TimelineFact, timelineFacts } from "./timeline-model";
import { type Sentences, entryOf } from "./timeline-sentences";
import { policyAppliedOf } from "./use-history";
import {
	type ObserveTarget,
	type PersonNames,
	readRefusal,
	scopedApp,
	usePeople,
} from "./use-observe-target";
import { useOlderRecords } from "./use-older-records";

const PAGE = 50;
const EVERYONE = "all";
const OPERATIONS_EVERY_MS = 30_000;
const LINK = "font-mono text-foreground hover:underline";

/* BG13: who sent which command (the owner, on an agent that lists its journal). */

function useOperations(
	target: ObserveTarget,
): Map<string, AgentOperation> | undefined {
	const workspace = useDeviceWorkspace();
	const { deviceId, features } = target;
	const enabled =
		target.owner &&
		livePhase(target) === "open" &&
		agentSupports(features, "operations");
	const query = useQuery({
		queryKey: ["devices", workspace.scopeKey, "observe-operations", deviceId],
		enabled,
		retry: false,
		gcTime: 0,
		staleTime: OPERATIONS_EVERY_MS,
		refetchInterval: OPERATIONS_EVERY_MS,
		queryFn: async () => {
			const read = await readOperations(
				deviceCall(workspace, deviceId, "poll"),
				features,
				{ limit: 50 },
			);
			return read.kind === "ok" ? read.data.operations : null;
		},
	});
	const rows = enabled ? query.data : undefined;
	return useMemo(
		() =>
			rows
				? new Map(rows.map((row) => [row.operation_id, row] as const))
				: undefined,
		[rows],
	);
}

const operationIdOf = (item: ActivityItem): string | undefined => {
	const resume = item.resume;
	return resume && "operationId" in resume ? resume.operationId : undefined;
};

/** "Stop support-bot": the commands sent from this computer, by command ID. */
function commandTitles(
	t: DevicesT,
	items: readonly ActivityItem[],
): Map<string, string> {
	const found = new Map<string, string>();
	for (const item of items) {
		const id = operationIdOf(item);
		if (!id) continue;
		const title = activityTitle(t, item);
		const service = item.target.serviceId;
		found.set(
			id,
			item.kind === "command" && service
				? t(
						"devices:observe.timeline.command.onService",
						"{{command}} {{service}}",
						{ command: title, service },
					)
				: title,
		);
	}
	return found;
}

function rolloutsOf(target: ObserveTarget): DeploymentRolloutStatus[] {
	const known = target.facts?.rollouts ?? [];
	const current = (target.services ?? []).flatMap((service) =>
		service.rollout ? [service.rollout] : [],
	);
	return [
		...new Map(
			[...known, ...current].map((row) => [row.rollout_id, row]),
		).values(),
	];
}

function accessOf(target: ObserveTarget) {
	const policy = target.policy;
	if (!policy) return undefined;
	const others = new Set(
		policy.grants
			.map((grant) => grant.user_id)
			.filter((user) => user !== target.me),
	);
	return {
		version: policy.policy_version,
		issuedAt: policy.issued_at,
		applied: policyAppliedOf(target),
		people: others.size,
	};
}

/** What the device lost or deleted, as rows that never hide behind a filter. */
function lossRows(
	t: DevicesT,
	gaps: readonly StreamGap[],
	options: { retention: number | undefined; showDeleted: boolean },
): TimelineRow[] {
	const rows: TimelineRow[] = [];
	const lost = sumGaps(gaps, "outbox_dropped");
	if (lost > 0)
		rows.push({
			id: "lost",
			gap: true,
			text: t("devices:observe.timeline.lost", {
				count: lost,
				defaultValue_one:
					"{{count, number}} change was lost before the device could store it.",
				defaultValue_other:
					"{{count, number}} changes were lost before the device could store them.",
			}),
		});
	const deleted = lastGap(gaps, "evicted_through")?.through;
	if (deleted === undefined || !options.showDeleted) return rows;
	const before = deleted + 1;
	rows.push({
		id: "evicted",
		gap: true,
		text:
			options.retention === undefined
				? t(
						"devices:observe.timeline.evicted",
						"Older entries were deleted to save space (before #{{before, number}}).",
						{ before },
					)
				: t(
						"devices:observe.timeline.evictedKeeps",
						"Older entries were deleted to save space (before #{{before, number}}). The device keeps the last {{keep, number}} changes.",
						{ before, keep: options.retention },
					),
	});
	return rows;
}

/** Commands of one person (BG13); every entry for "everyone". */
function byPerson(
	facts: readonly TimelineFact[],
	person: string,
	operations: ReadonlyMap<string, AgentOperation> | undefined,
): readonly TimelineFact[] {
	if (person === EVERYONE) return facts;
	return facts.filter(
		(fact) =>
			fact.type === "command" &&
			operations?.get(fact.operationId)?.actor.user_id === person,
	);
}

interface TimelineData {
	stream: LiveStreamView<unknown>;
	facts: TimelineFact[];
	operations: Map<string, AgentOperation> | undefined;
	titles: Map<string, string>;
	/** Why the read isn't sent; `null` when it may go out. */
	refusal: string | null;
	/** The one app whose activity a scoped viewer reads on the device page. */
	ownApp: string | null;
	older: ReturnType<typeof useOlderRecords>;
}

function useTimelineData(target: ObserveTarget): TimelineData {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { deviceId, serviceId } = target;
	const projectId = target.service?.projectId;
	const gate = useGate(
		"messages",
		deviceId,
		serviceId ? { placementId: serviceId, projectId } : undefined,
	);
	// Someone whose access covers one app reads that app's activity on the device page.
	const ownApp = serviceId ? null : scopedApp(target);
	const service = serviceId ? { serviceId, projectId } : null;
	const refusal = ownApp
		? null
		: readRefusal(t, target, gate, "logs", service, time);
	const stream = useLiveStream<unknown>(
		deviceId,
		liveWanted(target) && !refusal
			? { kind: "messages", placementId: serviceId, projectId: ownApp }
			: null,
	);
	const operations = useOperations(target);
	const tray = useActivity({ deviceId });
	const records = useMemo(() => recordsOf(stream.data), [stream.data]);
	const older = useOlderRecords(records.length, stream.loadOlder);
	const minute = Math.floor(time.nowS / 60) * 60;
	const facts = useMemo(
		() =>
			timelineFacts({
				records,
				serviceId,
				rollouts: rolloutsOf(target),
				host: target.inspection?.host,
				history: target.facts?.history,
				access: accessOf(target),
				now: minute,
			}),
		[records, serviceId, target, minute],
	);
	const titles = useMemo(() => commandTitles(t, tray.items), [t, tray.items]);
	return { stream, facts, operations, titles, refusal, ownApp, older };
}

function footText(
	t: DevicesT,
	target: ObserveTarget,
	ownApp: string | null,
	appName: (appId: string) => string | undefined,
): string {
	if (target.serviceId)
		return t(
			"devices:observe.timeline.footService",
			"Only entries for {{service}}. Newest first; gaps are marked where older entries were deleted.",
			{ service: target.serviceId },
		);
	if (ownApp)
		return t(
			"devices:observe.timeline.footApp",
			"Only entries of {{app}}: your access to {{device}} covers that app. Newest first.",
			{ app: appName(ownApp) ?? ownApp, device: target.name },
		);
	return t(
		"devices:observe.timeline.footDevice",
		"Newest first. The device keeps a limited number of changes; gaps are marked where older entries were deleted.",
	);
}

/** What the block shows instead of entries: refused, rejected by the device, not connected, empty or still reading. */
function TimelineNotice({
	target,
	data,
}: Readonly<{ target: ObserveTarget; data: TimelineData }>): ReactNode {
	const { t } = useTranslation("devices");
	const { stream, refusal } = data;
	if (refusal)
		return (
			<GateNotice
				kind="noaccess"
				title={t("devices:observe.timeline.noAccess", "No access to activity.")}
				text={refusal}
			/>
		);
	if (stream.rejected)
		return (
			<StateView
				kind={
					stream.freshness.age === "unsupported" ? "unsupported" : "noaccess"
				}
				title={t(
					"devices:observe.timeline.rejected",
					"{{device}} didn't return its activity",
					{ device: target.name },
				)}
				text={t(
					"devices:observe.timeline.rejectedText",
					"Reading activity needs Read logs on this device.",
				)}
			/>
		);
	if (livePhase(target) !== "open")
		return <LiveDataState target={target} what="activity" />;
	if (!stream.started || stream.freshness.age === "notloaded")
		return (
			<StateView
				kind="loading"
				title={t("devices:observe.timeline.reading", "Reading activity…")}
			/>
		);
	return (
		<StateView
			kind="empty"
			title={t("devices:observe.timeline.empty", "No activity recorded")}
			text={
				target.serviceId
					? t(
							"devices:observe.timeline.emptyService",
							"{{device}} has recorded no command or instance change for {{service}}.",
							{ device: target.name, service: target.serviceId },
						)
					: t(
							"devices:observe.timeline.emptyDevice",
							"{{device}} has recorded no command or instance change yet.",
							{ device: target.name },
						)
			}
		/>
	);
}

/** The accounts the device names as senders of commands (BG13). */
const actorsOf = (
	operations: Map<string, AgentOperation> | undefined,
): string[] => [
	...new Set(
		[...(operations?.values() ?? [])].flatMap((row) =>
			row.actor.user_id ? [row.actor.user_id] : [],
		),
	),
];

function TimelineStamp({
	target,
	data,
	hasEntries,
}: Readonly<{
	target: ObserveTarget;
	data: TimelineData;
	hasEntries: boolean;
}>) {
	const { t } = useTranslation("devices");
	const { freshness, started } = data.stream;
	if (started && !data.refusal)
		return (
			<FreshnessStamp
				{...stampOf(freshness)}
				{...(freshness.age === "live"
					? { text: t("devices:observe.timeline.following", "following") }
					: {})}
			/>
		);
	return hasEntries && target.source ? (
		<FreshnessStamp {...stampOf(target.source)} />
	) : null;
}

/** Filters the commands by who sent them, as far as the device names the sender. */
function PersonFilter({
	target,
	actors,
	people,
	value,
	onChange,
}: Readonly<{
	target: ObserveTarget;
	actors: readonly string[];
	people: PersonNames;
	value: string;
	onChange(value: string): void;
}>) {
	const { t } = useTranslation("devices");
	if (!actors.length) return null;
	const someone = t("devices:observe.timeline.someone", "Someone else");
	return (
		<SmallSelect
			value={value}
			onChange={onChange}
			label={t("devices:observe.timeline.person", "Who sent it")}
			options={[
				{
					value: EVERYONE,
					label: t("devices:observe.timeline.everyone", "Everyone"),
				},
				...actors.map((id) => ({
					value: id,
					label:
						id === target.me
							? t("devices:observe.timeline.you", "You")
							: (people(id) ?? someone),
				})),
			]}
		/>
	);
}

/** SPEC §5.2 "Timeline": what happened on the device or to one service, newest first. */
export function TimelineBlock({ target }: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	const data = useTimelineData(target);
	const { stream, facts, operations, titles, older } = data;
	const { deviceId, serviceId } = target;
	const [person, setPerson] = useState(EVERYONE);
	const [shown, setShown] = useState(PAGE);

	const actors = useMemo(() => actorsOf(operations), [operations]);
	const people = usePeople(actors);
	const sentences: Sentences = {
		t,
		target,
		titles,
		operations,
		people,
		service: (id) =>
			serviceId ? (
				<span className="font-mono">{id}</span>
			) : (
				<a
					className={LINK}
					{...link({
						screen: "service",
						deviceId,
						serviceId: id,
						tab: "status",
					})}
				>
					{id}
				</a>
			),
	};

	const visible = byPerson(facts, person, operations);
	const more = visible.length > shown;
	const rows: TimelineRow[] = [
		...visible.slice(0, shown).map((fact) => entryOf(sentences, fact)),
		...lossRows(t, stream.gaps, {
			retention: whole(metaOf(stream.data).retention_limit),
			showDeleted: !more && !older.available,
		}),
	];
	const loadOlder =
		more || older.available
			? () => {
					setShown((count) => count + PAGE);
					if (!more) older.load();
				}
			: undefined;

	const hasEntries = rows.length > 0 && !data.refusal;
	const withoutLive = stream.data === undefined && livePhase(target) !== "open";
	return (
		<Block
			id="observe-timeline"
			icon={History}
			title={t("devices:observe.timeline.title", "Timeline")}
			stamp={
				<TimelineStamp target={target} data={data} hasEntries={hasEntries} />
			}
			foot={footText(t, target, data.ownApp, appName)}
		>
			{hasEntries ? (
				<>
					{withoutLive ? (
						<GateInline kind="live" className="max-w-none">
							{t(
								"devices:observe.timeline.withoutLive",
								"Shown from the last status read. Commands and instance changes appear over a live connection.",
							)}
						</GateInline>
					) : null}
					<Timeline
						rows={rows}
						tools={
							<PersonFilter
								target={target}
								actors={actors}
								people={people}
								value={person}
								onChange={setPerson}
							/>
						}
						onLoadOlder={loadOlder}
						loadingOlder={older.loading}
					/>
				</>
			) : (
				<TimelineNotice target={target} data={data} />
			)}
		</Block>
	);
}
