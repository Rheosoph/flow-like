import {
	BaseEdge,
	type EdgeProps,
	getBezierPath,
	getSmoothStepPath,
	getStraightPath,
} from "@xyflow/react";
import { memo, useMemo } from "react";

export const FlowVeilEdge = memo(function FlowVeilEdge(props: EdgeProps) {
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

	// Determine path type based on connection mode or default to bezier
	const pathType = data?.pathType || "default";
	const [edgePath, labelX, labelY] = useMemo(() => {
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

	const baseColor = style.stroke || "var(--pin-fn-ref)";
	const isSelected = selected ?? false;

	// Calculate unique animation delays for particle variety
	const particleConfigs = useMemo(
		() => [
			{ delay: 0, key: "p0" },
			{ delay: 0.4, key: "p1" },
			{ delay: 0.8, key: "p2" },
		],
		[],
	);

	return (
		<>
			{/* Subtle outer glow layer - no animation */}
			<BaseEdge
				id={`${id}-glow`}
				path={edgePath}
				style={{
					stroke: baseColor,
					strokeWidth: isSelected ? 8 : 5,
					opacity: isSelected ? 0.08 : 0.04,
					strokeLinecap: "round",
					strokeLinejoin: "round",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>

			{/* Core solid line */}
			<BaseEdge
				id={`${id}-core`}
				path={edgePath}
				markerEnd={markerEnd}
				style={{
					stroke: baseColor,
					strokeWidth: isSelected ? 2.5 : 1.5,
					opacity: isSelected ? 0.5 : 0.3,
					strokeLinecap: "round",
					strokeLinejoin: "round",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>

			{/* Top highlight line - no animation */}
			<BaseEdge
				id={`${id}-highlight`}
				path={edgePath}
				style={{
					stroke: `color-mix(in oklch, ${baseColor} 80%, white 20%)`,
					strokeWidth: isSelected ? 1.2 : 0.8,
					opacity: isSelected ? 0.7 : 0.4,
					strokeLinecap: "round",
					strokeLinejoin: "round",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>
		</>
	);
});
