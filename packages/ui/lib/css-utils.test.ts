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

	test("preserves nested pseudo-classes and implicit descendant selectors", () => {
		const result = safeScopedCss(
			".card { &:hover { color: red; } .title { font-weight: bold; } > p { margin: 0; } }",
			".surface",
		);

		expect(String(result)).toBe(
			".surface .card { &:hover { color: red; } .title { font-weight: bold; } > p { margin: 0; } }",
		);
	});

	test("scopes independent rules in nested media queries once", () => {
		const result = safeScopedCss(
			"@layer custom { @media (width > 40rem) { .card { @supports (display: grid) { & > p { color: red; } } } .other { display: grid; } } }",
			".surface",
		);

		expect(result).toContain(".surface .card {");
		expect(result).toContain(".surface .other {");
		expect(result).toContain("& > p { color: red; }");
		expect(result).not.toContain(".surface &");
	});

	test("preserves keyframe steps inside conditional rules", () => {
		const result = safeScopedCss(
			"@media (prefers-reduced-motion: no-preference) { @keyframes fade { 0%, 100% { opacity: 0; } 50% { opacity: 1; } } .card { animation: fade 1s; } }",
			".surface",
		);

		expect(result).toContain("0%, 100% { opacity: 0; }");
		expect(result).toContain("50% { opacity: 1; }");
		expect(result).toContain(".surface .card {");
	});

	test("sanitizes nested declarations and blocked at-rules", () => {
		const result = safeScopedCss(
			'.card { &:hover { width: expression(alert(1)); color: red; } @import "https://example.com/style.css"; }',
			".surface",
		);

		expect(result).toContain("&:hover {");
		expect(result).toContain("color: red;");
		expect(result).not.toContain("expression");
		expect(result).not.toContain("@import");
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

		expect(String(result)).toBe(".surface .complete{display:grid}");
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
