import { describe, expect, test } from "bun:test";
import {
	DEFAULT_CHAT_AI_DISCLOSURE,
	createChatBackgroundImage,
	escapeCssAttributeValue,
	resolveChatAiDisclosure,
	resolveChatColorScheme,
} from "./chat-appearance";

describe("chat appearance helpers", () => {
	test("normalizes unsupported color schemes to the app theme", () => {
		expect(resolveChatColorScheme("light")).toBe("light");
		expect(resolveChatColorScheme("dark")).toBe("dark");
		expect(resolveChatColorScheme("sepia")).toBe("system");
		expect(resolveChatColorScheme(null)).toBe("system");
	});

	test("always provides a visible AI disclosure", () => {
		expect(resolveChatAiDisclosure("  AI at work  ")).toBe("AI at work");
		expect(resolveChatAiDisclosure("   ")).toBe(DEFAULT_CHAT_AI_DISCLOSURE);
		expect(resolveChatAiDisclosure(undefined)).toBe(DEFAULT_CHAT_AI_DISCLOSURE);
	});

	test("builds a quoted background image with the overlay token", () => {
		expect(createChatBackgroundImage(" https://example.com/a b.png ")).toBe(
			'linear-gradient(var(--fl-chat-background-overlay), var(--fl-chat-background-overlay)), url("https://example.com/a b.png")',
		);
		expect(createChatBackgroundImage("")).toBeUndefined();
		expect(
			createChatBackgroundImage("javascript:alert(document.domain)"),
		).toBeUndefined();
		expect(
			createChatBackgroundImage("asset://localhost/path/background.webp"),
		).toContain("asset://localhost/path/background.webp");
	});

	test("escapes event ids used in CSS attribute selectors", () => {
		expect(escapeCssAttributeValue('chat\\"one\nnext')).toBe(
			'chat\\\\\\"one\\a next',
		);
	});
});
