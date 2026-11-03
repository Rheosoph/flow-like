import { describe, expect, test } from "bun:test";
import { grantKeys, grantScopes } from "./tool-grant-scopes";

type Row = [
	name: string,
	tool: string,
	args: Record<string, unknown>,
	contextApp: string | undefined,
	scopes: string[] | null,
];

// Mirrors the table in packages/core/editor/src/flow/copilot/tool_grants.rs.
const ROWS: Row[] = [
	[
		"call_app_event takes the argument app",
		"call_app_event",
		{ app_id: "orders" },
		"ctx",
		["app:orders"],
	],
	[
		"call_app_event camelCase",
		"call_app_event",
		{ appId: "orders" },
		undefined,
		["app:orders"],
	],
	[
		"call_app_chat falls back to the chat's app",
		"call_app_chat",
		{},
		"ctx",
		["app:ctx"],
	],
	[
		"call_app_event without any app is never remembered",
		"call_app_event",
		{},
		undefined,
		null,
	],
	[
		"app ids are trimmed",
		"call_app_event",
		{ app_id: "  orders  " },
		undefined,
		["app:orders"],
	],
	[
		"a comma makes the app unusable",
		"call_app_event",
		{ app_id: "a,b" },
		undefined,
		null,
	],
	[
		"database_tool is per app",
		"database_tool",
		{ operation: "query", app_id: "orders" },
		undefined,
		["app:orders"],
	],
	[
		"delete_table is per table",
		"database_tool",
		{ operation: "delete_table", app_id: "orders", table_name: "t1" },
		undefined,
		["table:t1"],
	],
	[
		"delete_table without a table",
		"database_tool",
		{ operation: "delete_table", app_id: "orders" },
		undefined,
		null,
	],
	[
		"knowledge_drop is per index",
		"knowledge_drop",
		{ index: "notes" },
		undefined,
		["table:notes"],
	],
	[
		"interact_app_page prefers the event",
		"interact_app_page",
		{ app_id: "a", event_id: "e", page_id: "p" },
		undefined,
		["target:a:event:e"],
	],
	[
		"interact_app_page page target",
		"interact_app_page",
		{ app_id: "a", page_id: "p" },
		undefined,
		["target:a:page:p"],
	],
	[
		"interact_app_page uses the chat's app",
		"interact_app_page",
		{ pageId: "p" },
		"ctx",
		["target:ctx:page:p"],
	],
	[
		"interact_app_page without a target",
		"interact_app_page",
		{ app_id: "a" },
		undefined,
		null,
	],
	[
		"code_run grants each host sorted and lowercased",
		"code_run",
		{ network_hosts: ["B.com", "a.com", "b.com"] },
		undefined,
		["host:a.com", "host:b.com"],
	],
	["code_run without hosts", "code_run", {}, undefined, null],
	[
		"data_sources attach",
		"data_sources",
		{
			attach: {
				kind: "postgres",
				connection: { host: "db", port: 5432, database: "app" },
			},
		},
		undefined,
		["connection:postgres:db:5432:app"],
	],
	[
		"data_sources attach with a text port and no port",
		"data_sources",
		{
			attach: {
				kind: "mysql",
				connection: { host: "db", port: " 3306 ", database: "app" },
			},
		},
		undefined,
		["connection:mysql:db:3306:app"],
	],
	[
		"data_sources without a port",
		"data_sources",
		{ attach: { kind: "mysql", connection: { host: "db", database: "app" } } },
		undefined,
		["connection:mysql:db::app"],
	],
	["data_sources without attach", "data_sources", {}, undefined, null],
	[
		"browser_open is per site",
		"browser_open",
		{ url: "https://www.GitHub.com/a/b" },
		undefined,
		["browser:github.com"],
	],
	[
		"browser_open with a trailing dot host",
		"browser_open",
		{ url: "https://github.com./x" },
		undefined,
		["browser:github.com"],
	],
	[
		"browser_open with a bad url",
		"browser_open",
		{ url: "not a url" },
		undefined,
		null,
	],
	["computer_act", "computer_act", {}, undefined, ["computer:session"]],
	[
		"computer_screenshot",
		"computer_screenshot",
		{},
		undefined,
		["computer:session"],
	],
	[
		"email_send grants each recipient domain",
		"email_send",
		{ to: ["a@Example.com", "b@example.com"], cc: ["c@other.org"] },
		undefined,
		["email:example.com", "email:other.org"],
	],
	[
		"email_send with an unreadable recipient",
		"email_send",
		{ to: ["nobody"] },
		undefined,
		null,
	],
	["email_send without recipients", "email_send", {}, undefined, null],
	[
		"shell_run is per program",
		"shell_run",
		{ command: "/usr/bin/Git" },
		undefined,
		["command:git"],
	],
	[
		"shell_run strips Windows extensions",
		"shell_run",
		{ command: "C:\\tools\\Node.EXE" },
		undefined,
		["command:node"],
	],
	[
		"shell true is sh",
		"shell_run",
		{ command: "ls | wc", shell: true },
		undefined,
		["command:sh"],
	],
	["shell_run without a command", "shell_run", {}, undefined, null],
	[
		"http_request is asked every time",
		"http_request",
		{ url: "https://x.com" },
		undefined,
		null,
	],
	["forecast_tool is asked every time", "forecast_tool", {}, undefined, null],
	[
		"flowpilot_board_review is asked every time",
		"flowpilot_board_review",
		{},
		undefined,
		null,
	],
	["app_build is asked every time", "app_build", {}, undefined, null],
	[
		"apply_home_layout is asked every time",
		"apply_home_layout",
		{},
		undefined,
		null,
	],
	[
		"any other tool is granted whole",
		"file_write",
		{ path: "x" },
		undefined,
		["*"],
	],
];

describe("grantScopes", () => {
	test.each(ROWS)("%s", (_name, tool, args, contextApp, scopes) => {
		expect(grantScopes(tool, args, contextApp)).toEqual(scopes);
	});
});

describe("grantKeys", () => {
	test("pairs every scope with its tool", () => {
		expect(
			grantKeys("code_run", { network_hosts: ["a.com", "b.com"] }),
		).toEqual([
			["code_run", "host:a.com"],
			["code_run", "host:b.com"],
		]);
	});

	test("a call that is never remembered writes nothing", () => {
		expect(grantKeys("http_request", { url: "https://x.com" })).toEqual([]);
	});
});
