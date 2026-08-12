import { describe, expect, test } from "bun:test";
import {
	pageEventPersistenceReset,
	resolveAppEventTarget,
	resolveAppEventType,
} from "./app-event-target";

describe("FlowPilot app Event targets", () => {
	test("forces page targets to the dedicated page type", () => {
		expect(resolveAppEventType({ pageId: "page-1" })).toBe("page");
		expect(
			resolveAppEventType({
				pageId: "page-1",
				requestedEventType: "generic_form",
				existingEventType: "quick_action",
				supportedWorkflowEventTypes: ["generic_form"],
				defaultWorkflowEventType: "generic_form",
			}),
		).toBe("page");
	});

	test("keeps compatible workflow type selection behavior", () => {
		expect(
			resolveAppEventType({
				requestedEventType: "cron",
				supportedWorkflowEventTypes: ["quick_action", "cron"],
				defaultWorkflowEventType: "quick_action",
			}),
		).toBe("cron");
		expect(
			resolveAppEventType({
				existingEventType: "cron",
				supportedWorkflowEventTypes: ["quick_action", "cron"],
				defaultWorkflowEventType: "quick_action",
			}),
		).toBe("cron");
		expect(
			resolveAppEventType({
				existingEventType: "simple_chat",
				supportedWorkflowEventTypes: ["quick_action", "cron"],
				defaultWorkflowEventType: "quick_action",
			}),
		).toBe("quick_action");
	});

	test("separates page owner metadata from workflow entry bindings", () => {
		expect(
			resolveAppEventTarget({
				requestedPageId: " page-1 ",
				requestedBoardId: " board-1 ",
			}),
		).toEqual({
			ok: true,
			kind: "page",
			pageId: "page-1",
			boardId: "board-1",
			nodeId: "",
			preserveExistingPageMetadata: false,
		});
		expect(
			resolveAppEventTarget({
				requestedPageId: "page-1",
				requestedNodeId: "node-1",
			}),
		).toEqual(
			expect.objectContaining({
				ok: false,
				message: expect.stringContaining("different Event kinds"),
			}),
		);
	});

	test("repairs legacy page targets without inheriting stale workflow metadata", () => {
		expect(
			resolveAppEventTarget({
				existingPageId: "page-1",
				existingBoardId: "page-board",
				existingNodeId: "stale-node",
			}),
		).toEqual({
			ok: true,
			kind: "page",
			pageId: "page-1",
			boardId: "page-board",
			nodeId: "",
			preserveExistingPageMetadata: true,
		});
		expect(
			resolveAppEventTarget({
				requestedPageId: "page-2",
				existingPageId: "page-1",
				existingBoardId: "stale-board",
				existingNodeId: "stale-node",
			}),
		).toEqual({
			ok: true,
			kind: "page",
			pageId: "page-2",
			boardId: "",
			nodeId: "",
			preserveExistingPageMetadata: false,
		});
		expect(
			resolveAppEventTarget({
				requestedPageId: "page-1",
				requestedBoardId: "new-board",
				existingPageId: "page-1",
				existingBoardId: "old-board",
			}),
		).toEqual({
			ok: true,
			kind: "page",
			pageId: "page-1",
			boardId: "new-board",
			nodeId: "",
			preserveExistingPageMetadata: false,
		});
		expect(pageEventPersistenceReset("page-1")).toEqual({
			nodeId: "",
			config: [],
			inputs: [],
			canary: null,
		});
	});

	test("keeps workflow targets unchanged and rejects whitespace-only targets", () => {
		expect(
			resolveAppEventTarget({
				requestedBoardId: "board-1",
				requestedNodeId: "node-1",
			}),
		).toEqual({
			ok: true,
			kind: "workflow",
			pageId: "",
			boardId: "board-1",
			nodeId: "node-1",
			preserveExistingPageMetadata: false,
		});
		expect(
			resolveAppEventTarget({
				requestedPageId: "   ",
				requestedBoardId: " ",
				requestedNodeId: "\t",
			}),
		).toEqual(
			expect.objectContaining({
				ok: false,
				message: expect.stringContaining("Provide page_id"),
			}),
		);
	});
});
