import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	DAY_S,
	type DeviceFacts,
	MINUTE_S,
	activityS,
	attentionCandidate,
	compareVersions,
	deviceRoute,
	fleetFacts,
	hubSource,
	localSource,
	perDevice,
} from "../device-view";
import { CLOCK_SKEW_FLAG_S, classify } from "../freshness";
import { consentResources } from "./cloud";

const NEVER_CHECKED_IN_WARNING_S = 10 * MINUTE_S;
const STALE_STATUS_S = 10 * MINUTE_S;
const READER_RENEW_S = 30 * DAY_S;
const SLOTS_NEARLY_FULL = 0.9;
/** A tracked reboot or agent update that finished this close to the boot explains it. */
const TRACKED_REBOOT_WINDOW_S = 10 * MINUTE_S;

const subject = (device: DeviceFacts) =>
	({ kind: "device", deviceId: device.id }) as const;

function hasRequestedRunning(device: DeviceFacts) {
	return (
		Array.isArray(device.services) &&
		device.services.some((service) => service.desired === "running")
	);
}

function runningCount(device: DeviceFacts) {
	return Array.isArray(device.services)
		? device.services.filter((service) => service.desired === "running").length
		: 0;
}

const offlineSince = perDevice("offline_since", (input, device) => {
	if (!device.active || device.presence.kind !== "offline") return undefined;
	const critical = hasRequestedRunning(device);
	return attentionCandidate({
		key: "offline_since",
		severity: critical ? "critical" : "warning",
		subject: subject(device),
		params: {
			device: device.name,
			since: device.presence.since ?? 0,
			running: runningCount(device),
		},
		action: {
			code: "diagnose",
			target: { kind: "diagnose", deviceId: device.id },
		},
		source: hubSource(input),
		since: device.presence.since,
	});
});

const late = perDevice("late", (input, device) =>
	device.active && device.presence.kind === "late"
		? attentionCandidate({
				key: "late",
				severity: "info",
				subject: subject(device),
				params: { device: device.name, since: device.presence.since ?? 0 },
				source: hubSource(input),
				since: device.presence.since,
			})
		: undefined,
);

const noHeartbeat = perDevice(
	"no_heartbeat_since_enrollment",
	(input, device) => {
		if (
			!device.active ||
			device.presence.kind !== "never" ||
			device.relationship !== "owner"
		)
			return undefined;
		const registeredAt = device.row.registered_at;
		return attentionCandidate({
			key: "no_heartbeat_since_enrollment",
			severity:
				input.now - registeredAt > NEVER_CHECKED_IN_WARNING_S
					? "warning"
					: "info",
			subject: subject(device),
			params: { device: device.name, registeredAt },
			action: {
				code: "show_start_instructions",
				target: { kind: "diagnose", deviceId: device.id },
			},
			source: hubSource(input),
			since: registeredAt,
		});
	},
);

const revoked = perDevice("revoked", (input, device) => {
	if (device.active) return undefined;
	if (device.relationship === "shared" || device.relationship === "none")
		return undefined;
	const revokedAt = device.row.revoked_at ?? undefined;
	return attentionCandidate({
		key: "revoked",
		severity: "info",
		subject: subject(device),
		params:
			revokedAt === undefined
				? { device: device.name }
				: { device: device.name, revokedAt },
		...(device.vault
			? {
					action: {
						code: "delete_keys",
						target: {
							screen: "keys",
							focusDeviceId: device.id,
						},
					},
				}
			: {}),
		source: hubSource(input),
		since: revokedAt,
	});
});

const stillPaying = perDevice(
	"you_still_pay_for_a_revoked_device",
	(input, device) => {
		if (device.active) return undefined;
		const consent = consentResources(input, device.id);
		if (!consent) return undefined;
		const { billing } = consent;
		return attentionCandidate({
			key: "you_still_pay_for_a_revoked_device",
			severity: "warning",
			subject: { kind: "access", deviceId: device.id },
			params: billing
				? {
						device: device.name,
						kind: "billing",
						usedMicros: billing.usedMicros + billing.reservedMicros,
						limitMicros: billing.limitMicros,
						expiresAt: billing.expiresAt,
					}
				: { device: device.name, kind: "approval" },
			action: {
				code: billing ? "revoke_spending_limit" : "revoke",
				target: deviceRoute(device.id, "access"),
			},
			source: hubSource(input),
			since: device.row.revoked_at ?? undefined,
		});
	},
);

const identityMismatch = perDevice("identity_mismatch", (input, device) => {
	const error = device.keys.lastError;
	if (error?.code !== "identity_mismatch") return undefined;
	return attentionCandidate({
		key: "identity_mismatch",
		severity: "critical",
		subject: subject(device),
		params: { device: device.name, pinnedAt: error.pinnedAt },
		action: {
			code: "review_identity",
			target: { kind: "review_identity", deviceId: device.id },
		},
		source: localSource(input),
	});
});

