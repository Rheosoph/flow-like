import { describe, expect, test } from "bun:test";
import type { FormSessionState, RunEntry } from "../contracts";
import { formatTypedNumber } from "../run/format";
import { stageRunOf } from "../run/run-view";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";
import { copyKindOf, paneViewOf, railDiffersFrom } from "./pane-model";
import { secretCheckOf } from "./stage-text";
import { shortWords } from "./test-words";

const number = (text: string) => formatTypedNumber(text, "en-GB", ".");

function paneOf(
	name: FixtureName,
	options: {
		layout?: typeof DESKTOP_LAYOUT;
		run?: (state: FormSessionState) => RunEntry | null;
		comparing?: boolean;
	} = {},
) {
	const state = fixture(name);
	const run = options.run?.(state) ?? stageRunOf(state);
	if (!run) throw new Error(`no run on the stage of ${name}`);
	const layout = options.layout ?? DESKTOP_LAYOUT;
	return {
		state,
		run,
		pane: paneViewOf(run, {
			state,
			layout,
			now: fixtureClock(name).now,
			words: shortWords(),
			isSecret: secretCheckOf(state.memory.prefs.noSave),
			number,
			comparing: options.comparing ?? false,
			canPin: layout.compare && state.runs.length > 1,
		}),
	};
}

describe("paneViewOf: sections", () => {
	test("done: Steps, Answer, Result, Files, Inputs, and the links", () => {
		const { pane } = paneOf("done");
		expect(pane.sections).toEqual([
			"steps",
			"answer",
			"result",
			"files",
			"inputs",
		]);
		expect(pane.showLinks).toBe(true);
		expect(pane.offersRoutes).toBe(true);
		expect(pane.centered).toBe(false);
	});

	test("running: Steps and Inputs", () => {
		expect(paneOf("running").pane.sections).toEqual(["steps", "inputs"]);
	});

	test("streaming: Steps, Answer and Inputs", () => {
		expect(paneOf("streaming").pane.sections).toEqual([
			"steps",
			"answer",
			"inputs",
		]);
	});

	test("a queued run has Inputs only, so no links and no chips", () => {
		const { pane } = paneOf("queued", {
			run: (s) => s.runs.find((r) => r.status === "queued") ?? null,
		});
		expect(pane.sections).toEqual(["inputs"]);
		expect(pane.showLinks).toBe(false);
		expect(pane.chips).toBeNull();
		expect(pane.actions.removeFromQueue).toBe(true);
	});

	test("small form done: Result and Inputs", () => {
		expect(paneOf("small-done").pane.sections).toEqual(["result", "inputs"]);
	});

	test("a form without inputs has no Inputs section", () => {
		const { pane } = paneOf("none-done");
		expect(pane.sections).toEqual([]);
		expect(pane.inputs).toBeNull();
		expect(pane.showLinks).toBe(false);
	});
});

describe("paneViewOf: centred card", () => {
	test("zero fields, nothing returned: centred, with the routes on offer", () => {
		const { pane } = paneOf("none-done");
		expect(pane.centered).toBe(true);
		expect(pane.notes).toEqual([{ kind: "nothing" }]);
		expect(pane.offersRoutes).toBe(true);
	});

	test("zero fields, running without output: centred and pending", () => {
		const { pane } = paneOf("none-cap");
		expect(pane.centered).toBe(true);
		expect(pane.notes[0]?.kind).toBe("pending");
	});

	test("a failed run of a form without inputs is not centred (its failure card must show)", () => {
		const state = fixture("none-done");
		const run = state.runs[0] as RunEntry;
		const failed: RunEntry = {
			...run,
			status: "failed",
			outcome: {
				kind: "failed",
				failure: "flow",
				message: "Boom",
				detail: "Boom",
			},
		};
		const pane = paneViewOf(failed, {
			state: { ...state, runs: [failed] },
			layout: DESKTOP_LAYOUT,
			now: 0,
			words: shortWords(),
			isSecret: () => false,
			number,
			comparing: false,
			canPin: false,
		});
		expect(pane.centered).toBe(false);
		expect(pane.notes[0]?.kind).toBe("failure");
	});

	test("a form with inputs is never centred", () => {
		expect(paneOf("small-done").pane.centered).toBe(false);
	});
});

