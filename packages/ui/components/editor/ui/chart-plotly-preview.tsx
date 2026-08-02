"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getPlotlyChartLayout, useChartTokens } from "../../../lib/chart-theme";
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
	const [themeNode, setThemeNode] = useState<HTMLDivElement | null>(null);
	const tokens = useChartTokens(themeNode);

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

		const themed = getPlotlyChartLayout(tokens);
		const themedFontColor = tokens.text;
		const themedMutedColor = tokens.textMuted;
		const themedBorderColor = tokens.grid;

		result.layout = {
			...baseLayout,
			height: input.config.height || height,
			paper_bgcolor: baseLayout.paper_bgcolor ?? "transparent",
			plot_bgcolor: baseLayout.plot_bgcolor ?? "transparent",
			colorway: baseLayout.colorway ?? themed.colorway,
			hoverlabel: baseLayout.hoverlabel ?? themed.hoverlabel,
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
	}, [input, height, tokens]);

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

	// The theme node stays outside the graph div: Plotly writes its own classes
	// and inline styles onto the node it renders into, which the token observer
	// would otherwise read back as a theme change.
	return (
		<div
			ref={setThemeNode}
			className="w-full min-h-0"
			style={{ height: input.config.height || height }}
		>
			<div
				ref={containerRef}
				className="h-full w-full rounded-md overflow-hidden"
			/>
		</div>
	);
}

export default PlotlyChartPreview;
