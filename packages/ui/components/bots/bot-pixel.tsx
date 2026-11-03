"use client";

import { type RefObject, useEffect, useRef } from "react";
import type { BotFrame } from "./types";
import "./bot-pixel.css";

export const BOT_PIXEL_SIZE = 24;
const SCALE = BOT_PIXEL_SIZE / 100;
type Color = readonly [number, number, number, number];
type Paint = (x: number, y: number) => Color;
type ColorTransform = (value: Color) => Color;
const CLEAR: Color = [0, 0, 0, 0];
const SKIP = new Set([
	"defs",
	"clippath",
	"filter",
	"mask",
	"pattern",
	"lineargradient",
	"radialgradient",
	"title",
	"desc",
]);

function number(element: Element, name: string, fallback = 0) {
	const value = Number.parseFloat(element.getAttribute(name) ?? "");
	return Number.isFinite(value) ? value : fallback;
}

function geometry(element: Element): Path2D | undefined {
	const path = new Path2D();
	const x = number(element, "x");
	const y = number(element, "y");
	switch (element.tagName.toLowerCase()) {
		case "path":
			return new Path2D(element.getAttribute("d") ?? "");
		case "rect": {
			const width = number(element, "width");
			const height = number(element, "height");
			const rx = Math.min(
				width / 2,
				number(element, "rx", number(element, "ry")),
			);
			const ry = Math.min(height / 2, number(element, "ry", rx));
			path.moveTo(x + rx, y);
			path.lineTo(x + width - rx, y);
			path.quadraticCurveTo(x + width, y, x + width, y + ry);
			path.lineTo(x + width, y + height - ry);
			path.quadraticCurveTo(x + width, y + height, x + width - rx, y + height);
			path.lineTo(x + rx, y + height);
			path.quadraticCurveTo(x, y + height, x, y + height - ry);
			path.lineTo(x, y + ry);
			path.quadraticCurveTo(x, y, x + rx, y);
			path.closePath();
			return path;
		}
		case "circle":
		case "ellipse": {
			const radius = number(element, "r");
			path.ellipse(
				number(element, "cx"),
				number(element, "cy"),
				number(element, "rx", radius),
				number(element, "ry", radius),
				0,
				0,
				Math.PI * 2,
			);
			return path;
		}
		case "line":
			path.moveTo(number(element, "x1"), number(element, "y1"));
			path.lineTo(number(element, "x2"), number(element, "y2"));
			return path;
		case "polygon":
		case "polyline": {
			const points = (element.getAttribute("points") ?? "")
				.trim()
				.split(/[\s,]+/)
				.map(Number);
			for (let i = 0; i + 1 < points.length; i += 2) {
				if (i === 0) path.moveTo(points[i], points[i + 1]);
				else path.lineTo(points[i], points[i + 1]);
			}
			if (element.tagName.toLowerCase() === "polygon") path.closePath();
			return path;
		}
		default:
			return undefined;
	}
}

function reference(value: string): string | undefined {
	return value.match(/url\(["']?(?:[^#)]*)#([^"')]+)["']?\)/)?.[1];
}

function color(value: string): Color {
	if (value === "none" || value === "transparent") return CLEAR;
	if (value.startsWith("#")) {
		let hex = value.slice(1);
		if (hex.length === 3 || hex.length === 4)
			hex = [...hex].map((part) => part + part).join("");
		return [
			Number.parseInt(hex.slice(0, 2), 16),
			Number.parseInt(hex.slice(2, 4), 16),
			Number.parseInt(hex.slice(4, 6), 16),
			hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) / 255 : 1,
		];
	}
	const values = value.match(/[\d.]+/g)?.map(Number);
	if (values && values.length >= 3)
		return [values[0], values[1], values[2], values[3] ?? 1];
	return [40, 36, 55, 1];
}

function step(channel: number) {
	return Math.max(0, Math.min(255, Math.round(channel / 17) * 17));
}

function flatColor(value: string): Color {
	const parsed = color(value);
	return [step(parsed[0]), step(parsed[1]), step(parsed[2]), parsed[3]];
}

function transformed(path: Path2D, matrix: DOMMatrix): Path2D {
	const result = new Path2D();
	result.addPath(path, matrix);
	return result;
}

