import { describe, expect, test } from "bun:test";
import type { IPage, IPageState } from "../../state/backend-state/page-state";
import type { SurfaceComponent } from "../a2ui/types";
import {
	applyComponentsToPageTree,
	assertDetachedWriteSafe,
	findPersistedPage,
	pageWithAppliedComponents,
} from "./detached-page-edit";

const component = (
	id: string,
	inner: Record<string, unknown> = { type: "text" },
	extra: Record<string, unknown> = {},
): SurfaceComponent =>
	({ id, component: inner, ...extra }) as unknown as SurfaceComponent;

const column = (id: string, children: string[]) =>
	component(id, { type: "column", children: { explicitList: children } });

const inner = (candidate: SurfaceComponent) =>
	candidate.component as unknown as Record<string, unknown>;

const childrenOf = (components: SurfaceComponent[], id: string) => {
	const found = components.find((candidate) => candidate.id === id);
	const children = inner(found as SurfaceComponent).children as {
		explicitList: string[];
	};
	return children.explicitList;
};

const page = (overrides: Partial<IPage> = {}): IPage => ({
	id: "page-1",
	name: "Knowledge Sources",
	route: "/knowledge-sources",
	content: [],
	layoutType: "freeform",
	components: [],
	createdAt: "2026-01-01T00:00:00.000Z",
	updatedAt: "2026-01-02T00:00:00.000Z",
	boardId: "board-1",
	...overrides,
});

const pageStateOf = (pages: IPage[]) =>
	({
		async getPages(appId: string, boardId?: string) {
			return pages
				.filter((entry) => !boardId || entry.boardId === boardId)
				.map((entry) => ({
					appId,
					pageId: entry.id,
					boardId: entry.boardId,
					name: entry.name,
				}));
		},
		async getPage(_appId: string, pageId: string) {
			const found = pages.find((entry) => entry.id === pageId);
			if (!found) throw new Error(`Page not found: ${pageId}`);
			return found;
		},
	}) as unknown as IPageState;

describe("detached page component merge", () => {
	test("upserts by id and never deletes what the copilot did not mention", () => {
		const existing = [
			column("root", ["kept", "replaced"]),
			component("kept"),
			component("replaced", { type: "text", value: "old" }),
		];
		const merged = applyComponentsToPageTree(existing, [
			component("replaced", { type: "text", value: "new" }),
		]);
		expect(merged.map((entry) => entry.id)).toEqual([
			"root",
			"kept",
			"replaced",
		]);
		expect(inner(merged[2]).value).toBe("new");
	});

	test("carries over fields the incoming component omits", () => {
		const merged = applyComponentsToPageTree(
			[
				column("root", ["a"]),
				component("a", { type: "text" }, { style: { className: "p-4" } }),
			],
			[component("a", { type: "text", value: "next" })],
		);
		expect(
			(merged[1] as unknown as { style?: { className: string } }).style,
		).toEqual({ className: "p-4" });
	});

	test("links new top-level components into the root, once", () => {
		const merged = applyComponentsToPageTree(
			[column("root", ["existing"]), component("existing")],
			[column("card", ["label"]), component("label"), component("extra")],
		);
		// "label" is a child of an incoming component, so it is not linked to the root itself.
		expect(childrenOf(merged, "root")).toEqual(["existing", "card", "extra"]);
		expect(merged.map((entry) => entry.id)).toEqual([
			"root",
			"existing",
			"card",
			"label",
			"extra",
		]);
	});

	test("does not duplicate a top-level id the root already lists", () => {
		const merged = applyComponentsToPageTree(
			[column("root", ["card"]), component("card")],
			[component("card", { type: "text", value: "redesigned" })],
		);
		expect(childrenOf(merged, "root")).toEqual(["card"]);
	});

	test("a self-contained incoming root replaces the child list", () => {
		const merged = applyComponentsToPageTree(
			[column("root", ["old"]), component("old")],
			[column("root", ["fresh"]), component("fresh")],
		);
		// Nothing incoming is top-level, so the builder's root rewrite never runs and the
		// generated root stands. The old component itself is still there, just unlinked.
		expect(childrenOf(merged, "root")).toEqual(["fresh"]);
		expect(merged.map((entry) => entry.id)).toEqual(["root", "old", "fresh"]);
	});

	test("an unreferenced sibling restores the pre-merge child list, as the builder does", () => {
		const merged = applyComponentsToPageTree(
			[column("root", ["old"]), component("old")],
			[column("root", ["fresh"]), component("fresh"), component("loose")],
		);
		// The builder snapshots the root before upserting and writes that snapshot back once
		// anything is linked into it, so the generated root's own list is dropped.
		expect(childrenOf(merged, "root")).toEqual(["old", "loose"]);
	});

	test("seeds a root when the page has none", () => {
		const merged = applyComponentsToPageTree(
			[component("orphan")],
			[component("added")],
		);
		expect(merged[0].id).toBe("root");
		expect(childrenOf(merged, "root")).toEqual(["added"]);
	});

	test("an empty generation leaves the page untouched", () => {
		const existing = [column("root", []), component("a")];
		expect(applyComponentsToPageTree(existing, [])).toBe(existing);
	});
});

