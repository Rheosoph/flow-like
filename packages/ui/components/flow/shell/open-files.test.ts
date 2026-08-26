import { describe, expect, test } from "bun:test";
import {
	fileAfterClose,
	withFileClosed,
	withFileOpen,
	withMissingFilesDropped,
} from "./open-files";

describe("withFileOpen", () => {
	test("appends a file that is not open yet", () => {
		expect(withFileOpen(["a"], "b")).toEqual(["a", "b"]);
	});

	test("leaves tab order alone when the file is already open", () => {
		expect(withFileOpen(["a", "b", "c"], "b")).toEqual(["a", "b", "c"]);
	});
});

describe("fileAfterClose", () => {
	test("falls to the tab that slid into the closed one's place", () => {
		expect(fileAfterClose(["a", "b", "c"], "b")).toBe("c");
	});

	test("falls left when the closed tab was last", () => {
		expect(fileAfterClose(["a", "b", "c"], "c")).toBe("b");
	});

	test("closing a run of tabs walks left rather than jumping to main", () => {
		let open = ["a", "b", "c"];
		expect(fileAfterClose(open, "c")).toBe("b");
		open = withFileClosed(open, "c");
		expect(fileAfterClose(open, "b")).toBe("a");
	});

	test("returns null — meaning main.flow — when nothing is left", () => {
		expect(fileAfterClose(["a"], "a")).toBeNull();
	});

	test("returns null for a file that was never open", () => {
		expect(fileAfterClose(["a", "b"], "zzz")).toBeNull();
	});
});

describe("withMissingFilesDropped", () => {
	test("drops tabs whose module was deleted elsewhere", () => {
		const layers = new Set(["a", "c"]);
		expect(
			withMissingFilesDropped(["a", "b", "c"], (id) => layers.has(id)),
		).toEqual(["a", "c"]);
	});
});
