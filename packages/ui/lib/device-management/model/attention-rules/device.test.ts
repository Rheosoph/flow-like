import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	deviceRow,
	emptyInput,
	keySession,
	sampleFleet,
	vault,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey, TaskHealth } from "../types";
import { DEVICE_RULES } from "./device";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	DEVICE_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function withDevice(
	row: Partial<Parameters<typeof deviceRow>[0]> = {},
): AttentionInputExt {
	const input = emptyInput();
	input.devices.push(
		deviceRow({
			device_id: "dev-1",
			owner_id: input.me,
			name: "dev-one",
			status: "active",
			registered_at: SAMPLE_NOW - 86_400,
			last_seen_at: SAMPLE_NOW - 30,
			relationship: "owner",
			...row,
		}),
	);
	return input;
}

describe("presence", () => {
	test("offline > 10 min: Critical when the last known status had a service requested running", () => {
		const input = sampleFleet();
		const [warehouse] = evaluate("offline_since", input);
		expect(warehouse.severity).toBe("critical");
		expect(warehouse.copy.params).toMatchObject({
			device: "warehouse-pi",
			running: 1,
		});
		expect(warehouse.action?.code).toBe("diagnose");
	});

	test("offline without readable services is a Warning; late (2–10 min) is Info", () => {
		expect(
			evaluate(
				"offline_since",
				withDevice({ last_seen_at: SAMPLE_NOW - 601 }),
			)[0]?.severity,
		).toBe("warning");
		expect(
			evaluate("offline_since", withDevice({ last_seen_at: SAMPLE_NOW - 600 })),
		).toEqual([]);
		expect(
			evaluate("late", withDevice({ last_seen_at: SAMPLE_NOW - 300 }))[0]
				?.severity,
		).toBe("info");
		expect(
			evaluate("late", withDevice({ last_seen_at: SAMPLE_NOW - 60 })),
		).toEqual([]);
	});

	test("a device you only approved cloud access for is not yours to watch; shared and older-hub rows are", () => {
		const offline = (relationship?: "shared" | "cloud_approval") =>
			evaluate(
				"offline_since",
				withDevice({
					last_seen_at: SAMPLE_NOW - 601,
					owner_id: "someone",
					relationship,
				}),
			);
		expect(offline("cloud_approval")).toEqual([]);
		expect(offline("shared")).toHaveLength(1);
		expect(offline(undefined)).toHaveLength(1);
		expect(
			evaluate(
				"late",
				withDevice({
					last_seen_at: SAMPLE_NOW - 300,
					owner_id: "someone",
					relationship: "cloud_approval",
				}),
			),
		).toEqual([]);
	});

	test("never checked in: Info before 10 min, Warning after, owners only", () => {
		const fresh = withDevice({
			last_seen_at: null,
			registered_at: SAMPLE_NOW - 300,
		});
		expect(evaluate("no_heartbeat_since_enrollment", fresh)[0]?.severity).toBe(
			"info",
		);
		const old = withDevice({
			last_seen_at: null,
			registered_at: SAMPLE_NOW - 720,
		});
		expect(evaluate("no_heartbeat_since_enrollment", old)[0]?.severity).toBe(
			"warning",
		);
		const shared = withDevice({
			last_seen_at: null,
			relationship: "shared",
			owner_id: "someone",
		});
		expect(evaluate("no_heartbeat_since_enrollment", shared)).toEqual([]);
	});
});

describe("revocation and consent", () => {
	test("revoked is Info, with Delete keys only when keys are here", () => {
		const input = sampleFleet();
		const items = evaluate("revoked", input);
		const kiosk = items.find(
			(item) =>
				item.subject.kind === "device" &&
				item.subject.deviceId === SAMPLE_IDS.oldKiosk,
		);
		const partner = items.find(
			(item) =>
				item.subject.kind === "device" &&
				item.subject.deviceId === SAMPLE_IDS.partner,
		);
		expect(kiosk?.action?.code).toBe("delete_keys");
		expect(partner?.action).toBeUndefined();
		expect(items.every((item) => item.severity === "info")).toBe(true);
	});

	test("BG4 revoked_at dates the item", () => {
		const input = withDevice({
			status: "revoked",
			revoked_at: SAMPLE_NOW - 86_400,
		});
		expect(evaluate("revoked", input)[0].copy.params?.revokedAt).toBe(
			SAMPLE_NOW - 86_400,
		);
	});

	test("you still pay: billing first, then an approval you made; nothing once both end", () => {
		const input = sampleFleet();
		const [pay] = evaluate("you_still_pay_for_a_revoked_device", input);
		expect(pay.copy.params).toMatchObject({
			kind: "billing",
			usedMicros: 12_500_000,
			limitMicros: 50_000_000,
		});
		expect(pay.action?.code).toBe("revoke_spending_limit");
		const partner = input.resources[SAMPLE_IDS.partner];
		if (!partner) throw new Error("fixture");
		partner.billing[0].status = "revoked";
		partner.grants[0].delegating_user_id = input.me;
		const [approval] = evaluate("you_still_pay_for_a_revoked_device", {
			...input,
			resourceSummary: undefined,
		});
		expect(approval.copy.params?.kind).toBe("approval");
		partner.grants[0].status = "revoked";
		expect(
			evaluate("you_still_pay_for_a_revoked_device", {
				...input,
				resourceSummary: undefined,
			}),
		).toEqual([]);
	});
});

