import { describe, expect, test } from "bun:test";
import {
	type GrantStore,
	StandingApprovals,
	resolveFrontendToolApprovalScope,
} from "./frontend-tool-approval-scope";
import { grantKeys, grantScopes } from "./tool-grant-scopes";

function memoryStore(initial: Array<[string, string, string]> = []) {
	const rows = [...initial];
	const calls = { list: 0, put: [] as Array<[string, string, string]> };
	const store: GrantStore = {
		async list(botId) {
			calls.list += 1;
			return rows
				.filter(([bot]) => bot === botId)
				.map(([, tool, scope]) => ({ tool, scope }));
		},
		async put(botId, tool, scope) {
			calls.put.push([botId, tool, scope]);
			rows.push([botId, tool, scope]);
		},
	};
	return { store, calls };
}

const GIT = { command: "git", args: ["status"] };

describe("StandingApprovals", () => {
	test("an owner's Don't ask again is that owner's alone; an unknown owner keeps none", () => {
		const approvals = new StandingApprovals();
		approvals.add(null, "call_app_event:app:orders");
		approvals.add("bot_scout", "call_app_chat:app:orders");
		approvals.add(undefined, "call_app_chat:app:ledger");

		expect(approvals.has(null, "call_app_event:app:orders")).toBe(true);
		expect(approvals.has("bot_scout", "call_app_event:app:orders")).toBe(false);
		expect(approvals.has("bot_scout", "call_app_chat:app:orders")).toBe(true);
		expect(approvals.has(null, "call_app_chat:app:orders")).toBe(false);
		expect(approvals.has(undefined, "call_app_event:app:orders")).toBe(false);
		expect(approvals.has(undefined, "call_app_chat:app:ledger")).toBe(false);
	});
});

