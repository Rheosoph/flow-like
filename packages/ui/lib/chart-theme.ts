"use client";

import { useEffect, useState } from "react";

/**
 * One chart theme for every surface that renders a chart inside chat: the a2ui
 * nivoChart/plotlyChart components and the markdown ```nivo / ```plotly fences.
 *
 * Nivo inlines theme values straight into SVG attributes, so it can consume
 * `var(--token)` directly. Plotly paints to canvas/WebGL and cannot, so its
 * tokens are resolved to concrete colour strings and re-resolved on theme change.
 */

const PALETTE_TOKENS = [
	"--fl-chat-chart-1",
	"--fl-chat-chart-2",
	"--fl-chat-chart-3",
	"--fl-chat-chart-4",
	"--fl-chat-chart-5",
	"--fl-chat-chart-6",
	"--fl-chat-chart-7",
	"--fl-chat-chart-8",
] as const;

const SURFACE_TOKENS = {
	text: "--fl-chat-chart-text",
	textMuted: "--fl-chat-chart-text-muted",
	grid: "--fl-chat-chart-grid",
	axis: "--fl-chat-chart-axis",
	empty: "--fl-chat-chart-empty",
	tooltipBackground: "--fl-chat-chart-tooltip-background",
	tooltipForeground: "--fl-chat-chart-tooltip-foreground",
	tooltipBorder: "--fl-chat-chart-tooltip-border",
} as const;

/** Fallbacks so a chart rendered outside `.fl-chat-root` still looks deliberate. */
const FALLBACK = {
	palette: [
		"#fb562d",
		"#8b5cf6",
		"#2fb8c6",
		"#f0a93c",
		"#4f7fe0",
		"#3fb27f",
		"#e0559b",
		"#7c8593",
	],
	text: "#e8ebf0",
	textMuted: "#99a3b1",
	grid: "#2b313a",
	axis: "#39404a",
	empty: "#242a33",
	tooltipBackground: "#14171c",
	tooltipForeground: "#e8ebf0",
	tooltipBorder: "#242a33",
} as const;

export interface IChartTokens {
	readonly palette: readonly string[];
	readonly text: string;
	readonly textMuted: string;
	readonly grid: string;
	readonly axis: string;
	readonly empty: string;
	readonly tooltipBackground: string;
	readonly tooltipForeground: string;
	readonly tooltipBorder: string;
}

/** Palette entries as `var()` references — for nivo, which resolves them in SVG. */
export const CHAT_CHART_PALETTE_VARS: readonly string[] = PALETTE_TOKENS.map(
	(token) => `var(${token})`,
);

let canvasProbe: CanvasRenderingContext2D | null | undefined;

function getProbe(): CanvasRenderingContext2D | null {
	if (canvasProbe === undefined) {
		if (typeof document === "undefined") {
			canvasProbe = null;
		} else {
			const canvas = document.createElement("canvas");
			canvas.width = 1;
			canvas.height = 1;
			canvasProbe = canvas.getContext("2d", { willReadFrequently: true });
		}
	}
	return canvasProbe;
}

/**
 * Paints a CSS colour to a 1x1 canvas and reads the pixel back.
 *
 * Neither `ctx.fillStyle` round-tripping nor `getComputedStyle().color` converts
 * `oklch()` or `color-mix()` to rgb in current browsers — they echo the modern
 * syntax straight back. Rasterising is the one path that always yields channels,
 * which is what plotly's canvas renderer and our own canvas work both need.
 */
function paintToRgba(value: string): [number, number, number, number] | null {
	const trimmed = value.trim();
	if (!trimmed) return null;

	const ctx = getProbe();
	if (!ctx) return null;

	// An unparseable value leaves fillStyle untouched, so a distinctive sentinel
	// tells us the assignment was rejected.
	ctx.fillStyle = "#010203";
	ctx.fillStyle = trimmed;
	if (ctx.fillStyle === "#010203" && trimmed.toLowerCase() !== "#010203") {
		return null;
	}

	ctx.clearRect(0, 0, 1, 1);
	ctx.fillRect(0, 0, 1, 1);
	try {
		const { data } = ctx.getImageData(0, 0, 1, 1);
		return [data[0], data[1], data[2], data[3] / 255];
	} catch {
		return null;
	}
}

