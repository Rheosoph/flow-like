import { describe, expect, test } from "bun:test";
import type { FormSessionState } from "../contracts";
import { fixture } from "../testing/fixtures";
import { type RescueStep, rescueSteps } from "./focus-rescue";

const SELECTED_TAB: RescueStep = { kind: "selectedTab" };
const HOME = ["run", "run-again"];

const onStage = (state: FormSessionState) => ({
	runs: state.runs,
	view: state.view,
});

describe("rescueSteps", () => {
	test("an element without a hook goes to Run, then Run again", () => {
		expect(rescueSteps(null, onStage(fixture("done")))).toEqual(HOME);
	});

	test("Stop: a run that ended on its own offers Copy first; one the person stopped, Run again", () => {
		expect(rescueSteps("stop", onStage(fixture("done")))).toEqual([
			"stop",
			"copy",
			"run-again",
			...HOME,
		]);
		expect(rescueSteps("stop", onStage(fixture("stopped")))).toEqual([
			"stop",
			"run-again",
			"copy",
			...HOME,
		]);
	});

	test("an element drawn anew finds its hook again first; a run tab falls back to the selected tab", () => {
		const done = onStage(fixture("done"));
		expect(rescueSteps("field:invoice_file", done)).toEqual([
			"field:invoice_file",
			...HOME,
		]);
		expect(rescueSteps("tab:run-12", done)).toEqual([
			"tab:run-12",
			SELECTED_TAB,
			...HOME,
		]);
	});
});
