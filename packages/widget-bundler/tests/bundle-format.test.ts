import { describe, expect, test } from "bun:test";
import { isSafeEntryPath, unsafeArchiveEntryPaths } from "../src/bundle-format";

const UNSAFE_NAMES = [
	"",
	"/etc/passwd",
	"../x.txt",
	"shared/../../x.txt",
	"shared/./x.js",
	"shared//x.js",
	"shared\\x.js",
	"C:/x.txt",
	"C:x.txt",
	"c:/ProgramData/Microsoft/Windows/Start Menu/Programs/StartUp/x.bat",
	"widgets/sales-chart/index.html:ads",
	"widgets/sales-chart/index.html::$DATA",
	"shared/react\0.js",
];

describe("isSafeEntryPath", () => {
	test("accepts relative bundle paths", () => {
		for (const path of [
			"bundle.json",
			"shared/react-abc123.js",
			"widgets/sales-chart/index.html",
			"shared/a b/c.d.js",
		]) {
			expect([path, isSafeEntryPath(path)]).toEqual([path, true]);
		}
	});

	test("rejects traversal, drive prefixes, streams and NUL on every OS", () => {
		for (const path of UNSAFE_NAMES) {
			expect([path, isSafeEntryPath(path)]).toEqual([path, false]);
		}
	});
});

describe("unsafeArchiveEntryPaths", () => {
	test("reports every unsafe archive name", () => {
		expect(unsafeArchiveEntryPaths(["bundle.json", ...UNSAFE_NAMES])).toEqual(
			UNSAFE_NAMES.map((name) => `Unsafe widget bundle entry path: ${name}`),
		);
	});

	test("checks directory entries without their trailing slash", () => {
		expect(
			unsafeArchiveEntryPaths([
				"widgets/",
				"widgets/sales-chart/",
				"C:/",
				"../",
				"/",
				"widgets//",
			]),
		).toEqual([
			"Unsafe widget bundle entry path: C:/",
			"Unsafe widget bundle entry path: ../",
			"Unsafe widget bundle entry path: /",
			"Unsafe widget bundle entry path: widgets//",
		]);
	});
});
