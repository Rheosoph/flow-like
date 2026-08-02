"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getPlotlyChartLayout, useChartTokens } from "../../../lib/chart-theme";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type {
	BoundValue,
	ChartDataSource,
	ChartSeries,
	PlotlyChartComponent,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const CHART_TYPE_MAP: Record<string, string> = {
	line: "scatter",
	scatter: "scatter",
	bar: "bar",
	pie: "pie",
	area: "scatter",
	histogram: "histogram",
};

const DEFAULT_SAMPLE_DATA = {
	x: ["Jan", "Feb", "Mar", "Apr", "May", "Jun"],
	y: [20, 14, 25, 16, 18, 22],
};

function parseCSV(csv: string): { x: (string | number)[]; y: number[] } {
	const lines = csv.trim().split("\n");
	const x: (string | number)[] = [];
	const y: number[] = [];

	for (const line of lines) {
		const parts = line.split(",").map((s) => s.trim());
		if (parts.length >= 2) {
			x.push(parts[0]);
			const val = Number.parseFloat(parts[1]);
			y.push(isNaN(val) ? 0 : val);
		}
	}

	return { x, y };
}

function resolveDataSource(
	dataSource: ChartDataSource | undefined,
	resolve: (bv: BoundValue) => unknown,
): { x: unknown[]; y: unknown[] } {
	if (!dataSource) return DEFAULT_SAMPLE_DATA;

	if ("csv" in dataSource && dataSource.csv) {
		return parseCSV(dataSource.csv);
	}

	if ("xPath" in dataSource && "yPath" in dataSource) {
		const xData = resolve({ path: dataSource.xPath });
		const yData = resolve({ path: dataSource.yPath });
		return {
			x: Array.isArray(xData) ? xData : xData ? [xData] : DEFAULT_SAMPLE_DATA.x,
			y: Array.isArray(yData) ? yData : yData ? [yData] : DEFAULT_SAMPLE_DATA.y,
		};
	}

	return DEFAULT_SAMPLE_DATA;
}

const DEFAULT_CONFIG = {
	responsive: true,
	displayModeBar: true,
	displaylogo: false,
};

interface PlotlyModule {
	react: (
		root: HTMLElement,
		data: unknown[],
		layout?: Record<string, unknown>,
		config?: Record<string, unknown>,
	) => Promise<void>;
	relayout: (
		root: HTMLElement,
		update: Record<string, unknown>,
	) => Promise<void>;
	Plots: {
		resize: (root: HTMLElement) => void;
	};
	purge: (root: HTMLElement) => void;
}

