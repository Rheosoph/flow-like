import { describe, expect, it } from "bun:test";
import { isBenignBrowserError, parseErrorFrames } from "./errors";

describe("isBenignBrowserError", () => {
	it("matches the stackless ResizeObserver delivery notices", () => {
		expect(
			isBenignBrowserError(
				"ResizeObserver loop completed with undelivered notifications.",
			),
		).toBe(true);
		expect(isBenignBrowserError("ResizeObserver loop limit exceeded")).toBe(
			true,
		);
	});

	it("matches the notice when it arrives as an Error", () => {
		expect(
			isBenignBrowserError(
				new Error(
					"ResizeObserver loop completed with undelivered notifications",
				),
			),
		).toBe(true);
	});

	it("keeps real failures reportable", () => {
		expect(isBenignBrowserError(new TypeError("x is not a function"))).toBe(
			false,
		);
		expect(isBenignBrowserError("Failed to fetch")).toBe(false);
		expect(isBenignBrowserError(undefined)).toBe(false);
		expect(isBenignBrowserError(null)).toBe(false);
		expect(isBenignBrowserError({ message: 42 })).toBe(false);
	});
});

describe("parseErrorFrames", () => {
	it("parses V8 frames", () => {
		expect(
			parseErrorFrames(
				[
					"TypeError: x is not a function",
					"    at Object.<anonymous> (/app/src/index.js:10:15)",
					"    at async load (webpack-internal:///./src/a.tsx:12:3)",
					"    at Array.map (<anonymous>)",
					"    at /app/node_modules/foo/index.js:3:4",
					"    at t (chunk-ABC.js:1:12345)",
					"    at native",
					"    at fn (C:\\Users\\x\\app\\main.js:3:4)",
				].join("\n"),
			),
		).toEqual([
			{ in_app: true, file: "/app/src/index.js", lineno: 10, colno: 15 },
			{
				in_app: true,
				function: "load",
				file: "webpack-internal:///./src/a.tsx",
				lineno: 12,
				colno: 3,
			},
			{ in_app: false, function: "Array.map", file: "<anonymous>" },
			{
				in_app: false,
				file: "/app/node_modules/foo/index.js",
				lineno: 3,
				colno: 4,
			},
			{
				in_app: true,
				function: "t",
				file: "chunk-ABC.js",
				lineno: 1,
				colno: 12345,
			},
			{ in_app: false, file: "native" },
			{
				in_app: true,
				function: "fn",
				file: "C:\\Users\\x\\app\\main.js",
				lineno: 3,
				colno: 4,
			},
		]);
	});

	it("parses Firefox and Safari frames", () => {
		expect(
			parseErrorFrames(
				[
					"foo@http://localhost:3000/app.js:1:2",
					"@http://localhost:3000/app.js:1:2",
					"promiseReactionJob@[native code]",
					"noAtSignHere",
				].join("\n"),
			),
		).toEqual([
			{
				in_app: true,
				function: "foo",
				file: "http://localhost:3000/app.js",
				lineno: 1,
				colno: 2,
			},
			{
				in_app: true,
				file: "http://localhost:3000/app.js",
				lineno: 1,
				colno: 2,
			},
			{ in_app: false, function: "promiseReactionJob", file: "[native code]" },
		]);
	});

	it("spans the first opening to the last closing parenthesis", () => {
		expect(parseErrorFrames("at a (b) (c)")).toEqual([
			{ in_app: true, function: "a", file: "b) (c" },
		]);
	});

	it("stays linear on adversarial frames", () => {
		const started = performance.now();
		parseErrorFrames(`at\t${"(a".repeat(40000)}`);
		parseErrorFrames(`@${"@a".repeat(40000)}`);
		expect(performance.now() - started).toBeLessThan(500);
	});
});
