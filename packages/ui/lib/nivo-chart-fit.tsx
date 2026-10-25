"use client";

import { useEffect, useLayoutEffect, useRef } from "react";

const SVG_NS = "http://www.w3.org/2000/svg";
const ELLIPSIS = "…";

export const NIVO_LEGEND_SYMBOL_SIZE = 12;
/** nivo's `legendDefaults.symbolSpacing`; the label starts after symbol + spacing. */
const LEGEND_LABEL_OFFSET = NIVO_LEGEND_SYMBOL_SIZE + 8;
const LEGEND_ITEM_GAP = 16;
const LEGEND_ITEM_MIN_WIDTH = 48;
const LEGEND_ITEM_MAX_WIDTH = 180;
const LEGEND_FIT_TOLERANCE = 1;
const DEFAULT_FONT_FAMILY = "ui-sans-serif, system-ui, sans-serif";

const TOOLTIP_SELECTOR =
	'div[style*="pointer-events: none"][style*="z-index: 10"]';
const TOOLTIP_INSET = 4;

let measureContext: CanvasRenderingContext2D | null | undefined;

function getMeasureContext(): CanvasRenderingContext2D | null {
	if (measureContext === undefined) {
		measureContext =
			typeof document === "undefined"
				? null
				: document.createElement("canvas").getContext("2d");
	}
	return measureContext;
}

/** Measures labels in the legend font the nivo chart theme resolves to. */
function createLegendLabelMeasure(fontSize = 11): (label: string) => number {
	const context = getMeasureContext();
	if (!context) return (label) => label.length * fontSize * 0.6;
	const family =
		getComputedStyle(document.documentElement)
			.getPropertyValue("--font-sans")
			.trim() || DEFAULT_FONT_FAMILY;
	const font = `${fontSize}px ${family}`;
	return (label) => {
		context.font = font;
		return context.measureText(label).width;
	};
}

/**
 * Width for every item of a nivo box legend: fits the longest label, capped so
 * one long series name cannot push the others out, and shrunk to an equal
 * share of `available` (pass 0 when the legend is not laid out in a row).
 */
export function fitNivoLegendItemWidth(
	labels: readonly string[],
	available: number,
	measure: (label: string) => number = createLegendLabelMeasure(),
): number {
	const longest = labels.reduce(
		(widest, label) => Math.max(widest, measure(label)),
		0,
	);
	const natural =
		Math.ceil(longest) +
		LEGEND_FIT_TOLERANCE +
		LEGEND_LABEL_OFFSET +
		LEGEND_ITEM_GAP;
	const share =
		available > 0 && labels.length > 0
			? Math.floor(available / labels.length)
			: Number.POSITIVE_INFINITY;
	return Math.max(
		LEGEND_ITEM_MIN_WIDTH,
		Math.min(natural, LEGEND_ITEM_MAX_WIDTH, share),
	);
}

/** Longest prefix of `full` that fits `budget` once an ellipsis is appended. */
export function ellipsizeLabel(
	full: string,
	budget: number,
	measure: (text: string) => number,
): string {
	if (measure(full) <= budget + LEGEND_FIT_TOLERANCE) return full;
	const chars = Array.from(full);
	let fitted = ELLIPSIS;
	let low = 1;
	let high = chars.length - 1;
	while (low <= high) {
		const middle = Math.floor((low + high) / 2);
		const candidate = `${chars.slice(0, middle).join("").trimEnd()}${ELLIPSIS}`;
		if (measure(candidate) <= budget) {
			fitted = candidate;
			low = middle + 1;
		} else {
			high = middle - 1;
		}
	}
	return fitted;
}

function syncTitle(target: Element, text: string | null) {
	let title = target.querySelector(":scope > title");
	if (text === null) {
		title?.remove();
		return;
	}
	if (!title) {
		title = document.createElementNS(SVG_NS, "title");
		target.appendChild(title);
	}
	title.textContent = text;
}

/** nivo legend items render `<rect>` (hit area), the symbol, then `<text>`. */
function legendItemParts(symbol: SVGElement) {
	const item = symbol.parentNode;
	if (!(item instanceof SVGGElement)) return null;
	const hitArea = item.querySelector(":scope > rect");
	const labels = item.querySelectorAll<SVGTextElement>(":scope > text");
	const label = labels[0];
	if (!hitArea || !label) return null;
	const budget =
		Number(hitArea.getAttribute("width")) -
		Number(label.getAttribute("x")) -
		LEGEND_ITEM_GAP;
	return { hitArea, labels, label, budget };
}

