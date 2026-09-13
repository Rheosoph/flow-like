import { describe, expect, test } from "bun:test";
import { graphRgba } from "./graph-render-color";

describe("graph WebGL opacity", () => {
	test("fades color channels as well as alpha for Sigma's blend mode", () => {
		expect(graphRgba("#64a0f0", 0.05)).toBe("rgba(5,8,12,0.05)");
		expect(graphRgba("#64a0f0", 0)).toBe("rgba(0,0,0,0)");
		expect(graphRgba("#64a0f0", 1)).toBe("rgba(100,160,240,1)");
	});

	test("composites dimmed context against both light and dark backgrounds", () => {
		const rgba = graphRgba("#64a0f0", 0.05)
			.match(/[\d.]+/g)
			?.map(Number);
		if (!rgba) throw new Error("Expected RGBA channels");
		const [r, g, b, alpha] = rgba;
		for (const background of [0, 250]) {
			const composited = [r, g, b].map(
				(channel) => channel + background * (1 - alpha),
			);
			for (const channel of composited) {
				expect(Math.abs(channel - background)).toBeLessThanOrEqual(13);
			}
		}
	});

	test("supports shorthand colors and clamps opacity", () => {
		expect(graphRgba("#0af", 0.1)).toBe("rgba(0,17,26,0.1)");
		expect(graphRgba("#ffffff", 2)).toBe("rgba(255,255,255,1)");
		expect(graphRgba("#ffffff", -1)).toBe("rgba(0,0,0,0)");
	});
});