/** Normalises any CSS colour to an `rgb()`/`rgba()` string. */
function toPaintableColor(value: string, fallback: string): string {
	const rgba = paintToRgba(value);
	if (!rgba) return fallback;
	const [r, g, b, a] = rgba;
	return a >= 0.999
		? `rgb(${r}, ${g}, ${b})`
		: `rgba(${r}, ${g}, ${b}, ${Number(a.toFixed(3))})`;
}

function readToken(styles: CSSStyleDeclaration, token: string): string {
	return styles.getPropertyValue(token).trim();
}

/**
 * Resolves any CSS colour (`oklch()`, `color-mix()`, `var()` already resolved) to
 * an `[r, g, b]` triple, for canvas work that needs to interpolate channels.
 */
export function resolveColorToRgb(
	value: string,
	fallback: readonly [number, number, number],
): [number, number, number] {
	const rgba = paintToRgba(value);
	if (!rgba) return [...fallback];
	return [rgba[0], rgba[1], rgba[2]];
}

/**
 * Resolves the chart tokens as they apply at `element` (pass the chart's own
 * node so chat-scoped and preset overrides are honoured).
 */
export function resolveChartTokens(element?: Element | null): IChartTokens {
	if (typeof window === "undefined") {
		return { ...FALLBACK, palette: [...FALLBACK.palette] };
	}

	const target = element ?? document.documentElement;
	const styles = window.getComputedStyle(target);

	const palette = PALETTE_TOKENS.map((token, index) =>
		toPaintableColor(
			readToken(styles, token),
			FALLBACK.palette[index] ?? FALLBACK.palette[0],
		),
	);

	const surface = {} as Record<keyof typeof SURFACE_TOKENS, string>;
	for (const [key, token] of Object.entries(SURFACE_TOKENS) as [
		keyof typeof SURFACE_TOKENS,
		string,
	][]) {
		surface[key] = toPaintableColor(readToken(styles, token), FALLBACK[key]);
	}

	return { palette, ...surface };
}

function sameTokens(a: IChartTokens, b: IChartTokens): boolean {
	if (
		a.text !== b.text ||
		a.textMuted !== b.textMuted ||
		a.grid !== b.grid ||
		a.axis !== b.axis ||
		a.empty !== b.empty ||
		a.tooltipBackground !== b.tooltipBackground ||
		a.tooltipForeground !== b.tooltipForeground ||
		a.tooltipBorder !== b.tooltipBorder ||
		a.palette.length !== b.palette.length
	) {
		return false;
	}
	return a.palette.every((color, index) => color === b.palette[index]);
}

/**
 * Chart tokens that re-resolve whenever the theme, the chat colour scheme or the
 * injected dynamic-theme stylesheet changes.
 *
 * The identity is held stable while the resolved colours are unchanged: chart
 * libraries write classes and inline styles onto the nodes they own, and a fresh
 * object per mutation would feed straight back into the observer as a re-render
 * loop that tears down the chart — and its pan/zoom state — on every frame.
 */
