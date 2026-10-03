import type { AttentionInputExt, AttentionRuleExt } from "../attention";
import {
	DAY_S,
	attentionCandidate,
	fleetFacts,
	hubSource,
	readableServices,
	serviceRoute,
} from "../device-view";
import type { AttentionInput, AttentionSubject } from "../types";

const ENDING_S = 7 * DAY_S;
const LOW_RATIO = 0.8;

export interface CloudApproval {
	deviceId: string;
	placementId: string;
	grantId: string;
	appId: string | null;
	active: boolean;
	revoked: boolean;
	effectiveExpiresAt: number;
	onlineWriteBlocked: boolean;
	approverIsMe: boolean;
}

export interface CloudBilling {
	deviceId: string;
	billingGrantId: string;
	grantId: string;
	limitMicros: number;
	usedMicros: number;
	reservedMicros: number;
	expiresAt: number;
	active: boolean;
	payerIsMe: boolean;
}

interface CloudFacts {
	approvals: CloudApproval[];
	billing: CloudBilling[];
}

const cache = new WeakMap<AttentionInput, CloudFacts>();

type SummaryDevice = NonNullable<
	AttentionInputExt["resourceSummary"]
>["devices"][number];
type DeviceResourceLists = NonNullable<AttentionInputExt["resources"][string]>;

const summaryApprovals = (
	device: SummaryDevice,
	now: number,
): CloudApproval[] =>
	device.approvals.map((approval) => ({
		deviceId: device.device_id,
		placementId: approval.placement_id,
		grantId: approval.grant_id,
		appId: approval.app_id,
		revoked: approval.status === "revoked",
		active: approval.status === "active" && approval.effective_expires_at > now,
		effectiveExpiresAt: approval.effective_expires_at,
		onlineWriteBlocked: approval.online_write_blocked === "storage_full",
		approverIsMe: approval.approver_is_me,
	}));

const summaryBilling = (device: SummaryDevice, now: number): CloudBilling[] =>
	device.billing.map((billing) => ({
		deviceId: device.device_id,
		billingGrantId: billing.billing_grant_id,
		grantId: billing.grant_id,
		limitMicros: billing.limit_micros,
		usedMicros: billing.used_micros,
		reservedMicros: billing.reserved_micros,
		expiresAt: billing.expires_at,
		active: billing.expires_at > now,
		payerIsMe: billing.payer_is_me,
	}));

const resourceApprovals = (
	deviceId: string,
	resources: DeviceResourceLists,
	input: AttentionInputExt,
): CloudApproval[] =>
	resources.grants.map((grant) => ({
		deviceId,
		placementId: grant.placement_id,
		grantId: grant.grant_id,
		appId: grant.app_id ?? grant.project_id,
		revoked: grant.status === "revoked",
		active:
			grant.status === "active" &&
			(grant.effective_expires_at ?? grant.expires_at) > input.now,
		// E15: a hub that sends the tighter end (sharing grant, access rules) and the storage block sends them here too.
		effectiveExpiresAt: grant.effective_expires_at ?? grant.expires_at,
		onlineWriteBlocked: grant.online_write_blocked === "storage_full",
		approverIsMe: grant.delegating_user_id === input.me,
	}));

const resourceBilling = (
	deviceId: string,
	resources: DeviceResourceLists,
	input: AttentionInputExt,
): CloudBilling[] =>
	resources.billing.map((billing) => ({
		deviceId,
		billingGrantId: billing.billing_grant_id,
		grantId: billing.grant_id,
		limitMicros: billing.limit_micros,
		usedMicros: billing.used_micros,
		reservedMicros: billing.reserved_micros,
		expiresAt: billing.expires_at,
		active: billing.status === "active" && billing.expires_at > input.now,
		payerIsMe: billing.payer_id === input.me,
	}));

const fromSummary = (input: AttentionInputExt): CloudFacts => {
	const devices = input.resourceSummary?.devices ?? [];
	return {
		approvals: devices.flatMap((device) => summaryApprovals(device, input.now)),
		billing: devices.flatMap((device) => summaryBilling(device, input.now)),
	};
};

