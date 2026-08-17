"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type MouseEvent,
	type WheelEvent,
	useCallback,
	useEffect,
	useRef,
} from "react";
import { cn } from "../../lib";
import { useBuilder } from "./BuilderContext";

export interface CanvasProps {
	children?: React.ReactNode;
	className?: string;
	width?: number;
	height?: number;
	dropRef?: (node: HTMLDivElement | null) => void;
}

export function Canvas({
	children,
	className,
	width = 1440,
	height = 900,
	dropRef,
}: CanvasProps) {
	const { t } = useTranslation("flow");
	const containerRef = useRef<HTMLDivElement>(null);
	const canvasRef = useRef<HTMLDivElement>(null);
	const isPanningRef = useRef(false);
	const lastPanPointRef = useRef({ x: 0, y: 0 });

	const { zoom, setZoom, pan, setPan, showGrid, gridSize, deselectAll } =
		useBuilder();

	// Handle wheel for zoom
	const handleWheel = useCallback(
		(e: WheelEvent<HTMLDivElement>) => {
			if (e.ctrlKey || e.metaKey) {
				e.preventDefault();
				const delta = e.deltaY > 0 ? -0.1 : 0.1;
				const newZoom = Math.max(0.1, Math.min(4, zoom + delta));
				setZoom(newZoom);
			}
		},
		[zoom, setZoom],
	);

	// Handle mouse down for pan
	const handleMouseDown = useCallback(
		(e: MouseEvent<HTMLDivElement>) => {
			if (e.button === 1 || (e.button === 0 && e.altKey)) {
				// Middle click or Alt+left click for panning
				isPanningRef.current = true;
				lastPanPointRef.current = { x: e.clientX, y: e.clientY };
				e.preventDefault();
			} else if (e.target === canvasRef.current) {
				// Click on canvas background deselects
				deselectAll();
			}
		},
		[deselectAll],
	);

	const handleMouseMove = useCallback(
		(e: MouseEvent<HTMLDivElement>) => {
			if (isPanningRef.current) {
				const dx = e.clientX - lastPanPointRef.current.x;
				const dy = e.clientY - lastPanPointRef.current.y;
				setPan({ x: pan.x + dx, y: pan.y + dy });
				lastPanPointRef.current = { x: e.clientX, y: e.clientY };
			}
		},
		[pan, setPan],
	);

	const handleMouseUp = useCallback(() => {
		isPanningRef.current = false;
	}, []);

	// Keyboard shortcuts
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			// Zoom shortcuts
			if ((e.ctrlKey || e.metaKey) && e.key === "=") {
				e.preventDefault();
				setZoom(Math.min(4, zoom + 0.25));
			} else if ((e.ctrlKey || e.metaKey) && e.key === "-") {
				e.preventDefault();
				setZoom(Math.max(0.1, zoom - 0.25));
			} else if ((e.ctrlKey || e.metaKey) && e.key === "0") {
				e.preventDefault();
				setZoom(1);
				setPan({ x: 0, y: 0 });
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [zoom, setZoom, setPan]);

	// Grid pattern
	const gridPattern = showGrid
		? `url("data:image/svg+xml,%3Csvg width='${gridSize}' height='${gridSize}' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M ${gridSize} 0 L 0 0 0 ${gridSize}' fill='none' stroke='%23e5e7eb' stroke-width='0.5'/%3E%3C/svg%3E")`
		: "none";

	return (
		<div
			ref={containerRef}
			className={cn("relative overflow-hidden bg-muted/50", className)}
			onWheel={handleWheel}
			onMouseDown={handleMouseDown}
			onMouseMove={handleMouseMove}
			onMouseUp={handleMouseUp}
			onMouseLeave={handleMouseUp}
		>
			{/* Zoom indicator */}
			<div className="absolute top-4 left-4 z-10 flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5 text-sm text-muted-foreground backdrop-blur">
				<span>{Math.round(zoom * 100)}%</span>
			</div>

			{/* Canvas area */}
			<div
				className="absolute inset-0 flex items-center justify-center"
				style={{
					transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
					transformOrigin: "center center",
				}}
			>
				<div
					ref={(node) => {
						(
							canvasRef as React.MutableRefObject<HTMLDivElement | null>
						).current = node;
						dropRef?.(node);
					}}
					className="relative bg-background shadow-lg rounded-lg border"
					style={{
						width,
						height,
						backgroundImage: gridPattern,
						backgroundSize: `${gridSize}px ${gridSize}px`,
					}}
				>
					{children}
				</div>
			</div>
		</div>
	);
}