describe("resolveFrontendToolApprovalScope", () => {
	test("forecast saves approve one exact request without a session waiver", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "save-1",
				toolName: "forecast_tool",
				approvalKind: "mutating",
				approvalSessionKey: "forecast:app",
				arguments: { operation: "save", run_id: "run", name: "Forecast" },
			}),
		).toEqual({
			sessionKey: "forecast_tool:save:request:save-1",
			rememberable: false,
		});
	});
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
			appId: "orders",
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
			appId: "inventory",
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

	test("requires separate approval for each HTTP request", () => {
		const scopes = ["request-1", "request-2"].map((requestId) =>
			resolveFrontendToolApprovalScope({
				requestId,
				toolName: "http_request",
				arguments: { url: "https://example.com", method: "POST" },
				approvalKind: "execute",
				approvalSessionKey: "http_request:shared",
			}),
		);
		expect(scopes[0]).toEqual({
			sessionKey: "http_request:request:request-1",
			rememberable: false,
		});
		expect(scopes[1].rememberable).toBe(false);
		expect(scopes[1].sessionKey).not.toBe(scopes[0].sessionKey);
	});

	test("app tools remember an approval for the app they act in", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-4",
				toolName: "database_tool",
				arguments: { app_id: "orders" },
				approvalKind: "mutating",
				approvalSessionKey: "database:delete_table:customers",
			}),
		).toEqual({
			sessionKey: "database:delete_table:customers:app:orders",
			rememberable: true,
			appId: "orders",
		});
		const event = (appId: string) =>
			resolveFrontendToolApprovalScope({
				requestId: "request-8",
				toolName: "call_app_event",
				arguments: { appId },
				approvalKind: "execute",
				approvalSessionKey: "call_app_event",
			}).sessionKey;
		expect(event("orders")).toBe("call_app_event:app:orders");
		expect(event("ledger")).not.toBe(event("orders"));
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-11",
				toolName: "call_app_chat",
				arguments: { app_id: "orders" },
				approvalKind: "execute",
				approvalSessionKey: "call_app_chat",
			}).appId,
		).toBe("orders");
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-9",
				toolName: "call_app_chat",
				arguments: {},
				approvalKind: "execute",
				approvalSessionKey: "call_app_chat",
				contextAppId: "orders",
			}).sessionKey,
		).toBe("call_app_chat:app:orders");
	});

	test("an app tool naming no app is approved once", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-10",
				toolName: "call_app_event",
				arguments: { event_id: "send" },
				approvalKind: "execute",
				approvalSessionKey: "call_app_event",
			}),
		).toEqual({
			sessionKey: "call_app_event:request:request-10",
			rememberable: false,
		});
	});

	test("keeps the existing key behavior for other tools", () => {
		expect(
			resolveFrontendToolApprovalScope({
				requestId: "request-4",
				toolName: "flowpilot_board",
				arguments: { app_id: "orders" },
				approvalKind: "execute",
				approvalSessionKey: "flowpilot_board",
			}),
		).toEqual({
			sessionKey: "flowpilot_board",
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

describe("shell_run approval scope", () => {
	const scope = (args: Record<string, unknown>) =>
		resolveFrontendToolApprovalScope({
			requestId: "r1",
			toolName: "shell_run",
			arguments: args,
			approvalKind: "mutating",
		});

	test("a standing approval covers one program, not the tool", () => {
		expect(scope({ command: "/usr/bin/Git" }).sessionKey).toBe(
			"shell_run:command:git",
		);
		expect(scope({ command: "node.exe" }).sessionKey).toBe(
			"shell_run:command:node",
		);
		expect(scope({ command: "git" }).sessionKey).not.toBe(
			scope({ command: "rm" }).sessionKey,
		);
	});

	test("every shell call is the sh program whatever it runs", () => {
		expect(scope({ command: "rm -rf x", shell: true }).sessionKey).toBe(
			"shell_run:command:sh",
		);
	});
});

describe("StandingApprovals with persisted grants", () => {
	test("Don't ask again writes one grant per scope, and the next identical call is covered", async () => {
		const { store, calls } = memoryStore();
		const approvals = new StandingApprovals(() => store);
		const scopes = grantScopes("shell_run", GIT) ?? [];

		expect(await approvals.coveredByGrant("bot_a", "shell_run", scopes)).toBe(
			false,
		);
		await approvals.remember(
			"bot_a",
			"shell_run:command:git",
			grantKeys("shell_run", GIT),
		);

		expect(calls.put).toEqual([["bot_a", "shell_run", "command:git"]]);
		expect(approvals.has("bot_a", "shell_run:command:git")).toBe(true);
		expect(await approvals.coveredByGrant("bot_a", "shell_run", scopes)).toBe(
			true,
		);
		expect(
			await approvals.coveredByGrant("bot_a", "shell_run", ["command:rm"]),
		).toBe(false);
	});

	test("a fresh session finds grants an earlier one wrote", async () => {
		const { store } = memoryStore([["bot_a", "shell_run", "command:git"]]);
		const approvals = new StandingApprovals(() => store);

		expect(
			await approvals.coveredByGrant("bot_a", "shell_run", ["command:git"]),
		).toBe(true);
		expect(approvals.has("bot_a", "shell_run:command:git")).toBe(false);
	});

	test("a whole-tool grant covers every scope; a multi-scope call needs all of them", async () => {
		const { store } = memoryStore([
			["bot_a", "file_write", "*"],
			["bot_a", "code_run", "host:a.com"],
		]);
		const approvals = new StandingApprovals(() => store);

		expect(
			await approvals.coveredByGrant("bot_a", "file_write", ["app:x"]),
		).toBe(true);
		expect(
			await approvals.coveredByGrant("bot_a", "code_run", ["host:a.com"]),
		).toBe(true);
		expect(
			await approvals.coveredByGrant("bot_a", "code_run", [
				"host:a.com",
				"host:b.com",
			]),
		).toBe(false);
	});

	test("grants stay with their bot; Flux and unknown owners persist nothing", async () => {
		const { store, calls } = memoryStore([
			["bot_a", "shell_run", "command:git"],
		]);
		const approvals = new StandingApprovals(() => store);

		expect(
			await approvals.coveredByGrant("bot_b", "shell_run", ["command:git"]),
		).toBe(false);
		expect(
			await approvals.coveredByGrant(null, "shell_run", ["command:git"]),
		).toBe(false);
		expect(
			await approvals.coveredByGrant(undefined, "shell_run", ["command:git"]),
		).toBe(false);
		await approvals.remember(null, "k", [["shell_run", "command:git"]]);
		await approvals.remember(undefined, "k", [["shell_run", "command:git"]]);
		expect(calls.put).toEqual([]);
		expect(approvals.has(null, "k")).toBe(true);
	});

	test("a call with no scopes is never covered", async () => {
		const { store } = memoryStore([["bot_a", "http_request", "*"]]);
		expect(
			await new StandingApprovals(() => store).coveredByGrant(
				"bot_a",
				"http_request",
				[],
			),
		).toBe(false);
	});

	test("an unreadable store covers nothing and a refused write still remembers the session", async () => {
		const failing: GrantStore = {
			list: async () => {
				throw new Error("offline");
			},
			put: async () => {
				throw new Error("offline");
			},
		};
		const warn = console.warn;
		console.warn = () => undefined;
		try {
			const approvals = new StandingApprovals(() => failing);
			expect(
				await approvals.coveredByGrant("bot_a", "shell_run", ["command:git"]),
			).toBe(false);
			await expect(
				approvals.remember("bot_a", "k", [["shell_run", "command:git"]]),
			).rejects.toThrow("offline");
			expect(approvals.has("bot_a", "k")).toBe(true);
		} finally {
			console.warn = warn;
		}
	});

	test("grants are read again once the short cache expires, so a revoke takes effect", async () => {
		const { store, calls } = memoryStore([
			["bot_a", "shell_run", "command:git"],
		]);
		const approvals = new StandingApprovals(() => store);
		const now = Date.now;
		let clock = now();
		Date.now = () => clock;
		try {
			await approvals.coveredByGrant("bot_a", "shell_run", ["command:git"]);
			await approvals.coveredByGrant("bot_a", "shell_run", ["command:git"]);
			expect(calls.list).toBe(1);
			clock += 6_000;
			await approvals.coveredByGrant("bot_a", "shell_run", ["command:git"]);
			expect(calls.list).toBe(2);
		} finally {
			Date.now = now;
		}
	});
});
