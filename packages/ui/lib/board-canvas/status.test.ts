import { describe, expect, test } from "bun:test";
import type { IQualityMark } from "../board-quality";
import type { ILogMetadata } from "../schema/flow/log-metadata";
import type { IRunLogSummary } from "../schema/flow/log-query";
import type { FlowSceneNode } from "./scene";
import { collectNodeStatus, collectRunActivity } from "./status";

function node(id: string, data: Record<string, unknown> = {}): FlowSceneNode {
	return {
		id,
		type: "node",
		position: { x: 0, y: 0 },
		data: { node: { id, name: id, pins: {} }, boardId: "board", ...data },
	};
}

const metadata = (nodes: [string, number][]) =>
	({ run_id: "run", nodes }) as unknown as ILogMetadata;
const summary = (nodes: Record<string, number[]>) =>
	({ nodes }) as unknown as IRunLogSummary;
const mark = (severity: IQualityMark["severity"], total: number) =>
	({
		severity,
		counts: { error: total, warning: 0, info: 0 },
	}) as unknown as IQualityMark;

describe("collectNodeStatus", () => {
	test("run state comes from the live run before a completed one", () => {
		const runs = new Map([
			[
				"old",
				{ nodes: new Set<string>(), already_executed: new Set(["a", "b"]) },
			],
			["live", { nodes: new Set(["a"]), already_executed: new Set<string>() }],
		]);
		const status = collectNodeStatus([node("a"), node("b"), node("idle")], {
			runs,
		});
		expect(status).toEqual({
			a: { run: "running", runId: "live" },
			b: { run: "done", runId: "old" },
		});
	});

	test("log badges count the current run's summary and mark nodes that ran", () => {
		const sources = {
			currentMetadata: metadata([
				["errors", 3],
				["warned", 2],
				["quiet", 1],
				["unsummarized", 4],
			]),
			currentSummaryRunId: "run",
			currentSummary: summary({
				errors: [3, 0, 1, 2, 0],
				warned: [1200, 0, 4, 0, 0],
				quiet: [0, 2, 0, 0, 0],
			}),
		};
		const status = collectNodeStatus(
			["errors", "warned", "quiet", "unsummarized"].map((id) => node(id)),
			sources,
		);
		expect(status.errors).toMatchObject({
			logs: { count: "2", problems: true, warnings: false },
			severity: "error",
		});
		expect(status.warned).toMatchObject({
			logs: {
				count: new Intl.NumberFormat(undefined, {
					notation: "compact",
					maximumFractionDigits: 1,
				}).format(1204),
				problems: false,
				warnings: true,
			},
			severity: "warning",
		});
		expect(status.quiet).toEqual({
			logs: { count: "2", problems: false, warnings: false },
		});
		expect(status.unsummarized).toEqual({ logs: {}, severity: "fatal" });

		const stale = collectNodeStatus([node("errors")], {
			...sources,
			currentSummaryRunId: "older",
		});
		expect(stale.errors).toEqual({ logs: {}, severity: "error" });
	});

	test("heat, lint marks and collaborator runs follow their toggles", () => {
		const nodes = [
			node("hot"),
			node("cold"),
			node("peer", { remoteExecuting: true }),
		];
		const sources = {
			heatmapEnabled: true,
			heatmap: {
				maxVisits: 4,
				runCount: 4,
				nodes: {
					hot: { visits: 4, errors: 1 },
					cold: { visits: 1, errors: 0 },
				},
			},
			qualityInline: true,
			qualityMarks: {
				board: { hot: mark("warning", 3), cold: mark("info", 1) },
			},
		};
		const status = collectNodeStatus(nodes, sources);
		expect(status.hot).toEqual({
			heat: { intensity: 1, visits: "4×", errors: "1!" },
			quality: { severity: "warning", count: "3" },
		});
		expect(status.cold).toEqual({
			heat: { intensity: 0.25, visits: "1×", errors: undefined },
			quality: { severity: "info", count: undefined },
		});
		expect(status.peer).toEqual({ remote: true });
		expect(
			collectNodeStatus(nodes, {
				...sources,
				heatmapEnabled: false,
				qualityInline: false,
			}),
		).toEqual({ peer: { remote: true } });
	});
});

describe("collectRunActivity", () => {
	test("formats each run's last activity and grades how fresh it is", () => {
		const activity = collectRunActivity(
			new Map([
				["fresh", { lastNodeUpdateMs: 95_000 }],
				["slow", { lastNodeUpdateMs: 55_000 }],
				["stuck", { lastNodeUpdateMs: 0 }],
				["future", { lastNodeUpdateMs: 120_000 }],
			]),
			100_000,
		);
		expect(activity.fresh.tone).toBe("active");
		expect(activity.slow.tone).toBe("stale");
		expect(activity.stuck.tone).toBe("warning");
		expect(activity.future.tone).toBe("active");
		expect(activity.fresh.text).toMatch(/5/);
		expect(activity.stuck.text).toMatch(/1/);
	});
});
