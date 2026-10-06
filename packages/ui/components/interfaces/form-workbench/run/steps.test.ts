import { describe, expect, test } from "bun:test";
import type { IPlanStep } from "../../chat-default/chat-db";
import type { RunOutcome, RunStep } from "../contracts";
import {
	failedAtOf,
	markEndedSteps,
	splitStepText,
	stepReachedOf,
	toRunSteps,
} from "./steps";

const plan = (
	steps: readonly [string, IPlanStep["status"], string?, string?][],
): IPlanStep[] =>
	steps.map(([title, status, description, reasoning], index) => ({
		id: `step-${index + 1}`,
		title,
		status,
		...(description === undefined ? {} : { description }),
		...(reasoning === undefined ? {} : { reasoning }),
	}));

const running = toRunSteps(
	plan([
		[
			"Read documents",
			"done",
			"Loading the invoice and 2 supporting documents",
		],
		["Run OCR", "done", "Recognising text on 6 scanned pages"],
		["Extract fields", "done", "Vendor, dates, totals and line items"],
		[
			"Match purchase order",
			"progress",
			"Comparing 4 line items with PO-48213",
			"3 of 4 line items matched.",
		],
	]),
	"step-4",
);

const failedOutcome: RunOutcome = {
	kind: "failed",
	failure: "flow",
	message: null,
	detail: null,
};

describe("toRunSteps", () => {
	test("numbers steps by position and keeps the active message", () => {
		expect(running.map((step) => step.number)).toEqual([1, 2, 3, 4]);
		expect(running.map((step) => step.state)).toEqual([
			"done",
			"done",
			"done",
			"active",
		]);
		expect(running[3]).toEqual({
			id: "step-4",
			number: 4,
			title: "Match purchase order",
			detail: "Comparing 4 line items with PO-48213",
			message: "3 of 4 line items matched.",
			state: "active",
		});
		expect(running[0]?.message).toBeNull();
	});

	test("rejoins a title the chat parser split at a URL", () => {
		const [fetch, withDetail] = toRunSteps(
			plan([
				["Fetch https", "done", "//portal.example.com/orders"],
				[
					"Fetch https",
					"progress",
					"//portal.example.com/orders?id=4: Loading 4 orders",
				],
			]),
			"step-2",
		);
		expect(fetch?.title).toBe("Fetch https://portal.example.com/orders");
		expect(fetch?.detail).toBeNull();
		expect(withDetail?.title).toBe(
			"Fetch https://portal.example.com/orders?id=4",
		);
		expect(withDetail?.detail).toBe("Loading 4 orders");
	});

	test("a planned current step is the active one, failed stays failed", () => {
		const steps = toRunSteps(
			plan([
				["Think", "failed"],
				["Answer", "planned"],
			]),
			"step-2",
		);
		expect(steps.map((step) => step.state)).toEqual(["failed", "active"]);
	});
});

describe("splitStepText", () => {
	test("splits at the first colon outside a URL", () => {
		expect(splitStepText("Step 2: OCR: 6 pages")).toEqual({
			title: "Step 2",
			detail: "OCR: 6 pages",
		});
		expect(splitStepText("Upload: ")).toEqual({
			title: "Upload",
			detail: null,
		});
		expect(splitStepText("Thinking")).toEqual({
			title: "Thinking",
			detail: null,
		});
		expect(splitStepText("Open https://example.com:8080/x: now")).toEqual({
			title: "Open https://example.com:8080/x",
			detail: "now",
		});
	});
});

describe("markEndedSteps", () => {
	const stateOfLast = (steps: readonly RunStep[]) => steps.at(-1)?.state;

	test("the active step takes the run's ending", () => {
		expect(stateOfLast(markEndedSteps(running, { kind: "succeeded" }))).toBe(
			"done",
		);
		expect(stateOfLast(markEndedSteps(running, failedOutcome))).toBe("failed");
		expect(stateOfLast(markEndedSteps(running, { kind: "stopped" }))).toBe(
			"stopped",
		);
		expect(stateOfLast(markEndedSteps(running, { kind: "unknown" }))).toBe(
			"stopped",
		);
	});

	test("a refused start keeps its steps; finished steps never change", () => {
		expect(markEndedSteps(running, { kind: "noPlace" })).toBe(running);
		const ended = markEndedSteps(running, failedOutcome);
		expect(ended.slice(0, 3)).toEqual(running.slice(0, 3));
		expect(markEndedSteps(ended, { kind: "succeeded" })).toBe(ended);
	});
});

describe("failedAtOf and stepReachedOf", () => {
	test("the failed step of a failed run", () => {
		const atOcr = toRunSteps(
			plan([
				["Read documents", "done"],
				["Run OCR", "progress"],
			]),
			"step-2",
		);
		const steps = markEndedSteps(atOcr, failedOutcome);
		expect(failedAtOf(steps)).toEqual({ number: 2, title: "Run OCR" });
		expect(failedAtOf(atOcr)).toBeNull();
	});

	test("the furthest step reached", () => {
		expect(stepReachedOf(running)).toEqual({
			number: 4,
			title: "Match purchase order",
		});
		const withPlanned = toRunSteps(
			plan([
				["Think", "done"],
				["Answer", "planned"],
			]),
		);
		expect(stepReachedOf(withPlanned)).toEqual({ number: 1, title: "Think" });
		expect(stepReachedOf([])).toBeNull();
	});
});
