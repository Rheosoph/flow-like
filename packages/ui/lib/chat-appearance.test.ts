import { describe, expect, test } from "bun:test";
import {
	CHAT_PLACEHOLDER_VISUALS,
	DEFAULT_CHAT_AI_DISCLOSURE,
	chatPlaceholderSupportsTypingMotion,
	createChatBackgroundImage,
	escapeCssAttributeValue,
	resolveChatAiDisclosure,
	resolveChatColorScheme,
	resolveChatPlaceholderTypingMotion,
	resolveChatPlaceholderVisual,
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
			'linear-gradient(to bottom, var(--fl-chat-background-overlay) 0%, var(--fl-chat-background-overlay) 48%, var(--fl-chat-background-overlay-strong, var(--fl-chat-background-overlay)) 100%), url("https://example.com/a b.png")',
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

describe("placeholder typing motion", () => {
	test("is opt-in — an interface that never set it keeps a still mark", () => {
		expect(resolveChatPlaceholderTypingMotion(undefined)).toBe(false);
		expect(resolveChatPlaceholderTypingMotion(null)).toBe(false);
		expect(resolveChatPlaceholderTypingMotion(false)).toBe(false);
	});

	test("only a real `true` enables it, so a stray value cannot start the motion", () => {
		expect(resolveChatPlaceholderTypingMotion(true)).toBe(true);
		expect(resolveChatPlaceholderTypingMotion("true")).toBe(false);
		expect(resolveChatPlaceholderTypingMotion(1)).toBe(false);
	});

	test("offers the setting only for the marks that can animate", () => {
		expect(chatPlaceholderSupportsTypingMotion("planet")).toBe(true);
		expect(chatPlaceholderSupportsTypingMotion("bubble")).toBe(true);
		expect(chatPlaceholderSupportsTypingMotion("image")).toBe(false);
		expect(chatPlaceholderSupportsTypingMotion("none")).toBe(false);
	});

	test("classifies every visual the config screen can offer", () => {
		for (const option of CHAT_PLACEHOLDER_VISUALS) {
			expect(typeof chatPlaceholderSupportsTypingMotion(option.value)).toBe(
				"boolean",
			);
			expect(resolveChatPlaceholderVisual(option.value)).toBe(option.value);
		}
	});
});