function listMatrix(list?: SVGTransformList): DOMMatrix {
	let result = new DOMMatrix();
	for (let index = 0; index < (list?.numberOfItems ?? 0); index++) {
		const matrix = list?.getItem(index).matrix;
		if (matrix) result = result.multiply(matrix);
	}
	return result;
}

function matrixColor(values: number[]): ColorTransform {
	return (value) => {
		const input = [value[0] / 255, value[1] / 255, value[2] / 255, value[3], 1];
		const output = [0, 0, 0, 0];
		for (let row = 0; row < 4; row++)
			for (let column = 0; column < 5; column++)
				output[row] += values[row * 5 + column] * input[column];
		return [
			step(output[0] * 255),
			step(output[1] * 255),
			step(output[2] * 255),
			Math.max(0, Math.min(1, output[3])),
		];
	};
}

function saturation(value: number): ColorTransform {
	const r = 0.213;
	const g = 0.715;
	const b = 0.072;
	return matrixColor([
		r + (1 - r) * value,
		g - g * value,
		b - b * value,
		0,
		0,
		r - r * value,
		g + (1 - g) * value,
		b - b * value,
		0,
		0,
		r - r * value,
		g - g * value,
		b + (1 - b) * value,
		0,
		0,
		0,
		0,
		0,
		1,
		0,
	]);
}

