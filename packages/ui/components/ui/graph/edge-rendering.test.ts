import { describe, expect, test } from "bun:test";
import {
	getParallelCurvature,
	getParallelEdgeRenderAttributes,
} from "./edge-rendering";

const DEFAULT_EDGE_CURVATURE = 0.25;

describe("graph edge rendering", () => {
	test("uses straight arrows for unique and zero-index parallel edges", () => {
		expect(
			getParallelEdgeRenderAttributes(null, null, DEFAULT_EDGE_CURVATURE),
		).toEqual({
			type: "arrow",
		});
		expect(
			getParallelEdgeRenderAttributes(
				undefined,
				undefined,
				DEFAULT_EDGE_CURVATURE,
			),
		).toEqual({
			type: "arrow",
		});
		expect(
			getParallelEdgeRenderAttributes(0, 1, DEFAULT_EDGE_CURVATURE),
		).toEqual({
			type: "arrow",
		});
	});

	test("uses curved arrows only for nonzero parallel indices", () => {
		const positive = getParallelEdgeRenderAttributes(
			1,
			2,
			DEFAULT_EDGE_CURVATURE,
		);
		const negative = getParallelEdgeRenderAttributes(
			-1,
			2,
			DEFAULT_EDGE_CURVATURE,
		);

		expect(positive.type).toBe("curvedArrow");
		expect(negative.type).toBe("curvedArrow");
		if (positive.type !== "curvedArrow" || negative.type !== "curvedArrow") {
			throw new Error("Expected curved edge attributes");
		}
		expect(positive.curvature).toBeGreaterThan(0);
		expect(negative.curvature).toBe(-positive.curvature);
	});

	test("scales curvature from Sigma's default", () => {
		const amplitude = 3.5;
		const expected =
			amplitude * (1 - Math.exp(-1 / amplitude)) * DEFAULT_EDGE_CURVATURE;

		expect(getParallelCurvature(1, 1, DEFAULT_EDGE_CURVATURE)).toBeCloseTo(
			expected,
		);
		expect(getParallelCurvature(-1, 1, DEFAULT_EDGE_CURVATURE)).toBeCloseTo(
			-expected,
		);
	});

	test("rejects an invalid maximum index", () => {
		expect(() => getParallelCurvature(1, 0, DEFAULT_EDGE_CURVATURE)).toThrow(
			"Invalid maxIndex",
		);
	});
});
