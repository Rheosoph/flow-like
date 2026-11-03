// @vitest-environment happy-dom

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BotEyes, BotFace } from "./bot-face";
import { defaultBotCatalog } from "./catalog";
import {
	BOT_EXPRESSIONS,
	type BotExpression,
	type BotRenderContext,
	type BotShapeDefinition,
} from "./types";

function renderFace(
	kind: string,
	expression: BotExpression,
	overrides: Partial<BotRenderContext> = {},
	shapeOverrides: Partial<BotShapeDefinition> = {},
) {
	const shape = { ...defaultBotCatalog.shapes[kind], ...shapeOverrides };
	const context: BotRenderContext = {
		anchors: shape.anchors,
		palette: shape.palette,
		expression,
		pose: "center",
		idPrefix: "face-test",
		...overrides,
	};
	const container = document.createElement("div");
	container.innerHTML = renderToStaticMarkup(
		<svg aria-hidden="true">
			<g data-part="eyes">
				<BotEyes shape={shape} context={context} />
			</g>
			<g data-part="face">
				<BotFace shape={shape} context={context} />
			</g>
		</svg>,
	);
	return container;
}

function find(container: Element, selector: string) {
	const result = container.querySelector(selector);
	if (!result) throw new Error(`Missing ${selector}`);
	return result;
}

