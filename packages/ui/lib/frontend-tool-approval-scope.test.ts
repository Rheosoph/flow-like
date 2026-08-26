import { describe, expect, test } from "bun:test";
import { resolveFrontendToolApprovalScope } from "./frontend-tool-approval-scope";

describe("resolveFrontendToolApprovalScope", () => {
	test("scopes page interaction to the argument app and canonical Event", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-1",
				toolName: "interact_app_page",
				arguments: {
					app_id: "orders",
					event_id: "checkout",
					page_id: "checkout-surface",
				},
				approvalSessionKey: "unsafe-backend-key",
			}),
		).toEqual({
			sessionKey: "interact_app_page:orders:event:checkout",
			rememberable: true,
		});
	});

	test("uses a context app fallback and page target", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-2",
				toolName: "interact_app_page",
				arguments: { pageId: "details" },
				contextAppId: "inventory",
			}),
		).toEqual({
			sessionKey: "interact_app_page:inventory:page:details",
			rememberable: true,
		});
	});

	test.each([
		{ arguments: { event_id: "checkout" }, contextAppId: undefined },
		{ arguments: { app_id: "orders" }, contextAppId: undefined },
	])(
		"uses a non-rememberable request key when scope is incomplete",
		(input) => {
			expect(
				resolveFrontendToolApprovalScope({
					requestId: "request-3",
					toolName: "interact_app_page",
					...input,
				}),
			).toEqual({
				sessionKey: "interact_app_page:request:request-3",
				rememberable: false,
			});
		},
	);

	test("keeps the existing key behavior for every other tool", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-4",
				toolName: "database_tool",
				arguments: {},
				approvalKind: "mutating",
				approvalSessionKey: "database:drop:customers",
			}),
		).toEqual({
			sessionKey: "database:drop:customers",
			rememberable: true,
		});

		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-5",
				toolName: "execute_event",
				arguments: {},
				approvalKind: "execute",
			}),
		).toEqual({
			sessionKey: "execute_event:execute",
			rememberable: true,
		});
	});

	test("escapes identifier delimiters so distinct scopes cannot collide", () => {
		const scoped = resolveFrontendToolApprovalScope({
			requestId: "request-6",
			toolName: "interact_app_page",
			arguments: { app_id: "app:event", event_id: "page/one" },
		});
		expect(scoped.sessionKey).toBe(
			"interact_app_page:app%3Aevent:event:page%2Fone",
		);
	});

	test("does not collapse distinct identifiers by trimming them", () => {
		const scoped = resolveFrontendToolApprovalScope({
			requestId: "request-7",
			toolName: "interact_app_page",
			arguments: { app_id: " orders", event_id: "checkout " },
		});
		expect(scoped.sessionKey).toBe(
			"interact_app_page:%20orders:event:checkout%20",
		);
	});
});
