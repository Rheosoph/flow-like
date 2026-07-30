import { describe, expect, it } from "bun:test";
import {
	type FlowPilotOrbState,
	ORB_STATE_PARAMS,
	classifyOrbTool,
	selectActiveOrbTool,
} from "./flowpilot-orb-state";

describe("classifyOrbTool", () => {
	it("treats reading, searching and reasoning as thinking", () => {
		for (const tool of [
			"think",
			"analyze",
			"catalog_search",
			"internet_search",
			"open_url",
			"archive_lookup",
			"get_current_flowscript",
			"get_declarations",
			"get_node_details",
			"list_board_nodes",
			"get_unconfigured_nodes",
			"ui_inspect",
			"query_execution_logs",
			"database_tool",
			"storage_tool",
		]) {
			expect(classifyOrbTool(tool)).toBe("thinking");
		}
	});

	it("treats writing, emitting and executing as working", () => {
		for (const tool of [
			"write_flowscript",
			"patch_flowscript",
			"edit_flowscript",
			"check_flowscript",
			"commit_flowscript",
			"emit_commands",
			"emit_surface",
			"emit_ui",
			"modify_component",
			"execute_event",
			"execute_node",
		]) {
			expect(classifyOrbTool(tool)).toBe("working");
		}
	});

	it("hands back to the user when a tool asks a question", () => {
		expect(classifyOrbTool("ask_user")).toBe("ready");
	});

	it("defaults an unknown or missing tool to thinking, never to working", () => {
		// A newly added read-only tool must not claim to be applying changes.
		expect(classifyOrbTool("some_future_tool")).toBe("thinking");
		expect(classifyOrbTool(undefined)).toBe("thinking");
		expect(classifyOrbTool("")).toBe("thinking");
	});

	it("is case-insensitive", () => {
		expect(classifyOrbTool("EMIT_COMMANDS")).toBe("working");
		expect(classifyOrbTool("Ask_User")).toBe("ready");
	});
});

describe("selectActiveOrbTool", () => {
	it("follows the newest live message and its latest in-progress step", () => {
		expect(
			selectActiveOrbTool([
				{
					plan_steps: [
						{
							id: "older-working",
							title: "Editing",
							status: "progress",
							toolName: "edit_flowscript",
						},
					],
				},
				{
					plan_steps: [
						{
							id: "newer-finished",
							title: "Searching",
							status: "completed",
							toolName: "catalog_search",
						},
						{
							id: "newer-current",
							title: "Reading",
							status: "progress",
							toolName: "open_url",
						},
					],
				},
			]),
		).toBe("open_url");
	});

	it("does not leak an older run's tool into a newer composing run", () => {
		expect(
			selectActiveOrbTool([
				{
					plan_steps: [
						{
							id: "older-working",
							title: "Editing",
							status: "progress",
							toolName: "edit_flowscript",
						},
					],
				},
				{ plan_steps: [] },
			]),
		).toBeUndefined();
	});
});

describe("ORB_STATE_PARAMS", () => {
	const states: FlowPilotOrbState[] = ["idle", "ready", "thinking", "working"];

	it("gives every state the same numeric keys, so the lerp can never produce NaN", () => {
		const reference = Object.keys(ORB_STATE_PARAMS.idle).sort();
		for (const state of states) {
			expect(Object.keys(ORB_STATE_PARAMS[state]).sort()).toEqual(reference);
			for (const value of Object.values(ORB_STATE_PARAMS[state])) {
				expect(Number.isFinite(value)).toBe(true);
			}
		}
	});

	it("keeps the states visually distinct in the ways the shader reads", () => {
		// thinking is the only round-with-satellites state; working is the only cog.
		expect(ORB_STATE_PARAMS.thinking.sat).toBeGreaterThan(0.5);
		expect(ORB_STATE_PARAMS.working.sat).toBe(0);
		expect(ORB_STATE_PARAMS.working.teeth).toBeGreaterThan(0.5);
		expect(ORB_STATE_PARAMS.thinking.teeth).toBe(0);
		// idle must be visibly calmer than every working state.
		for (const state of ["ready", "thinking", "working"] as const) {
			expect(ORB_STATE_PARAMS[state].rate).toBeGreaterThan(
				ORB_STATE_PARAMS.idle.rate,
			);
		}
		// satellites orbit at 0.63 +/- 0.05 and must clear the largest bubble the shader draws.
		const maxBubble = 0.48 * ORB_STATE_PARAMS.thinking.scale;
		expect(0.63 - 0.05 - 0.046 * 1.32).toBeGreaterThan(maxBubble);
	});
});
