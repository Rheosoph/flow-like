import { describe, expect, test } from "bun:test";
import {
	type FlowScriptFileBuffer,
	createFlowScriptFileStore,
	dirtyFlowScriptFileIds,
	isFlowScriptFileDirty,
} from "./use-flowscript-files";

const buffer = (
	text: string,
	baseline = text,
	extra: Partial<FlowScriptFileBuffer> = {},
): FlowScriptFileBuffer => ({ text, baseline, ...extra });

describe("FlowScript file buffers", () => {
	test("a draft that moved away from its render is dirty", () => {
		expect(isFlowScriptFileDirty(buffer("a", "a"))).toBe(false);
		expect(isFlowScriptFileDirty(buffer("a", "b"))).toBe(true);
	});

	test("dirty ids cover only the files holding unapplied edits", () => {
		const map = new Map([
			["main", buffer("x")],
			["module-a", buffer("y", "z")],
		]);
		expect([...dirtyFlowScriptFileIds(map)]).toEqual(["module-a"]);
	});
});

describe("FlowScript file store", () => {
	test("restores exactly what a file was left with", () => {
		const store = createFlowScriptFileStore();
		const seat = { scrollTop: 40 };
		store.stash("module-a", {
			text: "draft",
			baseline: "rendered",
			scopeAnchors: ["anchor-1"],
			viewState: seat,
		});
		expect(store.peek("module-a")).toEqual({
			text: "draft",
			baseline: "rendered",
			scopeAnchors: ["anchor-1"],
			viewState: seat,
		});
		// Reading keeps the buffer: switching back and forth must not lose the draft.
		expect(store.peek("module-a")?.text).toBe("draft");
	});

	test("an unknown file has no buffer, so the panel renders it from the board", () => {
		const store = createFlowScriptFileStore();
		expect(store.peek("main")).toBeUndefined();
	});

	test("tracks the dirty files and reports only real changes", () => {
		const changes: string[][] = [];
		const store = createFlowScriptFileStore((ids) => changes.push([...ids]));

		store.stash("main", buffer("same"));
		expect([...store.dirtyFileIds]).toEqual([]);
		expect(changes).toHaveLength(0);

		store.stash("module-a", buffer("draft", "rendered"));
		expect([...store.dirtyFileIds]).toEqual(["module-a"]);
		expect(changes).toEqual([["module-a"]]);

		// Re-stashing the same dirty file must not notify again.
		store.stash("module-a", buffer("draft2", "rendered"));
		expect(changes).toHaveLength(1);

		// Applying the draft settles the file.
		store.stash("module-a", buffer("rendered2"));
		expect([...store.dirtyFileIds]).toEqual([]);
		expect(changes).toEqual([["module-a"], []]);
	});

	test("dropping and clearing forget the buffers", () => {
		const store = createFlowScriptFileStore();
		store.stash("main", buffer("a", "b"));
		store.stash("module-a", buffer("c", "d"));

		store.drop("main");
		expect(store.peek("main")).toBeUndefined();
		expect([...store.dirtyFileIds]).toEqual(["module-a"]);

		store.clear();
		expect(store.peek("module-a")).toBeUndefined();
		expect([...store.dirtyFileIds]).toEqual([]);
	});
});
