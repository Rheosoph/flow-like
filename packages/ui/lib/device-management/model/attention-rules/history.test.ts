import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { HISTORY_RULES } from "./history";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	HISTORY_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function edgeHistory(input: AttentionInputExt) {
	const history = input.live[SAMPLE_IDS.edge].history;
	if (!history) throw new Error("fixture: edge history");
	return history;
}

describe("retained history", () => {
	test("an expired roster stops recording (Warning, owner, per scope and kind)", () => {
		const [item] = evaluate("history_paused_readers_expired", sampleFleet());
		expect(item.severity).toBe("warning");
		expect(item.copy.params).toMatchObject({
			kind: "metrics",
			scope: "device",
			since: 1_790_683_200,
		});
		expect(item.id).toBe(
			`history_paused_readers_expired:${SAMPLE_IDS.edge}/me:device:metrics`,
		);
	});

	test("BG30 recording status overrides the expiry heuristic", () => {
		const input = sampleFleet();
		const [logs, metrics] = edgeHistory(input);
		metrics.status = {
			state: "recording",
			reason: null,
			since: SAMPLE_NOW - 60,
		};
		logs.status = {
			state: "paused",
			reason: "rules_changed",
			since: SAMPLE_NOW - 120,
		};
		expect(evaluate("history_paused_readers_expired", input)).toEqual([]);
		const [changed] = evaluate("history_paused_access_changed", input);
		expect(changed.copy.params).toMatchObject({
			kind: "logs",
			since: SAMPLE_NOW - 120,
		});
	});

	test("older agents: a roster approved against older rules than the device applied", () => {
		const input = sampleFleet();
		expect(evaluate("history_paused_access_changed", input)).toEqual([]);
		edgeHistory(input)[0].policyVersion = 4;
		expect(evaluate("history_paused_access_changed", input)).toHaveLength(1);
	});

	test("recipients don't see the owner's history items", () => {
		const input = sampleFleet();
		const edge = input.devices.find((row) => row.device_id === SAMPLE_IDS.edge);
		if (!edge) throw new Error("fixture");
		edge.relationship = "shared";
		expect(evaluate("history_paused_readers_expired", input)).toEqual([]);
	});
});

describe("plan and storage (BG30 archive usage)", () => {
	test("a plan without history is Info; ≥ 90 % used is a Notice", () => {
		const input = sampleFleet();
		expect(evaluate("history_not_stored_by_plan", input)).toEqual([]);
		expect(evaluate("history_storage_nearly_full", input)).toEqual([]);
		if (!input.archiveUsage) throw new Error("fixture");
		input.archiveUsage.used_bytes = input.archiveUsage.max_bytes * 0.95;
		expect(evaluate("history_storage_nearly_full", input)[0]?.severity).toBe(
			"notice",
		);
		input.archiveUsage = {
			...input.archiveUsage,
			tier: "FREE",
			max_bytes: 0,
			used_bytes: 0,
		};
		expect(evaluate("history_not_stored_by_plan", input)[0]?.severity).toBe(
			"info",
		);
		input.archiveUsage = undefined;
		expect(evaluate("history_not_stored_by_plan", input)).toEqual([]);
	});
});

describe("shared metrics readers", () => {
	test("readers ending within 2 h are a Notice", () => {
		const input = sampleFleet();
		input.live[SAMPLE_IDS.edge].metricReaders = [
			{ scope: "device", expiresAt: SAMPLE_NOW + 3_600 },
			{ scope: "app_invoice_ai", expiresAt: SAMPLE_NOW + 3 * 3_600 },
		];
		const items = evaluate("metric_readers_expiring", input);
		expect(items.map((item) => item.copy.params?.scope)).toEqual(["device"]);
		expect(items[0].action?.target).toMatchObject({ tab: "metrics" });
	});
});
