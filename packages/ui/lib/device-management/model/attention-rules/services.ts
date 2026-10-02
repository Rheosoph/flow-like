import type { DeploymentRolloutStatus } from "../../deployment";
import type { ActivityItem } from "../../workspace/types";
import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	type DeviceFacts,
	HOUR_S,
	MINUTE_S,
	activityS,
	attentionCandidate,
	fleetFacts,
	hubSource,
	isLastKnown,
	localSource,
	readableServices,
	rolloutEndsAt,
	serviceRoute,
	serviceSubject,
} from "../device-view";
import type { AttentionKey, ServiceView } from "../types";

const DWELL_S = 2 * MINUTE_S;
const STAGED_WAIT_S = HOUR_S;
const SECRET_PENDING_S = 2 * MINUTE_S;
const ROLLBACK_FAILED = new Set(["rollback_timeout", "rollback_failed"]);
const LOOPBACK = /^(127\.|::1$|localhost$|\[::1\]$)/i;

type ServiceEvaluate = (
	input: AttentionInputExt,
	facts: DeviceFacts,
	service: ServiceView,
) => AttentionCandidateExt | undefined;

/** One rule per key over every readable service. */
function perService(
	key: AttentionKey,
	evaluate: ServiceEvaluate,
): AttentionRuleExt {
	return {
		key,
		evaluate: (input) =>
			readableServices(input).flatMap(({ facts, services }) =>
				services.flatMap((service) => evaluate(input, facts, service) ?? []),
			),
	};
}

function serviceItem(
	key: AttentionKey,
	severity: AttentionCandidateExt["severity"],
	facts: DeviceFacts,
	service: ServiceView,
	extra: {
		params?: Record<string, string | number>;
		action: AttentionCandidateExt["action"];
		since?: number;
		dwellS?: number;
	},
): AttentionCandidateExt {
	return attentionCandidate({
		key,
		severity,
		subject: serviceSubject(service),
		params: { service: service.serviceId, device: facts.name, ...extra.params },
		action: extra.action,
		source: service.freshness,
		lastKnown: isLastKnown(service.freshness),
		since: extra.since,
		dwellS: extra.dwellS,
	});
}

const statusAction = (service: ServiceView) => ({
	code: "view_status" as const,
	target: serviceRoute(service.deviceId, service.serviceId, "status"),
});

const crashLooping = perService(
	"service_crash_looping",
	(_input, facts, service) =>
		service.conv === "crash_looping"
			? serviceItem("service_crash_looping", "critical", facts, service, {
					params: {
						observed: service.observed,
						ready: service.instances.ready,
						requested: service.instances.requested,
						...(service.diagnostics?.lastError
							? { lastError: service.diagnostics.lastError }
							: {}),
					},
					action: {
						code: "diagnose",
						target: {
							kind: "diagnose",
							deviceId: service.deviceId,
							serviceId: service.serviceId,
						},
					},
				})
			: undefined,
);

const BUSY = new Set<ServiceView["conv"]>([
	"crash_looping",
	"update_in_progress",
]);

function notAsRequested(service: ServiceView) {
	if (BUSY.has(service.conv)) return false;
	if (service.desired === "running") return service.observed !== "running";
	return (
		service.desired === "stopped" &&
		(service.observed === "running" || service.observed === "starting")
	);
}

const notAsRequestedRule = perService(
	"service_not_as_requested",
	(_input, facts, service) =>
		notAsRequested(service)
			? serviceItem("service_not_as_requested", "warning", facts, service, {
					params: { observed: service.observed, desired: service.desired },
					action: statusAction(service),
					dwellS: DWELL_S,
				})
			: undefined,
);

const settingsNotApplied = perService(
	"service_settings_not_applied",
	(_input, facts, service) => {
		const { applied, latest } = service.settings;
		if (BUSY.has(service.conv) || applied === null || applied >= latest)
			return undefined;
		return serviceItem(
			"service_settings_not_applied",
			"warning",
			facts,
			service,
			{
				params: { applied, latest },
				action: statusAction(service),
				dwellS: DWELL_S,
			},
		);
	},
);

const degraded = perService("service_degraded", (_input, facts, service) => {
	const { ready, requested } = service.instances;
	if (
		BUSY.has(service.conv) ||
		service.desired !== "running" ||
		service.observed !== "running" ||
		ready >= requested
	)
		return undefined;
	return serviceItem("service_degraded", "warning", facts, service, {
		params: { ready, requested },
		action: {
			code: "view_instances",
			target: serviceRoute(service.deviceId, service.serviceId, "status"),
		},
		dwellS: DWELL_S,
	});
});

