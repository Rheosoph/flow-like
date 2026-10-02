import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey, PlacementStatusPlus } from "../types";
import { SERVICE_RULES } from "./services";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	SERVICE_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

/** Edge is live: replace its support-bot row. */
function edgeWith(patch: Partial<PlacementStatusPlus>) {
	const input = sampleFleet();
	const inspection = input.live[SAMPLE_IDS.edge].inspection;
	if (!inspection) throw new Error("fixture: edge inspection");
	inspection.value.placements[0] = {
		...inspection.value.placements[0],
		...patch,
	};
	return input;
}

function edgeRollout(patch: Record<string, unknown>) {
	const input = sampleFleet();
	const rollouts = input.live[SAMPLE_IDS.edge].rollouts;
	if (!rollouts) throw new Error("fixture: rollouts");
	rollouts[0] = { ...rollouts[0], ...patch };
	return input;
}

describe("convergence", () => {
	test("crash looping is Critical; live keeps crashing, snapshot is last known", () => {
		const [last] = evaluate("service_crash_looping", sampleFleet());
		expect(last.lastKnown).toBe(true);
		expect(last.subject).toMatchObject({
			deviceId: SAMPLE_IDS.warehouse,
			serviceId: "scanner-ingest",
		});
		const live = evaluate(
			"service_crash_looping",
			edgeWith({ observed_state: "backoff", last_error: "exit status 101" }),
		).find(
			(item) =>
				item.subject.kind === "service" &&
				item.subject.deviceId === SAMPLE_IDS.edge,
		);
		expect(live?.lastKnown).toBe(false);
		expect(live?.copy.params?.lastError).toBe("exit status 101");
	});

	test("BG7 restarts.crash_looping counts as crashing even while running", () => {
		const input = edgeWith({
			restarts: {
				failures: 5,
				max_restarts: 5,
				crash_looping: true,
				retry_in_seconds: 30,
				last_started_at: SAMPLE_NOW - 10,
			},
		});
		expect(
			evaluate("service_crash_looping", input).some(
				(item) => item.copy.params?.service === "support-bot",
			),
		).toBe(true);
	});

	test("not as requested, settings not applied and degraded wait 2 min", () => {
		const notRunning = evaluate(
			"service_not_as_requested",
			edgeWith({ observed_state: "stopped" }),
		);
		expect(notRunning[0]).toMatchObject({ severity: "warning", dwellS: 120 });
		const stillRunning = evaluate(
			"service_not_as_requested",
			edgeWith({ desired_state: "stopped", observed_state: "running" }),
		);
		expect(stillRunning).toHaveLength(1);
		const settings = evaluate(
			"service_settings_not_applied",
			edgeWith({ config_revision: 8 }),
		);
		expect(settings[0].copy.params).toMatchObject({ applied: 7, latest: 8 });
		const degraded = evaluate(
			"service_degraded",
			edgeWith({ ready_replicas: 1 }),
		);
		expect(degraded[0].copy.params).toMatchObject({ ready: 1, requested: 2 });
		expect(degraded[0].dwellS).toBe(120);
	});

	test("an update in progress isn't 'not as requested'", () => {
		const input = sampleFleet();
		for (const key of [
			"service_not_as_requested",
			"service_settings_not_applied",
			"service_degraded",
		] as const)
			expect(evaluate(key, input)).toEqual([]);
	});
});

describe("rollouts", () => {
	test("in progress is Info with Follow", () => {
		const [item] = evaluate("rollout_in_progress", sampleFleet());
		expect(item).toMatchObject({
			severity: "info",
			action: { code: "follow" },
		});
		expect(item.copy.params?.state).toBe("activating");
	});

	test("failure codes map to stopped (Critical), rolled back (Warning), not applied (Notice)", () => {
		const stopped = edgeRollout({
			state: "failed",
			failure_code: "rollback_failed",
		});
		expect(
			evaluate("rollout_failed_service_stopped", stopped)[0]?.severity,
		).toBe("critical");
		const rolledBack = edgeRollout({
			state: "rolled_back",
			failure_code: "candidate_failed",
		});
		expect(
			evaluate("rollout_rolled_back", rolledBack)[0]?.copy.params?.failure,
		).toBe("candidate_failed");
		const notApplied = edgeRollout({
			state: "failed",
			failure_code: "staging_timeout",
		});
		expect(evaluate("rollout_not_applied", notApplied)[0]?.severity).toBe(
			"notice",
		);
		expect(evaluate("rollout_failed_service_stopped", notApplied)).toEqual([]);
	});

	test("a staged update waiting over 1 h is a Notice with Activate", () => {
		const fresh = edgeRollout({
			state: "staged",
			created_at: SAMPLE_NOW - 600,
		});
		expect(evaluate("rollout_staged_waiting", fresh)).toEqual([]);
		const old = edgeRollout({
			state: "staged",
			created_at: SAMPLE_NOW - 3_700,
			deadline_at: null,
		});
		const [item] = evaluate("rollout_staged_waiting", old);
		expect(item?.action?.code).toBe("activate");
		// A staged update has no deadline yet: the device discards it a day after staging.
		expect(item?.copy.params?.deadlineAt).toBe(SAMPLE_NOW - 3_700 + 86_400);
	});
});

describe("secrets and endpoints", () => {
	/** `startedS` is unix seconds; the tray stores milliseconds. */
	function withSecret(state: "waiting" | "failed", startedS: number) {
		const startedAt = startedS * 1_000;
		const input = sampleFleet();
		input.activity.push({
			id: "secret-1",
			kind: "secret_write",
			target: {
				deviceId: SAMPLE_IDS.edge,
				serviceId: "invoice-extractor",
				projectId: "app_invoice_ai",
			},
			state,
			label: { code: "secret_write" },
			startedAt,
			updatedAt: startedAt,
			startedBy: "you",
			actions: [],
			resume: {
				type: "secret",
				operationId: "op",
				placementId: "invoice-extractor",
				name: "ERP password",
			},
		});
		return input;
	}

	test("a secret write unconfirmed after 2 min is a Notice; a failed one a Warning", () => {
		expect(
			evaluate("secret_write_pending", withSecret("waiting", SAMPLE_NOW - 60)),
		).toEqual([]);
		const [pending] = evaluate(
			"secret_write_pending",
			withSecret("waiting", SAMPLE_NOW - 180),
		);
		expect(pending.copy.params).toMatchObject({
			secret: "ERP password",
			service: "invoice-extractor",
		});
		expect(
			evaluate("secret_write_failed", withSecret("failed", SAMPLE_NOW - 30))[0]
				?.severity,
		).toBe("warning");
	});

	test("an unencrypted endpoint beyond loopback is a Notice", () => {
		const input = sampleFleet();
		expect(evaluate("endpoint_unencrypted_exposed", input)).toEqual([]);
		const placements = input.live[SAMPLE_IDS.edge].placements;
		if (!placements) throw new Error("fixture");
		placements["invoice-extractor"] = {
			host: "0.0.0.0",
			port: 8_081,
			tlsCertificateId: null,
		};
		const [item] = evaluate("endpoint_unencrypted_exposed", { ...input });
		expect(item.copy.params).toMatchObject({ address: "0.0.0.0", port: 8_081 });
		placements["invoice-extractor"] = { host: "::1", port: 8_081 };
		expect(evaluate("endpoint_unencrypted_exposed", { ...input })).toEqual([]);
	});
});
