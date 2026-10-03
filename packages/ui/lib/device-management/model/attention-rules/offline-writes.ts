import type { OfflineQueueStatus } from "../../offline-queue";
import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	type DeviceFacts,
	attentionCandidate,
	isLastKnown,
	readableServices,
	serviceRoute,
	serviceSubject,
} from "../device-view";
import type { AttentionKey, CopyParams, ServiceView } from "../types";

const BACKLOG_AGE_RATIO = 0.5;
const BACKLOG_BYTES_RATIO = 0.8;

type Head = NonNullable<OfflineQueueStatus["head"]>;

const HEAD_KEYS: Partial<Record<Head["state"], AttentionKey>> = {
	conflict: "offline_writes_conflict",
	blocked: "offline_writes_blocked",
	outcome_unknown: "offline_writes_outcome_unknown",
};

/** `{"kind":"table","table":"notes",…}` → "notes"; files by path. */
export function queueResource(resource: string): {
	kind: string;
	name: string;
} {
	try {
		const parsed = JSON.parse(resource) as Record<string, unknown>;
		const kind = typeof parsed.kind === "string" ? parsed.kind : "resource";
		const name =
			typeof parsed.table === "string"
				? parsed.table
				: typeof parsed.path === "string"
					? parsed.path
					: typeof parsed.prefix === "string"
						? parsed.prefix
						: kind;
		return { kind, name };
	} catch {
		return { kind: "resource", name: resource.slice(0, 64) };
	}
}

interface QueueContext {
	input: AttentionInputExt;
	facts: DeviceFacts;
	service: ServiceView;
	queue: OfflineQueueStatus;
}

function queueItem(
	key: AttentionKey,
	severity: AttentionCandidateExt["severity"],
	{ input, facts, service, queue }: QueueContext,
	params: CopyParams,
	tab: "offline" | "cloud",
): AttentionCandidateExt {
	return attentionCandidate({
		key,
		severity,
		subject: serviceSubject(service),
		params: { service: service.serviceId, device: facts.name, ...params },
		action: {
			code:
				tab === "cloud"
					? "fix_cloud_access"
					: key === "offline_writes_backlog" || key === "offline_mirror_error"
						? "view_queue"
						: "review_change",
			target: serviceRoute(facts.id, service.serviceId, tab),
		},
		source: service.freshness,
		lastKnown: !facts.liveOpen,
		since: queue.head?.created_at ?? queue.oldest_at ?? undefined,
		part: queue.scope.slice(0, 12),
	});
}

/** Live queue rows per readable service (P3; BG11 summaries carry no head). */
function eachQueue(
	input: AttentionInputExt,
	evaluate: (context: QueueContext) => AttentionCandidateExt | undefined,
): AttentionCandidateExt[] {
	return readableServices(input).flatMap(({ facts, services }) =>
		services.flatMap((service) =>
			(facts.liveInput?.offlineQueues?.[service.serviceId] ?? []).flatMap(
				(queue) => evaluate({ input, facts, service, queue }) ?? [],
			),
		),
	);
}

function headRule(key: AttentionKey): AttentionRuleExt {
	return {
		key,
		evaluate: (input) =>
			eachQueue(input, (context) => {
				const { queue } = context;
				if (queue.quarantined || !queue.head) return undefined;
				if (HEAD_KEYS[queue.head.state] !== key) return undefined;
				const resource = queueResource(queue.head.resource);
				return queueItem(
					key,
					"warning",
					context,
					{
						resource: resource.name,
						resourceKind: resource.kind,
						behind: Math.max(0, queue.pending_count - 1),
						...(queue.head.error ? { error: queue.head.error } : {}),
					},
					"offline",
				);
			}),
	};
}

const quarantined: AttentionRuleExt = {
	key: "offline_writes_quarantined",
	evaluate(input) {
		const fromLive = eachQueue(input, (context) =>
			context.queue.quarantined
				? queueItem(
						"offline_writes_quarantined",
						"warning",
						context,
						{
							count: context.queue.pending_count,
							...(context.queue.oldest_at !== null
								? { oldestAt: context.queue.oldest_at }
								: {}),
						},
						"cloud",
					)
				: undefined,
		);
		return [...fromLive, ...summaryQuarantined(input)];
	},
};

const backlog: AttentionRuleExt = {
	key: "offline_writes_backlog",
	evaluate: (input) =>
		eachQueue(input, (context) => {
			const { queue, facts, service } = context;
			const limits =
				facts.liveInput?.placements?.[service.serviceId]?.offlineWrites;
			if (!limits || queue.quarantined || queue.pending_count === 0)
				return undefined;
			const oldAge =
				queue.oldest_at !== null &&
				input.now - queue.oldest_at > limits.maxAgeS * BACKLOG_AGE_RATIO;
			const large = queue.pending_bytes > limits.maxBytes * BACKLOG_BYTES_RATIO;
			if (!oldAge && !large) return undefined;
			return queueItem(
				"offline_writes_backlog",
				"notice",
				context,
				{
					count: queue.pending_count,
					...(queue.oldest_at !== null ? { oldestAt: queue.oldest_at } : {}),
				},
				"offline",
			);
		}),
};

const mirrorError: AttentionRuleExt = {
	key: "offline_mirror_error",
	evaluate: (input) =>
		eachQueue(input, (context) =>
			context.queue.mirror_error
				? queueItem(
						"offline_mirror_error",
						"notice",
						context,
						{ error: context.queue.mirror_error },
						"offline",
					)
				: undefined,
		),
};

/** BG11 summaries (snapshot or live rows) for services whose live queues aren't loaded. */
function summaryQuarantined(input: AttentionInputExt): AttentionCandidateExt[] {
	return readableServices(input).flatMap(({ facts, services }) =>
		services.flatMap((service) => {
			const writes = service.offlineWrites;
			if (
				facts.liveInput?.offlineQueues?.[service.serviceId] ||
				typeof writes !== "object" ||
				!writes.quarantined
			)
				return [];
			return [
				attentionCandidate({
					key: "offline_writes_quarantined",
					severity: "warning",
					subject: serviceSubject(service),
					params: {
						service: service.serviceId,
						device: facts.name,
						count: writes.pending,
					},
					action: {
						code: "fix_cloud_access",
						target: serviceRoute(facts.id, service.serviceId, "cloud"),
					},
					source: service.freshness,
					lastKnown: isLastKnown(service.freshness),
				}),
			];
		}),
	);
}

export const OFFLINE_WRITE_RULES: readonly AttentionRuleExt[] = [
	headRule("offline_writes_conflict"),
	headRule("offline_writes_blocked"),
	headRule("offline_writes_outcome_unknown"),
	quarantined,
	backlog,
	mirrorError,
];