const snapshotIntegrity = perDevice(
	"snapshot_integrity_error",
	(input, device) => {
		const error = input.fleet[device.id]?.error;
		if (error?.kind !== "integrity") return undefined;
		return attentionCandidate({
			key: "snapshot_integrity_error",
			severity: "critical",
			subject: subject(device),
			params: { device: device.name },
			action: {
				code: "review_diagnostics",
				target: { kind: "diagnose", deviceId: device.id },
			},
			source: input.fleet[device.id]?.freshness.status ?? hubSource(input),
			since: error.at,
		});
	},
);

function deviceSkew(input: AttentionInputExt, device: DeviceFacts) {
	const rejection = device.row.auth_rejection;
	if (rejection?.code === "clock_skew" && rejection.skew_seconds !== null)
		return { skewS: rejection.skew_seconds, since: rejection.first_at };
	const skewS = input.clock?.deviceSkewS[device.id];
	return skewS === undefined ? undefined : { skewS, since: undefined };
}

const clockSkew: AttentionRuleExt = {
	key: "clock_skew",
	evaluate(input) {
		const items: AttentionCandidateExt[] = [];
		const offset = input.clock?.hubOffsetS;
		if (offset !== undefined && Math.abs(offset) > CLOCK_SKEW_FLAG_S)
			items.push(
				attentionCandidate({
					key: "clock_skew",
					severity: "warning",
					subject: { kind: "hub" },
					params: { minutes: Math.round(Math.abs(offset) / MINUTE_S) },
					action: { code: "fix_clock", target: { kind: "fix_clock" } },
					source: localSource(input),
				}),
			);
		for (const device of fleetFacts(input).devices) {
			if (!device.active || device.relationship !== "owner") continue;
			const skew = deviceSkew(input, device);
			if (!skew || Math.abs(skew.skewS) <= CLOCK_SKEW_FLAG_S) continue;
			items.push(
				attentionCandidate({
					key: "clock_skew",
					severity: "warning",
					subject: subject(device),
					params: {
						device: device.name,
						minutes: Math.round(Math.abs(skew.skewS) / MINUTE_S),
					},
					action: {
						code: "fix_clock",
						target: { kind: "fix_clock", deviceId: device.id },
					},
					source: hubSource(input),
					since: skew.since,
				}),
			);
		}
		return items;
	},
};

const accessDenied = perDevice("access_denied", (input, device) => {
	const rejection = device.row.auth_rejection;
	if (
		!device.active ||
		device.relationship !== "owner" ||
		rejection?.code !== "revoked_credential"
	)
		return undefined;
	return attentionCandidate({
		key: "access_denied",
		severity: "critical",
		subject: subject(device),
		params: { device: device.name, count: rejection.count },
		action: {
			code: "show_recovery_steps",
			target: { kind: "diagnose", deviceId: device.id },
		},
		source: hubSource(input),
		since: rejection.first_at,
	});
});

const statusStale = perDevice("status_stale_while_online", (input, device) => {
	const status = input.fleet[device.id]?.status;
	if (
		!device.active ||
		device.presence.kind !== "online" ||
		device.liveOpen ||
		device.keys.state !== "unlocked" ||
		!status ||
		input.now - status.observedAt <= STALE_STATUS_S
	)
		return undefined;
	return attentionCandidate({
		key: "status_stale_while_online",
		severity: "notice",
		subject: subject(device),
		params: { device: device.name, observedAt: status.observedAt },
		action: {
			code: "connect_live",
			target: { kind: "connect", deviceId: device.id },
		},
		source: classify("fleet_status", {
			now: input.now,
			at: status.observedAt,
			loaded: true,
		}),
		since: status.observedAt + STALE_STATUS_S,
	});
});

const backgroundTask = perDevice("background_task_failing", (input, device) => {
	const failing = device.inspection?.tasks?.filter(
		(task) => task.state === "failing",
	);
	const source = device.inspectionSource;
	if (!device.active || !failing?.length || !source) return undefined;
	return attentionCandidate({
		key: "background_task_failing",
		severity: "warning",
		subject: subject(device),
		params: {
			device: device.name,
			task: failing[0].name,
			more: failing.length - 1,
		},
		action: {
			code: "diagnose",
			target: { kind: "diagnose", deviceId: device.id },
		},
		source,
		lastKnown: !device.liveOpen,
		since: Math.min(...failing.map((task) => task.since)),
	});
});

