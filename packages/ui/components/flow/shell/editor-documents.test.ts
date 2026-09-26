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
	tabByKey,
	withBoardTabPosition,
	withDocumentOpened,
	withMissingTabsDropped,
	withPinnedFirst,
	withTabClosed,
	withTabColor,
	withTabLayerPath,
	withTabPinned,
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

	// One app has exactly one stylesheet, so its key carries no id and every
	// reference to it collapses onto the same tab.
	test("the app stylesheet is a single document", () => {
		expect(documentKey({ kind: "styles" })).toBe("styles:app");
		expect(sameDocument({ kind: "styles" }, { kind: "styles" })).toBe(true);
	});
});

describe("styles document persistence", () => {
	test("survives a serialize/deserialize round trip", () => {
		const tabs = withDocumentOpened([], { kind: "styles" }).tabs;
		const restored = deserializeTabs(serializeTabs(tabs, "styles:app"));
		expect(restored.tabs.map((tab) => tab.doc)).toEqual([{ kind: "styles" }]);
		expect(restored.activeKey).toBe("styles:app");
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

describe("withBoardTabPosition", () => {
	test("moves only the keyed board instance when navigation crosses files", () => {
		let tabs = open(main, module("a"), { kind: "page", pageId: "page" });
		tabs = withDocumentOpened(tabs, module("a"), { newTab: true }).tabs;
		tabs = withTabLayerPath(tabs, "board:a#2", "a/caller");
		const next = withBoardTabPosition(tabs, "board:a#2", "b", "b/function");

		expect(next[3]).toEqual({
			key: "board:a#2",
			doc: { kind: "board", fileId: "b" },
			layerPath: "b/function",
		});
		expect(next[0]).toBe(tabs[0]);
		expect(next[1]).toBe(tabs[1]);
		expect(next[2]).toBe(tabs[2]);
		expect(tabs[3].doc).toEqual(module("a"));
		expect(tabs[3].layerPath).toBe("a/caller");
	});

	test("returning to main clears the previous file and layer together", () => {
		const tabs = withTabLayerPath(open(module("a")), "board:a", "a/function");
		const next = withBoardTabPosition(tabs, "board:a", "main", undefined);
		expect(next[0]).toEqual({ key: "board:a", doc: main });
		expect("layerPath" in next[0]).toBe(false);
	});

	test("moving within a file preserves its document identity", () => {
		const tabs = open(module("a"));
		const next = withBoardTabPosition(tabs, "board:a", "a", "a/function");
		expect(next[0]).not.toBe(tabs[0]);
		expect(next[0].doc).toBe(tabs[0].doc);
		expect(next[0].layerPath).toBe("a/function");
	});

	test("an unchanged canvas position preserves the tab collection", () => {
		const tabs = withTabLayerPath(open(module("a")), "board:a", "a/function");
		expect(withBoardTabPosition(tabs, "board:a", "a", "a/function")).toBe(tabs);
	});

	test("ignores missing tabs and document tabs", () => {
		const tabs = open(main, { kind: "page", pageId: "page" });
		expect(withBoardTabPosition(tabs, "missing", "a", "a/function")).toBe(tabs);
		expect(withBoardTabPosition(tabs, "page:page", "a", "a/function")).toBe(
			tabs,
		);
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

describe("pinned tabs", () => {
	const keys = (tabs: readonly IEditorTab[]) => tabs.map((tab) => tab.key);

	test("pinning moves a tab to the end of the pinned group", () => {
		let tabs = open(main, module("a"), module("b"), module("c"));
		tabs = withTabPinned(tabs, "board:c", true);
		tabs = withTabPinned(tabs, "board:a", true);
		expect(keys(tabs)).toEqual(["board:c", "board:a", "board:main", "board:b"]);
		expect(tabs.filter((tab) => tab.pinned).map((tab) => tab.key)).toEqual([
			"board:c",
			"board:a",
		]);
	});

	test("unpinning puts a tab first among the unpinned ones", () => {
		let tabs = open(main, module("a"), module("b"));
		tabs = withTabPinned(tabs, "board:a", true);
		tabs = withTabPinned(tabs, "board:b", true);
		tabs = withTabPinned(tabs, "board:a", false);
		expect(keys(tabs)).toEqual(["board:b", "board:a", "board:main"]);
		expect(tabByKey(tabs, "board:a")).not.toHaveProperty("pinned");
	});

	// Opening next to a pinned tab would otherwise land inside the pinned group and the
	// strip would render it in a lane its flag does not belong to.
	test("a document opened beside a pinned tab lands after the group", () => {
		let tabs = open(main, module("a"), module("b"));
		tabs = withTabPinned(tabs, "board:b", true);
		const result = withDocumentOpened(tabs, module("c"), { after: "board:b" });
		expect(keys(result.tabs)).toEqual([
			"board:b",
			"board:c",
			"board:main",
			"board:a",
		]);
	});

	test("a split of a pinned tab is an ordinary tab", () => {
		const tabs = withTabPinned(open(main, module("a")), "board:a", true);
		const result = withDocumentOpened(tabs, module("a"), {
			newTab: true,
			after: "board:a",
		});
		expect(tabByKey(result.tabs, result.key)?.pinned).toBeUndefined();
		expect(keys(result.tabs)).toEqual(["board:a", "board:a#2", "board:main"]);
	});

	test("the canvas position keeps the pin and colour", () => {
		let tabs = withTabPinned(open(main, module("a")), "board:a", true);
		tabs = withTabColor(tabs, "board:a", "teal");
		tabs = withBoardTabPosition(tabs, "board:a", "a", "a/fn");
		tabs = withTabLayerPath(tabs, "board:a", undefined);
		expect(tabByKey(tabs, "board:a")).toMatchObject({
			pinned: true,
			color: "teal",
		});
	});
});

describe("withTabColor", () => {
	test("sets and clears a colour", () => {
		let tabs = open(main, module("a"));
		tabs = withTabColor(tabs, "board:a", "blue");
		expect(tabByKey(tabs, "board:a")?.color).toBe("blue");
		tabs = withTabColor(tabs, "board:a", undefined);
		expect(tabByKey(tabs, "board:a")).not.toHaveProperty("color");
	});
});

describe("pin and colour persistence", () => {
	test("round-trips pin and colour", () => {
		let tabs = open(main, module("a"));
		tabs = withTabPinned(tabs, "board:a", true);
		tabs = withTabColor(tabs, "board:main", "red");
		expect(deserializeTabs(serializeTabs(tabs, null)).tabs).toEqual(tabs);
	});

	test("restores pinned tabs first and drops an unknown colour", () => {
		const raw = JSON.stringify({
			v: 1,
			tabs: [
				{ key: "board:main", doc: main, color: "chartreuse" },
				{ key: "board:a", doc: module("a"), pinned: true, color: "pink" },
				{ key: "board:b", doc: module("b"), pinned: "yes" },
			],
		});
		expect(deserializeTabs(raw).tabs).toEqual([
			{ key: "board:a", doc: module("a"), pinned: true, color: "pink" },
			{ key: "board:main", doc: main },
			{ key: "board:b", doc: module("b") },
		]);
	});

	test("an unpinned main tab joins after the pinned group", () => {
		const restored = [
			{ key: "board:a", doc: module("a"), pinned: true },
		] satisfies IEditorTab[];
		expect(
			withPinnedFirst([{ key: "board:main", doc: main }, ...restored]).map(
				(tab) => tab.key,
			),
		).toEqual(["board:a", "board:main"]);
	});
});
