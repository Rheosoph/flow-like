import type { AttentionInputExt } from "./attention";
import { countAttention } from "./attention";
import { type Coverage, coverage } from "./coverage";
import {
	type DeviceFacts,
	fleetFacts,
	isLastKnown,
	itemsForDevice,
	rolloutEndsAt,
} from "./device-view";
import type { AttentionItem, CopyParams, CopyRef, ServiceView } from "./types";

/** SPEC §6.4 (fleet) and APP §7.4 (app) conclusion sentences; `DV/copy/headline-copy.ts` renders them. */
export type HeadlineCode =
	| "fleet.critical"
	| "fleet.critical_items"
	| "fleet.nothing_broken"
	| "fleet.all_clear"
	| "fleet.soon_later"
	| "fleet.coverage"
	| "app.no_devices"
	| "app.no_devices_next"
	| "app.all_locked"
	| "app.locked_detail"
	| "app.never_deployed"
	| "app.mode"
	| "app.crashing"
	| "app.crashing_last_known"
	| "app.others_ok"
	| "app.offline_age"
	| "app.writes"
	| "app.writes_detail"
	| "app.updating"
	| "app.updating_deadline"
	| "app.staged"
	| "app.staged_detail"
	| "app.applying"
	| "app.unknown_state"
	| "app.stopped_events_nowhere"
	| "app.events_nowhere"
	| "app.upload_paused"
	| "app.all_as_asked"
	| "app.all_stopped"
	| "app.drift"
	| "app.coverage"
	| "app.not_checked_in";

export interface HeadlineRef extends CopyRef<HeadlineCode> {
	/** Name lists (devices, events), formatted by the copy layer. */
	lists?: Record<string, readonly string[]>;
}

/** The lead sentence plus the "rest" sentences, in order (SPEC §4.24). */
export interface Headline extends HeadlineRef {
	rest: HeadlineRef[];
}

/** App facts the device model doesn't know (from the app catalogue, W1-PLAN). */
export interface HeadlineApp {
	appId: string;
	name: string;
	localOnly: boolean;
	events: { total: number; eligible: number; nowhere: readonly string[] };
	latestLabel?: string;
	/** Services running an older version than `latestLabel`. */
	olderServices?: number;
}

export interface HeadlineFilter {
	/** The computed attention list (`computeAttention`). */
	items: readonly AttentionItem[];
	app?: HeadlineApp;
}

function ref(
	code: HeadlineCode,
	params?: CopyParams,
	lists?: HeadlineRef["lists"],
): HeadlineRef {
	return {
		code,
		...(params ? { params } : {}),
		...(lists ? { lists } : {}),
	};
}

function names(input: AttentionInputExt, ids: readonly string[]): string[] {
	const byId = fleetFacts(input).byId;
	return ids.map((id) => byId.get(id)?.name ?? id.slice(0, 8));
}

export function headline(
	input: AttentionInputExt,
	filter: HeadlineFilter,
): Headline {
	return filter.app
		? appHeadline(input, filter.app)
		: fleetHeadline(input, filter.items);
}

function fleetHeadline(
	input: AttentionInputExt,
	items: readonly AttentionItem[],
): Headline {
	const counts = countAttention(items);
	const critical = fleetFacts(input)
		.devices.filter((device) =>
			itemsForDevice(items, device.id).some(
				(item) => item.severity === "critical",
			),
		)
		.map((device) => device.name);
	const covered = coverage(input);
	const rest: HeadlineRef[] = [];
	if (counts.total > 0)
		rest.push(
			ref("fleet.soon_later", { soon: counts.warning, later: counts.notice }),
		);
	rest.push(
		ref(
			"fleet.coverage",
			{
				readable: covered.readable,
				active: covered.total,
				locked: covered.locked.length,
			},
			{ locked: names(input, covered.locked) },
		),
	);
	const lead = critical.length
		? ref("fleet.critical", { count: critical.length }, { names: critical })
		: counts.critical > 0
			? ref("fleet.critical_items", { count: counts.critical })
			: counts.total > 0
				? ref("fleet.nothing_broken")
				: ref("fleet.all_clear");
	return { ...lead, rest };
}

