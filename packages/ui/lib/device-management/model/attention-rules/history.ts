import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	type DeviceFacts,
	HOUR_S,
	attentionCandidate,
	deviceRoute,
	fleetFacts,
	hubSource,
	perDevice,
} from "../device-view";
import type { AttentionKey, LiveDeviceInput } from "../types";

const STORAGE_NEARLY_FULL = 0.9;
const READERS_EXPIRING_S = 2 * HOUR_S;

type Roster = NonNullable<LiveDeviceInput["history"]>[number];

function historySource(device: DeviceFacts, input: AttentionInputExt) {
	return device.inspectionSource ?? hubSource(input);
}

function rosterItem(
	key: AttentionKey,
	input: AttentionInputExt,
	device: DeviceFacts,
	roster: Roster,
	since: number,
): AttentionCandidateExt {
	return attentionCandidate({
		key,
		severity: "warning",
		subject: { kind: "access", deviceId: device.id },
		params: {
			device: device.name,
			kind: roster.kind,
			scope: roster.scope,
			since,
		},
		action: {
			code: "resume_recording",
			target: deviceRoute(device.id, "activity"),
		},
		source: historySource(device, input),
		lastKnown: !device.liveOpen,
		since,
		part: `${roster.scope}:${roster.kind}`,
	});
}

function ownerRosters(device: DeviceFacts): Roster[] {
	return device.active && device.relationship === "owner"
		? (device.liveInput?.history ?? [])
		: [];
}

/** BG30 says why recording paused; older agents only show the roster expiry. */
function readersExpired(roster: Roster, now: number) {
	return roster.status
		? roster.status.state === "paused" &&
				(roster.status.reason === "roster_expired" ||
					roster.status.reason === "rules_expired")
		: roster.expiresAt <= now;
}

const readersExpiredRule = perDevice(
	"history_paused_readers_expired",
	(input, device) =>
		ownerRosters(device)
			.filter((roster) => readersExpired(roster, input.now))
			.map((roster) =>
				rosterItem(
					"history_paused_readers_expired",
					input,
					device,
					roster,
					roster.status?.since ?? roster.expiresAt,
				),
			),
);

const accessChanged = perDevice(
	"history_paused_access_changed",
	(input, device) => {
		const applied = input.policies[device.id]?.applied_version;
		return ownerRosters(device)
			.filter((roster) =>
				roster.status
					? roster.status.state === "paused" &&
						roster.status.reason === "rules_changed"
					: !readersExpired(roster, input.now) &&
						applied !== undefined &&
						roster.policyVersion < applied,
			)
			.map((roster) =>
				rosterItem(
					"history_paused_access_changed",
					input,
					device,
					roster,
					roster.status?.since ?? input.now,
				),
			);
	},
);

function ownsDevices(input: AttentionInputExt) {
	return fleetFacts(input).devices.some(
		(device) => device.active && device.relationship === "owner",
	);
}

const notStoredByPlan: AttentionRuleExt = {
	key: "history_not_stored_by_plan",
	evaluate(input) {
		const usage = input.archiveUsage;
		if (!usage || usage.max_bytes > 0 || !ownsDevices(input)) return [];
		return [
			attentionCandidate({
				key: "history_not_stored_by_plan",
				severity: "info",
				subject: { kind: "hub" },
				params: { tier: usage.tier },
				action: { code: "see_plans", target: { kind: "see_plans" } },
				source: hubSource(input),
			}),
		];
	},
};

const storageNearlyFull: AttentionRuleExt = {
	key: "history_storage_nearly_full",
	evaluate(input) {
		const usage = input.archiveUsage;
		if (
			!usage ||
			usage.max_bytes <= 0 ||
			usage.used_bytes < usage.max_bytes * STORAGE_NEARLY_FULL
		)
			return [];
		return [
			attentionCandidate({
				key: "history_storage_nearly_full",
				severity: "notice",
				subject: { kind: "hub" },
				params: { usedBytes: usage.used_bytes, maxBytes: usage.max_bytes },
				action: { code: "see_plans", target: { kind: "see_plans" } },
				source: hubSource(input),
			}),
		];
	},
};

const metricReaders = perDevice("metric_readers_expiring", (input, device) => {
	if (!device.active || device.relationship !== "owner") return undefined;
	return (device.liveInput?.metricReaders ?? [])
		.filter(
			(readers) =>
				readers.expiresAt > input.now &&
				readers.expiresAt - input.now <= READERS_EXPIRING_S,
		)
		.map((readers) =>
			attentionCandidate({
				key: "metric_readers_expiring",
				severity: "notice",
				subject: { kind: "access", deviceId: device.id },
				params: {
					device: device.name,
					scope: readers.scope,
					expiresAt: readers.expiresAt,
				},
				action: {
					code: "renew_readers",
					target: deviceRoute(device.id, "metrics"),
				},
				source: historySource(device, input),
				lastKnown: !device.liveOpen,
				part: readers.scope,
			}),
		);
});

export const HISTORY_RULES: readonly AttentionRuleExt[] = [
	readersExpiredRule,
	accessChanged,
	notStoredByPlan,
	storageNearlyFull,
	metricReaders,
];
