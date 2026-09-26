import { describe, expect, test } from "bun:test";
import { ILogLevel } from "../../../lib/schema/flow/log";
import {
	embeddedJson,
	firstLine,
	formatAbsolute,
	formatDuration,
	formatRelative,
	levelIndex,
	logKey,
	prettyJson,
	sameLog,
	toMicros,
} from "./log-format";
import { makeLog } from "./test-fixtures";

describe("levelIndex", () => {
	test("reads serde names and numbers", () => {
		expect(levelIndex(ILogLevel.Debug)).toBe(0);
		expect(levelIndex("Warn")).toBe(2);
		expect(levelIndex(ILogLevel.Fatal)).toBe(4);
		expect(levelIndex(3)).toBe(3);
		expect(levelIndex(9)).toBe(0);
		expect(levelIndex(undefined)).toBe(0);
	});
});

describe("time formatting", () => {
	test("toMicros folds sub-second nanos", () => {
		expect(
			toMicros({ secs_since_epoch: 2, nanos_since_epoch: 1_902_999 }),
		).toBe(2_001_902);
	});

	test("absolute time is local wall clock with milliseconds", () => {
		const micros = new Date(2026, 8, 25, 14, 32, 9, 16).getTime() * 1_000 + 400;
		expect(formatAbsolute(micros)).toBe("14:32:09.016");
	});

	test("relative time is seconds from the run start", () => {
		expect(formatRelative(1_000_000 + 1_902_000, 1_000_000)).toBe("+1.902s");
		expect(formatRelative(1_000_000, 1_000_000)).toBe("+0.000s");
		expect(formatRelative(999_000, 1_000_000)).toBe("-0.001s");
	});

	test("durations pick a readable unit", () => {
		expect(formatDuration(0)).toBe("");
		expect(formatDuration(40)).toBe("40 µs");
		expect(formatDuration(200)).toBe("0.2 ms");
		expect(formatDuration(48_000)).toBe("48 ms");
		expect(formatDuration(1_840_000)).toBe("1.84 s");
		expect(formatDuration(125_000_000)).toBe("2m 5s");
	});
});

describe("firstLine", () => {
	test("counts the lines after the first", () => {
		expect(firstLine("POST x\nstatus: 200\nbody: 1")).toEqual({
			line: "POST x",
			extra: 2,
		});
		expect(firstLine("one line")).toEqual({ line: "one line", extra: 0 });
		expect(firstLine("a\r\nb\n")).toEqual({ line: "a", extra: 1 });
	});
});

describe("JSON detection", () => {
	test("pretty-prints a message that is entirely JSON", () => {
		expect(prettyJson('{"a":1,"b":[2]}')).toBe(
			'{\n  "a": 1,\n  "b": [\n    2\n  ]\n}',
		);
		expect(prettyJson("  [1,2] ")).toBe("[\n  1,\n  2\n]");
		expect(prettyJson("{not json}")).toBeUndefined();
		expect(prettyJson('"just a string"')).toBeUndefined();
	});

	test("finds JSON embedded in text", () => {
		expect(embeddedJson('Response {"ok":true,"n":2} received')).toBe(
			'{\n  "ok": true,\n  "n": 2\n}',
		);
		expect(embeddedJson('ExecutionFailed("x") in [iteration] {"a":"}"}')).toBe(
			'{\n  "a": "}"\n}',
		);
		expect(embeddedJson("empty {} and [] only")).toBeUndefined();
		expect(embeddedJson('{"whole":true}')).toBeUndefined();
	});
});

describe("log identity", () => {
	test("the key ignores position and survives re-fetching", () => {
		const a = makeLog({ message: "hello", start: 5_000_000 });
		const b = makeLog({ message: "hello", start: 5_000_000 });
		expect(logKey(a)).toBe(logKey(b));
		expect(sameLog(a, b)).toBe(true);
		expect(sameLog(a, makeLog({ message: "hello", start: 5_000_001 }))).toBe(
			false,
		);
	});
});
