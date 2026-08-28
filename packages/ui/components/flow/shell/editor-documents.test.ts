import { describe, expect, test } from "bun:test";
import {
	type IEditorDocument,
	type IEditorTab,
	boardTabsFor,
	deserializeTabs,
	documentKey,
	isTabClosable,
	nextTabKey,
	sameDocument,
	serializeTabs,
	tabAfterClose,
	withDocumentOpened,
	withMissingTabsDropped,
	withTabClosed,
	withTabLayerPath,
} from "./editor-documents";

const main: IEditorDocument = { kind: "board", fileId: "main" };
const module = (id: string): IEditorDocument => ({ kind: "board", fileId: id });

function open(...docs: IEditorDocument[]): IEditorTab[] {
	return docs.reduce<IEditorTab[]>(
		(tabs, doc) => withDocumentOpened(tabs, doc).tabs,
		[],
	);
}

describe("documentKey", () => {
	test("ignores where a board tab is parked", () => {
		expect(documentKey(main)).toBe("board:main");
		expect(sameDocument(main, { kind: "board", fileId: "main" })).toBe(true);
	});

	test("separates the two storage scopes", () => {
		expect(
			sameDocument(
				{ kind: "storage", scope: "app", location: "a.pdf" },
				{ kind: "storage", scope: "user", location: "a.pdf" },
			),
		).toBe(false);
	});

	test("separates kinds that share an id", () => {
		expect(documentKey({ kind: "page", pageId: "x" })).not.toBe(
			documentKey({ kind: "widget", widgetId: "x" }),
		);
	});
});

describe("withDocumentOpened", () => {
	test("focuses the existing tab rather than opening a second", () => {
		const tabs = open(main, module("a"));
		const result = withDocumentOpened(tabs, module("a"));
		expect(result.tabs).toHaveLength(2);
		expect(result.key).toBe("board:a");
	});

	test("newTab opens the same file a second time", () => {
		const tabs = open(main, module("a"));
		const result = withDocumentOpened(tabs, module("a"), { newTab: true });
		expect(result.tabs.map((tab) => tab.key)).toEqual([
			"board:main",
			"board:a",
			"board:a#2",
		]);
		expect(result.key).toBe("board:a#2");
	});

	test("a third instance skips the taken suffix", () => {
		let tabs = open(main);
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		expect(nextTabKey(tabs, module("a"))).toBe("board:a#3");
	});

	test("a new instance lands beside the tab it was opened from", () => {
		const tabs = open(main, module("a"), module("b"));
		const result = withDocumentOpened(tabs, module("a"), {
			newTab: true,
			after: "board:main",
		});
		expect(result.tabs.map((tab) => tab.key)).toEqual([
			"board:main",
			"board:a#2",
			"board:a",
			"board:b",
		]);
	});

	test("opening an existing tab with a layer path re-parks it", () => {
		const tabs = open(main, module("a"));
		const result = withDocumentOpened(tabs, module("a"), { layerPath: "a/fn" });
		expect(result.tabs[1].layerPath).toBe("a/fn");
	});
});

describe("tabAfterClose", () => {
	test("falls to the tab that slid into the closed one's place", () => {
		const tabs = open(main, module("a"), module("b"));
		expect(tabAfterClose(tabs, "board:a")).toBe("board:b");
	});

	test("falls left when the closed tab was last", () => {
		const tabs = open(main, module("a"), module("b"));
		expect(tabAfterClose(tabs, "board:b")).toBe("board:a");
	});

	test("closing a run of tabs walks left rather than jumping to main", () => {
		let tabs = open(main, module("a"), module("b"));
		expect(tabAfterClose(tabs, "board:b")).toBe("board:a");
		tabs = withTabClosed(tabs, "board:b");
		expect(tabAfterClose(tabs, "board:a")).toBe("board:main");
	});

	test("returns null for a tab that was never open", () => {
		expect(tabAfterClose(open(main), "board:zzz")).toBeNull();
	});
});