const fromResources = (input: AttentionInputExt): CloudFacts => {
	const devices = Object.entries(input.resources).flatMap(
		([deviceId, resources]) => (resources ? [{ deviceId, resources }] : []),
	);
	return {
		approvals: devices.flatMap(({ deviceId, resources }) =>
			resourceApprovals(deviceId, resources, input),
		),
		billing: devices.flatMap(({ deviceId, resources }) =>
			resourceBilling(deviceId, resources, input),
		),
	};
};

/** FG4 resource summary when the hub has it, else the per-device resource lists. */
export const cloudFacts = (input: AttentionInputExt): CloudFacts => {
	const cached = cache.get(input);
	if (cached) return cached;
	const facts = input.resourceSummary
		? fromSummary(input)
		: fromResources(input);
	cache.set(input, facts);
	return facts;
};

/** What the viewer still pays for or approved on a device (consent, IA §6.5). */
export function consentResources(
	input: AttentionInputExt,
	deviceId: string,
): { billing?: CloudBilling; approval?: CloudApproval } | undefined {
	const facts = cloudFacts(input);
	const billing = facts.billing.find(
		(entry) => entry.deviceId === deviceId && entry.active && entry.payerIsMe,
	);
	if (billing) return { billing };
	const approval = facts.approvals.find(
		(entry) =>
			entry.deviceId === deviceId && entry.active && entry.approverIsMe,
	);
	return approval ? { approval } : undefined;
}

/** Services whose approvals were revoked with none active left (dedupe rule c). */
export function revokedApprovalServices(input: AttentionInputExt): Set<string> {
	const byService = new Map<string, CloudApproval[]>();
	for (const approval of cloudFacts(input).approvals) {
		const key = `${approval.deviceId}/${approval.placementId}`;
		byService.set(key, [...(byService.get(key) ?? []), approval]);
	}
	const revoked = new Set<string>();
	for (const [key, approvals] of byService)
		if (
			approvals.some((approval) => approval.revoked) &&
			!approvals.some((approval) => approval.active)
		)
			revoked.add(key);
	return revoked;
}

function approvalSubject(approval: {
	deviceId: string;
	placementId: string;
	appId: string | null;
}): AttentionSubject {
	return approval.appId
		? {
				kind: "service",
				deviceId: approval.deviceId,
				serviceId: approval.placementId,
				projectId: approval.appId,
			}
		: {
				kind: "service",
				deviceId: approval.deviceId,
				serviceId: approval.placementId,
			};
}

function deviceActive(input: AttentionInputExt, deviceId: string) {
	return fleetFacts(input).byId.get(deviceId)?.active ?? false;
}

function isOwner(input: AttentionInputExt, deviceId: string) {
	return fleetFacts(input).byId.get(deviceId)?.relationship === "owner";
}

/** The service's bound grant when it no longer holds, else any approval when none is active. */
function invalidApproval(
	own: readonly CloudApproval[],
	bound: string | null | undefined,
): CloudApproval | undefined {
	if (bound != null)
		return own.find(
			(approval) => approval.grantId === bound && !approval.active,
		);
	return own.some((approval) => approval.active) ? undefined : own[0];
}

const cloudAccessInvalid: AttentionRuleExt = {
	key: "cloud_access_invalid",
	evaluate(input) {
		const { approvals } = cloudFacts(input);
		return readableServices(input).flatMap(({ facts, services }) =>
			services.flatMap((service) => {
				if (service.desired !== "running") return [];
				const own = approvals.filter(
					(approval) =>
						approval.deviceId === facts.id &&
						approval.placementId === service.serviceId,
				);
				const invalid = invalidApproval(
					own,
					facts.liveInput?.placements?.[service.serviceId]?.resourceGrantId,
				);
				if (!invalid) return [];
				return [
					attentionCandidate({
						key: "cloud_access_invalid",
						severity: "critical",
						subject: approvalSubject({
							...invalid,
							appId: service.projectId,
						}),
						params: {
							service: service.serviceId,
							device: facts.name,
							reason: invalid.revoked ? "revoked" : "expired",
						},
						action: {
							code: "fix_cloud_access",
							target: serviceRoute(facts.id, service.serviceId, "cloud"),
						},
						source: hubSource(input),
						since: invalid.revoked ? undefined : invalid.effectiveExpiresAt,
					}),
				];
			}),
		);
	},
};