describe("integrity and clocks", () => {
	test("identity mismatch and snapshot integrity are Critical", () => {
		const input = withDevice();
		input.keys.push({
			...keySession("dev-1", "blocked"),
			lastError: {
				code: "identity_mismatch",
				pinnedAt: 1,
				fingerprint: "a",
				reported: "b",
			},
		});
		input.fleet["dev-1"] = {
			deviceId: "dev-1",
			error: { kind: "integrity", message: "rolled back", at: SAMPLE_NOW - 60 },
			freshness: sampleFleet().fleet[SAMPLE_IDS.edge].freshness,
		};
		expect(evaluate("identity_mismatch", input)[0]?.severity).toBe("critical");
		expect(evaluate("snapshot_integrity_error", input)[0]?.severity).toBe(
			"critical",
		);
	});

	test("clock skew: this computer and a device (BG6 wins over the heuristic), > 2 min only", () => {
		const input = withDevice({
			auth_rejection: {
				code: "clock_skew",
				skew_seconds: -420,
				count: 3,
				first_at: SAMPLE_NOW - 900,
				last_at: SAMPLE_NOW - 60,
			},
		});
		input.clock = { hubOffsetS: 300, deviceSkewS: { "dev-1": 30 } };
		const items = evaluate("clock_skew", input);
		expect(items.map((item) => item.subject.kind).sort()).toEqual([
			"device",
			"hub",
		]);
		expect(
			items.find((item) => item.subject.kind === "device")?.copy.params
				?.minutes,
		).toBe(7);
		const quiet = withDevice();
		quiet.clock = { hubOffsetS: 120, deviceSkewS: { "dev-1": -120 } };
		expect(evaluate("clock_skew", quiet)).toEqual([]);
	});

	test("access denied needs BG6 revoked_credential", () => {
		expect(evaluate("access_denied", withDevice())).toEqual([]);
		const refused = withDevice({
			auth_rejection: {
				code: "revoked_credential",
				skew_seconds: null,
				count: 4,
				first_at: SAMPLE_NOW - 600,
				last_at: SAMPLE_NOW,
			},
		});
		expect(evaluate("access_denied", refused)[0]?.severity).toBe("critical");
	});
});

