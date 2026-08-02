import { describe, expect, test } from "bun:test";
import {
	CHAT_THEME_PRESETS,
	CUSTOM_CHAT_THEME_VALUE,
	DEFAULT_CHAT_THEME_CSS,
	resolveChatThemePreset,
} from "./chat-theme-presets";
import { safeScopedCss } from "./css-utils";

describe("chat theme presets", () => {
	test("exposes unique preset ids and a canonical default", () => {
		const ids = CHAT_THEME_PRESETS.map((preset) => preset.value);

		expect(ids).toHaveLength(20);
		expect(new Set(ids).size).toBe(ids.length);
		expect(ids).toContain("cyberpunk");
		expect(ids).toContain("neon-grid");
		expect(ids).toContain("typewriter");
		expect(DEFAULT_CHAT_THEME_CSS).toBe(CHAT_THEME_PRESETS[0].css);
	});

	test("recognizes only exact preset CSS", () => {
		for (const preset of CHAT_THEME_PRESETS) {
			expect(resolveChatThemePreset(preset.css)).toBe(preset.value);
		}

		expect(resolveChatThemePreset(`${DEFAULT_CHAT_THEME_CSS}\n`)).toBe(
			CUSTOM_CHAT_THEME_VALUE,
		);
		expect(resolveChatThemePreset("/* my theme */")).toBe(
			CUSTOM_CHAT_THEME_VALUE,
		);
		expect(resolveChatThemePreset(undefined)).toBe(CUSTOM_CHAT_THEME_VALUE);
	});

	test("every preset survives chat CSS sanitizing and scoping", () => {
		const scope = '[data-fl-chat-root="theme-test"]';

		for (const preset of CHAT_THEME_PRESETS) {
			const sanitized = safeScopedCss(preset.css, scope, { scopeRoot: true });

			expect(sanitized.length).toBeGreaterThan(100);
			expect(sanitized).toContain(scope);
			expect(sanitized).not.toContain(":root");
		}
	});

	test("preserves app mode and keeps animation names isolated", () => {
		const animationNames = CHAT_THEME_PRESETS.flatMap((preset) =>
			Array.from(
				preset.css.matchAll(/@keyframes\s+([^\s{]+)/g),
				(match) => match[1],
			),
		);

		for (const preset of CHAT_THEME_PRESETS) {
			expect(preset.css).not.toMatch(/^\s*--background\s*:/m);
			expect(preset.css).not.toMatch(/^\s*color-scheme\s*:/m);
			if (/\banimation\s*:/.test(preset.css)) {
				expect(preset.css).toContain("prefers-reduced-motion: reduce");
			}
		}

		expect(new Set(animationNames).size).toBe(animationNames.length);
	});
});
