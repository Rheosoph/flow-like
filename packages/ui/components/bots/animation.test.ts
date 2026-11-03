import { describe, expect, it } from "bun:test";
import {
	BOT_TIMING,
	clampGaze,
	getPointerGaze,
	getPoseGaze,
} from "./animation";

describe("bot gaze", () => {
	it("limits normalized input and replaces non-finite values", () => {
		expect(clampGaze(2, -5)).toEqual({ x: 3.5, y: -3 });
		expect(clampGaze(Number.NaN, Number.POSITIVE_INFINITY)).toEqual({
			x: 0,
			y: 0,
		});
		expect(clampGaze(0.5, 0.5)).toEqual({ x: 1.75, y: 1.5 });
	});

	it("scales pointer distance with tile width and caps diagonal movement radially", () => {
		const rect = { left: 100, top: 200, width: 100, height: 100 };
		expect(getPointerGaze({ x: 150, y: 250 }, rect)).toEqual({ x: 0, y: 0 });
		expect(getPointerGaze({ x: 205, y: 250 }, rect).x).toBeCloseTo(1.75);
		expect(getPointerGaze({ x: 1500, y: 250 }, rect)).toEqual({ x: 3.5, y: 0 });
		const diagonal = getPointerGaze({ x: 1250, y: 1350 }, rect);
		expect(diagonal.x).toBeCloseTo(3.5 / Math.sqrt(2));
		expect(diagonal.y).toBeCloseTo(3 / Math.sqrt(2));
	});

	it("centers when no usable layout or pointer exists", () => {
		for (const width of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
			expect(
				getPointerGaze(
					{ x: 20, y: 20 },
					{ left: 0, top: 0, width, height: 100 },
				),
			).toEqual({ x: 0, y: 0 });
		}
		expect(
			getPointerGaze(
				{ x: Number.NaN, y: 20 },
				{ left: 0, top: 0, width: 100, height: 100 },
			),
		).toEqual({ x: 0, y: 0 });
	});

	it("resolves fixed poses independently of tracking coordinates", () => {
		expect(getPoseGaze("left", 1, 1)).toEqual({ x: -3.5, y: 0 });
		expect(getPoseGaze("right")).toEqual({ x: 3.5, y: 0 });
		expect(getPoseGaze("up")).toEqual({ x: 0, y: -3 });
		expect(getPoseGaze("down")).toEqual({ x: 0, y: 3 });
		expect(getPoseGaze("track", -0.5, 1)).toEqual({ x: -1.75, y: 3 });
		expect(getPoseGaze("happy", 1, 1)).toEqual({ x: 0, y: 0 });
	});

	it("keeps transient actions shorter than the shortest repeat interval", () => {
		expect(BOT_TIMING.blinkMs).toBeLessThan(BOT_TIMING.blinkIntervalMs[0]);
		expect(BOT_TIMING.hopMs).toBeLessThan(BOT_TIMING.hopIntervalMs[0]);
		expect(BOT_TIMING.emotePopMs + BOT_TIMING.emoteHoldMs).toBeLessThan(
			BOT_TIMING.emoteIntervalMs[0],
		);
	});
});