describe("status, agent and host", () => {
	test("encrypted status older than 10 min while online and not live is a Notice", () => {
		const input = sampleFleet();
		input.live = {};
		const status = input.fleet[SAMPLE_IDS.edge].status;
		if (!status) throw new Error("fixture");
		status.observedAt = SAMPLE_NOW - 601;
		const items = evaluate("status_stale_while_online", input);
		expect(
			items.map(
				(item) => item.subject.kind === "device" && item.subject.deviceId,
			),
		).toEqual([SAMPLE_IDS.edge]);
		expect(items[0].action?.target).toEqual({
			kind: "connect",
			deviceId: SAMPLE_IDS.edge,
		});
	});

	test("failing background task needs BG10 task health", () => {
		const input = sampleFleet();
		expect(evaluate("background_task_failing", input)).toEqual([]);
		const inspection = input.live[SAMPLE_IDS.edge].inspection;
		if (!inspection) throw new Error("fixture");
		inspection.value.tasks = [
			{
				name: "fleet_publisher",
				state: "failing",
				since: SAMPLE_NOW - 300,
				consecutive_failures: 4,
			},
			{
				name: "archive_publisher",
				state: "stopped",
				since: SAMPLE_NOW - 100,
				consecutive_failures: 0,
			},
		];
		const [item] = evaluate("background_task_failing", input);
		expect(item.copy.params).toMatchObject({
			task: "fleet_publisher",
			more: 1,
		});
	});

	test("one failed pass is not a warning; without a counter a minute of failing is", () => {
		const withTask = (
			task: Partial<TaskHealth> & Pick<TaskHealth, "state" | "since">,
		) => {
			const input = sampleFleet();
			const inspection = input.live[SAMPLE_IDS.edge].inspection;
			if (!inspection) throw new Error("fixture");
			inspection.value.tasks = [{ name: "device_presence", ...task }];
			return evaluate("background_task_failing", input);
		};
		expect(
			withTask({
				state: "failing",
				since: SAMPLE_NOW - 600,
				consecutive_failures: 1,
			}),
		).toEqual([]);
		expect(
			withTask({
				state: "failing",
				since: SAMPLE_NOW - 5,
				consecutive_failures: 2,
			}),
		).toHaveLength(1);
		expect(withTask({ state: "failing", since: SAMPLE_NOW - 59 })).toEqual([]);
		expect(withTask({ state: "failing", since: SAMPLE_NOW - 60 })).toHaveLength(
			1,
		);
		expect(withTask({ state: "stopped", since: SAMPLE_NOW - 1 })).toHaveLength(
			1,
		);
		expect(withTask({ state: "ok", since: SAMPLE_NOW - 600 })).toEqual([]);
	});

	test("agent update: newer release than the running agent; Update agent on Linux, How to update elsewhere", () => {
		const input = sampleFleet();
		const [warehouse] = evaluate("agent_update_available", input);
		expect(warehouse.copy.params).toMatchObject({
			available: "0.9.4",
			running: "0.9.2",
		});
		expect(warehouse.lastKnown).toBe(true);
		expect(warehouse.source.src).toBe("saved");
		expect(warehouse.action?.code).toBe("update_agent");
		input.latestRelease = { version: "0.10.0", sequence: 50 };
		const studio = evaluate("agent_update_available", input).find(
			(item) =>
				item.subject.kind === "device" &&
				item.subject.deviceId === SAMPLE_IDS.studio,
		);
		expect(studio?.action?.code).toBe("how_to_update");
		expect(studio?.lastKnown).toBe(false);
		expect(studio?.copy.params?.running).toBe("0.9.4");
	});

	test("agent update compares release sequences, never the agent's constant crate version", () => {
		const live = (input: ReturnType<typeof sampleFleet>) =>
			evaluate("agent_update_available", input).filter(
				(item) =>
					item.subject.kind === "device" &&
					item.subject.deviceId !== SAMPLE_IDS.warehouse,
			);
		const current = sampleFleet();
		expect(current.live[SAMPLE_IDS.edge].inspection?.value.agent?.version).toBe(
			"0.1.0",
		);
		expect(live(current)).toEqual([]);
		const sameLabel = sampleFleet();
		sameLabel.latestRelease = { version: "0.9.4", sequence: 45 };
		expect(live(sameLabel)).toHaveLength(2);
		const sameSequence = sampleFleet();
		sameSequence.latestRelease = { version: "9.9.9", sequence: 44 };
		expect(live(sameSequence)).toEqual([]);
	});

	test("agent update is not shown to recipients without device-scope Update agent", () => {
		const input = sampleFleet();
		input.agentLastRead = {
			[SAMPLE_IDS.lab]: { version: "0.9.0", at: SAMPLE_NOW - 60 },
		};
		expect(evaluate("agent_update_available", input)).toEqual([]);
		const grant = input.myAccess?.[SAMPLE_IDS.lab]?.grants[0];
		if (!grant) throw new Error("fixture");
		grant.scope = { kind: "device" };
		grant.capabilities = ["update_agent"];
		expect(evaluate("agent_update_available", input)).toHaveLength(1);
	});

	test("a changed boot without a tracked reboot is Info", () => {
		const input = sampleFleet();
		input.fleet[SAMPLE_IDS.edge].previousBootId = "older-boot";
		expect(evaluate("rebooted_unexpectedly", input)).toEqual([]);
		const host = input.live[SAMPLE_IDS.edge].inspection?.value.host;
		if (!host) throw new Error("fixture");
		host.booted_at = SAMPLE_NOW - 300;
		const [item] = evaluate("rebooted_unexpectedly", { ...input });
		expect(item.severity).toBe("info");
		expect(item.copy.params?.bootedAt).toBe(SAMPLE_NOW - 300);
		input.activity.push({
			...input.activity[0],
			id: "reboot-1",
			kind: "reboot",
			resume: {
				type: "host_operation",
				kind: "reboot",
				operationId: "op-1",
				bootIdBefore: "older-boot",
			},
		});
		expect(evaluate("rebooted_unexpectedly", { ...input })).toEqual([]);
	});

	test("status reader expiring within 30 days is a Notice", () => {
		const input = sampleFleet();
		const reader = input.fleet[SAMPLE_IDS.edge].reader;
		if (!reader) throw new Error("fixture");
		reader.expiresAt = SAMPLE_NOW + 29 * 86_400;
		const [item] = evaluate("status_subscription_expiring", input);
		expect(item?.severity).toBe("notice");
		// Renew lives in N2 › Device settings › Encrypted status subscription.
		expect(item?.action).toEqual({
			code: "renew",
			target: { screen: "device", deviceId: SAMPLE_IDS.edge, tab: "settings" },
		});
	});

	test("device slots ≥ 90 % need BG3 usage", () => {
		const input = sampleFleet();
		expect(evaluate("device_slots_nearly_full", input)).toEqual([]);
		if (!input.usage) throw new Error("fixture");
		input.usage.usage.active_devices = 88;
		input.usage.usage.pending_enrollments = 2;
		expect(
			evaluate("device_slots_nearly_full", input)[0]?.copy.params,
		).toMatchObject({ used: 90, max: 100, pending: 2 });
		input.usage = undefined;
		expect(evaluate("device_slots_nearly_full", input)).toEqual([]);
	});

	test("every rule copes with a device that has nothing loaded", () => {
		const input = withDevice({ last_seen_at: null });
		input.local.vaults.push(vault("dev-1"));
		for (const rule of DEVICE_RULES)
			expect(() => rule.evaluate(input)).not.toThrow();
	});
});
