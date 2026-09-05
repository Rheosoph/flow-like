"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import type { ComponentProps, RenderChildFn } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, OverlayComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const anchorToClasses: Record<string, string> = {
	topLeft: "top-0 left-0",
	topCenter: "top-0 left-1/2",
	topRight: "top-0 right-0",
	centerLeft: "top-1/2 left-0",
	center: "top-1/2 left-1/2",
	centerRight: "top-1/2 right-0",
	bottomLeft: "bottom-0 left-0",
	bottomCenter: "bottom-0 left-1/2",
	bottomRight: "bottom-0 right-0",
};

const anchorToTransform: Record<string, string> = {
	topCenter: "translateX(-50%)",
	centerLeft: "translateY(-50%)",
	center: "translate(-50%, -50%)",
	centerRight: "translateY(-50%)",
	bottomCenter: "translateX(-50%)",
};

export function A2UIOverlay({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<OverlayComponent> & { renderChild: RenderChildFn }) {
	const { t } = useTranslation("common");
	const { resolve } = useData();

	return (
		<div
			ref={elementRef}
			className={cn("relative", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{renderChild(component.baseComponentId)}
			{(component.overlays ?? []).map((overlay, i) => {
				const anchor = overlay.anchor
					? (resolve(overlay.anchor) as string | undefined)
					: undefined;
				const anchorClass = anchorToClasses[anchor ?? "topLeft"] ?? "";
				const anchorTransform = anchorToTransform[anchor ?? "topLeft"];
				const offsetX = overlay.offsetX
					? (resolve(overlay.offsetX) as string | undefined)
					: undefined;
				const offsetY = overlay.offsetY
					? (resolve(overlay.offsetY) as string | undefined)
					: undefined;
				const zIndex = overlay.zIndex
					? (resolve(overlay.zIndex) as number | undefined)
					: undefined;
				const transforms = [
					anchorTransform,
					(offsetX || offsetY) &&
						`translate(${offsetX ?? "0"}, ${offsetY ?? "0"})`,
				].filter(Boolean);

				return (
					<div
						key={overlay.componentId}
						className={cn("absolute", anchorClass)}
						style={{
							transform: transforms.length ? transforms.join(" ") : undefined,
							zIndex: zIndex ?? i + 1,
						}}
					>
						{renderChild(overlay.componentId)}
					</div>
				);
			})}
		</div>
	);
}