interface AppService {
	device: DeviceFacts;
	service: ServiceView;
}

/** APP §7.4 order: crashing, buffered writes need you, updating, staged, converging, unknown, as asked. */
function serviceRank(entry: AppService): number {
	const { service } = entry;
	if (service.conv === "crash_looping" || service.conv === "failed_stopped")
		return 0;
	if (writesNeedYou(entry)) return 1;
	if (service.conv === "update_in_progress") return 2;
	if (service.rollout?.state === "staged") return 3;
	if (service.conv === "converging") return 4;
	if (service.conv === "unknown") return 5;
	return 6;
}

function writesDetail({ device, service }: AppService) {
	const queues = device.liveInput?.offlineQueues?.[service.serviceId];
	if (queues)
		return {
			conflicts: queues.filter(
				(queue) => !queue.quarantined && queue.head?.state === "conflict",
			).length,
			paused: queues
				.filter((queue) => queue.quarantined)
				.reduce((sum, queue) => sum + queue.pending_count, 0),
		};
	const writes = service.offlineWrites;
	if (typeof writes !== "object") return { conflicts: 0, paused: 0 };
	return {
		conflicts: writes.head?.state === "conflict" ? 1 : 0,
		paused: writes.quarantined ? writes.pending : 0,
	};
}

function writesNeedYou(entry: AppService) {
	const { conflicts, paused } = writesDetail(entry);
	return conflicts > 0 || paused > 0;
}

/** Unknown devices that could be read (locked, no keys, failed read): never-checked-in ones are counted on their own. */
const unknownCount = (covered: Coverage) =>
	covered.unknown.length - covered.never.length;

/** The one coverage sentence (APP §6.3): readable of total, then unknown, never checked in and no access. */
function appCoverage(app: HeadlineApp, covered: Coverage): HeadlineRef[] {
	return [
		ref("app.coverage", {
			app: app.name,
			readable: covered.readable,
			total: covered.total,
			unknown: unknownCount(covered),
			never: covered.never.length,
			noAccess: covered.noAccess.length,
		}),
	];
}

/** The settings version an update switches to: until it is active the device only reports the one it started from. */
function targetRevision({ rollout, settings }: ServiceView): number {
	if (rollout?.active_revision != null) return rollout.active_revision;
	return rollout?.base_revision === undefined
		? settings.latest
		: rollout.base_revision + 1;
}

function subjectParams({ device, service }: AppService): CopyParams {
	return { service: service.serviceId, device: device.name };
}

