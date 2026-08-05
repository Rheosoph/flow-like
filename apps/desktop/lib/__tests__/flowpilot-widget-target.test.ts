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
			pageTarget: null,
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
			pageTarget: null,
		});
	});

	test("explicit edit of the open page stages, and of another page detaches", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				boardId: "board-1",
				pageId: "page-1",
				surface: openPage,
			}),
		).toMatchObject({ ok: true, mode: "edit", surface: openPage });
		// A board that the builder is not on names no page, so there is nothing to detach to.
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				boardId: "board-2",
				surface: openPage,
			}),
		).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_EDIT_TARGET_REQUIRED",
		});
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				pageId: "page-2",
				surface: openPage,
			}),
		).toEqual({
			ok: true,
			mode: "edit",
			appId: "app-a",
			surface: null,
			pageTarget: { pageId: "page-2", appIdFromSurface: true },
		});
	});

	test("a persisted page can be edited with no builder open", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				pageId: "page-9",
			}),
		).toEqual({
			ok: true,
			mode: "edit",
			appId: "app-a",
			surface: null,
			pageTarget: { pageId: "page-9", appIdFromSurface: false },
		});
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				route: "/knowledge-sources",
			}),
		).toMatchObject({
			ok: true,
			mode: "edit",
			surface: null,
			pageTarget: { route: "/knowledge-sources" },
		});
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				pageName: "Knowledge Sources",
			}),
		).toMatchObject({
			ok: true,
			mode: "edit",
			pageTarget: { pageName: "Knowledge Sources" },
		});
	});

	test("an explicit app wins over the open builder's app", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-b",
				pageId: "page-1",
				surface: openPage,
			}),
		).toMatchObject({
			ok: true,
			mode: "edit",
			appId: "app-b",
			surface: null,
			pageTarget: { pageId: "page-1", appIdFromSurface: false },
		});
	});

	test("a route request is resolved against storage, never against the open builder", () => {
		// The surface carries no route, so it can never be proven to be the named page.
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				route: "/whatever",
				surface: openPage,
			}),
		).toMatchObject({
			ok: true,
			mode: "edit",
			appId: "app-a",
			surface: null,
			pageTarget: { route: "/whatever", appIdFromSurface: true },
		});
	});

	test("a reusable-widget builder does not block editing a persisted page", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				pageId: "page-1",
				surface: { kind: "widget", appId: "app-a", widgetId: "widget-1" },
			}),
		).toMatchObject({
			ok: true,
			mode: "edit",
			surface: null,
			pageTarget: { pageId: "page-1" },
		});
	});

	test("a named page without an app scope is refused", () => {
		expect(
			resolveFlowPilotWidgetTarget({ mode: "edit", pageId: "page-9" }),
		).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_EDIT_APP_ID_REQUIRED",
		});
	});

	test("an unknown mode is rejected but casing is not", () => {
		expect(
			resolveFlowPilotWidgetTarget({ mode: "explain", surface: openPage }),
		).toMatchObject({ ok: false, code: "FLOWPILOT_WIDGET_MODE_INVALID" });
		expect(
			resolveFlowPilotWidgetTarget({ mode: "EDIT", surface: openPage }),
		).toMatchObject({ ok: true, mode: "edit", surface: openPage });
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

	test("create requires an explicit app and edit requires a target", () => {
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "create",
				pageId: "page-2",
			}),
		).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_CREATE_APP_ID_REQUIRED",
		});
		// An app alone names no page: it must never fall back to "the app's only page".
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
			}),
		).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_EDIT_TARGET_REQUIRED",
		});
		expect(
			resolveFlowPilotWidgetTarget({
				mode: "edit",
				appId: "app-a",
				boardId: "board-1",
			}),
		).toMatchObject({
			ok: false,
			code: "FLOWPILOT_WIDGET_EDIT_TARGET_REQUIRED",
		});
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
