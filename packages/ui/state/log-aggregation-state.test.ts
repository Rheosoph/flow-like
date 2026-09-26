import { beforeEach, describe, expect, test } from "bun:test";
import type { ILogMetadata, IRunLogSummary } from "../lib";
import { nodeLogCounts, useLogAggregation } from "./log-aggregation-state";

function meta(runId: string): ILogMetadata {
	return {
		app_id: "app",
		board_id: "board",
		run_id: runId,
		start: 0,
		end: 0,
		event_id: "event",
		log_level: 0,
		node_id: "event",
		payload: [],
		version: "0.0.1",
	};
}

const SUMMARY: IRunLogSummary = {
	version: 1,
	fingerprinted: true,
	partial: false,
	total: 1_974,
	levels: [987, 0, 1, 986, 0],
	nodes: { upsert: [0, 0, 0, 986, 0], setf: [986, 0, 1, 0, 0] },
	groups: [],
	groups_truncated: false,
};

describe("current run summary", () => {
	beforeEach(() => {
		useLogAggregation.setState({
			currentMetadata: undefined,
			currentSummary: undefined,
			currentSummaryRunId: undefined,
		});
	});

	test("counts per node: problems, warnings and the total", () => {
		const { setCurrentMetadata, setCurrentSummary } =
			useLogAggregation.getState();
		setCurrentMetadata(meta("run_a"));
		setCurrentSummary("run_a", SUMMARY);
		const state = useLogAggregation.getState();
		expect(nodeLogCounts(state, "upsert")).toEqual({
			total: 986,
			problems: 986,
			warnings: 0,
		});
		expect(nodeLogCounts(state, "setf")).toEqual({
			total: 987,
			problems: 0,
			warnings: 1,
		});
		expect(nodeLogCounts(state, "missing")).toBeUndefined();
	});

	test("a summary for another run is ignored and a run change drops it", () => {
		const { setCurrentMetadata, setCurrentSummary } =
			useLogAggregation.getState();
		setCurrentMetadata(meta("run_a"));
		setCurrentSummary("run_b", SUMMARY);
		expect(useLogAggregation.getState().currentSummary).toBeUndefined();

		setCurrentSummary("run_a", SUMMARY);
		setCurrentMetadata(meta("run_a"));
		expect(useLogAggregation.getState().currentSummary).toBe(SUMMARY);

		setCurrentMetadata(meta("run_b"));
		expect(useLogAggregation.getState().currentSummary).toBeUndefined();
		expect(
			nodeLogCounts(useLogAggregation.getState(), "upsert"),
		).toBeUndefined();
	});
});
