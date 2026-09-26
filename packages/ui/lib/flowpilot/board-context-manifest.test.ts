import { beforeEach, describe, expect, it, vi } from "vitest";
import type { IPage } from "../../state/backend-state/page-state";
import {
	buildFlowPilotBoardContextAugmentation,
	clearFlowPilotBoardContextManifestCacheForTests,
} from "./board-context-manifest";
import { inspectUiPageList } from "./ui-page-inspection";

describe("FlowPilot board context augmentation", () => {
	beforeEach(() => clearFlowPilotBoardContextManifestCacheForTests());

	it("keeps failed page reads incomplete and recovers on a fresh inspection", async () => {
		let unavailable = true;
		const readPage = vi.fn(
			async ({ pageId }: { pageId: string }): Promise<IPage> => {
				if (pageId === "page-missing" && unavailable) {
					throw new Error(
						`Backend details that should not be forwarded: ${"x".repeat(5000)}`,
					);
				}
				return {
					id: pageId,
					name: pageId,
					createdAt: "2026-01-01T00:00:00.000Z",
					updatedAt: "2026-01-01T00:00:00.000Z",
					layoutType: "stack",
					content: [],
					components:
						pageId === "page-missing"
							? []
							: [
									{
										id: "heading",
										component: {
											id: "heading",
											type: "text",
											content: { literalString: "Ready" },
										},
									},
								],
				};
			},
		);
		const execute = async (tool: string) => {
			if (tool === "database_tool")
				return { project_tables: [], user_tables: [] };
			if (tool !== "ui_inspect") return { items: [] };
			return {
				status: "ok",
				pages: await inspectUiPageList(
					[
						{ pageId: "page-ready", name: "Ready page", boardId: "board-1" },
						{
							pageId: "page-missing",
							name: "Unavailable page",
							boardId: "board-1",
						},
					],
					readPage,
				),
				widgets: [{ widget_id: "widget-1" }],
			};
		};

		const failed = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"same-source",
		);
		expect(failed.ui.complete).toBe(false);
		expect(failed.ui.truncated).toBe(false);
		expect(failed.ui.errors).toEqual([
			"Page page-missing: Page details could not be loaded.",
		]);
		expect(failed.ui.pages).toEqual([
			expect.objectContaining({
				page_id: "page-ready",
				element_refs: ["page-ready/heading"],
			}),
			{
				page_id: "page-missing",
				name: "Unavailable page",
				element_refs: [],
				error: "Page details could not be loaded.",
			},
		]);
		expect(failed.ui.widgets).toEqual([{ widget_id: "widget-1" }]);
		expect(failed.data.complete).toBe(true);
		expect(failed.storage.complete).toBe(true);
		expect(JSON.stringify(failed)).not.toContain("Backend details");

		unavailable = false;
		const recovered = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"same-source",
		);
		expect(recovered.ui.complete).toBe(true);
		expect(recovered.ui.errors).toEqual([]);
		expect(recovered.ui.pages[1]).toMatchObject({
			page_id: "page-missing",
			element_refs: [],
		});
		expect(recovered.ui.pages[1]).not.toHaveProperty("error");
		expect(readPage).toHaveBeenCalledTimes(4);
	});

	it("honors an incomplete UI result even when detail errors are absent", async () => {
		const manifest = await buildFlowPilotBoardContextAugmentation(
			async (tool) =>
				tool === "ui_inspect"
					? { pages: [], widgets: [], complete: false }
					: tool === "database_tool"
						? { project_tables: [], user_tables: [] }
						: { items: [] },
			"app-1",
			"board-1",
			"source",
		);
		expect(manifest.ui.complete).toBe(false);
		expect(manifest.ui.errors).toEqual([
			"UI inspection did not return complete page details.",
		]);
	});

	it("collects one shared deterministic inventory without reading sample rows", async () => {
		const execute = vi.fn(
			async (toolName: string, args: Record<string, unknown>) => {
				if (toolName === "database_tool" && args.operation === "list_tables") {
					return { project_tables: ["zeta", "alpha"], user_tables: ["mine"] };
				}
				if (
					toolName === "database_tool" &&
					args.operation === "describe_table"
				) {
					return {
						schema: { fields: [{ name: "id", type: "string" }] },
						indices: [{ name: "id_idx" }],
					};
				}
				if (toolName === "ui_inspect") {
					return { pages: [{ id: "page-1" }], widgets: [{ id: "widget-1" }] };
				}
				return { items: [{ path: `${String(args.user_scoped)}.txt` }] };
			},
		);

		const first = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"revision-1",
		);

		expect(first.data.complete).toBe(true);
		expect(first.data.truncated).toBe(false);
		expect(first.ui.complete).toBe(true);
		expect(first.storage.complete).toBe(true);
		expect(first.data.tables.map((table) => table.table_name)).toEqual([
			"alpha",
			"zeta",
			"mine",
		]);
		const describeCalls = execute.mock.calls.filter(
			([toolName, args]) =>
				toolName === "database_tool" && args.operation === "describe_table",
		);
		expect(describeCalls).toHaveLength(3);
		for (const [, args] of describeCalls) {
			expect(args).toMatchObject({ include_sample: false });
			expect(args).not.toHaveProperty("limit");
		}
	});

	it("recollects live context when the legacy cache identity is reused", async () => {
		let inventoryRevision = 0;
		const execute = vi.fn(
			async (toolName: string, args: Record<string, unknown>) => {
				if (toolName === "database_tool" && args.operation === "list_tables") {
					inventoryRevision += 1;
					return { project_tables: [`table-v${inventoryRevision}`] };
				}
				if (
					toolName === "database_tool" &&
					args.operation === "describe_table"
				) {
					return { schema: { fields: [] }, indices: [] };
				}
				if (toolName === "ui_inspect") return { pages: [], widgets: [] };
				return { items: [] };
			},
		);

		const first = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"unchanged-board-counts",
		);
		const second = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"unchanged-board-counts",
		);

		expect(first.data.tables[0]?.table_name).toBe("table-v1");
		expect(second.data.tables[0]?.table_name).toBe("table-v2");
		expect(second).not.toBe(first);
		expect(inventoryRevision).toBe(2);
	});

	it("marks every collection cap incomplete with explicit truncation metadata", async () => {
		const execute = vi.fn(
			async (toolName: string, args: Record<string, unknown>) => {
				if (toolName === "database_tool" && args.operation === "list_tables") {
					return { project_tables: ["wide-table"], user_tables: [] };
				}
				if (
					toolName === "database_tool" &&
					args.operation === "describe_table"
				) {
					return {
						schema: {
							fields: Array.from({ length: 100 }, (_, index) => ({
								name: `field-${index}`,
							})),
						},
						indices: Array.from({ length: 50 }, (_, index) => ({
							name: `index-${index}`,
						})),
					};
				}
				if (toolName === "ui_inspect") {
					return {
						pages: Array.from({ length: 100 }, (_, index) => ({ id: index })),
						widgets: Array.from({ length: 97 }, (_, index) => ({ id: index })),
					};
				}
				return {
					items: Array.from({ length: 100 }, (_, index) => ({
						path: `${String(args.user_scoped)}-${index}`,
					})),
				};
			},
		);

		const manifest = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"revision-wide",
		);

		expect(manifest.truncated).toBe(true);
		expect(manifest.data.complete).toBe(false);
		expect(manifest.data.truncated).toBe(true);
		expect(manifest.data.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					resource: "schema_fields",
					available: 100,
					included: 96,
				}),
				expect.objectContaining({
					resource: "indices",
					available: 50,
					included: 48,
				}),
			]),
		);
		expect(manifest.ui.complete).toBe(false);
		expect(manifest.ui.pages).toHaveLength(96);
		expect(manifest.ui.widgets).toHaveLength(96);
		expect(manifest.ui.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({ resource: "pages", available: 100 }),
				expect.objectContaining({ resource: "widgets", available: 97 }),
			]),
		);
		expect(manifest.storage.complete).toBe(false);
		expect(manifest.storage.project_items).toHaveLength(96);
		expect(manifest.storage.user_items).toHaveLength(96);
		expect(manifest.storage.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({ resource: "project_items", available: 100 }),
				expect.objectContaining({ resource: "user_items", available: 100 }),
			]),
		);
		expect(manifest.data.errors).toEqual(
			expect.arrayContaining([expect.stringContaining("schema_fields")]),
		);
	});

	it("marks partial schema discovery incomplete and enforces the transport byte ceiling", async () => {
		const huge = "x".repeat(20_000);
		const execute = vi.fn(
			async (toolName: string, args: Record<string, unknown>) => {
				if (toolName === "database_tool" && args.operation === "list_tables") {
					return {
						project_tables: Array.from(
							{ length: 50 },
							(_, index) => `table-${index}`,
						),
						user_tables: [],
					};
				}
				if (
					toolName === "database_tool" &&
					args.operation === "describe_table"
				) {
					if (args.table_name === "table-0")
						throw new Error("schema unavailable");
					return {
						schema: {
							fields: Array.from({ length: 96 }, (_, index) => ({
								name: `field-${index}`,
								description: huge,
							})),
						},
						indices: [],
					};
				}
				if (toolName === "ui_inspect") {
					return { pages: [{ id: "page-1", payload: huge }], widgets: [] };
				}
				return { items: [{ location: "large.bin", payload: huge }] };
			},
		);

		const manifest = await buildFlowPilotBoardContextAugmentation(
			execute,
			"app-1",
			"board-1",
			"revision-large",
		);

		expect(manifest.truncated).toBe(true);
		expect(manifest.data.complete).toBe(false);
		expect(manifest.ui.complete).toBe(false);
		expect(manifest.storage.complete).toBe(false);
		expect(
			manifest.data.errors.some((error) => error.includes("table-0")),
		).toBe(true);
		expect(manifest.data.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					resource: "schema_field_details",
					reason: "transport_summarization",
				}),
			]),
		);
		expect(manifest.ui.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					resource: "pages",
					reason: "transport_summarization",
				}),
			]),
		);
		expect(manifest.storage.truncations).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					resource: "project_items",
					reason: "transport_summarization",
				}),
			]),
		);
		expect(manifest.storage.project_items[0]).toMatchObject({
			location: "large.bin",
		});
		expect(
			new TextEncoder().encode(JSON.stringify(manifest)).length,
		).toBeLessThanOrEqual(160_000);
	});
});