/** Samples vector geometry at grid centers, without a bitmap resampling filter. */
export function drawBotPixels(
	source: SVGSVGElement,
	canvas: HTMLCanvasElement,
	ink: string,
): boolean {
	const context = canvas.getContext("2d", { willReadFrequently: true });
	const rootMatrix = source.getCTM();
	if (!context || !rootMatrix || typeof Path2D === "undefined") return false;
	const base = new DOMMatrix().scale(SCALE).multiply(rootMatrix.inverse());
	const image = context.createImageData(BOT_PIXEL_SIZE, BOT_PIXEL_SIZE);
	const surface = image.data;
	const inkColor = flatColor(ink);
	const expression = source
		.closest("[data-expression]")
		?.getAttribute("data-expression");
	const sleeping =
		source.closest("[data-pose]")?.getAttribute("data-pose") === "sleep";
	const styles = new Map<Element, CSSStyleDeclaration>();
	const styleOf = (element: Element) => {
		let style = styles.get(element);
		if (!style) {
			style = getComputedStyle(element);
			styles.set(element, style);
		}
		return style;
	};
	const matrixOf = (element: SVGGraphicsElement) => {
		const matrix = element.getCTM();
		return matrix ? base.multiply(matrix) : new DOMMatrix().scale(SCALE);
	};
	const find = (id: string): Element | null => {
		const result = source.ownerDocument.getElementById(id);
		return result && source.contains(result) ? result : null;
	};
	const blend = (index: number, value: Color, opacity: number) => {
		const alpha = value[3] * opacity;
		if (alpha <= 0) return;
		const offset = index * 4;
		const prior = surface[offset + 3] / 255;
		const total = alpha + prior * (1 - alpha);
		for (let channel = 0; channel < 3; channel++) {
			surface[offset + channel] =
				(value[channel] * alpha +
					surface[offset + channel] * prior * (1 - alpha)) /
				total;
		}
		surface[offset + 3] = total * 255;
	};
	const paint = (value: string, element: SVGGraphicsElement): Paint => {
		const id = reference(value);
		const server = id ? find(id) : null;
		if (!server) {
			const solid = flatColor(value);
			return () => solid;
		}
		const stops = [...server.querySelectorAll("stop")].map((stop) => ({
			offset:
				number(stop, "offset") /
				(stop.getAttribute("offset")?.includes("%") ? 100 : 1),
			color: [
				...flatColor(styleOf(stop).stopColor).slice(0, 3),
				Number.parseFloat(styleOf(stop).stopOpacity || "1"),
			] as unknown as Color,
		}));
		if (stops.length) {
			const inverse = matrixOf(element).inverse();
			const userSpace =
				server.getAttribute("gradientUnits") === "userSpaceOnUse";
			const bounds = userSpace
				? { x: 0, y: 0, width: 1, height: 1 }
				: element.getBBox();
			const coordinate = (key: string, fallback: number, axis: "x" | "y") => {
				const raw = server.getAttribute(key);
				let result = number(server, key, fallback);
				if (raw?.includes("%")) result /= 100;
				return (
					(axis === "x" ? bounds.x : bounds.y) +
					result * (axis === "x" ? bounds.width : bounds.height)
				);
			};
			const x1 = coordinate("x1", 0, "x");
			const y1 = coordinate("y1", 0, "y");
			const x2 = coordinate("x2", userSpace ? 100 : 1, "x");
			const y2 = coordinate("y2", 0, "y");
			return (x, y) => {
				const local = new DOMPoint(x, y).matrixTransform(inverse);
				const dx = x2 - x1;
				const dy = y2 - y1;
				const position = Math.max(
					0,
					Math.min(
						1,
						((local.x - x1) * dx + (local.y - y1) * dy) /
							(dx * dx + dy * dy || 1),
					),
				);
				let nearest = stops[0];
				for (const stop of stops)
					if (
						Math.abs(stop.offset - position) <
						Math.abs(nearest.offset - position)
					)
						nearest = stop;
				return nearest.color;
			};
		}
		// Patterns keep their base and motif colors on the same fixed grid.
		const shapes = [...server.children]
			.map((child) => {
				const path = geometry(child);
				const transform = listMatrix(
					(child as SVGGraphicsElement).transform?.baseVal,
				);
				return {
					path: path ? transformed(path, transform) : undefined,
					fill: flatColor(styleOf(child).fill),
					stroke: flatColor(styleOf(child).stroke),
					width: Number.parseFloat(styleOf(child).strokeWidth || "1"),
				};
			})
			.filter((part) => part.path);
		const inverse = matrixOf(element).inverse();
		const patternInverse = listMatrix(
			(server as SVGPatternElement).patternTransform?.baseVal,
		).inverse();
		const width = number(server, "width", 8);
		const height = number(server, "height", 8);
		return (x, y) => {
			const local = new DOMPoint(x, y)
				.matrixTransform(inverse)
				.matrixTransform(patternInverse);
			const px = ((local.x % width) + width) % width;
			const py = ((local.y % height) + height) % height;
			let selected = CLEAR;
			for (const part of shapes)
				if (part.path) {
					if (part.fill[3] && context.isPointInPath(part.path, px, py))
						selected = part.fill;
					context.lineWidth = part.width;
					if (part.stroke[3] && context.isPointInStroke(part.path, px, py))
						selected = part.stroke;
				}
			return selected;
		};
	};
	const clipMask = (
		element: SVGGraphicsElement,
		parent: Uint8Array | undefined,
	): Uint8Array | undefined => {
		const id = reference(styleOf(element).clipPath);
		if (!id || id.endsWith("-frame")) return parent;
		const clip = find(id) as SVGGraphicsElement | null;
		if (!clip) return parent;
		const path = new Path2D();
		const clipMatrix =
			typeof clip.getCTM === "function"
				? clip.getCTM()?.inverse()
				: rootMatrix.inverse();
		for (const child of clip.querySelectorAll<SVGGraphicsElement>(
			"path, rect, circle, ellipse, polygon, polyline",
		)) {
			const shape = geometry(child);
			const matrix = child.getCTM();
			if (shape && matrix)
				path.addPath(
					shape,
					matrixOf(element).multiply(
						clipMatrix ? clipMatrix.multiply(matrix) : new DOMMatrix(),
					),
				);
		}
		const mask = new Uint8Array(BOT_PIXEL_SIZE * BOT_PIXEL_SIZE);
		const rule = [...clip.children].some(
			(child) => styleOf(child).clipRule === "evenodd",
		)
			? "evenodd"
			: "nonzero";
		for (let index = 0; index < mask.length; index++) {
			mask[index] =
				(!parent || parent[index]) &&
				context.isPointInPath(
					path,
					(index % BOT_PIXEL_SIZE) + 0.5,
					Math.floor(index / BOT_PIXEL_SIZE) + 0.5,
					rule,
				)
					? 1
					: 0;
		}
		return mask;
	};
	const filterColors = (
		element: SVGGraphicsElement,
		parent: ColorTransform,
	): ColorTransform => {
		const filter = styleOf(element).filter;
		const definition = reference(filter);
		const transforms: ColorTransform[] = [];
		const hue = filter.match(/hue-rotate\(([-\d.]+)deg\)/);
		if (hue) {
			const angle = (Number(hue[1]) * Math.PI) / 180;
			const c = Math.cos(angle);
			const s = Math.sin(angle);
			transforms.push(
				matrixColor([
					0.213 + 0.787 * c - 0.213 * s,
					0.715 - 0.715 * c - 0.715 * s,
					0.072 - 0.072 * c + 0.928 * s,
					0,
					0,
					0.213 - 0.213 * c + 0.143 * s,
					0.715 + 0.285 * c + 0.14 * s,
					0.072 - 0.072 * c - 0.283 * s,
					0,
					0,
					0.213 - 0.213 * c - 0.787 * s,
					0.715 - 0.715 * c + 0.715 * s,
					0.072 + 0.928 * c + 0.072 * s,
					0,
					0,
					0,
					0,
					0,
					1,
					0,
				]),
			);
		}
		const server = definition ? find(definition) : null;
		for (const node of server?.children ?? []) {
			if (node.tagName.toLowerCase() === "fecolormatrix") {
				const values = (node.getAttribute("values") ?? "")
					.trim()
					.split(/[\s,]+/)
					.map(Number);
				if (node.getAttribute("type") === "saturate")
					transforms.push(saturation(values[0] ?? 1));
				else if (values.length === 20) transforms.push(matrixColor(values));
			} else if (node.tagName.toLowerCase() === "fecomponenttransfer") {
				const tables = ["feFuncR", "feFuncG", "feFuncB"].map((name) =>
					(node.querySelector(name)?.getAttribute("tableValues") ?? "")
						.trim()
						.split(/[\s,]+/)
						.map(Number),
				);
				transforms.push(
					(value) =>
						[0, 1, 2]
							.map((channel) => {
								const table = tables[channel];
								if (table.length < 2) return value[channel];
								const position = (value[channel] / 255) * (table.length - 1);
								const index = Math.floor(position);
								return step(
									(table[index] +
										((table[index + 1] ?? table[index]) - table[index]) *
											(position - index)) *
										255,
								);
							})
							.concat(value[3]) as unknown as Color,
				);
			}
		}
		return transforms.length
			? (value) =>
					parent(
						transforms.reduce((result, transform) => transform(result), value),
					)
			: parent;
	};
	const maskPaint = (element: SVGGraphicsElement): Paint | undefined => {
		const id = reference(
			element.getAttribute("mask") ?? styleOf(element).maskImage,
		);
		const mask = id ? find(id) : null;
		if (!mask) return undefined;
		const parts = [
			...mask.querySelectorAll<SVGGraphicsElement>(
				"path, rect, circle, ellipse",
			),
		].map((child) => ({
			path: geometry(child),
			fill: paint(styleOf(child).fill, element),
		}));
		const inverse = matrixOf(element).inverse();
		return (x, y) => {
			const local = new DOMPoint(x, y).matrixTransform(inverse);
			let opacity = 0;
			for (const part of parts)
				if (part.path && context.isPointInPath(part.path, local.x, local.y)) {
					const value = part.fill(x, y);
					opacity =
						(value[3] *
							(0.2126 * value[0] + 0.7152 * value[1] + 0.0722 * value[2])) /
						255;
				}
			return [255, 255, 255, opacity];
		};
	};
	const walk = (
		element: SVGGraphicsElement,
		inheritedOpacity: number,
		inheritedClip?: Uint8Array,
		inheritedColors: ColorTransform = (value) => value,
	) => {
		if (SKIP.has(element.tagName.toLowerCase())) return;
		const style = styleOf(element);
		if (style.display === "none" || style.visibility === "hidden") return;
		const opacity = inheritedOpacity * Number.parseFloat(style.opacity || "1");
		if (opacity < 0.02) return;
		const clip = clipMask(element, inheritedClip);
		const transformColors = filterColors(element, inheritedColors);
		const mask = maskPaint(element);
		const shape = geometry(element);
		if (shape) {
			const matrix = matrixOf(element);
			const path = transformed(shape, matrix);
			const fillPaint = paint(style.fill, element);
			const strokePaint = paint(style.stroke, element);
			const fill: Paint = (x, y) => transformColors(fillPaint(x, y));
			const stroke: Paint = (x, y) => transformColors(strokePaint(x, y));
			const fillOpacity = opacity * Number.parseFloat(style.fillOpacity || "1");
			const strokeOpacity =
				opacity * Number.parseFloat(style.strokeOpacity || "1");
			const width = Math.max(
				1,
				Math.round(
					Number.parseFloat(style.strokeWidth || "1") *
						Math.hypot(matrix.a, matrix.b),
				),
			);
			context.lineCap = style.strokeLinecap as CanvasLineCap;
			context.lineJoin = style.strokeLinejoin as CanvasLineJoin;
			const outline = element.hasAttribute("data-pixel-outline");
			const fillRule = style.fillRule === "evenodd" ? "evenodd" : "nonzero";
			const happyEye = element.classList.contains("fl-bot-happy-eye");
			const winkEye =
				element.parentElement?.getAttribute("data-feature") === "wink-eye";
			const mouth =
				element.parentElement?.getAttribute("data-feature") === "mouth" &&
				element.parentElement.children.length === 1 &&
				style.fill === "none";
			if (happyEye || winkEye || mouth) {
				const bounds = element.getBBox();
				const midpoint = new DOMPoint(
					bounds.x + bounds.width / 2,
					mouth ? 0 : bounds.y + bounds.height,
				).matrixTransform(matrix);
				const cx = Math.floor(midpoint.x);
				const cy = Math.floor(midpoint.y);
				const blinking = element.closest('[data-blinking="true"]') !== null;
				const pixels =
					happyEye || winkEye
						? [
								[-1, 0],
								[0, blinking ? 0 : -1],
								[1, 0],
							]
						: expression === "focused" ||
								(!sleeping && /H/.test(element.getAttribute("d") ?? ""))
							? [
									[-1, 0],
									[0, 0],
									[1, 0],
								]
							: expression === "oops"
								? [
										[-1, 0],
										[0, 1],
										[1, 0],
										[2, 1],
									]
								: expression === "sad" ||
										expression === "angry" ||
										expression === "concerned"
									? [
											[-1, 1],
											[0, 0],
											[1, 1],
										]
									: [
											[-1, 0],
											[0, 1],
											[1, 0],
										];
				for (const [dx, dy] of pixels) {
					const x = cx + dx;
					const y = cy + dy;
					if (x < 0 || y < 0 || x >= BOT_PIXEL_SIZE || y >= BOT_PIXEL_SIZE)
						continue;
					const index = y * BOT_PIXEL_SIZE + x;
					if (!clip || clip[index])
						blend(index, stroke(x + 0.5, y + 0.5), strokeOpacity);
				}
				return;
			}
			const bounds = element.getBBox();
			const corners = [
				[bounds.x, bounds.y],
				[bounds.x + bounds.width, bounds.y],
				[bounds.x, bounds.y + bounds.height],
				[bounds.x + bounds.width, bounds.y + bounds.height],
			].map(([x, y]) => new DOMPoint(x, y).matrixTransform(matrix));
			const padding = Math.max(2, width);
			const minX = Math.max(
				0,
				Math.floor(Math.min(...corners.map((point) => point.x)) - padding),
			);
			const maxX = Math.min(
				BOT_PIXEL_SIZE - 1,
				Math.ceil(Math.max(...corners.map((point) => point.x)) + padding),
			);
			const minY = Math.max(
				0,
				Math.floor(Math.min(...corners.map((point) => point.y)) - padding),
			);
			const maxY = Math.min(
				BOT_PIXEL_SIZE - 1,
				Math.ceil(Math.max(...corners.map((point) => point.y)) + padding),
			);
			for (let row = minY; row <= maxY; row++)
				for (let column = minX; column <= maxX; column++) {
					const index = row * BOT_PIXEL_SIZE + column;
					if (clip && !clip[index]) continue;
					const x = (index % BOT_PIXEL_SIZE) + 0.5;
					const y = Math.floor(index / BOT_PIXEL_SIZE) + 0.5;
					const coverage = mask?.(x, y)[3] ?? 1;
					if (outline) {
						context.lineWidth = 2;
						if (context.isPointInStroke(path, x, y))
							blend(index, inkColor, opacity);
					}
					if (
						style.fill !== "none" &&
						context.isPointInPath(path, x, y, fillRule)
					)
						blend(index, fill(x, y), fillOpacity * coverage);
					context.lineWidth = width;
					if (style.stroke !== "none" && context.isPointInStroke(path, x, y))
						blend(index, stroke(x, y), strokeOpacity * coverage);
				}
		} else if (element.tagName.toLowerCase() === "text") {
			const glyph = source.ownerDocument.createElement("canvas");
			glyph.width = glyph.height = BOT_PIXEL_SIZE;
			const glyphContext = glyph.getContext("2d", { willReadFrequently: true });
			if (glyphContext) {
				const matrix = matrixOf(element);
				glyphContext.setTransform(matrix);
				glyphContext.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
				glyphContext.textAlign =
					style.textAnchor === "middle"
						? "center"
						: style.textAnchor === "end"
							? "right"
							: "left";
				glyphContext.textBaseline = ["central", "middle"].includes(
					style.dominantBaseline,
				)
					? "middle"
					: "alphabetic";
				glyphContext.fillStyle = "#fff";
				glyphContext.fillText(
					element.textContent ?? "",
					number(element, "x"),
					number(element, "y"),
				);
				const pixels = glyphContext.getImageData(
					0,
					0,
					BOT_PIXEL_SIZE,
					BOT_PIXEL_SIZE,
				).data;
				const fill = paint(style.fill, element);
				for (let index = 0; index < BOT_PIXEL_SIZE * BOT_PIXEL_SIZE; index++)
					if ((!clip || clip[index]) && pixels[index * 4 + 3] > 65)
						blend(
							index,
							fill(
								(index % BOT_PIXEL_SIZE) + 0.5,
								Math.floor(index / BOT_PIXEL_SIZE) + 0.5,
							),
							opacity,
						);
			}
		}
		for (const child of element.children)
			if (child instanceof SVGElement)
				walk(child as SVGGraphicsElement, opacity, clip, transformColors);
	};
	for (const child of source.children)
		if (child instanceof SVGElement) walk(child as SVGGraphicsElement, 1);
	context.putImageData(image, 0, 0);
	return true;
}