describe("detached page lookup", () => {
	test("resolves by id", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageId: "page-1",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({ ok: true, page: { id: "page-1" } });
	});

	test("reports an authoritative miss as a missing page", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageId: "nope",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_NOT_FOUND",
		});
	});

	test("rethrows a transport failure instead of calling it a missing page", async () => {
		const broken = {
			async getPage() {
				throw new Error("network unavailable");
			},
			async getPages() {
				return [];
			},
		} as unknown as IPageState;
		await expect(
			findPersistedPage(broken, "app-a", {
				pageId: "page-1",
				appIdFromSurface: false,
			}),
		).rejects.toThrow("network unavailable");
	});

	test("matches a route regardless of slashes and casing", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			route: "Knowledge Sources",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({ ok: true, page: { id: "page-1" } });
	});

	test("matches a page name case-insensitively", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageName: "  knowledge sources ",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({ ok: true, page: { id: "page-1" } });
	});

	test("refuses to guess between two matches", async () => {
		const pages = [page(), page({ id: "page-2", route: "/knowledge-sources" })];
		const found = await findPersistedPage(pageStateOf(pages), "app-a", {
			route: "/knowledge-sources",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_AMBIGUOUS",
		});
	});

	test("lists the app's pages when nothing matches", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageName: "Missing",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_NOT_FOUND",
		});
		expect((found as { message: string }).message).toContain("page-1");
	});

	test("treats a contradicting identifier as an error, not a preference", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageId: "page-1",
			route: "/somewhere-else",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_TARGET_CONFLICT",
		});
	});

	test("refuses a page owned by another board", async () => {
		const found = await findPersistedPage(pageStateOf([page()]), "app-a", {
			pageId: "page-1",
			boardId: "board-2",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_BOARD_MISMATCH",
		});
	});

	test("refuses a page that has no owning board", async () => {
		const found = await findPersistedPage(
			pageStateOf([page({ boardId: undefined })]),
			"app-a",
			{ pageId: "page-1", appIdFromSurface: false },
		);
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_UNOWNED",
		});
	});

	test("demands a page_id when the app has too many pages to scan", async () => {
		const many = Array.from({ length: 51 }, (_, index) =>
			page({ id: `page-${index}`, route: `/route-${index}` }),
		);
		const found = await findPersistedPage(pageStateOf(many), "app-a", {
			route: "/route-3",
			appIdFromSurface: false,
		});
		expect(found).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_PAGE_SCAN_TOO_LARGE",
		});
	});
});

describe("detached write safety", () => {
	test("passes when the page is untouched", () => {
		expect(assertDetachedWriteSafe(page(), page())).toEqual({ ok: true });
	});

	test("refuses to overwrite a page saved since the snapshot", () => {
		expect(
			assertDetachedWriteSafe(
				page(),
				page({ updatedAt: "2026-01-03T00:00:00.000Z" }),
			),
		).toMatchObject({ ok: false, code: "FLOWPILOT_WIDGET_PAGE_CHANGED" });
	});

	test("refuses a page whose board changed", () => {
		expect(
			assertDetachedWriteSafe(page(), page({ boardId: "board-9" })),
		).toMatchObject({ ok: false, code: "FLOWPILOT_WIDGET_PAGE_BOARD_CHANGED" });
	});
});

describe("detached page payload", () => {
	test("preserves everything the edit does not own", () => {
		const original = page({
			title: "Sources",
			version: [1, 2, 3],
			onLoadEventId: "node-1",
			onIntervalSeconds: 30,
			cache: true,
			widgetRefs: { instance: { id: "w" } as never },
			components: [column("root", [])],
			canvasSettings: { padding: "8px", customCss: ".x{}" },
		});
		const next = pageWithAppliedComponents(
			original,
			[component("added")],
			{ padding: "16px" },
			"2026-02-01T00:00:00.000Z",
		);
		expect(next).toMatchObject({
			id: "page-1",
			name: "Knowledge Sources",
			route: "/knowledge-sources",
			title: "Sources",
			boardId: "board-1",
			createdAt: "2026-01-01T00:00:00.000Z",
			version: [1, 2, 3],
			onLoadEventId: "node-1",
			onIntervalSeconds: 30,
			cache: true,
			updatedAt: "2026-02-01T00:00:00.000Z",
		});
		expect(next.widgetRefs).toEqual(original.widgetRefs);
		// Canvas settings merge, so a copilot that only sets padding cannot wipe custom CSS.
		expect(next.canvasSettings).toEqual({ padding: "16px", customCss: ".x{}" });
		expect(childrenOf(next.components, "root")).toEqual(["added"]);
	});
});
