"use client";

import { Fragment, useEffect, useRef } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps, RenderChildFn } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Canvas2DComponent, Children } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function getChildIds(children: Children | undefined): string[] {
	if (!children) return [];
	if ("explicitList" in children) return children.explicitList;
	return [];
}

export function A2UICanvas2D({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<Canvas2DComponent> & { renderChild: RenderChildFn }) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const width = useResolved<number>(component.width) ?? 800;
	const height = useResolved<number>(component.height) ?? 600;
	const backgroundColor = useResolved<string>(component.backgroundColor);
	const pixelPerfect = useResolved<boolean>(component.pixelPerfect);
	const childIds = getChildIds(component.children);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		if (pixelPerfect) {
			ctx.imageSmoothingEnabled = false;
		}

		if (backgroundColor) {
			ctx.fillStyle = backgroundColor;
			ctx.fillRect(0, 0, width, height);
		}
	}, [width, height, backgroundColor, pixelPerfect]);

	return (
		<div
			ref={elementRef}
			className={cn("relative", resolveStyle(style))}
			style={{ width, height, ...resolveInlineStyle(style) }}
		>
			<canvas
				ref={canvasRef}
				width={width}
				height={height}
				className="absolute inset-0"
			/>
			<div className="absolute inset-0 pointer-events-none">
				{childIds.map((id) => (
					<Fragment key={id}>{renderChild(id)}</Fragment>
				))}
			</div>
		</div>
	);
}
