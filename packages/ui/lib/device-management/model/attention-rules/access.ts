import type {
	Capability,
	ManagementGrant,
	ManagementPolicy,
} from "../../types";
import type {
	AccessRequestRecord,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	DAY_S,
	type DeviceFacts,
	HOUR_S,
	MINUTE_S,
	attentionCandidate,
	deviceLabel,
	deviceListKnown,
	deviceRoute,
	fleetFacts,
	hubSource,
	localSource,
	perDevice,
} from "../device-view";

const ENDING_SOON_S = 6 * HOUR_S;
const POLICY_EXPIRING_S = 7 * DAY_S;
const POLICY_WAIT_DWELL_S = 2 * MINUTE_S;
const POLICY_WAIT_WARNING_S = 10 * MINUTE_S;
const MAX_GRANTS = 24;
const GRANTS_NEARLY_FULL = 20;
const RUNS_CODE = new Set<Capability>(["deploy", "start", "restart", "scale"]);

/** Requests made on this computer that the hub doesn't list as active access yet. */
export const pendingAccessRequests = (
	input: AttentionInputExt,
): AccessRequestRecord[] => {
	const listed = fleetFacts(input).byId;
	return (input.accessRequests ?? []).filter(
		(request) => !request.approved && !listed.get(request.deviceId)?.active,
	);
};

const ownerPolicy = (
	input: AttentionInputExt,
	device: DeviceFacts,
): ManagementPolicy | undefined => {
	if (!device.active || device.relationship !== "owner") return undefined;
	return input.policies[device.id]?.policy;
};

const activeGrants = (
	input: AttentionInputExt,
	policy: ManagementPolicy,
): ManagementGrant[] =>
	policy.grants.filter((grant) => grant.expires_at > input.now);

function hasHistoryReaders(input: AttentionInputExt, deviceId: string) {
	return (input.live[deviceId]?.history?.length ?? 0) > 0;
}

const accessRoute = (deviceId: string) => deviceRoute(deviceId, "access");

/** My access end: BG1 row field, else BG22 my-access, else the verified policy after unlock. */
function myAccessEnd(input: AttentionInputExt, device: DeviceFacts) {
	if (device.row.access_expires_at != null) return device.row.access_expires_at;
	const grants = input.myAccess?.[device.id]?.grants;
	if (grants?.length)
		return Math.max(...grants.map((grant) => grant.expires_at));
	return input.fleet[device.id]?.policy?.myGrant?.expires_at;
}

const sharedExpiring = perDevice("shared_access_expiring", (input, device) => {
	if (!device.active || device.relationship !== "shared") return undefined;
	const end = myAccessEnd(input, device);
	if (end === undefined || end <= input.now || end - input.now >= ENDING_SOON_S)
		return undefined;
	return attentionCandidate({
		key: "shared_access_expiring",
		severity: "warning",
		subject: { kind: "access", deviceId: device.id },
		params: { device: device.name, expiresAt: end },
		action: {
			code: "ask_to_renew",
			target: { kind: "ask_to_renew", deviceId: device.id },
		},
		source: hubSource(input),
	});
});

const sharedEnded: AttentionRuleExt = {
	key: "shared_access_ended",
	evaluate(input) {
		const facts = fleetFacts(input);
		const listedEnded = facts.devices
			.filter(
				(device) =>
					device.active &&
					device.relationship === "shared" &&
					device.row.access_expires_at != null &&
					device.row.access_expires_at <= input.now,
			)
			.map((device) => ({
				deviceId: device.id,
				since: device.row.access_expires_at ?? undefined,
				source: hubSource(input),
			}));
		const pending = new Set(
			pendingAccessRequests(input).map((request) => request.deviceId),
		);
		// "Gone from the list" needs the list: an unloaded or unreadable one ends nobody's access.
		const goneFromList =
			input.accessRequests && deviceListKnown(input)
				? input.local.vaults
						.filter(
							(vault) =>
								vault.role === "shared" &&
								!facts.byId.has(vault.deviceId) &&
								!pending.has(vault.deviceId),
						)
						.map((vault) => ({
							deviceId: vault.deviceId,
							since: undefined,
							source: localSource(input),
						}))
				: [];
		return [...listedEnded, ...goneFromList].map(
			({ deviceId, since, source }) =>
				attentionCandidate({
					key: "shared_access_ended",
					severity: "notice",
					subject: { kind: "access", deviceId },
					params: { device: deviceLabel(input, deviceId) },
					action: {
						code: "ask_to_renew",
						target: { kind: "ask_to_renew", deviceId },
					},
					secondary: {
						code: "remove_from_computer",
						target: { screen: "keys", focusDeviceId: deviceId },
					},
					source,
					since,
				}),
		);
	},
};

const grantExpiring = perDevice("grant_expiring", (input, device) => {
	const policy = ownerPolicy(input, device);
	if (!policy || policy.expires_at <= input.now) return undefined;
	return activeGrants(input, policy)
		.filter((grant) => grant.expires_at - input.now <= ENDING_SOON_S)
		.map((grant) =>
			attentionCandidate({
				key: "grant_expiring",
				severity: "notice",
				subject: {
					kind: "access",
					deviceId: device.id,
					personId: grant.user_id,
				},
				params: {
					person: grant.user_id,
					device: device.name,
					expiresAt: grant.expires_at,
				},
				action: { code: "renew", target: accessRoute(device.id) },
				source: hubSource(input),
			}),
		);
});

