export type GraphEdgeRenderAttributes =
	| { type: "arrow" }
	| { type: "curvedArrow"; curvature: number };

/**
 * Matches Sigma's recommended curvature scaling for parallel edges.
 * The curve grows smoothly with the number of parallel edges while staying
 * anchored to the renderer's default curvature.
 */
export function getParallelCurvature(
	index: number,
	maxIndex: number,
	defaultCurvature: number,
): number {
	if (maxIndex <= 0) throw new Error("Invalid maxIndex");
	if (index < 0)
		return -getParallelCurvature(-index, maxIndex, defaultCurvature);

	const amplitude = 3.5;
	const maxCurvature =
		amplitude * (1 - Math.exp(-maxIndex / amplitude)) * defaultCurvature;
	return (maxCurvature * index) / maxIndex;
}

/**
 * Curved-edge geometry degenerates at curvature zero, so unique edges and the
 * center edge in an odd parallel group must use Sigma's straight-arrow program.
 */
export function getParallelEdgeRenderAttributes(
	index: number | null | undefined,
	maxIndex: number | null | undefined,
	defaultCurvature: number,
): GraphEdgeRenderAttributes {
	if (
		typeof index !== "number" ||
		index === 0 ||
		typeof maxIndex !== "number" ||
		maxIndex <= 0
	) {
		return { type: "arrow" };
	}

	return {
		type: "curvedArrow",
		curvature: getParallelCurvature(index, maxIndex, defaultCurvature),
	};
}
