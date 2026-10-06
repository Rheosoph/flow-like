import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import type { Announcement } from "../contracts";
import { announcementText } from "./live-region";

const t = createI18n({ language: "en" }).getFixedT("en", "interfaces");

const ended = (
	kind: "done" | "empty" | "failed" | "stopped" | "unknown",
	seconds: number,
	step: { number: number; title: string } | null = null,
): Announcement => ({ seq: 1, kind, n: 15, seconds, step });

describe("announcementText", () => {
	test("a run that is done", () => {
		expect(announcementText(t, ended("done", 41))).toBe("Run 15 done in 41 s.");
	});

	test("a run that returned nothing reads like a done run", () => {
		expect(announcementText(t, ended("empty", 2))).toBe("Run 15 done in 2 s.");
	});

	test("a failure names the step it ended at", () => {
		expect(
			announcementText(t, ended("failed", 12, { number: 2, title: "Run OCR" })),
		).toBe("Run 15 failed after 12 s at step 2: Run OCR.");
		expect(announcementText(t, ended("failed", 12))).toBe(
			"Run 15 failed after 12 s.",
		);
	});

	test("a stopped run reads its clock", () => {
		expect(announcementText(t, ended("stopped", 37))).toBe(
			"Run 15 stopped at 0:37.",
		);
	});

	test("a run whose connection ended before a result says so", () => {
		expect(announcementText(t, ended("unknown", 9))).toBe(
			"Run 15: the connection ended before it reported a result.",
		);
	});

	test("long runs read in minutes", () => {
		expect(announcementText(t, ended("done", 72))).toBe(
			"Run 15 done in 1 min 12 s.",
		);
	});

	test("a refused zero-field press says how many runs are going", () => {
		expect(
			announcementText(t, { seq: 2, kind: "capReached", running: 3 }),
		).toBe("3 runs are going. You can run again when one ends.");
		expect(
			announcementText(t, { seq: 2, kind: "capReached", running: 1 }),
		).toBe("1 run is going. You can run again when one ends.");
	});
});
