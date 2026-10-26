import { describe, expect, test } from "bun:test";
import type { IBoardSummary } from "../../../lib/schema/flow/board-summary";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { PageListItem } from "../../../state/backend-state/page-state";
import { buildFlowRow, indexPages } from "./flows-overview-model";

const board = (id: string) =>
	({
		id,
		name: id,
		description: "",
		nodeCount: 0,
		connectionCount: 0,
		variableCount: 0,
		layerCount: 0,
		pages: [],
	}) as unknown as IBoardSummary;

const index = (pages: PageListItem[]) => indexPages("app", pages);

const page = (pageId: string, boardId?: string): PageListItem => ({
	appId: "app",
	pageId,
	boardId,
	name: `Page ${pageId}`,
});

const event = (id: string, boardId: string, defaultPageId?: string): IEvent =>
	({
		id,
		name: id,
		board_id: boardId,
		default_page_id: defaultPageId ?? null,
		node_id: defaultPageId ? "" : "start",
	}) as unknown as IEvent;

const pageIds = (pages: PageListItem[]) => pages.map((item) => item.pageId);

describe("flow row pages", () => {
	test("lists a page an event binds to the board even when another board owns it", () => {
		const pages = index([page("setup", "editor"), page("own", "studio")]);
		const row = buildFlowRow(
			board("studio"),
			[event("route", "studio", "setup")],
			pages,
		);

		expect(pageIds(row.pages)).toEqual(["own", "setup"]);
		expect(row.pages.find((item) => item.pageId === "setup")?.boardId).toBe(
			"editor",
		);
		expect(row.ownedPageCount).toBe(1);
	});

	test("does not repeat an owned page its own route binds", () => {
		const row = buildFlowRow(
			board("studio"),
			[event("route", "studio", "own"), event("again", "studio", "own")],
			index([page("own", "studio")]),
		);

		expect(pageIds(row.pages)).toEqual(["own"]);
		expect(row.ownedPageCount).toBe(1);
	});

	test("names a bound page the listing does not carry as unavailable", () => {
		const row = buildFlowRow(
			board("studio"),
			[event("route", "studio", "gone")],
			index([]),
		);

		expect(row.pages).toEqual([
			{ appId: "app", pageId: "gone", name: "gone", unavailable: true },
		]);
		expect(row.ownedPageCount).toBe(0);
	});

	test("flags nothing before the page listing loads", () => {
		const row = buildFlowRow(
			board("studio"),
			[event("route", "studio", "setup")],
			undefined,
		);

		expect(row.pages).toEqual([]);
	});

	test("ignores workflow events and events bound to other boards", () => {
		const row = buildFlowRow(
			board("studio"),
			[event("chat", "studio"), event("elsewhere", "editor", "setup")],
			index([page("setup", "editor")]),
		);

		expect(row.pages).toEqual([]);
	});
});
