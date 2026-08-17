"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useMemo, useRef } from "react";
import { cn } from "../../lib";

export interface RulersProps {
	width: number;
	height: number;
	zoom: number;
	pan: { x: number; y: number };
	rulerSize?: number;
	className?: string;
	showGuides?: boolean;
	guidesColor?: string;
}

const RULER_SIZE = 20;
const TICK_LEVELS = [
	{ minZoom: 0, interval: 100, tickHeight: 10 },
	{ minZoom: 0.5, interval: 50, tickHeight: 6 },
	{ minZoom: 1, interval: 10, tickHeight: 4 },
	{ minZoom: 2, interval: 5, tickHeight: 3 },
];

export function Rulers({
	width,
	height,
	zoom,
	pan,
	rulerSize = RULER_SIZE,
	className,
}: RulersProps) {
	const { t } = useTranslation("flow");
	const horizontalCanvasRef = useRef<HTMLCanvasElement>(null);
	const verticalCanvasRef = useRef<HTMLCanvasElement>(null);

	const tickConfig = useMemo(() => {
		for (let i = TICK_LEVELS.length - 1; i >= 0; i--) {
			if (zoom >= TICK_LEVELS[i].minZoom) {
				return TICK_LEVELS[i];
			}
		}
		return TICK_LEVELS[0];
	}, [zoom]);

	useEffect(() => {
		const hCanvas = horizontalCanvasRef.current;
		if (!hCanvas) return;

		const ctx = hCanvas.getContext("2d");
		if (!ctx) return;

		const dpr = window.devicePixelRatio || 1;
		hCanvas.width = width * dpr;
		hCanvas.height = rulerSize * dpr;
		ctx.scale(dpr, dpr);

		ctx.fillStyle = "#f8fafc";
		ctx.fillRect(0, 0, width, rulerSize);

		ctx.strokeStyle = "#e2e8f0";
		ctx.lineWidth = 1;
		ctx.beginPath();
		ctx.moveTo(0, rulerSize - 0.5);
		ctx.lineTo(width, rulerSize - 0.5);
		ctx.stroke();

		const startX =
			Math.floor(-pan.x / zoom / tickConfig.interval) * tickConfig.interval;
		const endX =
			Math.ceil((-pan.x + width) / zoom / tickConfig.interval) *
			tickConfig.interval;

		ctx.fillStyle = "#64748b";
		ctx.strokeStyle = "#94a3b8";
		ctx.font = `10px system-ui, sans-serif`;
		ctx.textAlign = "center";

		for (let x = startX; x <= endX; x += tickConfig.interval) {
			const screenX = x * zoom + pan.x;
			if (screenX < 0 || screenX > width) continue;

			ctx.beginPath();
			ctx.moveTo(screenX, rulerSize);
			ctx.lineTo(screenX, rulerSize - tickConfig.tickHeight);
			ctx.stroke();

			if (x % 100 === 0) {
				ctx.fillText(x.toString(), screenX, 12);
			}
		}
	}, [width, rulerSize, zoom, pan.x, tickConfig]);

	useEffect(() => {
		const vCanvas = verticalCanvasRef.current;
		if (!vCanvas) return;

		const ctx = vCanvas.getContext("2d");
		if (!ctx) return;

		const dpr = window.devicePixelRatio || 1;
		vCanvas.width = rulerSize * dpr;
		vCanvas.height = height * dpr;
		ctx.scale(dpr, dpr);

		ctx.fillStyle = "#f8fafc";
		ctx.fillRect(0, 0, rulerSize, height);

		ctx.strokeStyle = "#e2e8f0";
		ctx.lineWidth = 1;
		ctx.beginPath();
		ctx.moveTo(rulerSize - 0.5, 0);
		ctx.lineTo(rulerSize - 0.5, height);
		ctx.stroke();

		const startY =
			Math.floor(-pan.y / zoom / tickConfig.interval) * tickConfig.interval;
		const endY =
			Math.ceil((-pan.y + height) / zoom / tickConfig.interval) *
			tickConfig.interval;

		ctx.fillStyle = "#64748b";
		ctx.strokeStyle = "#94a3b8";
		ctx.font = `10px system-ui, sans-serif`;
		ctx.textAlign = "right";

		for (let y = startY; y <= endY; y += tickConfig.interval) {
			const screenY = y * zoom + pan.y;
			if (screenY < 0 || screenY > height) continue;

			ctx.beginPath();
			ctx.moveTo(rulerSize, screenY);
			ctx.lineTo(rulerSize - tickConfig.tickHeight, screenY);
			ctx.stroke();

			if (y % 100 === 0) {
				ctx.save();
				ctx.translate(12, screenY);
				ctx.rotate(-Math.PI / 2);
				ctx.textAlign = "center";
				ctx.fillText(y.toString(), 0, 0);
				ctx.restore();
			}
		}
	}, [height, rulerSize, zoom, pan.y, tickConfig]);

	return (
		<div className={cn("pointer-events-none", className)}>
			<canvas
				ref={horizontalCanvasRef}
				className="absolute top-0 left-0"
				style={{
					width,
					height: rulerSize,
					marginLeft: rulerSize,
				}}
			/>

			<canvas
				ref={verticalCanvasRef}
				className="absolute top-0 left-0"
				style={{
					width: rulerSize,
					height,
					marginTop: rulerSize,
				}}
			/>

			<div
				className="absolute top-0 left-0 bg-slate-50 border-r border-b border-slate-200"
				style={{
					width: rulerSize,
					height: rulerSize,
				}}
			/>
		</div>
	);
}
