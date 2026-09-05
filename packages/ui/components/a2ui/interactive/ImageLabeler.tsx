"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { normalizeBoxes } from "../bbox-utils";
import { useAssetUrl } from "../hooks/use-asset-url";
import { useElementRef } from "../hooks/use-element-ref";
import type { BoundValue, ImageLabelerComponent, LabelBox } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

interface DrawingBox {
	x: number;
	y: number;
	width: number;
	height: number;
}

const LABEL_COLORS = [
	"#ef4444", // red
	"#f97316", // orange
	"#eab308", // yellow
	"#22c55e", // green
	"#06b6d4", // cyan
	"#3b82f6", // blue
	"#8b5cf6", // violet
	"#ec4899", // pink
];

export function A2UIImageLabeler({
	elementRef,
	component,
	style,
	surfaceId,
	componentId,
}: ComponentProps<ImageLabelerComponent>) {
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const containerRef = useRef<HTMLDivElement>(null);
	const rootRef = useElementRef(elementRef, containerRef);
	const imageRef = useRef<HTMLImageElement>(null);
	const canvasRef = useRef<HTMLCanvasElement>(null);

	const { url: src } = useAssetUrl(useResolved<string>(component.src));
	const alt =
		useResolved<string>(component.alt) ?? t("imageToLabel", "Image to label");
	const rawBoxes = useResolved<unknown>(component.boxes);
	const labels = useResolved<string[]>(component.labels) ?? [];
	const disabled = useResolved<boolean>(component.disabled) ?? false;
	const showLabels = useResolved<boolean>(component.showLabels) ?? true;
	const minBoxSize = useResolved<number>(component.minBoxSize) ?? 10;

	// Accept boxes in any workflow format (detection output, corner coords, …)
	// and coerce them into the editable LabelBox shape.
	const initialBoxes = useMemo<LabelBox[]>(
		() =>
			normalizeBoxes(rawBoxes).map((box) => ({
				id: box.id,
				x: box.x,
				y: box.y,
				width: box.width,
				height: box.height,
				label: box.label ?? "",
			})),
		[rawBoxes],
	);

	const [boxes, setBoxes] = useState<LabelBox[]>(initialBoxes);
	const [selectedBoxId, setSelectedBoxId] = useState<string | null>(null);
	const [currentLabel, setCurrentLabel] = useState<string>(labels[0] ?? "");
	const [isDrawing, setIsDrawing] = useState(false);
	const [drawingBox, setDrawingBox] = useState<DrawingBox | null>(null);
	const [imageLoaded, setImageLoaded] = useState(false);
	const [imageSize, setImageSize] = useState({ width: 0, height: 0 });

	// Sync with external boxes prop
	useEffect(() => {
		setBoxes(initialBoxes);
	}, [initialBoxes]);

	const labelColorMap = useMemo(() => {
		const map: Record<string, string> = {};
		labels.forEach((label, i) => {
			map[label] = LABEL_COLORS[i % LABEL_COLORS.length];
		});
		return map;
	}, [labels]);

	const getMousePos = useCallback(
		(
			e: React.MouseEvent<HTMLCanvasElement>,
		): { x: number; y: number } | null => {
			const canvas = canvasRef.current;
			if (!canvas) return null;
			const rect = canvas.getBoundingClientRect();
			return {
				x: ((e.clientX - rect.left) / rect.width) * imageSize.width,
				y: ((e.clientY - rect.top) / rect.height) * imageSize.height,
			};
		},
		[imageSize],
	);

	const handleMouseDown = useCallback(
		(e: React.MouseEvent<HTMLCanvasElement>) => {
			if (disabled) return;
			const pos = getMousePos(e);
			if (!pos) return;

			// Check if clicking on existing box
			const clickedBox = boxes.find(
				(box) =>
					pos.x >= box.x &&
					pos.x <= box.x + box.width &&
					pos.y >= box.y &&
					pos.y <= box.y + box.height,
			);

			if (clickedBox) {
				setSelectedBoxId(clickedBox.id);
				return;
			}

			// Start drawing new box
			setIsDrawing(true);
			setSelectedBoxId(null);
			setDrawingBox({ x: pos.x, y: pos.y, width: 0, height: 0 });
		},
		[disabled, getMousePos, boxes],
	);

	const handleMouseMove = useCallback(
		(e: React.MouseEvent<HTMLCanvasElement>) => {
			if (!isDrawing || disabled) return;
			const pos = getMousePos(e);
			if (!pos || !drawingBox) return;

			setDrawingBox({
				...drawingBox,
				width: pos.x - drawingBox.x,
				height: pos.y - drawingBox.y,
			});
		},
		[isDrawing, disabled, getMousePos, drawingBox],
	);

	// Persist the current boxes as this component's element value so workflow /
	// widget action events can read them back (mirrors the input "change" pattern).
	const fireBoxesChange = useCallback(
		(updatedBoxes: LabelBox[]) => {
			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: updatedBoxes },
			});

			void triggerEvent("change", component, {
				value: updatedBoxes,
				boxes: updatedBoxes,
			});
		},
		[component, componentId, onAction, surfaceId, triggerEvent],
	);

	const handleMouseUp = useCallback(() => {
		if (!isDrawing || !drawingBox) {
			setIsDrawing(false);
			return;
		}

		// Normalize box (handle negative width/height)
		let { x, y, width, height } = drawingBox;
		if (width < 0) {
			x += width;
			width = Math.abs(width);
		}
		if (height < 0) {
			y += height;
			height = Math.abs(height);
		}

		// Only add if box is large enough
		if (width >= minBoxSize && height >= minBoxSize) {
			const newBox: LabelBox = {
				id: `box_${Date.now()}`,
				x,
				y,
				width,
				height,
				label: currentLabel || labels[0] || "",
			};
			const updatedBoxes = [...boxes, newBox];
			setBoxes(updatedBoxes);
			setSelectedBoxId(newBox.id);
			fireBoxesChange(updatedBoxes);
		}

		setIsDrawing(false);
		setDrawingBox(null);
	}, [
		isDrawing,
		drawingBox,
		minBoxSize,
		currentLabel,
		labels,
		boxes,
		fireBoxesChange,
	]);

	const handleDeleteSelected = useCallback(() => {
		if (!selectedBoxId) return;
		const updatedBoxes = boxes.filter((b) => b.id !== selectedBoxId);
		setBoxes(updatedBoxes);
		setSelectedBoxId(null);
		fireBoxesChange(updatedBoxes);
	}, [selectedBoxId, boxes, fireBoxesChange]);

	const handleLabelChange = useCallback(
		(label: string) => {
			setCurrentLabel(label);
			if (!selectedBoxId) return;

			const updatedBoxes = boxes.map((b) =>
				b.id === selectedBoxId ? { ...b, label } : b,
			);
			setBoxes(updatedBoxes);
			fireBoxesChange(updatedBoxes);
		},
		[selectedBoxId, boxes, fireBoxesChange],
	);

	// Draw canvas
	useEffect(() => {
		const canvas = canvasRef.current;
		const ctx = canvas?.getContext("2d");
		if (!canvas || !ctx || !imageLoaded) return;

		ctx.clearRect(0, 0, canvas.width, canvas.height);

		// Draw existing boxes
		for (const box of boxes) {
			const color = labelColorMap[box.label] ?? LABEL_COLORS[0];
			const isSelected = box.id === selectedBoxId;

			ctx.strokeStyle = color;
			ctx.lineWidth = isSelected ? 3 : 2;
			ctx.strokeRect(box.x, box.y, box.width, box.height);

			// Fill with transparent color
			ctx.fillStyle = `${color}20`;
			ctx.fillRect(box.x, box.y, box.width, box.height);

			// Draw label
			if (showLabels && box.label) {
				ctx.fillStyle = color;
				ctx.font = "12px sans-serif";
				const textMetrics = ctx.measureText(box.label);
				const textHeight = 16;
				ctx.fillRect(
					box.x,
					box.y - textHeight,
					textMetrics.width + 8,
					textHeight,
				);
				ctx.fillStyle = "#fff";
				ctx.fillText(box.label, box.x + 4, box.y - 4);
			}
		}

		// Draw current drawing box
		if (drawingBox) {
			const color = labelColorMap[currentLabel] ?? LABEL_COLORS[0];
			ctx.strokeStyle = color;
			ctx.lineWidth = 2;
			ctx.setLineDash([5, 5]);
			ctx.strokeRect(
				drawingBox.x,
				drawingBox.y,
				drawingBox.width,
				drawingBox.height,
			);
			ctx.setLineDash([]);
		}
	}, [
		boxes,
		drawingBox,
		selectedBoxId,
		labelColorMap,
		showLabels,
		currentLabel,
		imageLoaded,
	]);

	const handleImageLoad = useCallback(() => {
		if (imageRef.current) {
			setImageSize({
				width: imageRef.current.naturalWidth,
				height: imageRef.current.naturalHeight,
			});
			setImageLoaded(true);
		}
	}, []);

	// Handle keyboard shortcuts
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if (e.key === "Delete" || e.key === "Backspace") {
				handleDeleteSelected();
			}
		};
		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [handleDeleteSelected]);

	return (
		<div
			ref={rootRef}
			className={cn("flex flex-col gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{/* Label selector */}
			{labels.length > 0 && (
				<div className="flex flex-wrap gap-2 p-2 bg-muted/30 rounded">
					{labels.map((label) => (
						<button
							key={label}
							type="button"
							onClick={() => handleLabelChange(label)}
							className={cn(
								"px-3 py-1 text-sm rounded-full border transition-colors",
								currentLabel === label
									? "border-primary bg-primary text-primary-foreground"
									: "border-border hover:border-primary",
							)}
							style={{
								backgroundColor:
									currentLabel === label ? labelColorMap[label] : undefined,
							}}
						>
							{label}
						</button>
					))}
				</div>
			)}

			{/* Image with canvas overlay */}
			<div className="relative">
				<img
					ref={imageRef}
					src={src}
					alt={alt}
					onLoad={handleImageLoad}
					className="max-w-full h-auto"
					style={{ display: imageLoaded ? "block" : "none" }}
				/>
				{imageLoaded && (
					<canvas
						ref={canvasRef}
						width={imageSize.width}
						height={imageSize.height}
						onMouseDown={handleMouseDown}
						onMouseMove={handleMouseMove}
						onMouseUp={handleMouseUp}
						onMouseLeave={handleMouseUp}
						className="absolute top-0 left-0 w-full h-full cursor-crosshair"
						style={{ pointerEvents: disabled ? "none" : "auto" }}
					/>
				)}
				{!imageLoaded && src && (
					<div className="flex items-center justify-center h-64 bg-muted/30 text-muted-foreground">
						{t("loadingImage", "Loading image...")}
					</div>
				)}
			</div>

			{/* Selected box info */}
			{selectedBoxId && (
				<div className="flex items-center gap-2 p-2 bg-muted/30 rounded text-sm">
					<span className="text-muted-foreground">
						{t("selected", "Selected:")}
					</span>
					<span>
						{boxes.find((b) => b.id === selectedBoxId)?.label ||
							t("noLabel", "No label")}
					</span>
					<button
						type="button"
						onClick={handleDeleteSelected}
						className="ml-auto px-2 py-1 text-destructive hover:bg-destructive/10 rounded"
					>
						{t("delete", "Delete")}
					</button>
				</div>
			)}
		</div>
	);
}
