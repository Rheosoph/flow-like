import { describe, expect, test } from "bun:test";
import type { RunEntry } from "../contracts";
import { SECRET_MASK } from "../contracts";
import type { IsSecret } from "../model/values";
import { formatTypedNumber } from "../run/format";
import { stageRunOf } from "../run/run-view";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT, layoutFor } from "../testing/layouts";
import { compareViewOf } from "./compare-model";
import { shortWords } from "./test-words";

const words = shortWords();
const secretNone = () => false;
const number = (text: string) => formatTypedNumber(text, "en-GB", ".");

function viewFor(
	state: ReturnType<typeof fixture>,
	layout = DESKTOP_LAYOUT,
	isSecret: IsSecret = secretNone,
) {
	return compareViewOf({
		state,
		layout,
		stageRun: stageRunOf(state),
		words,
		isSecret,
		number,
	});
}

describe("compareViewOf", () => {
	test("nothing pinned: off", () => {
		expect(viewFor(fixture("done")).mode).toBe("off");
	});

	test("a pinned run on a wide stage: two panes and the inputs that differ", () => {
		const view = viewFor(fixture("compare"));
		expect(view.mode).toBe("panes");
		expect(view.pinned?.n).toBe(12);
		expect(view.selected?.n).toBe(14);
		expect(view.samePinned).toBe(false);
		expect(view.rows.map((row) => row.label)).toEqual([
			"Invoice",
			"Expected total",
			"Run OCR",
		]);
		expect(view.title).toEqual({ kind: "differ", count: 3 });
		const ocr = view.rows.find((row) => row.label === "Run OCR");
		expect(ocr).toMatchObject({ from: "Off", to: "On" });
		const total = view.rows.find((row) => row.label === "Expected total");
		expect(total).toMatchObject({ from: "6,188.00", to: "11,769.10" });
	});

	test("a 900 to 999 px box names the pinned run instead", () => {
		const view = viewFor(fixture("compare"), layoutFor(960, 836));
		expect(view.mode).toBe("note");
		expect(view.title).toEqual({ kind: "pinned" });
		expect(view.rows).toEqual([]);
		expect(view.pinned?.n).toBe(12);
	});

	test("a phone shows no compare", () => {
		expect(viewFor(fixture("compare"), PHONE_LAYOUT).mode).toBe("off");
	});

	test("the pinned run on the stage has nothing to put beside it", () => {
		const state = fixture("compare");
		const pinnedOnStage = {
			...state,
			view: { ...state.view, selectedRunId: state.view.pinnedRunId },
		};
		const view = viewFor(pinnedOnStage);
		expect(view.samePinned).toBe(true);
		expect(view.title).toEqual({ kind: "pinned" });
		expect(view.rows).toEqual([]);
	});

	test("two runs with the same inputs say so", () => {
		const state = fixture("compare");
		const selected = state.runs.find(
			(r) => r.id === state.view.selectedRunId,
		) as RunEntry;
		const twin: RunEntry = {
			...(state.runs.find((r) => r.id === state.view.pinnedRunId) as RunEntry),
			copy: selected.copy,
		};
		const runs = state.runs.map((r) => (r.id === twin.id ? twin : r));
		expect(viewFor({ ...state, runs }).title).toEqual({ kind: "same" });
	});

	test("a pinned run that was removed is no pin", () => {
		const state = fixture("compare");
		const runs = state.runs.filter((r) => r.id !== state.view.pinnedRunId);
		expect(viewFor({ ...state, runs }).mode).toBe("off");
	});

	test("secrets read the mask on both sides", () => {
		const state = fixture("compare");
		const view = viewFor(state, DESKTOP_LAYOUT, () => true);
		expect(view.rows.length).toBeGreaterThan(0);
		for (const row of view.rows) {
			expect(row.from).toBe(SECRET_MASK);
			expect(row.to).toBe(SECRET_MASK);
		}
	});
});