describe("bot expressions", () => {
	it("renders distinct expressions for round, small, light, and mouthless faces", () => {
		for (const kind of ["mochi", "pip", "astro", "drip", "hexa", "bambu"]) {
			const faces = BOT_EXPRESSIONS.map((expression) => {
				const face = renderFace(kind, expression);
				expect(face.innerHTML).not.toMatch(/NaN|undefined|Infinity/);
				return face.innerHTML;
			});
			expect(new Set(faces).size).toBe(BOT_EXPRESSIONS.length);
		}
	});

	it("gives wink one open eye and uses readable closed-eye colors", () => {
		for (const kind of ["mochi", "pip", "astro", "bambu"]) {
			const shape = defaultBotCatalog.shapes[kind];
			const face = renderFace(kind, "wink");
			expect(
				face.querySelectorAll('[data-part="pupils"] ellipse'),
			).toHaveLength(1);
			expect(
				find(face, '[data-feature="wink-eye"]').getAttribute("stroke"),
			).toBe(
				shape.palette[
					shape.closedEyeColor ??
						(shape.eyeStyle === "light" ? "sclera" : "ink")
				],
			);
		}
	});

	it("draws happy eyes as closed curves without eye whites", () => {
		for (const kind of ["mochi", "pip", "hexa", "bambu", "astro"]) {
			const shape = defaultBotCatalog.shapes[kind];
			const face = renderFace(kind, "happy");
			const eyes = find(face, '[data-part="eyes"]');
			expect(eyes.querySelectorAll(".fl-bot-happy-eye")).toHaveLength(2);
			expect(eyes.querySelector(".sclera, ellipse, clipPath")).toBeNull();
			expect(find(eyes, '[data-part="pupils"]').getAttribute("fill")).toBe(
				"none",
			);
			expect(find(eyes, '[data-part="pupils"]').getAttribute("stroke")).toBe(
				shape.palette[
					shape.closedEyeColor ??
						(shape.eyeStyle === "light" ? "sclera" : "ink")
				],
			);
		}
	});

	it("shows love hearts, a sad tear, focus lids, and squeezed oops eyes", () => {
		expect(
			renderFace("mochi", "love").querySelectorAll(
				'[data-feature="heart-eyes"] path',
			),
		).toHaveLength(2);
		const sad = renderFace("hexa", "sad");
		expect(sad.querySelectorAll('[data-feature="tear"]')).toHaveLength(1);
		expect(
			sad.querySelectorAll('[data-feature="lowered-lids"] path'),
		).toHaveLength(2);
		const focused = renderFace("mochi", "focused");
		expect(
			focused.querySelectorAll('[data-feature="lowered-lids"] path'),
		).toHaveLength(2);
		expect(find(focused, '[data-feature="mouth"] path').getAttribute("d")).toBe(
			"M-3 0H3",
		);
		const oops = renderFace("drip", "oops");
		expect(
			oops.querySelectorAll('[data-feature="squeezed-eyes"] path'),
		).toHaveLength(2);
		expect(oops.querySelector('[data-feature="mouth"]')).not.toBeNull();
		expect(oops.querySelector(".sclera")).toBeNull();
		expect(
			renderFace("drip", "neutral").querySelector('[data-feature="mouth"]'),
		).toBeNull();
	});

	it("gives surprise larger eyes and a small open mouth", () => {
		const neutral = renderFace("mochi", "neutral");
		const surprised = renderFace("mochi", "surprised");
		expect(
			Number(find(surprised, ".sclera").getAttribute("ry")),
		).toBeGreaterThan(Number(find(neutral, ".sclera").getAttribute("ry")));
		expect(
			surprised.querySelector('[data-feature="mouth"] ellipse'),
		).not.toBeNull();
	});

	it("keeps pupils clipped within each instance's visible eye whites", () => {
		for (const expression of BOT_EXPRESSIONS) {
			if (
				expression === "happy" ||
				expression === "love" ||
				expression === "oops"
			)
				continue;
			for (const idPrefix of ["first", "second"]) {
				const face = renderFace("mochi", expression, { idPrefix });
				const pupils = find(face, '[data-part="pupils"]');
				expect(pupils.parentElement?.getAttribute("clip-path")).toBe(
					`url(#${idPrefix}-eye-whites)`,
				);
				expect(
					face.querySelector(`clipPath[id="${idPrefix}-eye-whites"]`),
				).not.toBeNull();
				for (const clip of face.querySelectorAll("clipPath")) {
					expect(clip.id.startsWith(`${idPrefix}-`)).toBe(true);
				}
			}
		}
	});

	it("lets sleep override every expression without tears, brows, or expressive mouths", () => {
		for (const expression of BOT_EXPRESSIONS) {
			for (const kind of ["mochi", "astro", "drip", "bambu"]) {
				const face = renderFace(kind, expression, { pose: "sleep" });
				expect(
					face.querySelectorAll('[data-feature="closed-eyes"] path'),
				).toHaveLength(2);
				expect(
					face.querySelector(
						'[data-part="pupils"], [data-feature="brows"], [data-feature="tear"]',
					),
				).toBeNull();
				const mouth = face.querySelector('[data-feature="mouth"] path');
				if (kind === "drip") expect(mouth).toBeNull();
				else expect(mouth?.getAttribute("d")).toBe("M-2 0q2 1.5 4 0");
			}
		}
	});

	it("keeps hearts and tears within LCD, blueprint, and offline palettes", () => {
		for (const kind of ["mochi", "astro", "bambu"]) {
			const shape = defaultBotCatalog.shapes[kind];
			for (const effect of ["lcd", "blueprint", "offline"] as const) {
				for (const [expression, feature] of [
					["love", "heart-eyes"],
					["sad", "tear"],
				] as const) {
					const eyeColor = shape.eyeStyle === "light" ? "sclera" : "ink";
					const expected =
						shape.palette[
							expression === "love"
								? (shape.closedEyeColor ?? eyeColor)
								: eyeColor
						];
					const face = renderFace(kind, expression, { effect });
					expect(
						find(face, `[data-feature="${feature}"]`).getAttribute("fill"),
					).toBe(expected);
				}
			}
		}
	});

	it("preserves beaks, cat mouths, mouth scale and color, and custom face details", () => {
		const bird = renderFace("pino", "oops");
		expect(find(bird, '[data-feature="mouth"] path').getAttribute("fill")).toBe(
			defaultBotCatalog.shapes.pino.palette.accent,
		);
		const cat = renderFace("bambu", "love");
		expect(find(cat, '[data-feature="mouth"] path').getAttribute("d")).toBe(
			"M-4 -1q2 5 4 0q2 5 4 0",
		);
		const customized = renderFace(
			"mochi",
			"angry",
			{},
			{
				mouthScale: 1.75,
				mouthColor: "clothing",
				renderFaceDetails: () => <circle data-feature="custom-nose" r="2" />,
			},
		);
		expect(
			find(customized, '[data-feature="mouth"]').getAttribute("transform"),
		).toContain("scale(1.75)");
		expect(
			find(customized, '[data-feature="mouth"] path').getAttribute("stroke"),
		).toBe(defaultBotCatalog.shapes.mochi.palette.clothing);
		expect(
			customized.querySelector('[data-feature="custom-nose"]'),
		).not.toBeNull();
	});
});