function appHeadline(input: AttentionInputExt, app: HeadlineApp): Headline {
	const facts = fleetFacts(input);
	const active = facts.devices.filter((device) => device.active);
	if (!active.length)
		return {
			...ref("app.no_devices"),
			rest: [ref("app.no_devices_next", { app: app.name })],
		};
	const covered = coverage(input, app.appId);
	const coverageParts = appCoverage(app, covered);
	const services: AppService[] = active.flatMap((device) =>
		Array.isArray(device.services)
			? device.services
					.filter((service) => service.projectId === app.appId)
					.map((service) => ({ device, service }))
			: [],
	);
	if (!services.length) {
		const unknown = unknownCount(covered);
		// A device that never checked in runs nothing: it neither asks for an unlock nor makes the answer uncertain.
		if (!covered.readable && unknown > 0) {
			const locked = covered.locked.filter(
				(id) => !covered.never.includes(id),
			).length;
			return {
				...ref("app.all_locked", { app: app.name }),
				rest: [
					ref("app.locked_detail", { count: locked || unknown }),
					...(covered.never.length
						? [
								ref(
									"app.not_checked_in",
									{ count: covered.never.length },
									{ names: names(input, covered.never) },
								),
							]
						: []),
				],
			};
		}
		return {
			...ref("app.never_deployed", {
				app: app.name,
				seen: unknown > 0 || covered.noAccess.length > 0 ? 1 : 0,
			}),
			rest: [
				ref("app.mode", {
					localOnly: app.localOnly ? 1 : 0,
					eligible: app.events.eligible,
					total: app.events.total,
				}),
			],
		};
	}
	const ranked = [...services].sort((a, b) => serviceRank(a) - serviceRank(b));
	const top = ranked[0];
	const rank = serviceRank(top);
	const subject = subjectParams(top);
	const rollout = top.service.rollout;
	switch (rank) {
		case 0: {
			const lastKnown =
				top.device.presence.kind === "offline" ||
				isLastKnown(top.service.freshness);
			if (lastKnown && top.device.presence.since !== undefined)
				return {
					...ref("app.crashing_last_known", subject),
					rest: [
						ref("app.offline_age", {
							device: top.device.name,
							since: top.device.presence.since,
						}),
						...coverageParts,
					],
				};
			const others = services.filter(
				(entry) => entry !== top && serviceRank(entry) === 6,
			).length;
			return {
				...ref("app.crashing", subject),
				rest: [
					...(others ? [ref("app.others_ok", { count: others })] : []),
					...coverageParts,
				],
			};
		}
		case 1:
			return {
				...ref("app.writes", subject),
				rest: [ref("app.writes_detail", writesDetail(top))],
			};
		case 2:
			return {
				...ref("app.updating", { ...subject, to: targetRevision(top.service) }),
				rest: [
					...(rollout?.deadline_at && rollout.base_revision !== undefined
						? [
								ref("app.updating_deadline", {
									deadlineAt: rollout.deadline_at,
									from: rollout.base_revision,
								}),
							]
						: []),
					...coverageParts,
				],
			};
		case 3: {
			const expiresAt = rollout ? rolloutEndsAt(rollout) : undefined;
			return {
				...ref("app.staged", subject),
				rest: [
					ref("app.staged_detail", {
						to: targetRevision(top.service),
						...(expiresAt ? { expiresAt } : {}),
					}),
				],
			};
		}
		case 4:
			return { ...ref("app.applying", subject), rest: coverageParts };
		case 5:
			return { ...ref("app.unknown_state", subject), rest: coverageParts };
	}
	const stopped = services.find(
		(entry) => entry.service.conv === "stopped_by_user",
	);
	if (stopped && app.events.nowhere.length) {
		const upload = input.activity.find(
			(item) =>
				item.kind === "upload" &&
				item.state === "paused" &&
				item.target.projectId === app.appId &&
				!item.dismissed,
		);
		const progress =
			upload?.progress && upload.progress !== "indeterminate"
				? upload.progress
				: undefined;
		return {
			...ref("app.stopped_events_nowhere", subjectParams(stopped)),
			rest: [
				ref(
					"app.events_nowhere",
					{ count: app.events.nowhere.length },
					{ events: app.events.nowhere },
				),
				...(upload && progress
					? [
							ref("app.upload_paused", {
								device:
									upload.target.deviceName ??
									facts.byId.get(upload.target.deviceId)?.name ??
									upload.target.deviceId,
								done: progress.done,
								total: progress.total,
							}),
						]
					: []),
			],
		};
	}
	const deviceCount = new Set(services.map((entry) => entry.device.id)).size;
	const allStopped = services.every(
		(entry) => entry.service.conv === "stopped_by_user",
	);
	const drift =
		app.olderServices && app.latestLabel
			? [
					ref("app.drift", {
						count: app.olderServices,
						version: app.latestLabel,
					}),
				]
			: [];
	return {
		...ref(allStopped ? "app.all_stopped" : "app.all_as_asked", {
			app: app.name,
			count: deviceCount,
		}),
		rest: [...drift, ...coverageParts],
	};
}
