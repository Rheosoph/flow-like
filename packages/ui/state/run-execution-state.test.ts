import { beforeEach, describe, expect, test } from "bun:test";
import { useRunExecutionStore } from "./run-execution-state";

describe("useRunExecutionStore.pushUpdate", () => {
	beforeEach(() => useRunExecutionStore.setState({ runs: new Map() }));

	test("applies a mixed batch in one store write", async () => {
		const store = useRunExecutionStore.getState();
		await store.addRun("run", "board", ["entry"]);
		let writes = 0;
		const unsubscribe = useRunExecutionStore.subscribe(() => {
			writes += 1;
		});

		store.pushUpdate("run", [
			{ runId: "run", nodeIds: ["a", "b"], method: "add" },
			{ runId: "run", nodeIds: ["a"], method: "remove" },
			{ runId: "run", nodeIds: ["c"], method: "add" },
			{ runId: "unknown", nodeIds: ["x"], method: "add" },
		]);
		unsubscribe();

		const run = useRunExecutionStore.getState().runs.get("run");
		expect(writes).toBe(1);
		expect([...(run?.nodes ?? [])]).toEqual(["b", "c"]);
		expect([...(run?.already_executed ?? [])]).toEqual(["a"]);
		expect(run?.totalExecutionsStarted).toBe(3);
		expect(run?.totalExecutionsCompleted).toBe(1);
		expect(run?.lastActiveNodeId).toBe("c");
		expect(useRunExecutionStore.getState().runs.has("unknown")).toBe(false);
	});

	test("a batch for no known run leaves the store untouched", () => {
		let writes = 0;
		const unsubscribe = useRunExecutionStore.subscribe(() => {
			writes += 1;
		});
		useRunExecutionStore
			.getState()
			.pushUpdate("gone", [{ runId: "gone", nodeIds: ["a"], method: "add" }]);
		unsubscribe();
		expect(writes).toBe(0);
	});
});
