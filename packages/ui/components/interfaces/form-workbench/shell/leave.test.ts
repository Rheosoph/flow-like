import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import type { FileSlot, FormSessionState, RunEntry } from "../contracts";
import { fixture } from "../testing/fixtures";
import { hasWaiting, leaveBody, leaveCountsOf } from "./leave";

const t = createI18n({ language: "en" }).getFixedT("en", "interfaces");

function slot(name: string): FileSlot {
	return {
		id: `slot-${name}`,
		name,
		size: 1,
		type: "application/pdf",
		state: "waiting",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

function withRuns(
	state: FormSessionState,
	runs: readonly RunEntry[],
): FormSessionState {
	return { ...state, runs };
}

describe("leaveCountsOf", () => {
	test("a form with nothing waiting has nothing to lose", () => {
		const counts = leaveCountsOf(fixture("done"));
		expect(counts).toEqual({ runs: 0, files: 0 });
		expect(hasWaiting(counts)).toBe(false);
	});

	test("counts queued and sending runs of this session, never running ones or history", () => {
		const base = fixture("series");
		const sample = base.runs[0];
		const runs = [
			{ ...sample, id: "a", status: "queued" },
			{ ...sample, id: "b", status: "sending" },
			{ ...sample, id: "c", status: "running" },
			{ ...sample, id: "d", status: "asking" },
			{ ...sample, id: "e", status: "starting" },
			{ ...sample, id: "f", status: "queued", origin: "history" },
		] as const satisfies readonly RunEntry[];
		expect(leaveCountsOf(withRuns(base, runs)).runs).toBe(2);
	});

	test("counts every next file of every field", () => {
		const base = fixture("done");
		const state: FormSessionState = {
			...base,
			rail: {
				...base.rail,
				nextFiles: {
					invoice_file: [slot("a"), slot("b"), slot("c")],
					supporting_documents: [slot("d")],
					empty: [],
				},
			},
		};
		expect(leaveCountsOf(state).files).toBe(4);
	});

	test("the series fixture waits with queued runs and next files", () => {
		const counts = leaveCountsOf(fixture("leave"));
		expect(counts.runs).toBeGreaterThan(0);
		expect(counts.files).toBeGreaterThan(0);
		expect(hasWaiting(counts)).toBe(true);
	});
});

describe("leaveBody", () => {
	test("the spec's sentences, one run and five next files", () => {
		expect(leaveBody(t, { runs: 1, files: 5 })).toBe(
			"1 run will not start and 5 next files will be removed.",
		);
	});

	test("one part alone, singular and plural", () => {
		expect(leaveBody(t, { runs: 3, files: 0 })).toBe("3 runs will not start.");
		expect(leaveBody(t, { runs: 1, files: 0 })).toBe("1 run will not start.");
		expect(leaveBody(t, { runs: 0, files: 5 })).toBe(
			"5 next files will be removed.",
		);
		expect(leaveBody(t, { runs: 0, files: 1 })).toBe(
			"1 next file will be removed.",
		);
	});

	test("nothing waiting has no sentence", () => {
		expect(leaveBody(t, { runs: 0, files: 0 })).toBeNull();
	});
});
