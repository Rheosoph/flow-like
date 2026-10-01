import { describe, expect, test } from "bun:test";
import type { OfflineQueueStatus } from "../../offline-queue";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { OFFLINE_WRITE_RULES, queueResource } from "./offline-writes";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	OFFLINE_WRITE_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function studioQueues(input: AttentionInputExt): OfflineQueueStatus[] {
	const queues = input.live[SAMPLE_IDS.studio].offlineQueues?.["field-notes"];
	if (!queues) throw new Error("fixture: studio queues");
	return queues;
}

describe("queue heads (live)", () => {
	test("conflict names the resource and the changes behind it", () => {
		const [item] = evaluate("offline_writes_conflict", sampleFleet());
		expect(item.severity).toBe("warning");
		expect(item.copy.params).toMatchObject({
			resource: "notes",
			resourceKind: "table",
			behind: 13,
		});
		expect(item.action?.target).toMatchObject({
			screen: "service",
			tab: "offline",
		});
		expect(item.lastKnown).toBe(false);
	});

	test("a quarantined queue reports only quarantine, not its blocked head", () => {
		const input = sampleFleet();
		expect(evaluate("offline_writes_blocked", input)).toEqual([]);
		const [quarantined] = evaluate("offline_writes_quarantined", input);
		expect(quarantined.copy.params).toMatchObject({
			count: 3,
			oldestAt: 1_789_992_000,
		});
		expect(quarantined.action?.code).toBe("fix_cloud_access");
		studioQueues(input)[1].quarantined = false;
		const [blocked] = evaluate("offline_writes_blocked", { ...input });
		expect(blocked.copy.params).toMatchObject({
			resourceKind: "file",
			resource: "exports/2026-09-21-summary.pdf",
		});
	});

	test("outcome unknown and mirror errors", () => {
		const input = sampleFleet();
		const [first] = studioQueues(input);
		if (!first.head) throw new Error("fixture");
		first.head.state = "outcome_unknown";
		first.mirror_error = "Mirror refresh failed: 503";
		expect(evaluate("offline_writes_outcome_unknown", input)).toHaveLength(1);
		expect(evaluate("offline_writes_conflict", input)).toEqual([]);
		expect(evaluate("offline_mirror_error", input)[0]?.severity).toBe("notice");
	});

	test("backlog: oldest over half the max age, or over 80 % of the byte budget", () => {
		const input = sampleFleet();
		expect(evaluate("offline_writes_backlog", input)).toEqual([]);
		studioQueues(input)[0].oldest_at = SAMPLE_NOW - 4 * 86_400;
		expect(evaluate("offline_writes_backlog", { ...input })).toHaveLength(1);
		studioQueues(input)[0].oldest_at = SAMPLE_NOW;
		studioQueues(input)[0].pending_bytes = 220_000_000;
		expect(evaluate("offline_writes_backlog", { ...input })).toHaveLength(1);
		input.live[SAMPLE_IDS.studio].placements = undefined;
		expect(evaluate("offline_writes_backlog", { ...input })).toEqual([]);
	});

	test("each queue is its own root cause", () => {
		const input = sampleFleet();
		const [first] = studioQueues(input);
		studioQueues(input).push({ ...first, scope: "a".repeat(64) });
		const ids = evaluate("offline_writes_conflict", input).map(
			(item) => item.id,
		);
		expect(new Set(ids).size).toBe(2);
	});
});

describe("BG11 summaries and parsing", () => {
	test("without live queues a summary still reports quarantine", () => {
		const input = sampleFleet();
		const live = input.live[SAMPLE_IDS.studio];
		live.offlineQueues = undefined;
		const inspection = live.inspection;
		if (!inspection) throw new Error("fixture");
		inspection.value.placements[0].offline_writes = {
			scopes: 2,
			pending_count: 17,
			pending_bytes: 3_200_000,
			oldest_at: 1_789_992_000,
			quarantined_scopes: 1,
			needs_attention: 2,
			mirror_error: false,
		};
		const [item] = evaluate("offline_writes_quarantined", input);
		expect(item.copy.params).toMatchObject({ count: 17 });
		expect(evaluate("offline_writes_conflict", input)).toEqual([]);
	});

	test("resource descriptors", () => {
		expect(queueResource('{"kind":"table","table":"notes"}')).toEqual({
			kind: "table",
			name: "notes",
		});
		expect(queueResource('{"kind":"file","path":"a.pdf"}')).toEqual({
			kind: "file",
			name: "a.pdf",
		});
		expect(queueResource("not json")).toEqual({
			kind: "resource",
			name: "not json",
		});
	});
});
