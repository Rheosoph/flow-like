import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { resolveMobileViewportHeight } from "../mobile-viewport";

describe("resolveMobileViewportHeight", () => {
	it("falls back to the layout viewport without visual viewport metrics", () => {
		expect(resolveMobileViewportHeight(null, 844)).toBe(844);
	});

	it("uses the shrunken visual viewport when the keyboard does not pan it", () => {
		expect(
			resolveMobileViewportHeight({ height: 500, offsetTop: 0, scale: 1 }, 844),
		).toBe(500);
	});

	it("does not add visual viewport panning to the shell height", () => {
		expect(
			resolveMobileViewportHeight(
				{ height: 500, offsetTop: 280, scale: 1 },
				844,
			),
		).toBe(500);
	});

	it("normalises the visual viewport height when pinch-zoomed", () => {
		expect(
			resolveMobileViewportHeight(
				{ height: 250, offsetTop: 100, scale: 2 },
				844,
			),
		).toBe(500);
	});

	it("never exceeds the layout viewport", () => {
		expect(
			resolveMobileViewportHeight(
				{ height: 422, offsetTop: 120, scale: 2 },
				844,
			),
		).toBe(844);
	});

	it("keeps the desktop WebKit shell on the native dynamic viewport", () => {
		const css = readFileSync(
			new URL("../../../../packages/ui/global.css", import.meta.url),
			"utf8",
		);
		const webkitRule = css.match(
			/html\[data-desktop-app\]\[data-engine="webkit"\] \.h-vvh\s*\{([^}]*)\}/,
		);

		expect(webkitRule?.[1]).toMatch(/height:\s*100dvh\s*;/);
	});
});
