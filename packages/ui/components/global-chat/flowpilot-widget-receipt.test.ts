import { describe, expect, test } from "bun:test";
import type { IPage } from "../../state/backend-state/page-state";
import { normalizePageForPersistence } from "../a2ui/style-normalization";
import {
	assertWidgetPageReadback,
	persistFlowPilotWidgetPage,
	stagedFlowPilotWidgetReceipt,
} from "./flowpilot-widget-receipt";

function page(): IPage {
	return {
		id: "page",
		boardId: "board",
		name: "Intake",
		route: "/intake",
		layoutType: "freeform",
		content: [],
		createdAt: "2026-09-09T00:00:00Z",
		updatedAt: "2026-09-09T00:00:00Z",
		components: [
			{
				id: "root",
				component: {
					id: "root",
					type: "column",
					children: { explicitList: ["submit"] },
				},
			},
			{
				id: "submit",
				component: {
					id: "submit",
					type: "button",
					label: { literalString: "Submit" },
					eventHandlers: {
						click: [{ name: "workflow_event", context: { nodeId: "entry" } }],
					},
				},
			},
		],
	};
}

describe("UI host persistence receipt", () => {
	test("waits for the host save and authoritative read before replacing the specialist's earlier claim", async () => {
		const calls: string[] = [];
		const expected = page();
		const specialistMessage = "Read-back reports this page does not exist.";
		const result = await persistFlowPilotWidgetPage(
			{
				async updatePage(appId, input) {
					expect(appId).toBe("app");
					expect(input).toBe(expected);
					await Promise.resolve();
					calls.push("saved");
				},
				async getPageAuthoritative(appId, pageId, boardId) {
					expect([appId, pageId, boardId]).toEqual(["app", "page", "board"]);
					expect(calls).toEqual(["saved"]);
					calls.push("read");
					return structuredClone(expected);
				},
			},
			"app",
			expected,
			"create",
			specialistMessage,
		);
		expect(calls).toEqual(["saved", "read"]);
		expect(result).toMatchObject({
			status: "ok",
			applied: true,
			staged: false,
			persistence_verified: true,
			component_count: 2,
			specialist_message: specialistMessage,
			specialist_message_timing: "before_host_apply",
		});
		expect(result.message).toContain("Created page");
		expect(result.message).not.toContain(specialistMessage);
	});

	test("reports failed authoritative reads as unknown and preserves the exact target", async () => {
		const result = await persistFlowPilotWidgetPage(
			{
				async updatePage() {},
				async getPageAuthoritative() {
					throw new Error("authority unavailable");
				},
			},
			"app",
			page(),
			"edit",
			"Everything is saved.",
		);
		expect(result).toMatchObject({
			status: "error",
			outcome: "unknown",
			persistence_verified: false,
			app_id: "app",
			board_id: "board",
			page: { id: "page" },
		});
		expect(result).not.toHaveProperty("applied");
		expect(result.message).toContain("authority unavailable");
		expect(result.message).toContain("Read this exact page before retrying");
	});

	test("does not claim persistence when a save failed after a possible local write", async () => {
		let reads = 0;
		const result = await persistFlowPilotWidgetPage(
			{
				async updatePage() {
					throw new Error("remote save failed");
				},
				async getPageAuthoritative() {
					reads++;
					return page();
				},
			},
			"app",
			page(),
			"create",
		);
		expect(result).toMatchObject({ status: "error", outcome: "unknown" });
		expect(reads).toBe(0);
	});

	test("rejects wrong identity, content, route, name and action bindings", () => {
		const expected = page();
		const altered = (change: (value: IPage) => void) => {
			const result = structuredClone(expected);
			change(result);
			return result;
		};
		for (const actual of [
			{ ...expected, id: "other" },
			{ ...expected, boardId: "other" },
			{ ...expected, name: "other" },
			{ ...expected, route: "/other" },
			{ ...expected, components: [] },
			altered((value) => {
				value.components[1].component.eventHandlers = {};
			}),
			altered((value) => {
				value.components[0].component.children = { explicitList: [] };
			}),
		])
			expect(() => assertWidgetPageReadback(expected, actual)).toThrow();
	});

	test("accepts normalized styles and server-added default fields", () => {
		const expected = page();
		expected.components[0].style = { padding: { value: "1rem" } } as never;
		const actual = normalizePageForPersistence(structuredClone(expected));
		actual.updatedAt = "2026-09-09T00:00:01Z";
		actual.components[1].component.hidden = { literalBool: false };
		expect(() => assertWidgetPageReadback(expected, actual)).not.toThrow();
	});

	test("never presents staged builder output as a saved page", () => {
		const result = stagedFlowPilotWidgetReceipt(4, "I saved the page.");
		expect(result).toMatchObject({
			status: "ok",
			staged: true,
			applied: false,
			persistence_verified: false,
			specialist_message: "I saved the page.",
		});
		expect(result.message).toContain("Apply the pending changes to save them");
	});
});
