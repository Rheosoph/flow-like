"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { type NormalizedBox, normalizeBoxes } from "../bbox-utils";
import { useAssetUrl } from "../hooks/use-asset-url";
import { useElementRef } from "../hooks/use-element-ref";
import type { BoundValue, BoundingBoxOverlayComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const DEFAULT_COLORS = [
	"#ef4444", // red
	"#f97316", // orange
	"#eab308", // yellow
	"#22c55e", // green
	"#06b6d4", // cyan
	"#3b82f6", // blue
	"#8b5cf6", // violet
	"#ec4899", // pink
];

export function A2UIBoundingBoxOverlay({
	elementRef,
	component,
	style,
	componentId,
}: ComponentProps<BoundingBoxOverlayComponent>) {
	const { t } = useTranslation("common");
	const triggerEvent = useComponentEventTrigger(componentId);
	const containerRef = useRef<HTMLDivElement>(null);
	const rootRef = useElementRef(elementRef, containerRef);
	const imageRef = useRef<HTMLImageElement>(null);

	const { url: src } = useAssetUrl(useResolved<string>(component.src));
	const alt =
		useResolved<string>(component.alt) ??
		t("imageWithBoundingBoxes", "Image with bounding boxes");
	const rawBoxes = useResolved<unknown>(component.boxes);
	const showLabels = useResolved<boolean>(component.showLabels) ?? true;
	const showConfidence = useResolved<boolean>(component.showConfidence) ?? true;
	const strokeWidth = useResolved<number>(component.strokeWidth) ?? 2;
	const fontSize = useResolved<number>(component.fontSize) ?? 12;
	const fit = useResolved<string>(component.fit) ?? "contain";
	const normalized = useResolved<boolean>(component.normalized) ?? false;
	const interactive = useResolved<boolean>(component.interactive) ?? false;

	const [imageLoaded, setImageLoaded] = useState(false);
	const [imageSize, setImageSize] = useState({ width: 0, height: 0 });
	const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
	const [hoveredBoxId, setHoveredBoxId] = useState<string | null>(null);

	// Parse boxes from various formats (top-left/size, corner coords, detection output, …)
	const boxes = useMemo(
		(): NormalizedBox[] => normalizeBoxes(rawBoxes),
		[rawBoxes],
	);

	// Create color map for labels
	const labelColorMap = useMemo(() => {
		const uniqueLabels = [
			...new Set(boxes.map((b) => b.label).filter(Boolean)),
		];
		const map: Record<string, string> = {};
		uniqueLabels.forEach((label, i) => {
			if (label) map[label] = DEFAULT_COLORS[i % DEFAULT_COLORS.length];
		});
		return map;
	}, [boxes]);

	const handleImageLoad = useCallback(() => {
		if (imageRef.current) {
			setImageSize({
				width: imageRef.current.naturalWidth,
				height: imageRef.current.naturalHeight,
			});
			setImageLoaded(true);
		}
	}, []);

	// Track container size for proper scaling
	useEffect(() => {
		if (!containerRef.current) return;
		const observer = new ResizeObserver((entries) => {
			for (const entry of entries) {
				setContainerSize({
					width: entry.contentRect.width,
					height: entry.contentRect.height,
				});
			}
		});
		observer.observe(containerRef.current);
		return () => observer.disconnect();
	}, []);

	// Calculate scaling factor for box positions
	const scale = useMemo(() => {
		if (!imageLoaded || !imageSize.width || !imageSize.height) {
			return { x: 1, y: 1, offsetX: 0, offsetY: 0 };
		}

		const imgAspect = imageSize.width / imageSize.height;
		const containerAspect =
			containerSize.width / (containerSize.height || containerSize.width);

		let displayWidth: number;
		let displayHeight: number;
		let offsetX = 0;
		let offsetY = 0;

		if (fit === "contain") {
			if (imgAspect > containerAspect) {
				displayWidth = containerSize.width;
				displayHeight = containerSize.width / imgAspect;
				offsetY = (containerSize.height - displayHeight) / 2;
			} else {
				displayHeight = containerSize.height || containerSize.width / imgAspect;
				displayWidth = displayHeight * imgAspect;
				offsetX = (containerSize.width - displayWidth) / 2;
			}
		} else {
			displayWidth = containerSize.width;
			displayHeight = containerSize.height || containerSize.width / imgAspect;
		}

		return {
			x: displayWidth / (normalized ? 1 : imageSize.width),
			y: displayHeight / (normalized ? 1 : imageSize.height),
			offsetX,
			offsetY,
		};
	}, [imageLoaded, imageSize, containerSize, fit, normalized]);

	const handleBoxClick = useCallback(
		(box: NormalizedBox) => {
			if (!interactive) return;
			void triggerEvent("boxClick", component, { box });
		},
		[component, interactive, triggerEvent],
	);

	const fitClass =
		{
			contain: "object-contain",
			cover: "object-cover",
			fill: "object-fill",
		}[fit] ?? "object-contain";

	return (
		<div
			data-card-action-stop
			ref={rootRef}
			className={cn("relative", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<img
				ref={imageRef}
				src={src}
				alt={alt}
				onLoad={handleImageLoad}
				className={cn("w-full h-full", fitClass)}
			/>

			{/* Bounding box overlays */}
			{imageLoaded &&
				boxes.map((box, index) => {
					const color =
						box.color ??
						labelColorMap[box.label ?? ""] ??
						DEFAULT_COLORS[index % DEFAULT_COLORS.length];
					const isHovered = hoveredBoxId === (box.id ?? `box_${index}`);
					const boxId = box.id ?? `box_${index}`;

					const left = box.x * scale.x + scale.offsetX;
					const top = box.y * scale.y + scale.offsetY;
					const width = box.width * scale.x;
					const height = box.height * scale.y;

					return (
						<button
							type="button"
							key={boxId}
							className={cn(
								"absolute appearance-none bg-transparent p-0 text-left border transition-opacity",
								interactive && "cursor-pointer hover:opacity-80",
							)}
							disabled={!interactive}
							aria-label={
								box.label
									? t("boundingBoxLabel", "Bounding box: {{label}}", {
											label: box.label,
										})
									: t("boundingBoxVal", "Bounding box {{val}}", {
											val: index + 1,
										})
							}
							style={{
								left: `${left}px`,
								top: `${top}px`,
								width: `${width}px`,
								height: `${height}px`,
								borderColor: color,
								borderWidth: `${isHovered ? strokeWidth + 1 : strokeWidth}px`,
								backgroundColor: `${color}${isHovered ? "30" : "15"}`,
							}}
							onClick={() => handleBoxClick(box)}
							onMouseEnter={() => setHoveredBoxId(boxId)}
							onMouseLeave={() => setHoveredBoxId(null)}
						>
							{/* Label */}
							{showLabels && box.label && (
								<div
									className="absolute -top-6 left-0 px-1.5 py-0.5 text-white whitespace-nowrap"
									style={{
										backgroundColor: color,
										fontSize: `${fontSize}px`,
										lineHeight: 1.2,
									}}
								>
									{box.label}
									{showConfidence && box.confidence !== undefined && (
										<span className="ml-1 opacity-80">
											{(box.confidence * 100).toFixed(0)}%
										</span>
									)}
								</div>
							)}
						</button>
					);
				})}
		</div>
	);
}