const cloudAccessEnding: AttentionRuleExt = {
	key: "cloud_access_ending",
	evaluate(input) {
		return cloudFacts(input).approvals.flatMap((approval) =>
			approval.active &&
			approval.effectiveExpiresAt - input.now <= ENDING_S &&
			deviceActive(input, approval.deviceId) &&
			(approval.approverIsMe || isOwner(input, approval.deviceId))
				? [
						attentionCandidate({
							key: "cloud_access_ending",
							severity: "warning",
							subject: approvalSubject(approval),
							params: {
								service: approval.placementId,
								expiresAt: approval.effectiveExpiresAt,
							},
							action: {
								code: "replace_approval",
								target: serviceRoute(
									approval.deviceId,
									approval.placementId,
									"cloud",
								),
							},
							source: hubSource(input),
						}),
					]
				: [],
		);
	},
};

function billingTarget(input: AttentionInputExt, billing: CloudBilling) {
	return cloudFacts(input).approvals.find(
		(entry) =>
			entry.deviceId === billing.deviceId && entry.grantId === billing.grantId,
	);
}

const spendingLow: AttentionRuleExt = {
	key: "spending_limit_low",
	evaluate(input) {
		return cloudFacts(input).billing.flatMap((billing) => {
			const approval = billingTarget(input, billing);
			const spent = billing.usedMicros + billing.reservedMicros;
			if (
				!billing.active ||
				!approval ||
				billing.limitMicros <= 0 ||
				spent < billing.limitMicros * LOW_RATIO ||
				!(billing.payerIsMe || approval.approverIsMe)
			)
				return [];
			return [
				attentionCandidate({
					key: "spending_limit_low",
					severity: spent >= billing.limitMicros ? "critical" : "warning",
					subject: approvalSubject(approval),
					params: {
						service: approval.placementId,
						usedMicros: spent,
						limitMicros: billing.limitMicros,
					},
					action: {
						code: "replace_limit",
						target: serviceRoute(
							approval.deviceId,
							approval.placementId,
							"cloud",
						),
					},
					source: hubSource(input),
				}),
			];
		});
	},
};

const spendingEnding: AttentionRuleExt = {
	key: "spending_limit_ending",
	evaluate(input) {
		return cloudFacts(input).billing.flatMap((billing) => {
			const approval = billingTarget(input, billing);
			if (
				!billing.active ||
				!billing.payerIsMe ||
				!approval ||
				billing.expiresAt - input.now > ENDING_S
			)
				return [];
			return [
				attentionCandidate({
					key: "spending_limit_ending",
					severity: "notice",
					subject: approvalSubject(approval),
					params: {
						service: approval.placementId,
						expiresAt: billing.expiresAt,
					},
					action: {
						code: "replace_limit",
						target: serviceRoute(
							approval.deviceId,
							approval.placementId,
							"cloud",
						),
					},
					source: hubSource(input),
				}),
			];
		});
	},
};

const onlineFilesReadOnly: AttentionRuleExt = {
	key: "online_files_read_only",
	evaluate(input) {
		return cloudFacts(input).approvals.flatMap((approval) =>
			approval.active && approval.onlineWriteBlocked && approval.approverIsMe
				? [
						attentionCandidate({
							key: "online_files_read_only",
							severity: "warning",
							subject: approvalSubject(approval),
							params: { service: approval.placementId },
							action: {
								code: "open_app_storage",
								target: serviceRoute(
									approval.deviceId,
									approval.placementId,
									"cloud",
								),
							},
							source: hubSource(input),
						}),
					]
				: [],
		);
	},
};

export const CLOUD_RULES: readonly AttentionRuleExt[] = [
	cloudAccessInvalid,
	cloudAccessEnding,
	spendingLow,
	spendingEnding,
	onlineFilesReadOnly,
];
