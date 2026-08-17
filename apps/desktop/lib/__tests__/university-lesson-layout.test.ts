import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (relative: string) =>
	readFileSync(new URL(relative, import.meta.url), "utf8");

describe("University lesson reading layout", () => {
	const page = read("../../app/learn/lesson/page.tsx");
	const lessonContent = read(
		"../../../../packages/ui/components/learn/lesson-content.tsx",
	);
	const css = read("../../../../packages/ui/global.css");

	it("keeps the desktop lesson canvas wide enough for instructional media", () => {
		const lessonBodyClasses = page.match(
			/const lessonBody = \(\s*<div className="([^"]+)"/,
		)?.[1];

		expect(
			lessonBodyClasses,
			"lessonBody class list must be discoverable",
		).toBeDefined();
		expect(lessonBodyClasses).toContain("w-full");
		expect(lessonBodyClasses).toMatch(/\bmax-w-(?:5xl|6xl|7xl|full)\b/);
		expect(lessonBodyClasses).not.toContain("max-w-3xl");
	});

	it("connects the named prose hook to measured text and wider figures", () => {
		expect(lessonContent).toContain('className="fl-lesson-prose"');

		const proseRule = css.match(/\.fl-lesson-prose\s*\{([^}]*)\}/)?.[1];
		expect(proseRule, ".fl-lesson-prose must have a CSS rule").toBeDefined();
		expect(proseRule).toMatch(/width:\s*min\(100%,\s*66ch\)/);

		const figureRule = css.match(/\.fl-lesson-prose figure\s*\{([^}]*)\}/)?.[1];
		expect(
			figureRule,
			"lesson figures must have a breakout rule",
		).toBeDefined();
		expect(figureRule).toMatch(/width:\s*min\(56rem,\s*100cqi\)/);
		// The editor writes a resized figure's width as an inline style, which
		// outranks `width` here — the cap has to be a real value, never `none`,
		// or a wide image escapes the reading column.
		expect(figureRule).toMatch(/max-width:\s*min\(56rem,\s*100cqi\)/);
	});
});
