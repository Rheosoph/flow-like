import { describe, expect, test } from "bun:test";
import { safeScopedCss } from "./css-utils";

describe("safeScopedCss", () => {
	test("can map :root to an isolated surface", () => {
		const result = safeScopedCss(
			":root { --primary: rebeccapurple; } .message { color: red; }",
			'[data-fl-chat-root="chat-1"]',
			{ scopeRoot: true },
		);

		expect(result).toContain(
			'[data-fl-chat-root="chat-1"] { --primary: rebeccapurple; }',
		);
		expect(result).toContain(
			'[data-fl-chat-root="chat-1"] .message { color: red; }',
		);
		expect(result).not.toContain(":root");
	});

	test("keeps the legacy document-root behavior unless requested", () => {
		expect(safeScopedCss(":root { --primary: red; }", ".surface")).toContain(
			":root",
		);
	});

	test("scopes root tokens inside conditional rules", () => {
		const result = safeScopedCss(
			"@media (min-width: 40rem) { :root { --primary: blue; } }",
			".chat",
			{ scopeRoot: true },
		);

		expect(result).toContain(".chat { --primary: blue; }");
	});

	test("repairs known generated CSS transport sentinels", () => {
		const result = safeScopedCss(
			".card{content:__codex_directive_escaped_double_quote____codex_directive_escaped_double_quote__;color:red__codex_directive_quoted_closing_brace__.later{color:blue__codex_directive_quoted_closing_brace__",
			".surface",
		);

		expect(result).toContain('.surface .card{content:"";color:red}');
		expect(result).toContain(".surface .later{color:blue}");
		expect(result).not.toContain("__codex_directive_");
	});

	test("keeps valid blocks around a malformed rule", () => {
		const result = safeScopedCss(
			".before{color:red}.broken{this is not css}.after{color:blue}",
			".surface",
		);

		expect(result).toContain(".surface .before{color:red}");
		expect(result).not.toContain(".broken");
		expect(result).toContain(".surface .after{color:blue}");
	});

	test("keeps complete rules when the final rule is incomplete", () => {
		const result = safeScopedCss(
			".complete{display:grid}.incomplete{grid-template",
			".surface",
		);

		expect(result).toBe(".surface .complete{display:grid}");
	});

	test("still sanitizes declarations after best-effort recovery", () => {
		const result = safeScopedCss(
			".safe{color:red}.broken{this is not css}.danger{width:expression(alert(1))}",
			".surface",
		);

		expect(result).toContain(".surface .safe{color:red}");
		expect(result).toContain(".surface .danger{}");
		expect(result).not.toContain("expression");
	});
});
