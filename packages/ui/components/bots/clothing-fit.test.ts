import { describe, expect, it } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { defaultBotCatalog } from "./catalog";
import { getBotClothingFit } from "./clothing-fit";
import type { BotRenderContext } from "./types";

describe("bot clothing fit", () => {
	it("keeps existing attachment defaults when a character has no chest override", () => {
		const fit = getBotClothingFit({
			...defaultBotCatalog.shapes.mochi.anchors,
			head: { x: 50, y: 32, width: 48 },
			neck: { y: 79, bottom: 100 },
		});
		expect(fit).toMatchObject({
			x: 50,
			width: 48,
			top: 79,
			bottom: 100,
			height: 21,
			scaleX: 1,
			scaleY: 0.875,
		});
	});

	it("fits a narrow asymmetric chest independently of the head and limits details to the visible torso", () => {
		const anchors = {
			...defaultBotCatalog.shapes.mochi.anchors,
			head: { x: 68, y: 22, width: 52 },
			neck: { x: 47, width: 18, y: 92, bottom: 108 },
		};
		const fit = getBotClothingFit(anchors);
		expect(fit).toMatchObject({
			x: 47,
			width: 18,
			top: 92,
			bottom: 100,
			height: 8,
			scaleX: 0.375,
		});
		expect(fit.scaleY).toBeCloseTo(1 / 3);
		expect(fit.x + 24 * fit.scaleX).toBe(56);
		expect(fit.top + 24 * fit.scaleY).toBe(100);
	});

	it("keeps every garment attached to its chest when headwear anchors change", () => {
		const shape = defaultBotCatalog.shapes.mochi;
		const context: BotRenderContext = {
			anchors: {
				...shape.anchors,
				neck: { x: 37, width: 18, y: 80, bottom: 90 },
			},
			palette: shape.palette,
			pose: "center",
			expression: "neutral",
			idPrefix: "fit-test",
		};
		const otherHead: BotRenderContext = {
			...context,
			anchors: { ...context.anchors, head: { x: 70, y: 12, width: 80 } },
		};
		const garments = Object.values(defaultBotCatalog.equipment).filter(
			(item) => item.slot === "neck",
		);
		expect(garments.length).toBeGreaterThan(140);
		for (const garment of garments) {
			const render = (input: BotRenderContext) =>
				renderToStaticMarkup(createElement("svg", null, garment.render(input)));
			expect(render(otherHead), garment.id).toBe(render(context));
		}
	});
});