export function useChartTokens(element?: Element | null): IChartTokens {
	const [tokens, setTokens] = useState<IChartTokens>(() =>
		resolveChartTokens(element),
	);

	useEffect(() => {
		if (typeof window === "undefined") return;

		let frame = 0;
		const sync = () => {
			cancelAnimationFrame(frame);
			frame = requestAnimationFrame(() => {
				const next = resolveChartTokens(element);
				setTokens((previous) => (sameTokens(previous, next) ? previous : next));
			});
		};

		sync();

		const observer = new MutationObserver(sync);
		observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "data-theme", "style"],
		});
		if (element) {
			observer.observe(element, {
				attributes: true,
				attributeFilter: ["class", "data-fl-chat-color-scheme", "style"],
			});
		}

		const media = window.matchMedia("(prefers-color-scheme: dark)");
		media.addEventListener("change", sync);

		return () => {
			cancelAnimationFrame(frame);
			observer.disconnect();
			media.removeEventListener("change", sync);
		};
	}, [element]);

	return tokens;
}

/**
 * Nivo theme built from `var()` references. Safe to build once — the browser
 * re-resolves the variables when the theme changes, so it needs no React state.
 */
export function getNivoChartTheme(fontSize = 11) {
	const text = "var(--fl-chat-chart-text-muted, #99a3b1)";
	const axis = "var(--fl-chat-chart-axis, #39404a)";
	const grid = "var(--fl-chat-chart-grid, #2b313a)";

	return {
		background: "transparent",
		text: {
			fill: text,
			fontSize,
			fontFamily: "var(--font-sans, ui-sans-serif, system-ui, sans-serif)",
		},
		axis: {
			domain: { line: { stroke: axis, strokeWidth: 1 } },
			ticks: {
				line: { stroke: axis, strokeWidth: 1 },
				text: { fill: text, fontSize: fontSize - 1 },
			},
			legend: {
				text: {
					fill: "var(--fl-chat-chart-text, #e8ebf0)",
					fontSize,
					fontWeight: 500,
				},
			},
		},
		grid: { line: { stroke: grid, strokeWidth: 1 } },
		legends: { text: { fill: text, fontSize } },
		labels: { text: { fill: "var(--fl-chat-chart-text, #e8ebf0)", fontSize } },
		annotations: {
			text: { fill: "var(--fl-chat-chart-text, #e8ebf0)" },
			link: { stroke: axis },
			outline: { stroke: axis },
		},
		tooltip: {
			container: {
				background: "var(--fl-chat-chart-tooltip-background, #14171c)",
				color: "var(--fl-chat-chart-tooltip-foreground, #e8ebf0)",
				border: "1px solid var(--fl-chat-chart-tooltip-border, #242a33)",
				fontSize: 12,
				borderRadius: "0.5rem",
				padding: "0.375rem 0.5rem",
				boxShadow: "0 0.75rem 2rem -1rem oklch(0 0 0 / 45%)",
			},
		},
		crosshair: { line: { stroke: "var(--primary)", strokeOpacity: 0.5 } },
	};
}

/** Plotly layout fragment. Needs resolved colours — canvas cannot read `var()`. */
export function getPlotlyChartLayout(tokens: IChartTokens) {
	const axis = {
		gridcolor: tokens.grid,
		zerolinecolor: tokens.axis,
		linecolor: tokens.axis,
		tickfont: { color: tokens.textMuted, size: 11 },
		titlefont: { color: tokens.text, size: 12 },
	} as const;

	return {
		paper_bgcolor: "transparent",
		plot_bgcolor: "transparent",
		colorway: [...tokens.palette],
		font: {
			color: tokens.textMuted,
			size: 12,
			family:
				'-apple-system, "SF Pro Text", "Segoe UI", system-ui, Inter, sans-serif',
		},
		xaxis: { ...axis },
		yaxis: { ...axis },
		legend: { font: { color: tokens.textMuted, size: 11 } },
		// Vertical, so the toolbar runs down the right edge instead of sitting on
		// top of a centred chart title.
		modebar: {
			orientation: "v",
			bgcolor: "transparent",
			color: tokens.textMuted,
			activecolor: tokens.text,
		},
		hoverlabel: {
			bgcolor: tokens.tooltipBackground,
			bordercolor: tokens.tooltipBorder,
			font: { color: tokens.tooltipForeground, size: 12 },
		},
	};
}
