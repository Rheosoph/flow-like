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
		// The title shares the body's reading measure instead of its own 66ch.
		expect(lessonContent).toContain("max-w-(--fl-lesson-measure)");

		const articleRule = css.match(/\.fl-lesson-article\s*\{([^}]*)\}/)?.[1];
		expect(
			articleRule,
			".fl-lesson-article must have a CSS rule",
		).toBeDefined();
		expect(articleRule).toMatch(/--fl-lesson-measure:\s*\d+(?:\.\d+)?rem/);
		expect(articleRule).toMatch(
			/--fl-lesson-figure-canvas:\s*min\(56rem,\s*100cqi\)/,
		);
		expect(articleRule).toMatch(/container-type:\s*inline-size/);

		// The prose spans the article; the reading measure is applied as padding
		// on the Plate root, so nothing has to escape TextEditor's overflow clip.
		const proseRule = css.match(/\.fl-lesson-prose\s*\{([^}]*)\}/)?.[1];
		expect(proseRule, ".fl-lesson-prose must have a CSS rule").toBeDefined();
		expect(proseRule).toMatch(/width:\s*100%/);

		const rootRule = css.match(
			/\.fl-lesson-prose \[data-slate-editor\]\s*\{([^}]*)\}/,
		)?.[1];
		expect(rootRule, "the Plate root must carry the measure").toBeDefined();
		expect(rootRule).toMatch(
			/padding-inline:\s*max\(0px,\s*calc\(\(100% - var\(--fl-lesson-measure\)\) \/ 2\)\)/,
		);

		// Media blocks pull back out to the figure canvas with negative margins.
		const mediaRule = css.match(
			/\.fl-lesson-prose \[data-slate-editor\] > :is\(\.slate-img, \.slate-video\)\s*\{([^}]*)\}/,
		)?.[1];
		expect(mediaRule, "media blocks must have a breakout rule").toBeDefined();
		expect(mediaRule).toMatch(/margin-inline:\s*min\(\s*0px,/);
		expect(mediaRule).toContain("var(--fl-lesson-figure-canvas)");

		const figureRule = css.match(/\.fl-lesson-prose figure\s*\{([^}]*)\}/)?.[1];
		expect(figureRule, "lesson figures must have a rule").toBeDefined();
		// The editor writes a resized figure's width as an inline style, which
		// outranks `width` here — the cap has to be a real value, never `none`,
		// or a wide image escapes the canvas.
		expect(figureRule).toMatch(/max-width:\s*100%/);
		// A transform breakout would be clipped by the wrapper again.
		expect(figureRule).not.toContain("translateX");
	});
});
