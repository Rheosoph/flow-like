import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	emptyInput,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { SETUP_RULES } from "./setup";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	SETUP_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function failingReadiness(input: AttentionInputExt) {
	if (!input.readiness) throw new Error("fixture: readiness");
	input.readiness = {
		...input.readiness,
		ready: false,
		checks: input.readiness.checks.map((check) =>
			check.id === "signaling" ? { ...check, ready: false } : check,
		),
	};
	return input;
}

describe("pending setups (BG2 or the local record)", () => {
	test("waiting is Info and resumes at step 6; expired is a Notice", () => {
		const input = sampleFleet();
		const [waiting] = evaluate("pending_setup_waiting", input);
		expect(waiting.severity).toBe("info");
		expect(waiting.action?.target).toMatchObject({ screen: "setup", step: 6 });
		expect(waiting.source.src).toBe("local");
		const [expired] = evaluate("pending_setup_expired", input);
		expect(expired.severity).toBe("notice");
		expect(expired.copy.params).toMatchObject({
			name: "test-vm",
			expiredAt: 1_790_661_600,
		});
	});

	test("hub-tracked setups stamp the hub; cancelled ones say nothing", () => {
		const input = emptyInput();
		input.pendingSetups = [
			{
				enrollmentId: "e1",
				name: "a",
				state: "expired",
				expiresAt: SAMPLE_NOW - 1,
				local: false,
			},
			{
				enrollmentId: "e2",
				name: "b",
				state: "cancelled",
				expiresAt: SAMPLE_NOW + 60,
				local: false,
			},
		];
		const items = [
			...evaluate("pending_setup_expired", input),
			...evaluate("pending_setup_waiting", input),
		];
		expect(items).toHaveLength(1);
		expect(items[0].source.src).toBe("hub");
	});

	test("no pending setups loaded: nothing", () => {
		const input = emptyInput();
		expect(evaluate("pending_setup_waiting", input)).toEqual([]);
		expect(evaluate("pending_setup_expired", input)).toEqual([]);
	});
});

describe("hub", () => {
	test("hub not ready is a Warning for people who set up devices", () => {
		const [item] = evaluate("hub_not_ready", failingReadiness(sampleFleet()));
		expect(item.severity).toBe("warning");
		expect(item.copy.params).toEqual({ check: "signaling" });
		expect(item.action?.target).toEqual({ screen: "hub" });
	});

	test("a recipient-only viewer doesn't see setup readiness", () => {
		const input = failingReadiness(sampleFleet());
		input.devices = input.devices.filter(
			(row) => row.device_id === SAMPLE_IDS.lab,
		);
		input.pendingSetups = [];
		expect(evaluate("hub_not_ready", input)).toEqual([]);
	});

	test("release trust missing only matters to owners of Linux devices", () => {
		const input = sampleFleet();
		expect(evaluate("release_trust_missing", input)).toEqual([]);
		input.releaseTrust = null;
		expect(evaluate("release_trust_missing", { ...input })).toHaveLength(1);
		const inspection = input.live[SAMPLE_IDS.edge].inspection;
		if (!inspection?.value.isolation) throw new Error("fixture");
		inspection.value.isolation.platform = "macos";
		expect(evaluate("release_trust_missing", { ...input })).toEqual([]);
	});

	test("readiness not loaded: nothing", () => {
		const input = sampleFleet();
		input.readiness = undefined;
		input.releaseTrust = undefined;
		expect(evaluate("hub_not_ready", input)).toEqual([]);
		expect(evaluate("release_trust_missing", input)).toEqual([]);
	});
});