export function A2UIPlotlyChart({
	component,
	style,
}: ComponentProps<PlotlyChartComponent>) {
	const containerRef = useRef<HTMLDivElement>(null);
	const plotlyRef = useRef<PlotlyModule | null>(null);
	const { resolve } = useData();

	// Plotly paints to canvas and cannot read CSS variables, so the chat tokens
	// are resolved against the chart's own node and re-resolved on theme change.
	const [themeNode, setThemeNode] = useState<HTMLDivElement | null>(null);
	const tokens = useChartTokens(themeNode);

	// Resolve simple props
	const chartTitle = useResolved<string>(component.title);
	const width = useResolved<string>(component.width);
	const height = useResolved<string>(component.height) ?? "400px";
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const showLegend = useResolved<boolean>(component.showLegend) ?? true;
	const legendPosition =
		useResolved<string>(component.legendPosition) ?? "bottom";

	// Raw data for legacy/advanced mode
	const rawData = useResolved<unknown>(component.data);
	const rawLayout = useResolved<unknown>(component.layout);
	const rawConfig = useResolved<unknown>(component.config);

	// Build Plotly data from structured series
	const data = useMemo(() => {
		// If raw data is provided, use it (legacy/advanced mode)
		if (rawData) {
			if (typeof rawData === "string") {
				try {
					return JSON.parse(rawData);
				} catch {
					return [];
				}
			}
			return Array.isArray(rawData) ? rawData : [rawData];
		}

		// Build from structured series
		if (!component.series || component.series.length === 0) {
			// Return sample data for preview
			return [
				{
					x: DEFAULT_SAMPLE_DATA.x,
					y: DEFAULT_SAMPLE_DATA.y,
					type: "scatter",
					mode: "lines+markers",
					name: "Sample Data",
					marker: { color: tokens.palette[0] },
				},
			];
		}

		return component.series.map((series: ChartSeries, index: number) => {
			const { x: xData, y: yData } = resolveDataSource(
				series.dataSource,
				resolve,
			);
			const plotlyType = CHART_TYPE_MAP[series.type] || "scatter";

			const trace: Record<string, unknown> = {
				x: xData,
				y: yData,
				type: plotlyType,
				name: series.name || "Series",
				marker: {
					color: series.color ?? tokens.palette[index % tokens.palette.length],
				},
			};

			// Add mode for line/scatter types
			if (series.type === "line" || series.type === "scatter") {
				trace.mode = series.mode || "lines+markers";
			}

			// Area charts are scatter with fill
			if (series.type === "area") {
				trace.fill = "tozeroy";
				trace.mode = series.mode || "lines";
			}

			return trace;
		});
	}, [rawData, component.series, resolve, tokens]);

	// Build layout from structured axis config or raw layout
	const layout = useMemo(() => {
		const legendOrientationMap: Record<
			string,
			{
				orientation: "h" | "v";
				x?: number;
				y?: number;
				xanchor?: string;
				yanchor?: string;
			}
		> = {
			bottom: { orientation: "h", y: -0.15, x: 0.5, xanchor: "center" },
			top: { orientation: "h", y: 1.1, x: 0.5, xanchor: "center" },
			left: { orientation: "v", x: -0.15, y: 0.5, yanchor: "middle" },
			right: { orientation: "v", x: 1.05, y: 0.5, yanchor: "middle" },
		};

		const themed = getPlotlyChartLayout(tokens);

		const base: Record<string, unknown> = {
			...themed,
			title: chartTitle || undefined,
			margin: { t: chartTitle ? 50 : 20, r: 20, b: 50, l: 50 },
			autosize: true,
			showlegend: showLegend,
			legend: {
				...themed.legend,
				...(legendOrientationMap[legendPosition] ||
					legendOrientationMap.bottom),
			},
		};

		// Build xaxis config
		const xAxis = component.xAxis;
		base.xaxis = {
			...themed.xaxis,
			title: xAxis?.title || undefined,
			type: xAxis?.type || undefined,
			range:
				xAxis?.min !== undefined && xAxis?.max !== undefined
					? [xAxis.min, xAxis.max]
					: undefined,
			showgrid: xAxis?.showGrid ?? true,
			automargin: true,
			tickformat: xAxis?.tickFormat || undefined,
		};

		// Build yaxis config
		const yAxis = component.yAxis;
		base.yaxis = {
			...themed.yaxis,
			title: yAxis?.title || undefined,
			type: yAxis?.type || undefined,
			range:
				yAxis?.min !== undefined && yAxis?.max !== undefined
					? [yAxis.min, yAxis.max]
					: undefined,
			showgrid: yAxis?.showGrid ?? true,
			automargin: true,
			tickformat: yAxis?.tickFormat || undefined,
		};

		// Merge with raw layout if provided (for advanced customization)
		if (rawLayout) {
			if (typeof rawLayout === "string") {
				try {
					return { ...base, ...JSON.parse(rawLayout) };
				} catch {
					return base;
				}
			}
			return { ...base, ...(rawLayout as object) };
		}

		return base;
	}, [
		chartTitle,
		showLegend,
		legendPosition,
		component.xAxis,
		component.yAxis,
		rawLayout,
		tokens,
	]);

	// Build config
	const config = useMemo(() => {
		const base = { ...DEFAULT_CONFIG, responsive };
		if (rawConfig) {
			if (typeof rawConfig === "string") {
				try {
					return { ...base, ...JSON.parse(rawConfig) };
				} catch {
					return base;
				}
			}
			return { ...base, ...(rawConfig as object) };
		}
		return base;
	}, [rawConfig, responsive]);

	// Debounced resize handler
	const resizeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	const handleResize = useCallback(() => {
		if (!containerRef.current || !plotlyRef.current) return;

		if (resizeTimeoutRef.current) {
			clearTimeout(resizeTimeoutRef.current);
		}

		resizeTimeoutRef.current = setTimeout(() => {
			if (containerRef.current && plotlyRef.current) {
				plotlyRef.current.Plots.resize(containerRef.current);
			}
		}, 100);
	}, []);

	useEffect(() => {
		let mounted = true;

		const loadAndRender = async () => {
			if (!containerRef.current) return;

			if (!plotlyRef.current) {
				const Plotly = await import("plotly.js-dist-min");
				plotlyRef.current = Plotly.default as unknown as PlotlyModule;
			}

			if (!mounted || !containerRef.current) return;

			await plotlyRef.current.react(
				containerRef.current,
				data as unknown[],
				{ ...layout, autosize: true } as Record<string, unknown>,
				config as Record<string, unknown>,
			);
		};

		loadAndRender();

		return () => {
			mounted = false;
			if (resizeTimeoutRef.current) {
				clearTimeout(resizeTimeoutRef.current);
			}
		};
	}, [data, layout, config]);

	// Purge on unmount only — tearing the plot down between renders would also
	// discard the pan and zoom the user has applied.
	useEffect(() => {
		const container = containerRef.current;
		return () => {
			if (container && plotlyRef.current) {
				plotlyRef.current.purge(container);
			}
		};
	}, []);

	// ResizeObserver for responsive behavior
	useEffect(() => {
		if (!containerRef.current) return;

		const resizeObserver = new ResizeObserver(handleResize);
		resizeObserver.observe(containerRef.current);

		window.addEventListener("resize", handleResize);

		return () => {
			resizeObserver.disconnect();
			window.removeEventListener("resize", handleResize);
		};
	}, [handleResize]);

	// The theme node stays outside the graph div: Plotly writes its own classes
	// and inline styles onto the node it renders into, which the token observer
	// would otherwise read back as a theme change.
	return (
		<div
			ref={setThemeNode}
			className={cn("w-full min-h-0", resolveStyle(style))}
			style={{
				...resolveInlineStyle(style),
				width: width ?? "100%",
				height,
				minWidth: 0,
			}}
		>
			<div ref={containerRef} className="h-full w-full min-w-0" />
		</div>
	);
}
