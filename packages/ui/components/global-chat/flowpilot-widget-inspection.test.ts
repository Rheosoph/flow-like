import { describe, expect, test } from "bun:test";
import type { IPage } from "../../state/backend-state/page-state";
import { inspectFlowPilotWidgetPage } from "./flowpilot-widget-inspection";

function savedPage(): IPage {
	return {
		id: "intake",
		boardId: "board",
		name: "Intake",
		route: "/intake",
		layoutType: "freeform",
		createdAt: "2026-09-10T00:00:00Z",
		updatedAt: "2026-09-10T01:00:00Z",
		content: [],
		components: [
			{
				id: "root",
				component: {
					id: "root",
					type: "column",
					children: { explicitList: ["summary", "submit", "result"] },
				},
			},
			{
				id: "summary",
				component: {
					id: "summary",
					type: "textField",
					value: { path: "/summary" },
					label: { literalString: "Summary" },
				},
			},
			{
				id: "submit",
				component: {
					id: "submit",
					type: "button",
					label: { literalString: "Submit" },
					eventHandlers: {
						click: [
							{
								name: "workflow_event",
								context: {
									nodeId: "submit-entry",
									input: { source: "intake" },
								},
							},
						],
					},
				},
			},
			{
				id: "result",
				component: {
					id: "result",
					type: "text",
					content: { literalString: "" },
				},
			},
		],
	};
}

describe("persisted FlowPilot page inspection", () => {
	test("returns exact component values, action contexts and lifecycle absence from one authoritative read", async () => {
		const page = savedPage();
		const before = JSON.stringify(page);
		const reads: unknown[][] = [];
		const result = await inspectFlowPilotWidgetPage(
			{
				getPageAuthoritative: async (...args) => {
					reads.push(args);
					return page;
				},
			},
			{ appId: "app", pageId: "intake", boardId: "board" },
		);
		expect(reads).toEqual([["app", "intake", "board"]]);
		expect(result).toMatchObject({
			status: "ok",
			mode: "inspect",
			read_only: true,
			source: "authoritative_persisted_page",
			page: {
				id: "intake",
				board_id: "board",
				component_count: 4,
				components: page.components,
				no_cache: false,
				lifecycle: {
					on_load_event_id: null,
					on_unload_event_id: null,
					on_interval_event_id: null,
					on_interval_seconds: null,
				},
			},
			coverage: { complete: true, truncated_fields: [] },
		});
		expect(result).not.toHaveProperty("applied");
		expect(JSON.stringify(page)).toBe(before);
	});

	test("reports actual lifecycle bindings and persisted content", async () => {
		const page = savedPage();
		page.onLoadEventId = "load-entry";
		page.onUnloadEventId = "unload-entry";
		page.onIntervalEventId = "poll-entry";
		page.onIntervalSeconds = 30;
		page.noCache = true;
		page.content = [{ ComponentRef: "root" }];
		const result = await inspectFlowPilotWidgetPage(
			{ getPageAuthoritative: async () => page },
			{ appId: "app", pageId: "intake" },
		);
		expect(result).toMatchObject({
			page: {
				content: page.content,
				no_cache: true,
				lifecycle: {
					on_load_event_id: "load-entry",
					on_unload_event_id: "unload-entry",
					on_interval_event_id: "poll-entry",
					on_interval_seconds: 30,
				},
			},
		});
	});

	test("does not read an ambient page when an exact target is missing", async () => {
		let reads = 0;
		const pageState = {
			getPageAuthoritative: async () => {
				reads++;
				return savedPage();
			},
		};
		for (const target of [
			{ appId: "", pageId: "intake" },
			{ appId: "app", pageId: "" },
		]) {
			expect(await inspectFlowPilotWidgetPage(pageState, target)).toMatchObject(
				{ status: "error", code: "FLOWPILOT_WIDGET_INSPECT_TARGET_REQUIRED" },
			);
		}
		expect(reads).toBe(0);
	});

	test("refuses authoritative read failures, wrong identities and wrong board ownership", async () => {
		for (const page of [
			{ ...savedPage(), id: "other" },
			{ ...savedPage(), boardId: "other" },
		]) {
			expect(
				await inspectFlowPilotWidgetPage(
					{ getPageAuthoritative: async () => page },
					{ appId: "app", pageId: "intake", boardId: "board" },
				),
			).toMatchObject({
				status: "error",
				code: "FLOWPILOT_WIDGET_INSPECT_TARGET_MISMATCH",
			});
		}
		const failed = await inspectFlowPilotWidgetPage(
			{
				getPageAuthoritative: async () => {
					throw new Error("private backend details");
				},
			},
			{ appId: "app", pageId: "intake" },
		);
		expect(failed).toMatchObject({
			status: "error",
			code: "FLOWPILOT_WIDGET_INSPECT_READ_FAILED",
		});
		expect(JSON.stringify(failed)).not.toContain("private backend details");
	});

	test("omits oversized raw components and reports bounded incomplete coverage", async () => {
		const page = savedPage();
		page.components[3] = {
			id: "result",
			component: {
				id: "result",
				type: "text",
				content: { literalString: "x".repeat(80_000) },
			},
		};
		const result = await inspectFlowPilotWidgetPage(
			{ getPageAuthoritative: async () => page },
			{ appId: "app", pageId: "intake" },
		);
		expect(result).toMatchObject({
			page: { component_count: 4, components: page.components.slice(0, 3) },
			coverage: { complete: false, truncated_fields: ["components"] },
		});
		expect(JSON.stringify(result).length).toBeLessThan(45_000);
	});

	test("caps item counts independently from text size", async () => {
		const page = savedPage();
		page.components = Array.from({ length: 100 }, (_, index) => ({
			id: `text-${index}`,
			component: {
				id: `text-${index}`,
				type: "text",
				content: { literalString: "hello" },
			},
		}));
		const result = await inspectFlowPilotWidgetPage(
			{ getPageAuthoritative: async () => page },
			{ appId: "app", pageId: "intake" },
		);
		expect(result).toMatchObject({
			page: { component_count: 100 },
			coverage: { complete: false, truncated_fields: ["components"] },
		});
		if (result.status !== "ok") throw new Error("Expected inspected page");
		expect(result.page.components).toHaveLength(64);
	});
});