/**
 * The untruncated label, or null when it is already fitted to `budget`. React
 * only rewrites the text when the label changes, so the full text is kept in
 * a data attribute to re-fit after a resize.
 */
function pendingLegendLabel(label: SVGTextElement, budget: number) {
	const current = label.textContent ?? "";
	if (current !== label.dataset.flFitted) return current;
	if (label.dataset.flBudget === String(budget)) return null;
	return label.dataset.flFull ?? current;
}

function fitLegendItemLabel(symbol: SVGElement) {
	const parts = legendItemParts(symbol);
	if (!parts) return;
	const { hitArea, labels, label, budget } = parts;
	const full = pendingLegendLabel(label, budget);
	if (full === null) return;

	const fitted = ellipsizeLabel(full, budget, (text) => {
		label.textContent = text;
		return label.getComputedTextLength();
	});
	for (const node of labels) node.textContent = fitted;
	Object.assign(label.dataset, {
		flFull: full,
		flFitted: fitted,
		flBudget: String(budget),
	});
	syncTitle(hitArea, fitted === full ? null : full);
}

interface LegendSymbolProps {
	x: number;
	y: number;
	size: number;
	fill: string;
	opacity?: number;
	borderWidth?: number;
	borderColor?: string;
}

/**
 * Circle legend symbol that also ellipsizes its item's label. nivo has no
 * label formatter shared by every chart's legend, but all of them accept a
 * custom `symbolShape`.
 */
export function FittedLegendSymbol({
	x,
	y,
	size,
	fill,
	opacity = 1,
	borderWidth = 0,
	borderColor = "transparent",
}: LegendSymbolProps) {
	const ref = useRef<SVGCircleElement>(null);

	useLayoutEffect(() => {
		if (ref.current) fitLegendItemLabel(ref.current);
	});

	return (
		<circle
			ref={ref}
			r={size / 2}
			cx={x + size / 2}
			cy={y + size / 2}
			fill={fill}
			opacity={opacity}
			strokeWidth={borderWidth}
			stroke={borderColor}
			style={{ pointerEvents: "none" }}
		/>
	);
}

/** Offset that moves `[start, end]` inside `[min, max]`, start-aligned if it cannot fit. */
export function shiftIntoRange(
	start: number,
	end: number,
	min: number,
	max: number,
) {
	if (end - start >= max - min || start < min) return min - start;
	if (end > max) return max - end;
	return 0;
}

type Offset = readonly [x: number, y: number];

const NO_OFFSET: Offset = [0, 0];

/** CSS-px offset that brings `tooltip` inside `boundary`, given the one applied. */
function containingOffset(
	tooltip: HTMLElement,
	boundary: HTMLElement,
	[previousX, previousY]: Offset,
) {
	const bounds = boundary.getBoundingClientRect();
	const scale =
		boundary.offsetWidth > 0 ? bounds.width / boundary.offsetWidth : 1;
	const rect = tooltip.getBoundingClientRect();
	const left = rect.left - previousX * scale;
	const top = rect.top - previousY * scale;
	const inset = TOOLTIP_INSET * scale;
	return [
		Math.round(
			shiftIntoRange(
				left,
				left + rect.width,
				bounds.left + inset,
				bounds.right - inset,
			) / scale,
		),
		Math.round(
			shiftIntoRange(
				top,
				top + rect.height,
				bounds.top + inset,
				bounds.bottom - inset,
			) / scale,
		),
	] as const;
}

/**
 * Keeps nivo's tooltip inside `boundary`, which clips it. nivo animates the
 * tooltip's `transform`, so the correction goes on the separate `translate`
 * property that composes with it.
 */
export function useNivoTooltipBounds(boundary: HTMLElement | null) {
	useEffect(() => {
		if (!boundary) return;
		const offsets = new WeakMap<HTMLElement, Offset>();

		const contain = () => {
			const tooltip = boundary.querySelector<HTMLElement>(TOOLTIP_SELECTOR);
			if (!tooltip) return;
			const previous = offsets.get(tooltip) ?? NO_OFFSET;
			const [x, y] = containingOffset(tooltip, boundary, previous);
			if (x === previous[0] && y === previous[1]) return;
			offsets.set(tooltip, [x, y]);
			tooltip.style.translate = x || y ? `${x}px ${y}px` : "";
		};

		const observer = new MutationObserver(contain);
		observer.observe(boundary, {
			subtree: true,
			childList: true,
			attributes: true,
			attributeFilter: ["style"],
		});
		return () => observer.disconnect();
	}, [boundary]);
}