describe("paneViewOf: change chips and the edited chip", () => {
	test("running: what changed against the run before", () => {
		const { pane } = paneOf("running");
		expect(pane.chips?.sinceRun).toBe(13);
		expect(pane.chips?.changes.map((c) => c.label)).toContain("Run OCR");
		expect(pane.chips?.edited).toBe(false);
	});

	test("numbers read grouped with their typed decimals, in the chips and the Inputs list", () => {
		const { pane } = paneOf("failed");
		expect(
			pane.chips?.changes.find((change) => change.label === "Expected total"),
		).toMatchObject({ from: "6,188.00", to: "11,769.10" });
		expect(
			pane.inputs?.rows.find((row) => row.label === "Expected total")?.text,
		).toEqual({ kind: "value", text: "11,769.10" });
		expect(
			pane.inputs?.rows.find((row) => row.label === "Max pages")?.text,
		).toEqual({ kind: "value", text: "20" });
	});

	test("no chips against a run after it in the list (the first run of a form)", () => {
		const { pane } = paneOf("none-done");
		expect(pane.chips).toBeNull();
	});

	test("two panes show no chips", () => {
		const { pane } = paneOf("compare", { comparing: true });
		expect(pane.chips).toBeNull();
	});

	test("the rail edited since the compared run adds the warning chip", () => {
		const state = fixture("done");
		const edited: FormSessionState = {
			...state,
			rail: {
				...state.rail,
				values: { ...state.rail.values, vendor_name: "Alpenfracht AG" },
			},
		};
		const run = stageRunOf(edited) as RunEntry;
		const pane = paneViewOf(run, {
			state: edited,
			layout: DESKTOP_LAYOUT,
			now: fixtureClock("done").now,
			words: shortWords(),
			isSecret: () => false,
			number,
			comparing: false,
			canPin: true,
		});
		expect(pane.chips?.edited).toBe(true);
		expect(pane.actions.use).toBe(true);
		expect(pane.actions.repeat).toBe("runAgain");
		expect(pane.inputs?.canUse).toBe(true);
		expect(
			pane.inputs?.rows.find((row) => row.label === "Vendor")?.differs,
		).toBe(true);
		expect(
			pane.inputs?.rows.find((row) => row.label === "Invoice")?.differs,
		).toBe(false);
	});

	test("per-run fields never count against the rail", () => {
		const state = fixture("done");
		const perRun: FormSessionState = {
			...state,
			rail: {
				...state.rail,
				values: { ...state.rail.values, vendor_name: "Alpenfracht AG" },
			},
			memory: {
				...state.memory,
				prefs: { ...state.memory.prefs, perRun: ["vendor_name"] },
			},
		};
		const run = stageRunOf(perRun) as RunEntry;
		expect(railDiffersFrom(perRun, run)).toBe(false);
	});

	test("a file to pick again counts as a difference", () => {
		const state = fixture("pick-again");
		const run = stageRunOf(state) as RunEntry;
		expect(railDiffersFrom(state, run)).toBe(true);
	});
});

describe("paneViewOf: inputs list", () => {
	test("rows in field order, the same-as-the-form note on an untouched rail", () => {
		const { pane, state } = paneOf("done");
		expect(pane.inputs?.rows.map((row) => row.label)).toEqual(
			state.form.fields.map((field) => field.label),
		);
		expect(pane.inputs?.sameAsForm).toBe(true);
		expect(pane.inputs?.canUse).toBe(false);
		expect(pane.inputs?.rows.every((row) => !row.differs)).toBe(true);
	});

	test("the note is gone while any field is per run", () => {
		const state = fixture("done");
		const perRun: FormSessionState = {
			...state,
			memory: {
				...state.memory,
				prefs: { ...state.memory.prefs, perRun: ["invoice_file"] },
			},
		};
		const pane = paneViewOf(stageRunOf(perRun) as RunEntry, {
			state: perRun,
			layout: DESKTOP_LAYOUT,
			now: 0,
			words: shortWords(),
			isSecret: () => false,
			number,
			comparing: false,
			canPin: false,
		});
		expect(pane.inputs?.sameAsForm).toBe(false);
	});

	test("an older run's file reads as its name", () => {
		const { pane } = paneOf("runs", {
			run: (s) => s.runs.find((r) => r.n === 9) ?? null,
		});
		expect(pane.inputs?.rows[0]?.text).toMatchObject({ kind: "value" });
	});
});

describe("paneViewOf: phone", () => {
	test("Copy moves into the menu", () => {
		const { pane } = paneOf("done", { layout: PHONE_LAYOUT });
		expect(pane.actions.copy).toBeNull();
		expect(pane.actions.menu.some((item) => item.id === "copy")).toBe(true);
	});
});

describe("paneViewOf: waiting for you", () => {
	test("an asking run lists its pending question", () => {
		const { pane } = paneOf("asking");
		expect(pane.waiting).toHaveLength(1);
		expect(pane.bar.status).toBe("asking");
	});
});

describe("copyKindOf", () => {
	test("the answer when there is one, else the result, else nothing", () => {
		const { run } = paneOf("done");
		expect(copyKindOf(run)).toBe("answer");
		const resultOnly = paneOf("small-done").run;
		expect(copyKindOf(resultOnly)).toBe("result");
		expect(copyKindOf({ output: null })).toBeNull();
	});
});
