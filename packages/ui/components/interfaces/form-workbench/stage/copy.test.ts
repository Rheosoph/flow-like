import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import type { MenuItem } from "./bar-actions";
import type { BarView } from "./bar-model";
import {
	blockerText,
	detailText,
	failureTitle,
	inLineText,
	leadText,
	menuLabel,
	queuedNoteText,
	sectionLabel,
	stepText,
} from "./copy";

const t = createI18n({ language: "en" }).getFixedT("en", "interfaces");

const view = (patch: Partial<BarView>): BarView => ({
	status: "running",
	lead: { kind: "status" },
	clockMs: null,
	detail: null,
	...patch,
});

describe("lead and detail", () => {
	test("the status word, or how long it took", () => {
		expect(leadText(t, view({ status: "running" }))).toBe("Running");
		expect(leadText(t, view({ status: "queued" }))).toBe("Queued");
		expect(leadText(t, view({ status: "asking" }))).toBe(
			"Waiting for your answer",
		);
		expect(
			leadText(
				t,
				view({ status: "done", lead: { kind: "doneIn", ms: 48_000 } }),
			),
		).toBe("Done in 48 s");
		expect(
			leadText(
				t,
				view({ status: "failed", lead: { kind: "failedAfter", ms: 12_000 } }),
			),
		).toBe("Failed after 12 s");
		expect(
			leadText(
				t,
				view({ status: "stopped", lead: { kind: "stoppedAt", ms: 37_000 } }),
			),
		).toBe("Stopped at 0:37");
	});

	test("places in line read as ordinals", () => {
		const places = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101];
		expect(places.map((place) => inLineText(t, place))).toEqual([
			"1st in line",
			"2nd in line",
			"3rd in line",
			"4th in line",
			"11th in line",
			"12th in line",
			"13th in line",
			"21st in line",
			"22nd in line",
			"23rd in line",
			"101st in line",
		]);
	});

	test("the detail after the lead", () => {
		expect(
			detailText(t, {
				kind: "step",
				step: { number: 4, title: "Match purchase order" },
			}),
		).toBe("Step 4: Match purchase order");
		expect(detailText(t, { kind: "inLine", place: 1 })).toBe("1st in line");
		expect(detailText(t, { kind: "waitingForPlace" })).toBe(
			"waiting for a free place",
		);
		expect(detailText(t, { kind: "sendingSoon", files: 1 })).toBe(
			"starts when its file is sent",
		);
		expect(detailText(t, { kind: "sendingSoon", files: 3 })).toBe(
			"starts when its files are sent",
		);
		expect(stepText(t, { number: 2, title: "Run OCR" })).toBe(
			"Step 2: Run OCR",
		);
	});

	test("the three reasons the spec words, and the two the runtime adds", () => {
		const reasons = (
			[
				"removedFromQueue",
				"fileNotSent",
				"formClosed",
				"declined",
				"promptCancelled",
			] as const
		).map((reason) => detailText(t, { kind: "notStarted", reason }));
		expect(reasons).toEqual([
			"taken out of the queue",
			"its file was not sent",
			"the form was closed before its turn",
			"it was not approved",
			"its settings were not filled in",
		]);
	});
});

describe("notes and menu", () => {
	test("the queued notes by cause", () => {
		expect(queuedNoteText(t, "device", 3)).toBe(
			"This device runs 3 at a time from this window. This one starts by itself when one of them ends.",
		);
		expect(queuedNoteText(t, "plan", 2)).toBe(
			"Your plan runs 2 at a time. This one starts by itself when one of them ends.",
		);
		expect(queuedNoteText(t, "noPlace", 2)).toContain(
			"Every place your plan has is taken",
		);
		expect(queuedNoteText(t, "hold", 3)).toContain("on hold");
	});

	test("a plain sentence per failure kind, the step named for a flow failure", () => {
		const step = { number: 2, title: "Run OCR" };
		expect(failureTitle(t, "flow", step)).toBe("The run failed at “Run OCR”.");
		expect(failureTitle(t, "flow", null)).toBe("The run failed.");
		expect(failureTitle(t, "quota", step)).toContain("limit was reached");
		expect(failureTitle(t, "network", null)).toContain("connection was lost");
	});

	test("a flow that failed before any file came back says only what the form knows: no files were saved", () => {
		const step = { number: 2, title: "Run OCR" };
		expect(failureTitle(t, "flow", step, true)).toBe(
			"The run failed at “Run OCR”. No files were saved.",
		);
		expect(failureTitle(t, "flow", null, true)).toBe(
			"The run failed. No files were saved.",
		);
		expect(failureTitle(t, "quota", step, true)).not.toContain("saved");
	});

	test("why a repeat is disabled, one sentence per blocker", () => {
		expect(blockerText(t, "files")).toContain("pick them again");
		expect(blockerText(t, "hidden")).toContain("enter them again");
		expect(blockerText(t, "form")).toBe(
			"Some of its inputs no longer fit this form. Use these inputs and check them.",
		);
	});

	test("section labels and menu labels", () => {
		expect(sectionLabel(t, "steps")).toBe("Steps");
		const item = (patch: Partial<MenuItem>): MenuItem => ({
			id: "repeat",
			separatorBefore: false,
			disabled: false,
			blocker: null,
			repeat: "runAgain",
			copy: null,
			...patch,
		});
		expect(menuLabel(t, item({}))).toBe("Run again");
		expect(menuLabel(t, item({ repeat: "tryAgain" }))).toBe("Try again");
		expect(menuLabel(t, item({ id: "copy", copy: "result" }))).toBe(
			"Copy the result",
		);
		expect(menuLabel(t, item({ id: "savePreset" }))).toBe(
			"Save these inputs as a preset…",
		);
		expect(blockerText(t, "files")).toContain("can no longer be sent");
		expect(blockerText(t, "hidden")).toContain("not saved on this device");
	});
});