function rolloutRule(
	key: AttentionKey,
	match: (rollout: DeploymentRolloutStatus, now: number) => boolean,
	severity: AttentionCandidateExt["severity"],
	action: (service: ServiceView) => AttentionCandidateExt["action"],
): AttentionRuleExt {
	return perService(key, (input, facts, service) => {
		const rollout = service.rollout;
		if (!rollout || !match(rollout, input.now)) return undefined;
		const endsAt = rolloutEndsAt(rollout);
		return serviceItem(key, severity, facts, service, {
			params: {
				state: rollout.state,
				...(rollout.failure_code ? { failure: rollout.failure_code } : {}),
				...(endsAt ? { deadlineAt: endsAt } : {}),
			},
			action: action(service),
			since: rollout.updated_at ?? rollout.created_at,
		});
	});
}

const viewUpdate = (service: ServiceView) => ({
	code: "view_update" as const,
	target: serviceRoute(service.deviceId, service.serviceId, "status"),
});

const ROLLOUT_RUNNING = new Set(["validating", "activating", "rolling_back"]);

const rolloutRules = [
	rolloutRule(
		"rollout_in_progress",
		(rollout) => ROLLOUT_RUNNING.has(rollout.state),
		"info",
		(service) => ({
			code: "follow",
			target: serviceRoute(service.deviceId, service.serviceId, "status"),
		}),
	),
	rolloutRule(
		"rollout_failed_service_stopped",
		(rollout) =>
			rollout.state === "failed" &&
			ROLLBACK_FAILED.has(rollout.failure_code ?? ""),
		"critical",
		(service) => ({
			code: "diagnose",
			target: {
				kind: "diagnose",
				deviceId: service.deviceId,
				serviceId: service.serviceId,
			},
		}),
	),
	rolloutRule(
		"rollout_rolled_back",
		(rollout) => rollout.state === "rolled_back",
		"warning",
		viewUpdate,
	),
	rolloutRule(
		"rollout_not_applied",
		(rollout) =>
			rollout.state === "failed" &&
			!ROLLBACK_FAILED.has(rollout.failure_code ?? ""),
		"notice",
		viewUpdate,
	),
	rolloutRule(
		"rollout_staged_waiting",
		(rollout, now) =>
			rollout.state === "staged" &&
			rollout.created_at !== undefined &&
			now - rollout.created_at > STAGED_WAIT_S,
		"notice",
		(service) => ({
			code: "activate",
			target: serviceRoute(service.deviceId, service.serviceId, "status"),
		}),
	),
];

function secretItems(
	input: AttentionInputExt,
	key: "secret_write_pending" | "secret_write_failed",
	match: (item: ActivityItem) => boolean,
): AttentionCandidateExt[] {
	const facts = fleetFacts(input);
	return input.activity.flatMap((item) => {
		const serviceId = item.target.serviceId;
		const device = facts.byId.get(item.target.deviceId);
		if (item.kind !== "secret_write" || !serviceId || !device || !match(item))
			return [];
		const secret =
			item.resume?.type === "secret" ? item.resume.name : undefined;
		return [
			attentionCandidate({
				key,
				severity: key === "secret_write_failed" ? "warning" : "notice",
				subject: {
					kind: "service",
					deviceId: device.id,
					serviceId,
					...(item.target.projectId
						? { projectId: item.target.projectId }
						: {}),
				},
				params: secret
					? { service: serviceId, device: device.name, secret }
					: { service: serviceId, device: device.name },
				action: {
					code: key === "secret_write_failed" ? "try_again" : "check_again",
					target: serviceRoute(device.id, serviceId, "configuration"),
				},
				source: localSource(input),
				since: activityS(item.startedAt),
			}),
		];
	});
}

const secretPending: AttentionRuleExt = {
	key: "secret_write_pending",
	evaluate: (input) =>
		secretItems(
			input,
			"secret_write_pending",
			(item) =>
				(item.state === "waiting" || item.state === "active") &&
				input.now - activityS(item.startedAt) > SECRET_PENDING_S,
		),
};

const secretFailed: AttentionRuleExt = {
	key: "secret_write_failed",
	evaluate: (input) =>
		secretItems(
			input,
			"secret_write_failed",
			(item) => item.state === "failed" && !item.dismissed,
		),
};

const unencryptedEndpoint = perService(
	"endpoint_unencrypted_exposed",
	(input, facts, service) => {
		const config = facts.liveInput?.placements?.[service.serviceId];
		if (
			!config?.host ||
			config.port === undefined ||
			LOOPBACK.test(config.host) ||
			config.tlsCertificateId
		)
			return undefined;
		return attentionCandidate({
			key: "endpoint_unencrypted_exposed",
			severity: "notice",
			subject: serviceSubject(service),
			params: {
				service: service.serviceId,
				device: facts.name,
				address: config.host,
				port: config.port,
			},
			action: {
				code: "assign_certificate",
				target: serviceRoute(service.deviceId, service.serviceId, "endpoint"),
			},
			source: facts.inspectionSource ?? hubSource(input),
			lastKnown: !facts.liveOpen,
		});
	},
);

export const SERVICE_RULES: readonly AttentionRuleExt[] = [
	crashLooping,
	notAsRequestedRule,
	settingsNotApplied,
	degraded,
	...rolloutRules,
	secretPending,
	secretFailed,
	unencryptedEndpoint,
];
