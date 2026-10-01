import type { ActivityItem } from "../../workspace/types";
import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
} from "../attention";
import {
	DAY_S,
	activityS,
	attentionCandidate,
	deviceLabel,
	deviceRoute,
	fleetFacts,
	localSource,
} from "../device-view";
import type {
	AttentionActionCode,
	AttentionKey,
	AttentionSubject,
	CopyParams,
	HostOperationView,
} from "../types";

const UNCONFIRMED_WINDOW_S = DAY_S;
const HOST_KINDS = new Set<ActivityItem["kind"]>(["reboot", "agent_update"]);
const HOST_FAILED = new Set<HostOperationView["state"]>([
	"failed",
	"rolled_back",
]);

function activitySubject(item: ActivityItem): AttentionSubject {
	const { deviceId, serviceId, projectId } = item.target;
	if (!serviceId) return { kind: "device", deviceId };
	return projectId
		? { kind: "service", deviceId, serviceId, projectId }
		: { kind: "service", deviceId, serviceId };
}

function activityItem(
	input: AttentionInputExt,
	item: ActivityItem,
	key: AttentionKey,
	severity: AttentionCandidateExt["severity"],
	code: AttentionActionCode,
	params: CopyParams,
): AttentionCandidateExt {
	const { deviceId } = item.target;
	return attentionCandidate({
		key,
		severity,
		subject: activitySubject(item),
		params: {
			device: item.target.deviceName ?? deviceLabel(input, deviceId),
			...(item.target.serviceId ? { service: item.target.serviceId } : {}),
			...params,
		},
		action: { code, target: item.href ?? deviceRoute(deviceId, "activity") },
		source: localSource(input),
		since: activityS(item.startedAt),
	});
}

const visible = (item: ActivityItem) => !item.dismissed;

/** The device keeps a command's result for 24 h from when it was issued. */
const issuedAt = (item: ActivityItem) =>
	item.resume?.type === "operation"
		? item.resume.issuedAt
		: activityS(item.startedAt);

/** A paused upload resumes until its transfer expires on the device. */
const resumableUntil = (item: ActivityItem) => {
	if (item.resume?.type === "transfer") return item.resume.expiresAt;
	return item.deadlineAt === undefined
		? undefined
		: activityS(item.deadlineAt);
};

const unconfirmed: AttentionRuleExt = {
	key: "unconfirmed_command",
	evaluate(input) {
		return input.activity
			.filter(
				(item) =>
					visible(item) &&
					item.kind === "command" &&
					item.state === "unknown" &&
					input.now - issuedAt(item) <= UNCONFIRMED_WINDOW_S,
			)
			.map((item) =>
				activityItem(
					input,
					item,
					"unconfirmed_command",
					"warning",
					"check_result",
					item.resume?.type === "operation"
						? { command: item.resume.command }
						: {},
				),
			);
	},
};

function hostOperationItems(
	input: AttentionInputExt,
	key: "device_operation_failed" | "device_operation_unknown",
	matches: (state: HostOperationView["state"]) => boolean,
	activityState: ActivityItem["state"],
	code: AttentionActionCode,
): AttentionCandidateExt[] {
	const fromDevices = fleetFacts(input).devices.flatMap((device) => {
		const operation = device.inspection?.hostOperation;
		if (!device.active || !operation || !matches(operation.state)) return [];
		return [
			attentionCandidate({
				key,
				severity: "warning",
				subject: { kind: "device", deviceId: device.id },
				params: { device: device.name, operation: operation.kind },
				action: { code, target: deviceRoute(device.id, "activity") },
				source: device.inspectionSource ?? localSource(input),
				lastKnown: !device.liveOpen,
				since: operation.created_at,
			}),
		];
	});
	const fromActivity = input.activity
		.filter(
			(item) =>
				visible(item) &&
				HOST_KINDS.has(item.kind) &&
				item.state === activityState,
		)
		.map((item) =>
			activityItem(input, item, key, "warning", code, {
				operation: item.kind === "reboot" ? "reboot" : "update_agent",
			}),
		);
	return [...fromDevices, ...fromActivity];
}

const operationFailed: AttentionRuleExt = {
	key: "device_operation_failed",
	evaluate: (input) =>
		hostOperationItems(
			input,
			"device_operation_failed",
			(state) => HOST_FAILED.has(state),
			"failed",
			"view_details",
		),
};

const operationUnknown: AttentionRuleExt = {
	key: "device_operation_unknown",
	evaluate: (input) =>
		hostOperationItems(
			input,
			"device_operation_unknown",
			(state) => state === "unknown",
			"unknown",
			"check_status",
		),
};

const uploadPaused: AttentionRuleExt = {
	key: "upload_paused",
	evaluate(input) {
		return input.activity.flatMap((item) => {
			const until = resumableUntil(item);
			if (
				!visible(item) ||
				item.kind !== "upload" ||
				item.state !== "paused" ||
				(until !== undefined && until <= input.now)
			)
				return [];
			return [
				activityItem(input, item, "upload_paused", "info", "resume", {
					...(item.target.projectId ? { app: item.target.projectId } : {}),
					...(until === undefined ? {} : { until }),
				}),
			];
		});
	},
};

export const OPERATION_RULES: readonly AttentionRuleExt[] = [
	unconfirmed,
	operationFailed,
	operationUnknown,
	uploadPaused,
];
