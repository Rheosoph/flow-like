import { describe, expect, test } from "bun:test";
import {
	DEFAULT_EVENT_DEBOUNCE_MS,
	MIN_EVENT_DEBOUNCE_MS,
	resolveEventDebounceMs,
} from "../hooks/use-debounced-trigger";
import { resolveEnterIntent, shouldReportCommit } from "./text-field-events";

const key = (
	name: string,
	modifiers: { metaKey?: boolean; ctrlKey?: boolean } = {},
) => ({
	key: name,
	metaKey: modifiers.metaKey ?? false,
	ctrlKey: modifiers.ctrlKey ?? false,
});

describe("resolveEnterIntent", () => {
	test("submits a single-line field on plain Enter", () => {
		expect(resolveEnterIntent(key("Enter"), false)).toEqual({
			kind: "submit",
			via: "enter",
		});
		expect(resolveEnterIntent(key("Enter"), undefined)).toEqual({
			kind: "submit",
			via: "enter",
		});
	});

	test("reports the modifier chord separately so composers can tell them apart", () => {
		expect(resolveEnterIntent(key("Enter", { metaKey: true }), false)).toEqual({
			kind: "submit",
			via: "modEnter",
		});
		expect(resolveEnterIntent(key("Enter", { ctrlKey: true }), true)).toEqual({
			kind: "submit",
			via: "modEnter",
		});
	});

	test("leaves plain Enter to the textarea when multiline", () => {
		expect(resolveEnterIntent(key("Enter"), true)).toEqual({ kind: "newline" });
	});

	test("ignores every other key", () => {
		for (const name of ["a", "Escape", "Tab", "ArrowDown"]) {
			expect(resolveEnterIntent(key(name), false)).toEqual({ kind: "ignore" });
			expect(resolveEnterIntent(key(name, { metaKey: true }), true)).toEqual({
				kind: "ignore",
			});
		}
	});
});

describe("shouldReportCommit", () => {
	test("stays quiet when focus leaves an untouched field", () => {
		expect(shouldReportCommit("seed", "seed")).toBe(false);
		expect(shouldReportCommit("", "")).toBe(false);
	});

	test("reports edits, including clearing the field", () => {
		expect(shouldReportCommit("seed", "seeded")).toBe(true);
		expect(shouldReportCommit("seed", "")).toBe(true);
	});
});

describe("resolveEventDebounceMs", () => {
	test("falls back to the default for unset or nonsense values", () => {
		for (const value of [
			undefined,
			0,
			-50,
			Number.NaN,
			Number.POSITIVE_INFINITY,
		]) {
			expect(resolveEventDebounceMs(value)).toBe(DEFAULT_EVENT_DEBOUNCE_MS);
		}
	});

	test("clamps a too-eager pause so typing cannot become per-keystroke", () => {
		expect(resolveEventDebounceMs(5)).toBe(MIN_EVENT_DEBOUNCE_MS);
		expect(resolveEventDebounceMs(250)).toBe(250);
	});
});
