import {
	BaseEdge,
	type EdgeProps,
	getBezierPath,
	getSmoothStepPath,
	getStraightPath,
} from "@xyflow/react";
import { memo, useMemo } from "react";

export const FlowExecutionEdge = memo(function FlowExecutionEdge(props: EdgeProps) {
	const {
		id,
		sourceX,
		sourceY,
		targetX,
		targetY,
		sourcePosition,
		targetPosition,
		style = {},
		markerEnd,
		selected,
		data,
	} = props;

	// Determine path type
	const pathType = data?.pathType || "default";
	const [edgePath] = useMemo(() => {
		if (pathType === "straight") {
			return getStraightPath({
				sourceX,
				sourceY,
				targetX,
				targetY,
			});
		}
		if (pathType === "step" || pathType === "smoothstep") {
			return getSmoothStepPath({
				sourceX,
				sourceY,
				targetX,
				targetY,
				sourcePosition,
				targetPosition,
			});
		}
		return getBezierPath({
			sourceX,
			sourceY,
			targetX,
			targetY,
			sourcePosition,
			targetPosition,
		});
	}, [
		pathType,
		sourceX,
		sourceY,
		targetX,
		targetY,
		sourcePosition,
		targetPosition,
	]);

	const baseColor = style.stroke || "var(--foreground)";
	const isSelected = selected ?? false;

	return (
		<>
			{/* Core solid line */}
			<BaseEdge
				id={`${id}-core`}
				path={edgePath}
				markerEnd={markerEnd}
				style={{
					stroke: baseColor,
					strokeWidth: isSelected ? 3 : 2,
					opacity: isSelected ? 0.9 : 0.7,
					strokeLinecap: "round",
					strokeLinejoin: "round",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>

			{/* Animated energy pulse traveling along the edge */}
			<BaseEdge
				id={`${id}-energy`}
				path={edgePath}
				style={{
					stroke: `color-mix(in oklch, ${baseColor} 70%, white 30%)`,
					strokeWidth: isSelected ? 4 : 3,
					opacity: 0,
					strokeLinecap: "round",
					strokeLinejoin: "round",
					animation: "exec-energy 3s ease-in-out infinite",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>
		</>
	);
});
