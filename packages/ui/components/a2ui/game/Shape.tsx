"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, ShapeComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIShape({
	elementRef,
	component,
	style,
}: ComponentProps<ShapeComponent>) {
	const x = useResolved<number>(component.x) ?? 0;
	const y = useResolved<number>(component.y) ?? 0;
	const width = useResolved<number>(component.width) ?? 100;
	const height = useResolved<number>(component.height) ?? 100;
	const radius = useResolved<number>(component.radius) ?? 50;
	const points = useResolved<[number, number][]>(component.points);
	const shapeType = useResolved<string>(component.shapeType);
	const fill = useResolved<string>(component.fill) ?? "transparent";
	const stroke = useResolved<string>(component.stroke) ?? "currentColor";
	const strokeWidth = useResolved<number>(component.strokeWidth) ?? 1;

	const renderShape = () => {
		switch (shapeType) {
			case "rectangle":
				return (
					<rect
						x={0}
						y={0}
						width={width}
						height={height}
						fill={fill}
						stroke={stroke}
						strokeWidth={strokeWidth}
					/>
				);

			case "circle":
				return (
					<circle
						cx={radius}
						cy={radius}
						r={radius}
						fill={fill}
						stroke={stroke}
						strokeWidth={strokeWidth}
					/>
				);

			case "ellipse":
				return (
					<ellipse
						cx={width / 2}
						cy={height / 2}
						rx={width / 2}
						ry={height / 2}
						fill={fill}
						stroke={stroke}
						strokeWidth={strokeWidth}
					/>
				);

			case "polygon":
				if (!points || points.length === 0) return null;
				return (
					<polygon
						points={points.map((p) => p.join(",")).join(" ")}
						fill={fill}
						stroke={stroke}
						strokeWidth={strokeWidth}
					/>
				);

			case "line":
				return (
					<line
						x1={0}
						y1={0}
						x2={width}
						y2={height}
						stroke={stroke}
						strokeWidth={strokeWidth}
					/>
				);

			default:
				return null;
		}
	};

	const svgWidth = shapeType === "circle" ? radius * 2 : width;
	const svgHeight = shapeType === "circle" ? radius * 2 : height;

	return (
		<svg
			ref={elementRef}
			className={cn("absolute", resolveStyle(style))}
			style={{
				left: x,
				top: y,
				width: svgWidth,
				height: svgHeight,
				overflow: "visible",
				...resolveInlineStyle(style),
			}}
		>
			{renderShape()}
		</svg>
	);
}
