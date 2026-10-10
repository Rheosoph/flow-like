import { beforeEach, describe, expect, mock, test } from "bun:test";
import type { ILogMetadata, IRunLogSummary } from "../lib";
import type { IBackendState } from "./backend-state";
import type { IBoardState } from "./backend-state/board-state";
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

function backendWith(listRuns: IBoardState["listRuns"]): IBackendState {
	return { boardState: { listRuns } } as IBackendState;
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const FILTER = { appId: "app", boardId: "board", limit: 2 };

describe("run history pagination", () => {
	beforeEach(() => {
		useLogAggregation.setState(useLogAggregation.getInitialState());
	});

	test("appends older pages, sorts and deduplicates runs, then stops at the end", async () => {
		const pages = [
			[
				{ ...meta("new"), start: 30 },
				{ ...meta("middle"), start: 20 },
			],
			[
				{ ...meta("middle"), start: 20 },
				{ ...meta("old"), start: 10 },
			],
			[],
		];
		const listRuns = mock<IBoardState["listRuns"]>(
			async () => pages.shift() ?? [],
		);
		const backend = backendWith(listRuns);
		const { setFilter, loadMoreLogs } = useLogAggregation.getState();
		await setFilter(backend, FILTER);
		expect(useLogAggregation.getState().hasMore).toBe(true);
		await loadMoreLogs(backend);
		expect(
			useLogAggregation.getState().currentLogs.map((run) => run.run_id),
		).toEqual(["new", "middle", "old"]);
		expect(listRuns.mock.calls.map((args) => args[7])).toEqual([0, 2]);
		await loadMoreLogs(backend);
		expect(useLogAggregation.getState().hasMore).toBe(false);
		await loadMoreLogs(backend);
		expect(listRuns).toHaveBeenCalledTimes(3);
	});

	test("advances source offsets by page size when desktop merges local and remote pages", async () => {
		const local = { ...meta("shared"), is_remote: false };
		const remote = { ...meta("shared"), is_remote: true };
		const pages = [
			[local, meta("local"), { ...meta("remote"), is_remote: true }],
			[remote],
		];
		const listRuns = mock<IBoardState["listRuns"]>(
			async () => pages.shift() ?? [],
		);
		const backend = backendWith(listRuns);
		await useLogAggregation.getState().setFilter(backend, FILTER);
		await useLogAggregation.getState().loadMoreLogs(backend);
		expect(listRuns.mock.calls.map((args) => args[7])).toEqual([0, 2]);
		expect(useLogAggregation.getState().currentLogs).toHaveLength(3);
		expect(
			useLogAggregation
				.getState()
				.currentLogs.find((run) => run.run_id === "shared"),
		).toBe(local);
		expect(useLogAggregation.getState().hasMore).toBe(false);
	});

	test("allows only one next-page request at a time and updates heatmap counts", async () => {
		const pending = deferred<ILogMetadata[]>();
		const listRuns = mock<IBoardState["listRuns"]>(
			async (
				_appId,
				_boardId,
				_nodeId,
				_from,
				_to,
				_status,
				_lastMeta,
				offset,
			) =>
				offset === 0
					? [{ ...meta("first"), nodes: [["node", 0]] }]
					: pending.promise,
		);
		const backend = backendWith(listRuns);
		useLogAggregation.getState().setHeatmapEnabled(true);
		await useLogAggregation
			.getState()
			.setFilter(backend, { ...FILTER, limit: 1 });
		const loading = useLogAggregation.getState().loadMoreLogs(backend);
		await useLogAggregation.getState().loadMoreLogs(backend);
		expect(listRuns).toHaveBeenCalledTimes(2);
		expect(useLogAggregation.getState().isLoadingMore).toBe(true);
		expect(useLogAggregation.getState().currentLogs).toHaveLength(1);
		pending.resolve([{ ...meta("second"), nodes: [["node", 3]] }]);
		await loading;
		expect(useLogAggregation.getState().heatmap?.nodes.node).toEqual({
			visits: 2,
			errors: 1,
		});
		expect(useLogAggregation.getState().isLoadingMore).toBe(false);
		expect(listRuns.mock.calls[1][9]).toBe(true);
	});

	test("ignores a stale next page after the board or filter changes", async () => {
		const pending = deferred<ILogMetadata[]>();
		const listRuns = mock<IBoardState["listRuns"]>(
			async (
				_appId,
				boardId,
				_nodeId,
				_from,
				_to,
				_status,
				_lastMeta,
				offset,
			) => {
				if (boardId === "other") return [meta("other")];
				return offset === 0 ? [meta("first"), meta("second")] : pending.promise;
			},
		);
		const backend = backendWith(listRuns);
		await useLogAggregation.getState().setFilter(backend, FILTER);
		useLogAggregation.getState().setCurrentMetadata(meta("first"));
		const stale = useLogAggregation.getState().loadMoreLogs(backend);
		await useLogAggregation
			.getState()
			.setFilter(backend, { ...FILTER, boardId: "other" });
		pending.resolve([meta("stale")]);
		await stale;
		expect(
			useLogAggregation.getState().currentLogs.map((run) => run.run_id),
		).toEqual(["other"]);
		expect(useLogAggregation.getState().currentMetadata).toBeUndefined();
		expect(useLogAggregation.getState().hasMore).toBe(false);
		expect(useLogAggregation.getState().isLoadingMore).toBe(false);
	});

	test("ignores a stale first page when another filter finishes earlier", async () => {
		const pending = deferred<ILogMetadata[]>();
		const listRuns = mock<IBoardState["listRuns"]>(
			async (_appId, _boardId, nodeId) =>
				nodeId ? [meta("filtered")] : pending.promise,
		);
		const backend = backendWith(listRuns);
		const stale = useLogAggregation.getState().setFilter(backend, FILTER);
		await useLogAggregation
			.getState()
			.setFilter(backend, { ...FILTER, nodeId: "node" });
		pending.resolve([meta("stale"), meta("also-stale")]);
		await stale;
		expect(
			useLogAggregation.getState().currentLogs.map((run) => run.run_id),
		).toEqual(["filtered"]);
		expect(useLogAggregation.getState().hasMore).toBe(false);
	});

	test("retries a failed next page at the same offset without discarding loaded rows", async () => {
		const listRuns = mock<IBoardState["listRuns"]>()
			.mockResolvedValueOnce([meta("first"), meta("second")])
			.mockRejectedValueOnce(new Error("network unavailable"))
			.mockResolvedValueOnce([meta("third")]);
		const backend = backendWith(listRuns);
		await useLogAggregation.getState().setFilter(backend, FILTER);
		await useLogAggregation.getState().loadMoreLogs(backend);
		expect(useLogAggregation.getState().currentLogs).toHaveLength(2);
		expect(useLogAggregation.getState().loadMoreFailed).toBe(true);
		expect(useLogAggregation.getState().hasMore).toBe(true);
		await useLogAggregation.getState().loadMoreLogs(backend);
		expect(listRuns.mock.calls.map((args) => args[7])).toEqual([0, 2, 2]);
		expect(useLogAggregation.getState().currentLogs).toHaveLength(3);
		expect(useLogAggregation.getState().loadMoreFailed).toBe(false);
	});

	test("refresh restarts pagination and ignores an older in-flight page", async () => {
		const pending = deferred<ILogMetadata[]>();
		const listRuns = mock<IBoardState["listRuns"]>()
			.mockResolvedValueOnce([meta("first"), meta("second")])
			.mockImplementationOnce(() => pending.promise)
			.mockResolvedValueOnce([meta("new"), meta("first")])
			.mockResolvedValueOnce([meta("second")]);
		const backend = backendWith(listRuns);
		await useLogAggregation.getState().setFilter(backend, FILTER);
		const stale = useLogAggregation.getState().loadMoreLogs(backend);
		await useLogAggregation.getState().refetchLogs(backend);
		pending.resolve([meta("old")]);
		await stale;
		await useLogAggregation.getState().loadMoreLogs(backend);
		expect(listRuns.mock.calls.map((args) => args[7])).toEqual([0, 2, 0, 2]);
		expect(
			useLogAggregation.getState().currentLogs.map((run) => run.run_id),
		).toEqual(["new", "first", "second"]);
	});
});
