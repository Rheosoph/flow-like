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
});
