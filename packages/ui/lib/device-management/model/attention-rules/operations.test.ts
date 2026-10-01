import { describe, expect, test } from "bun:test";
import type { ActivityItem } from "../../workspace/types";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import {
	type AttentionInputExt,
	computeAttention,
	createAttentionMemory,
} from "../attention";
import type { AttentionKey } from "../types";
import { OPERATION_RULES } from "./operations";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	OPERATION_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

/** `startedS` is unix seconds; the tray stores milliseconds. */
function activity(
	row: Pick<ActivityItem, "kind" | "state"> & { startedS: number } & Partial<
			Omit<ActivityItem, "startedAt" | "updatedAt">
		>,
): ActivityItem {
	const { startedS, ...rest } = row;
	return {
		id: `op-${row.kind}-${startedS}`,
		target: { deviceId: SAMPLE_IDS.edge, deviceName: "edge-berlin-01" },
		label: { code: row.kind },
		startedAt: startedS * 1_000,
		updatedAt: startedS * 1_000,
		startedBy: "you",
		actions: [],
		...rest,
	};
}

describe("tracked operations", () => {
	test("a command without a reply within 24 h is a Warning", () => {
		const input = sampleFleet();
		input.activity.push(
			activity({
				kind: "command",
				state: "unknown",
				startedS: SAMPLE_NOW - 600,
				target: { deviceId: SAMPLE_IDS.edge, serviceId: "support-bot" },
				resume: {
					type: "operation",
					operationId: "op",
					command: "restart",
					issuedAt: SAMPLE_NOW - 600,
				},
			}),
			activity({
				kind: "command",
				state: "unknown",
				startedS: SAMPLE_NOW - 2 * 86_400,
			}),
			activity({
				kind: "command",
				state: "unknown",
				startedS: SAMPLE_NOW - 60,
				target: { deviceId: SAMPLE_IDS.edge, serviceId: "nightly-sync" },
				resume: {
					type: "operation",
					operationId: "old",
					command: "stop",
					issuedAt: SAMPLE_NOW - 86_401,
				},
			}),
		);
		const items = evaluate("unconfirmed_command", input);
		expect(items).toHaveLength(1);
		expect(items[0].subject).toMatchObject({
			kind: "service",
			serviceId: "support-bot",
		});
		expect(items[0].action?.code).toBe("check_result");
		const memory = createAttentionMemory("test", undefined);
		const shown = computeAttention(input, memory).find(
			(item) => item.key === "unconfirmed_command",
		);
		expect(shown?.firstSeenAt).toBe(SAMPLE_NOW - 600);
	});

	test("failed and unconfirmed host operations, from the agent (BG15) or the tray", () => {
		const input = sampleFleet();
		expect(evaluate("device_operation_failed", input)).toEqual([]);
		const inspection = input.live[SAMPLE_IDS.edge].inspection;
		if (!inspection?.value.hostOperation) throw new Error("fixture");
		inspection.value.hostOperation.state = "rolled_back";
		expect(
			evaluate("device_operation_failed", input)[0]?.copy.params,
		).toMatchObject({ operation: "update_agent" });
		inspection.value.hostOperation.state = "unknown";
		expect(evaluate("device_operation_unknown", input)).toHaveLength(1);
		input.activity.push(
			activity({
				kind: "reboot",
				state: "failed",
				startedS: SAMPLE_NOW - 300,
			}),
		);
		const failed = evaluate("device_operation_failed", input);
		expect(failed.map((item) => item.copy.params?.operation)).toEqual([
			"reboot",
		]);
	});

	test("a paused upload is Info until its resume window closes; dismissed items stay quiet", () => {
		const input = sampleFleet();
		const [upload] = evaluate("upload_paused", input);
		expect(upload).toMatchObject({
			severity: "info",
			action: { code: "resume" },
		});
		expect(upload.copy.params).toMatchObject({
			app: "app_crm_sync",
			until: 1_790_841_600,
		});
		expect(
			evaluate("upload_paused", { ...input, now: 1_790_841_599 }),
		).toHaveLength(1);
		expect(evaluate("upload_paused", { ...input, now: 1_790_841_600 })).toEqual(
			[],
		);
		const noHandle = sampleFleet();
		noHandle.activity = noHandle.activity.map((item) =>
			item.kind === "upload" ? { ...item, resume: undefined } : item,
		);
		expect(
			evaluate("upload_paused", noHandle)[0]?.copy.params?.until,
		).toBe(1_790_841_600);
		input.activity = input.activity.map((item) => ({
			...item,
			dismissed: true,
		}));
		expect(evaluate("upload_paused", input)).toEqual([]);
	});
});