describe("isTabClosable", () => {
	test("the last board tab keeps the canvas and cannot be closed", () => {
		expect(isTabClosable(open(main), "board:main")).toBe(false);
	});

	test("a board tab is closable once the file has a sibling", () => {
		const tabs = open(main, module("a"));
		expect(isTabClosable(tabs, "board:main")).toBe(true);
	});

	test("a non-board tab is always closable", () => {
		const tabs = open(main, { kind: "table", scope: "app", table: "users" });
		expect(isTabClosable(tabs, "table:app:users")).toBe(true);
	});
});

describe("withMissingTabsDropped", () => {
	test("drops tabs whose document was deleted elsewhere", () => {
		const tabs = open(main, module("a"), { kind: "page", pageId: "p1" });
		const alive = new Set(["board:main", "board:a"]);
		expect(
			withMissingTabsDropped(tabs, (doc) => alive.has(documentKey(doc))).map(
				(tab) => tab.key,
			),
		).toEqual(["board:main", "board:a"]);
	});

	test("prunes both instances of a file that is gone", () => {
		let tabs = open(main, module("a"));
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		expect(
			withMissingTabsDropped(tabs, (doc) =>
				doc.kind === "board" ? doc.fileId === "main" : true,
			),
		).toHaveLength(1);
	});
});

describe("withTabLayerPath", () => {
	test("parks one instance without moving its twin", () => {
		let tabs = open(main, module("a"));
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		tabs = withTabLayerPath(tabs, "board:a#2", "a/fn");
		expect(
			tabs.find((tab) => tab.key === "board:a")?.layerPath,
		).toBeUndefined();
		expect(tabs.find((tab) => tab.key === "board:a#2")?.layerPath).toBe("a/fn");
	});

	test("clearing to the file root drops the field", () => {
		let tabs = withTabLayerPath(open(main), "board:main", "x/y");
		tabs = withTabLayerPath(tabs, "board:main", undefined);
		expect("layerPath" in tabs[0]).toBe(false);
	});

	test("parking on the same path keeps the identity, so memoised tabs hold", () => {
		const tabs = withTabLayerPath(open(main), "board:main", "x");
		expect(withTabLayerPath(tabs, "board:main", "x")[0]).toBe(tabs[0]);
	});
});

describe("boardTabsFor", () => {
	test("finds every instance of one file", () => {
		let tabs = open(main, module("a"));
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		expect(boardTabsFor(tabs, "a").map((tab) => tab.key)).toEqual([
			"board:a",
			"board:a#2",
		]);
	});
});

describe("persistence", () => {
	test("round-trips tabs and the active key", () => {
		let tabs = open(main, module("a"), {
			kind: "storage",
			scope: "user",
			location: "docs/a.pdf",
		});
		tabs = withTabLayerPath(tabs, "board:a", "a/fn");
		const restored = deserializeTabs(serializeTabs(tabs, "board:a"));
		expect(restored.tabs).toEqual(tabs);
		expect(restored.activeKey).toBe("board:a");
	});

	test("survives garbage rather than throwing", () => {
		expect(deserializeTabs("{{{").tabs).toEqual([]);
		expect(deserializeTabs(null).tabs).toEqual([]);
		expect(deserializeTabs(JSON.stringify({ v: 99, tabs: [] })).tabs).toEqual(
			[],
		);
	});

	test("drops entries with an unknown kind and duplicate keys", () => {
		const raw = JSON.stringify({
			v: 1,
			tabs: [
				{ key: "board:main", doc: { kind: "board", fileId: "main" } },
				{ key: "board:main", doc: { kind: "board", fileId: "main" } },
				{ key: "ghost:1", doc: { kind: "ghost" } },
				{ key: "storage:app:x", doc: { kind: "storage", scope: "nope" } },
			],
			active: "ghost:1",
		});
		const restored = deserializeTabs(raw);
		expect(restored.tabs.map((tab) => tab.key)).toEqual(["board:main"]);
		expect(restored.activeKey).toBeNull();
	});
});
