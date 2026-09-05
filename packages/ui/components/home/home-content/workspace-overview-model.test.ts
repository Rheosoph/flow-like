import { describe, expect, it } from "bun:test";
import type { IExecutionUsageRecord } from "../../../lib/schema/usage/tracking";
import {
	workspaceProfileAppCount,
	workspacePulseHistory,
	workspacePulseMetrics,
	workspacePulseState,
} from "./workspace-overview-model";

const now = Date.parse("2026-09-05T12:00:00Z");
const record = (
	id: string,
	status: string,
	created_at = "2026-09-05T10:00:00Z",
): IExecutionUsageRecord => ({
	id,
	status,
	created_at,
	app_id: "app",
	board_id: "board",
	node_id: "node",
	version: "1",
	instance: null,
	technical_user_id: null,
	microseconds: 100,
});

describe("workspace pulse source truthfulness", () => {
	it("counts only accessible apps saved in the current profile and leaves missing sources unknown", () => {
		expect(
			workspaceProfileAppCount(["a", "b", "b", "hidden"], ["b", "deleted"]),
		).toBe(1);
		expect(workspaceProfileAppCount(["a", "b"], [])).toBe(0);
		expect(workspaceProfileAppCount(undefined, ["a"])).toBeUndefined();
		expect(workspaceProfileAppCount(["a"], undefined)).toBeUndefined();
	});
	it("keeps missing history distinct from a confirmed empty sample", () => {
		expect(workspacePulseHistory(undefined, 7, now)).toBeNull();
		const empty = workspacePulseHistory(
			{ items: [], total: 0, page: 0, page_size: 100 },
			7,
			now,
		);
		expect(empty?.volume).toBe(0);
		expect(empty?.partial).toBe(false);
	});
	it("keeps confirmed zero metrics while withholding disabled or failed cached sources", () => {
		const empty = workspacePulseHistory(
			{ items: [], total: 0, page: 0, page_size: 100 },
			7,
			now,
		);
		expect(workspacePulseMetrics(empty, true, false)?.volume).toBe(0);
		expect(workspacePulseMetrics(empty, false, false)).toBeNull();
		expect(workspacePulseMetrics(empty, true, true)).toBeNull();
		expect(workspacePulseMetrics(null, true, false)).toBeNull();
		const cached = workspacePulseHistory(
			{
				items: [record("cached", "Error")],
				total: 100,
				page: 0,
				page_size: 100,
			},
			7,
			now,
		);
		expect(workspacePulseMetrics(cached, false, false)).toBeNull();
		expect(workspacePulseMetrics(cached, true, true)).toBeNull();
		expect(workspacePulseMetrics(cached, true, false)?.attention).toHaveLength(
			1,
		);
	});
	it("preserves partial coverage when no sampled records fall inside the selected period", () => {
		const outsidePeriod = workspacePulseHistory(
			{
				items: [record("old", "Error", "2026-08-01T12:00:00Z")],
				total: 400,
				page: 0,
				page_size: 100,
			},
			7,
			now,
		);
		expect(outsidePeriod?.volume).toBe(0);
		expect(outsidePeriod?.attention).toEqual([]);
		expect(outsidePeriod?.partial).toBe(true);
		expect(outsidePeriod?.scanned).toBe(1);
		expect(outsidePeriod?.total).toBe(400);
	});
	it("counts unique Error/Fatal records in range without claiming workflow outcomes", () => {
		const data = workspacePulseHistory(
			{
				items: [
					record("a", "Info"),
					record("b", "Error"),
					record("b", "Error"),
					record("c", "fatal"),
					record("d", "Warn"),
					record("old", "Fatal", "2026-08-01T10:00:00Z"),
					record("future", "Error", "2026-09-06T10:00:00Z"),
					record("bad", "Error", "invalid"),
				],
				total: 400,
				page: 0,
				page_size: 100,
			},
			7,
			now,
		);
		expect(data?.volume).toBe(4);
		expect(data?.attention.map((item) => item.id)).toEqual(["b", "c"]);
		expect(data?.partial).toBe(true);
		expect(data?.total).toBe(400);
		expect(data?.invalidDates).toBe(1);
		expect(data?.buckets.at(-1)).toEqual({
			day: "2026-09-05",
			count: 4,
			attentionCount: 2,
		});
	});
	it("never exposes cached account activity for a guest or unsupported backend", () => {
		const state = {
			authenticated: true,
			supported: true,
			loading: false,
			error: false,
			volume: 500,
		};
		expect(workspacePulseState(state)).toBe("activity");
		expect(workspacePulseState({ ...state, authenticated: false })).toBe(
			"starter",
		);
		expect(workspacePulseState({ ...state, supported: false })).toBe("starter");
		expect(workspacePulseState({ ...state, error: true })).toBe("unavailable");
		expect(workspacePulseState({ ...state, volume: 0 })).toBe("starter");
		expect(
			workspacePulseState({ ...state, loading: true, volume: undefined }),
		).toBe("loading");
	});
});
