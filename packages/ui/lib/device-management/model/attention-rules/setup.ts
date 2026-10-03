import type { AttentionInputExt, AttentionRuleExt } from "../attention";
import {
	attentionCandidate,
	fleetFacts,
	hubSource,
	localSource,
} from "../device-view";
import type { PendingSetup } from "../types";

const RESUME_STEP = 6;

function setupSource(input: AttentionInputExt, setup: PendingSetup) {
	return setup.local ? localSource(input) : hubSource(input);
}

function isExpired(input: AttentionInputExt, setup: PendingSetup) {
	return (
		setup.state === "expired" ||
		(setup.state === "pending" && setup.expiresAt <= input.now)
	);
}

const waiting: AttentionRuleExt = {
	key: "pending_setup_waiting",
	evaluate(input) {
		return (input.pendingSetups ?? []).flatMap((setup) =>
			setup.state === "pending" && setup.expiresAt > input.now
				? [
						attentionCandidate({
							key: "pending_setup_waiting",
							severity: "info",
							subject: { kind: "setup", enrollmentId: setup.enrollmentId },
							params: { name: setup.name, expiresAt: setup.expiresAt },
							action: {
								code: "show_start_instructions",
								target: {
									screen: "setup",
									enrollmentId: setup.enrollmentId,
									step: RESUME_STEP,
								},
							},
							source: setupSource(input, setup),
							since: setup.createdAt,
						}),
					]
				: [],
		);
	},
};

const expired: AttentionRuleExt = {
	key: "pending_setup_expired",
	evaluate(input) {
		return (input.pendingSetups ?? []).flatMap((setup) =>
			isExpired(input, setup)
				? [
						attentionCandidate({
							key: "pending_setup_expired",
							severity: "notice",
							subject: { kind: "setup", enrollmentId: setup.enrollmentId },
							params: { name: setup.name, expiredAt: setup.expiresAt },
							action: { code: "set_up_again", target: { screen: "setup" } },
							source: setupSource(input, setup),
							since: setup.expiresAt,
						}),
					]
				: [],
		);
	},
};

/** "Only for owners": someone who owns a device, has a setup in flight, or has no devices yet. */
function setsUpDevices(input: AttentionInputExt) {
	const devices = fleetFacts(input).devices;
	return (
		devices.length === 0 ||
		(input.pendingSetups?.length ?? 0) > 0 ||
		devices.some((device) => device.relationship === "owner")
	);
}

const hubNotReady: AttentionRuleExt = {
	key: "hub_not_ready",
	evaluate(input) {
		const failing = input.readiness?.checks.find((check) => !check.ready);
		if (!input.readiness || input.readiness.ready || !setsUpDevices(input))
			return [];
		return [
			attentionCandidate({
				key: "hub_not_ready",
				severity: "warning",
				subject: { kind: "hub" },
				params: failing ? { check: failing.id } : {},
				action: { code: "view_hub_status", target: { screen: "hub" } },
				source: hubSource(input),
			}),
		];
	},
};

const releaseTrustMissing: AttentionRuleExt = {
	key: "release_trust_missing",
	evaluate(input) {
		const releaseCheck = input.readiness?.checks.find(
			(check) => check.id === "release",
		);
		const missing =
			input.releaseTrust === null || releaseCheck?.ready === false;
		const ownsLinux = fleetFacts(input).devices.some(
			(device) =>
				device.active &&
				device.relationship === "owner" &&
				device.inspection?.isolation?.platform === "linux",
		);
		if (!missing || !ownsLinux) return [];
		return [
			attentionCandidate({
				key: "release_trust_missing",
				severity: "notice",
				subject: { kind: "hub" },
				action: { code: "view_hub_status", target: { screen: "hub" } },
				source: hubSource(input),
			}),
		];
	},
};

export const SETUP_RULES: readonly AttentionRuleExt[] = [
	waiting,
	expired,
	hubNotReady,
	releaseTrustMissing,
];