const policyWaiting = perDevice(
	"sharing_policy_waiting_for_device",
	(input, device) => {
		const view = input.policies[device.id];
		if (
			!device.active ||
			device.relationship !== "owner" ||
			!view ||
			view.version <= view.applied_version
		)
			return undefined;
		const savedAt =
			view.policy?.policy_version === view.version
				? view.policy.issued_at
				: undefined;
		const escalated =
			savedAt !== undefined &&
			input.now - savedAt > POLICY_WAIT_WARNING_S &&
			device.presence.kind === "online";
		return attentionCandidate({
			key: "sharing_policy_waiting_for_device",
			severity: escalated ? "warning" : "notice",
			subject: { kind: "access", deviceId: device.id },
			params: {
				device: device.name,
				version: view.version,
				applied: view.applied_version,
			},
			action: { code: "view_access", target: accessRoute(device.id) },
			source: hubSource(input),
			since: savedAt,
			dwellS: POLICY_WAIT_DWELL_S,
		});
	},
);

const policyExpiring = perDevice("sharing_policy_expiring", (input, device) => {
	const policy = ownerPolicy(input, device);
	if (
		!policy ||
		policy.expires_at <= input.now ||
		policy.expires_at - input.now > POLICY_EXPIRING_S ||
		(activeGrants(input, policy).length === 0 &&
			!hasHistoryReaders(input, device.id))
	)
		return undefined;
	return attentionCandidate({
		key: "sharing_policy_expiring",
		severity: "warning",
		subject: { kind: "access", deviceId: device.id },
		params: { device: device.name, expiresAt: policy.expires_at },
		action: { code: "renew_access_rules", target: accessRoute(device.id) },
		source: hubSource(input),
	});
});

const policyExpired = perDevice("sharing_policy_expired", (input, device) => {
	const policy = ownerPolicy(input, device);
	if (!policy || policy.expires_at > input.now) return undefined;
	const mattered =
		policy.grants.length > 0 || hasHistoryReaders(input, device.id);
	return attentionCandidate({
		key: "sharing_policy_expired",
		severity: mattered ? "critical" : "notice",
		subject: { kind: "access", deviceId: device.id },
		params: { device: device.name, expiredAt: policy.expires_at },
		action: { code: "renew_access_rules", target: accessRoute(device.id) },
		source: hubSource(input),
		since: policy.expires_at,
	});
});

const slotsNearlyFull = perDevice(
	"access_slots_nearly_full",
	(input, device) => {
		const policy = ownerPolicy(input, device);
		const used = policy ? activeGrants(input, policy).length : 0;
		if (used < GRANTS_NEARLY_FULL) return undefined;
		return attentionCandidate({
			key: "access_slots_nearly_full",
			severity: "notice",
			subject: { kind: "access", deviceId: device.id },
			params: { device: device.name, used, max: MAX_GRANTS },
			action: { code: "review_access", target: accessRoute(device.id) },
			source: hubSource(input),
		});
	},
);

const requestPending: AttentionRuleExt = {
	key: "access_request_pending",
	evaluate(input) {
		return pendingAccessRequests(input).map((request) =>
			attentionCandidate({
				key: "access_request_pending",
				severity: "info",
				subject: { kind: "access", deviceId: request.deviceId },
				params: request.ownerId
					? {
							device: deviceLabel(input, request.deviceId),
							owner: request.ownerId,
						}
					: { device: deviceLabel(input, request.deviceId) },
				action: {
					code: "download_request_again",
					target: { screen: "access", tab: "shared" },
				},
				source: localSource(input),
				since: request.createdAt,
			}),
		);
	},
};

const codeWithoutSandbox = perDevice(
	"code_running_access_without_sandbox",
	(input, device) => {
		const policy = ownerPolicy(input, device);
		const isolation = device.inspection?.hostIsolation;
		if (!policy || !isolation || isolation === "required") return undefined;
		return activeGrants(input, policy)
			.filter((grant) =>
				grant.capabilities.some((capability) => RUNS_CODE.has(capability)),
			)
			.map((grant) =>
				attentionCandidate({
					key: "code_running_access_without_sandbox",
					severity: "notice",
					subject: {
						kind: "access",
						deviceId: device.id,
						personId: grant.user_id,
					},
					params: { person: grant.user_id, device: device.name },
					action: { code: "review_access", target: accessRoute(device.id) },
					source: device.inspectionSource ?? hubSource(input),
				}),
			);
	},
);

export const ACCESS_RULES: readonly AttentionRuleExt[] = [
	sharedExpiring,
	sharedEnded,
	grantExpiring,
	policyWaiting,
	policyExpiring,
	policyExpired,
	slotsNearlyFull,
	requestPending,
	codeWithoutSandbox,
];
