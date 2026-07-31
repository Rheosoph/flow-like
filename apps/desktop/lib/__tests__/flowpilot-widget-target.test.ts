import { creationRequestFingerprint } from "@flow-like/flow-like-ui/components/flowpilot/board-edit-guard";
import {
	flowPilotWidgetCreationScope,
	isFlowPilotPageNotFoundError,
	resolveFlowPilotWidgetTarget,
} from "@flow-like/flow-like-ui/components/global-chat/flowpilot-widget-target";
import { describe, expect, test } from "vitest";

const openPage = {
	kind: "page" as const,
	appId: "app-a",
	boardId: "board-1",
	pageId: "page-1",
};

describe("FlowPilot widget target resolution", () => {
	test("an explicit page target wins over an unrelated open builder", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				appId: "app-a",
				boardId: "board-2",
				pageId: "page-2",
				surface: openPage,
			}),
		).toEqual({
			ok: true,
			mode: "create",
			appId: "app-a",
			surface: null,
		});
	});

	test("an explicit app target cannot be hijacked by another app's builder", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				appId: "app-b",
				pageName: "New page",
				surface: openPage,
			}),
		).toMatchObject({
			ok: true,
			mode: "create",
			appId: "app-b",
		});
	});

	test("omitting persisted-page targets edits the open surface", () => {
		expect(resolveFlowPilotWidgetTarget({ surface: openPage })).toEqual({
			ok: true,
			mode: "edit",
			appId: "app-a",
			surface: openPage,
		});
	});

	test("explicit edit verifies page and board identity", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				boardId: "board-1",
				pageId: "page-1",
				surface: openPage,
			}),
		).toMatchObject({ ok: true, mode: "edit" });
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				boardId: "board-2",
				surface: openPage,
			}),
		).toMatchObject({ ok: false });
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				pageId: "page-2",
				surface: openPage,
			}),
		).toMatchObject({ ok: false });
	});

	test("a reusable-widget builder cannot hijack explicit page creation", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "create",
				appId: "app-a",
				boardId: "board-2",
				pageId: "page-2",
				surface: {
					kind: "widget",
					appId: "app-a",
					widgetId: "widget-1",
				},
			}),
		).toMatchObject({
			ok: true,
			mode: "create",
			appId: "app-a",
			surface: null,
		});
	});

	test("create requires an explicit app and edit requires a live surface", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "create",
				pageId: "page-2",
			}),
		).toMatchObject({ ok: false });
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
			}),
		).toMatchObject({ ok: false });
	});
});

describe("FlowPilot widget creation identity", () => {
	test("only authoritative not-found errors permit page creation", () => {
		expect(
			isFlowPilotPageNotFoundError(new Error("Page not found: page-1")),
		).toBe(true);
		expect(isFlowPilotPageNotFoundError({ status: 404 })).toBe(true);
		expect(isFlowPilotPageNotFoundError({ status: 403 })).toBe(false);
		expect(isFlowPilotPageNotFoundError(new Error("network unavailable"))).toBe(
			false,
		);
	});

	test("scopes equal prose to the exact app, board, and page target", () => {
		const pageOne = flowPilotWidgetCreationScope({
			appId: "app-a",
			boardId: "board-a",
			pageId: "page-1",
			route: "/shared",
		});
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-a",
				boardId: "board-a",
				pageId: "page-1",
				route: "/shared",
			}),
		).toBe(pageOne);
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-a",
				boardId: "board-b",
				pageId: "page-1",
			}),
		).not.toBe(pageOne);
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-a",
				boardId: "board-a",
				pageId: "page-2",
			}),
		).not.toBe(pageOne);
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-b",
				boardId: "board-a",
				pageId: "page-1",
			}),
		).not.toBe(pageOne);
	});

	test("uses route then name as stable fallback targets", () => {
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-a",
				boardId: "board-a",
				route: "/dashboard",
				pageName: "Ignored fallback",
			}),
		).toContain("page:/dashboard");
		expect(
			flowPilotWidgetCreationScope({
				appId: "app-a",
				boardId: "board-a",
				pageName: "Dashboard",
			}),
		).toContain("page:Dashboard");
	});

	test("keeps an explicit idempotency key scoped to its page target", () => {
		const firstScope = flowPilotWidgetCreationScope({
			appId: "app-a",
			boardId: "board-a",
			pageId: "page-1",
		});
		const secondScope = flowPilotWidgetCreationScope({
			appId: "app-a",
			boardId: "board-b",
			pageId: "page-2",
		});
		expect(
			creationRequestFingerprint({
				scope: firstScope,
				instruction: "Build the same layout",
				idempotencyKey: "dashboard",
			}),
		).not.toBe(
			creationRequestFingerprint({
				scope: secondScope,
				instruction: "Build the same layout",
				idempotencyKey: "dashboard",
			}),
		);
	});
});
