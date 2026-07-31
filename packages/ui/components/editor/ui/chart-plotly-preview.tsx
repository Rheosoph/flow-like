"use client";

import { useTheme } from "next-themes";
import { useCallback, useEffect, useMemo, useRef } from "react";
import type { ChartInput } from "./chart-data-parser";
import { normalizePlotlyTitle, toPlotlyData } from "./chart-data-parser";

interface PlotlyModule {
	react: (
		root: HTMLElement,
		data: unknown[],
		layout?: Record<string, unknown>,
		config?: Record<string, unknown>,
	) => Promise<void>;
	Plots: { resize: (root: HTMLElement) => void };
	purge: (root: HTMLElement) => void;
}

interface PlotlyChartPreviewProps {
	input: ChartInput;
	height?: number;
}

function objectValue(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

function PlotlyChartPreview({ input, height = 350 }: PlotlyChartPreviewProps) {
	const containerRef = useRef<HTMLDivElement>(null);
	const plotlyRef = useRef<PlotlyModule | null>(null);
	const { resolvedTheme } = useTheme();
	const isDark = resolvedTheme === "dark";

	const { data, layout, config } = useMemo(() => {
		const result = toPlotlyData(input);
		const baseLayout = result.layout as Record<string, unknown>;
		const currentFont = objectValue(baseLayout.font);
		const currentTitle = normalizePlotlyTitle(baseLayout.title);
		const currentTitleFont = objectValue(currentTitle.font);
		const currentLegend = objectValue(baseLayout.legend);
		const currentLegendFont = objectValue(currentLegend.font);
		const currentXAxis = objectValue(baseLayout.xaxis);
		const currentXAxisTitle = normalizePlotlyTitle(currentXAxis.title);
		const currentXAxisTitleFont = objectValue(currentXAxisTitle.font);
		const currentYAxis = objectValue(baseLayout.yaxis);
		const currentYAxisTitle = normalizePlotlyTitle(currentYAxis.title);
		const currentYAxisTitleFont = objectValue(currentYAxisTitle.font);

		const legacyFontColor =
			typeof currentFont.color === "string" && currentFont.color === "#888";

		const themedFontColor = isDark ? "#f3f4f6" : "#111827";
		const themedMutedColor = isDark ? "#9ca3af" : "#6b7280";
		const themedBorderColor = isDark ? "#374151" : "#d1d5db";

		result.layout = {
			...baseLayout,
			height: input.config.height || height,
			paper_bgcolor: baseLayout.paper_bgcolor ?? "transparent",
			plot_bgcolor: baseLayout.plot_bgcolor ?? "transparent",
			font: {
				...currentFont,
				color: legacyFontColor
					? themedFontColor
					: (currentFont.color ?? themedFontColor),
			},
			title: {
				...currentTitle,
				font: {
					...currentTitleFont,
					color: currentTitleFont.color ?? themedFontColor,
				},
			},
			legend: {
				...currentLegend,
				font: {
					...currentLegendFont,
					color: currentLegendFont.color ?? themedMutedColor,
				},
			},
			xaxis: {
				...currentXAxis,
				linecolor: currentXAxis.linecolor ?? themedBorderColor,
				gridcolor: currentXAxis.gridcolor ?? themedBorderColor,
				zerolinecolor: currentXAxis.zerolinecolor ?? themedBorderColor,
				tickfont: {
					...objectValue(currentXAxis.tickfont),
					color: objectValue(currentXAxis.tickfont).color ?? themedMutedColor,
				},
				title: {
					...currentXAxisTitle,
					font: {
						...currentXAxisTitleFont,
						color: currentXAxisTitleFont.color ?? themedMutedColor,
					},
				},
			},
			yaxis: {
				...currentYAxis,
				linecolor: currentYAxis.linecolor ?? themedBorderColor,
				gridcolor: currentYAxis.gridcolor ?? themedBorderColor,
				zerolinecolor: currentYAxis.zerolinecolor ?? themedBorderColor,
				tickfont: {
					...objectValue(currentYAxis.tickfont),
					color: objectValue(currentYAxis.tickfont).color ?? themedMutedColor,
				},
				title: {
					...currentYAxisTitle,
					font: {
						...currentYAxisTitleFont,
						color: currentYAxisTitleFont.color ?? themedMutedColor,
					},
				},
			},
		};

		return result;
	}, [input, height, isDark]);

	const handleResize = useCallback(() => {
		if (!containerRef.current || !plotlyRef.current) return;
		try {
			plotlyRef.current.Plots.resize(containerRef.current);
		} catch {
			// Ignore resize errors
		}
	}, []);

	useEffect(() => {
		let mounted = true;

		const loadAndRender = async () => {
			if (!containerRef.current) return;

			try {
				const PlotlyModule = await import("plotly.js-dist-min");
				if (!mounted) return;

				// plotly.js-dist-min exports default as the Plotly object
				const Plotly = (PlotlyModule.default ||
					PlotlyModule) as unknown as PlotlyModule;
				plotlyRef.current = Plotly;
				await Plotly.react(containerRef.current, data, layout, config);
			} catch (err) {
				console.error("Failed to load/render Plotly chart:", err);
			}
		};

		loadAndRender();

		return () => {
			mounted = false;
			if (containerRef.current && plotlyRef.current) {
				plotlyRef.current.purge(containerRef.current);
			}
		};
	}, [data, layout, config]);

	// Handle resize
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

	return (
		<div
			ref={containerRef}
			className="w-full min-h-0 rounded-md overflow-hidden"
			style={{ height: input.config.height || height }}
		/>
	);
}

export default PlotlyChartPreview;
