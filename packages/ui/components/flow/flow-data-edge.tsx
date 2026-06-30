import {
	BaseEdge,
	type EdgeProps,
	getBezierPath,
	getSmoothStepPath,
	getStraightPath,
} from "@xyflow/react";
import { memo, useMemo } from "react";
import { isWebkitLite } from "../../lib/platform";

export const FlowDataEdge = memo(function FlowDataEdge(props: EdgeProps) {
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

	// Particle configurations for flowing data packets
	const particleConfigs = useMemo(
		() => [
			{ delay: 0, key: "d0" },
			{ delay: 0.5, key: "d1" },
			{ delay: 1, key: "d2" },
			{ delay: 1.5, key: "d3" },
		],
		[],
	);

	return (
		<>
			{/* Dotted base line - round dots */}
			<BaseEdge
				id={`${id}-base`}
				path={edgePath}
				markerEnd={markerEnd}
				style={{
					stroke: baseColor,
					strokeWidth: isSelected ? 2.5 : 1.5,
					opacity: isSelected ? 0.5 : 0.35,
					strokeDasharray: "1.5,4",
					strokeLinecap: "round",
					strokeLinejoin: "round",
					transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
				}}
			/>

			{/* Bright highlight line - round dots. On WebKit, skip this second
			    overlapping path (and its color-mix stroke): it doubles per-edge SVG
			    paint, which scales badly with connection count in WKWebView. */}
			{!isWebkitLite() && (
				<BaseEdge
					id={`${id}-highlight`}
					path={edgePath}
					style={{
						stroke: `color-mix(in oklch, ${baseColor} 70%, white 30%)`,
						strokeWidth: isSelected ? 1.5 : 1,
						opacity: isSelected ? 0.8 : 0.5,
						strokeDasharray: "1.5,4",
						strokeLinecap: "round",
						strokeLinejoin: "round",
						transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
					}}
				/>
			)}
		</>
	);
});
