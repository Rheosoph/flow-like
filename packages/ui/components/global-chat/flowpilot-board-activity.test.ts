import { expect, test } from "bun:test";
import { FlowPilotBoardActivity } from "./flowpilot-board-activity";

test("commit completion preserves the queued workspace even when stream frames arrive first", () => {
	const activity = new FlowPilotBoardActivity();
	const run = activity.begin("app", "board", "request", { mode: "edit" });
	run.tool("commit_flowscript", "running");
	expect(activity.snapshot("app", "board")?.commit_state).toBe("running");
	run.update({ workspace_status: "queued", commit_state: "queued" });
	run.tool("commit_flowscript", "returned");
	expect(activity.snapshot("app", "board")).toMatchObject({
		commit_state: "queued",
		last_tool_status: "returned",
	});
	run.tool("", "running");
	expect(activity.snapshot("app", "board")?.last_tool).toBe(
		"commit_flowscript",
	);
	run.finish();
});

test("late commit tool frames preserve confirmed queue and apply progress", () => {
	const activity = new FlowPilotBoardActivity();
	const run = activity.begin("app", "board", "request", { mode: "edit" });
	for (const commitState of ["queued", "applying", "applied"] as const) {
		run.update({ commit_state: commitState });
		// A heartbeat or delayed tool-end frame carries no newer commit receipt.
		run.tool("commit_flowscript", "running");
		expect(activity.snapshot("app", "board")?.commit_state).toBe(commitState);
		run.tool("commit_flowscript", "returned");
		expect(activity.snapshot("app", "board")).toMatchObject({
			commit_state: commitState,
			last_tool_status: "returned",
		});
	}
	run.finish();
});

test("progress stays scoped to each active board and is removed only by its owner", () => {
	let now = 100;
	const activity = new FlowPilotBoardActivity(() => now);
	const old = activity.begin("app", "board", "old", { mode: "edit" });
	const sibling = activity.begin("app", "sibling", "sibling", {
		mode: "explain",
	});
	now = 200;
	const current = activity.begin("app", "board", "current", { mode: "edit" });
	old.update({ draft_id: "stale" });
	old.finish();
	now = 300;
	current.update({
		draft_id: "retained",
		revision: 15,
		diagnostic_count: 0,
		workspace_status: "submitted",
		last_tool: "check_flowscript",
		last_tool_status: "returned",
	});
	expect(activity.snapshot("app", "board")).toMatchObject({
		request_id: "current",
		started_at_ms: 200,
		last_activity_at_ms: 300,
		draft_id: "retained",
		revision: 15,
		commit_state: "not_observed",
	});
	expect(activity.snapshot("app", "sibling")).toMatchObject({
		request_id: "sibling",
		last_activity_at_ms: 100,
	});
	expect(activity.snapshot("other", "board")).toBeUndefined();
	const copy = activity.snapshot("app", "board");
	if (copy) copy.revision = 999;
	expect(activity.snapshot("app", "board")?.revision).toBe(15);
	current.finish();
	expect(activity.snapshot("app", "board")).toBeUndefined();
	expect(activity.snapshot("app", "sibling")).toBeDefined();
	sibling.finish();
});