export function BotPixel({
	sourceRef,
	animated,
	frame,
	ink,
}: {
	sourceRef: RefObject<SVGSVGElement | null>;
	animated: boolean;
	frame: BotFrame;
	ink: string;
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	useEffect(() => {
		const source = sourceRef.current;
		const canvas = canvasRef.current;
		if (!source || !canvas) return;
		let frameId = 0;
		let lastFrame = Number.NEGATIVE_INFINITY;
		let disposed = false;
		const draw = () => {
			if (disposed) return;
			try {
				if (drawBotPixels(source, canvas, ink)) {
					source.dataset.pixelReady = "true";
					canvas.dataset.ready = "true";
				}
			} catch {
				delete source.dataset.pixelReady;
				delete canvas.dataset.ready;
			}
		};
		const tick = (time: number) => {
			if (time - lastFrame >= 1000 / 12) {
				draw();
				lastFrame = time;
			}
			if (animated && !disposed) frameId = requestAnimationFrame(tick);
		};
		const queue = () => {
			if (animated) return;
			cancelAnimationFrame(frameId);
			frameId = requestAnimationFrame(draw);
		};
		const observer = new MutationObserver((records) => {
			if (records.some((record) => record.attributeName !== "data-pixel-ready"))
				queue();
		});
		observer.observe(source, {
			subtree: true,
			childList: true,
			attributes: true,
			characterData: true,
		});
		if (source.parentElement)
			observer.observe(source.parentElement, { attributes: true });
		draw();
		if (animated) frameId = requestAnimationFrame(tick);
		return () => {
			disposed = true;
			observer.disconnect();
			cancelAnimationFrame(frameId);
			delete source.dataset.pixelReady;
		};
	}, [sourceRef, animated, ink]);
	return (
		<canvas
			ref={canvasRef}
			width={BOT_PIXEL_SIZE}
			height={BOT_PIXEL_SIZE}
			className="fl-bot-pixel"
			data-pixel-grid={BOT_PIXEL_SIZE}
			data-frame={frame}
			aria-hidden="true"
			tabIndex={-1}
		/>
	);
}