/** Release sequences decide when both sides have one (M-BACK BG8); release versions otherwise. */
function isNewerRelease(
	latest: NonNullable<AttentionInputExt["latestRelease"]>,
	agent: NonNullable<DeviceFacts["agent"]>,
) {
	return latest.sequence != null && agent.sequence !== undefined
		? latest.sequence > agent.sequence
		: compareVersions(latest.version, agent.version) > 0;
}

const agentUpdate = perDevice("agent_update_available", (input, device) => {
	const latest = input.latestRelease;
	const agent = device.agent;
	if (!device.active || !latest || !agent || !isNewerRelease(latest, agent))
		return undefined;
	if (device.relationship !== "owner" && !canUpdateAgent(input, device.id))
		return undefined;
	const remote = device.inspection?.hostOperations?.update_agent !== false;
	return attentionCandidate({
		key: "agent_update_available",
		severity: "notice",
		subject: subject(device),
		params: {
			device: device.name,
			available: latest.version,
			running: agent.version,
			readAt: agent.source.at ?? 0,
		},
		action: remote
			? {
					code: "update_agent",
					target: { kind: "update_agent", deviceId: device.id },
				}
			: {
					code: "how_to_update",
					target: deviceRoute(device.id, "settings"),
				},
		source: agent.source,
		lastKnown: agent.source.src !== "live" || !device.liveOpen,
	});
});

function canUpdateAgent(input: AttentionInputExt, deviceId: string) {
	return (
		input.myAccess?.[deviceId]?.grants.some(
			(grant) =>
				grant.scope.kind === "device" &&
				grant.capabilities.includes("update_agent") &&
				grant.expires_at > input.now,
		) ?? false
	);
}

const rebooted = perDevice("rebooted_unexpectedly", (input, device) => {
	const fleet = input.fleet[device.id];
	const bootId = fleet?.status?.bootId;
	if (
		!device.active ||
		!fleet?.previousBootId ||
		!bootId ||
		bootId === fleet.previousBootId
	)
		return undefined;
	const bootedAt = device.inspection?.host?.booted_at ?? undefined;
	const previousBootId = fleet.previousBootId;
	const tracked = input.activity.some((item) => {
		if (
			item.target.deviceId !== device.id ||
			(item.kind !== "reboot" && item.kind !== "agent_update")
		)
			return false;
		if (item.resume?.type === "host_operation")
			return item.resume.bootIdBefore === previousBootId;
		return (
			item.finishedAt === undefined ||
			bootedAt === undefined ||
			activityS(item.finishedAt) >= bootedAt - TRACKED_REBOOT_WINDOW_S
		);
	});
	if (tracked) return undefined;
	return attentionCandidate({
		key: "rebooted_unexpectedly",
		severity: "info",
		subject: subject(device),
		params:
			bootedAt === undefined
				? { device: device.name }
				: { device: device.name, bootedAt },
		action: {
			code: "view_activity",
			target: deviceRoute(device.id, "activity"),
		},
		source: fleet.freshness.status,
		since: bootedAt,
	});
});

const subscriptionExpiring = perDevice(
	"status_subscription_expiring",
	(input, device) => {
		const reader = input.fleet[device.id]?.reader;
		if (
			!device.active ||
			!reader ||
			reader.expiresAt <= input.now ||
			reader.expiresAt - input.now > READER_RENEW_S
		)
			return undefined;
		return attentionCandidate({
			key: "status_subscription_expiring",
			severity: "notice",
			subject: subject(device),
			params: { device: device.name, expiresAt: reader.expiresAt },
			action: { code: "renew", target: deviceRoute(device.id, "access") },
			source: localSource(input),
		});
	},
);

const deviceSlots: AttentionRuleExt = {
	key: "device_slots_nearly_full",
	evaluate(input) {
		if (!input.usage) return [];
		const { limits, usage } = input.usage;
		const used = usage.active_devices + usage.pending_enrollments;
		if (
			limits.max_devices <= 0 ||
			used < limits.max_devices * SLOTS_NEARLY_FULL
		)
			return [];
		return [
			attentionCandidate({
				key: "device_slots_nearly_full",
				severity: "notice",
				subject: { kind: "hub" },
				params: {
					used,
					max: limits.max_devices,
					pending: usage.pending_enrollments,
				},
				action: { code: "review_pending_setups", target: { screen: "hub" } },
				source: hubSource(input),
			}),
		];
	},
};

export const DEVICE_RULES: readonly AttentionRuleExt[] = [
	offlineSince,
	late,
	noHeartbeat,
	revoked,
	stillPaying,
	identityMismatch,
	snapshotIntegrity,
	clockSkew,
	accessDenied,
	statusStale,
	backgroundTask,
	agentUpdate,
	rebooted,
	subscriptionExpiring,
	deviceSlots,
];
