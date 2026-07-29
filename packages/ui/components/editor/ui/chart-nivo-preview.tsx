"use client";

import { useTheme } from "next-themes";
import {
	type ComponentType,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { ChartInput } from "./chart-data-parser";
import { toNivoData } from "./chart-data-parser";

type ChartModule = ComponentType<Record<string, unknown>>;

interface ChartInfo {
	pkg: string;
	component: string;
	loader: () => Promise<ChartModule | null>;
}

export function isRenderableChartExport(value: unknown): boolean {
	if (typeof value === "function") return true;
	return Boolean(
		value &&
			typeof value === "object" &&
			"$$typeof" in value &&
			("render" in value || "type" in value),
	);
}

const loadChart = async (
	component: string,
	importFn: () => Promise<unknown>,
): Promise<ChartModule | null> => {
	try {
		const mod = await importFn();
		if (!mod || typeof mod !== "object") return null;
		const chart = (mod as Record<string, unknown>)[component];
		return isRenderableChartExport(chart) ? (chart as ChartModule) : null;
	} catch {
		return null;
	}
};

const CHART_PACKAGES: Record<string, ChartInfo> = {
	bar: {
		pkg: "@nivo/bar",
		component: "ResponsiveBar",
		loader: () => loadChart("ResponsiveBar", () => import("@nivo/bar")),
	},
	line: {
		pkg: "@nivo/line",
		component: "ResponsiveLine",
		loader: () => loadChart("ResponsiveLine", () => import("@nivo/line")),
	},
	pie: {
		pkg: "@nivo/pie",
		component: "ResponsivePie",
		loader: () => loadChart("ResponsivePie", () => import("@nivo/pie")),
	},
	radar: {
		pkg: "@nivo/radar",
		component: "ResponsiveRadar",
		loader: () => loadChart("ResponsiveRadar", () => import("@nivo/radar")),
	},
	heatmap: {
		pkg: "@nivo/heatmap",
		component: "ResponsiveHeatMap",
		loader: () => loadChart("ResponsiveHeatMap", () => import("@nivo/heatmap")),
	},
	scatter: {
		pkg: "@nivo/scatterplot",
		component: "ResponsiveScatterPlot",
		loader: () =>
			loadChart("ResponsiveScatterPlot", () => import("@nivo/scatterplot")),
	},
	funnel: {
		pkg: "@nivo/funnel",
		component: "ResponsiveFunnel",
		loader: () => loadChart("ResponsiveFunnel", () => import("@nivo/funnel")),
	},
	treemap: {
		pkg: "@nivo/treemap",
		component: "ResponsiveTreeMap",
		loader: () => loadChart("ResponsiveTreeMap", () => import("@nivo/treemap")),
	},
	sunburst: {
		pkg: "@nivo/sunburst",
		component: "ResponsiveSunburst",
		loader: () =>
			loadChart("ResponsiveSunburst", () => import("@nivo/sunburst")),
	},
	calendar: {
		pkg: "@nivo/calendar",
		component: "ResponsiveCalendar",
		loader: () =>
			loadChart("ResponsiveCalendar", () => import("@nivo/calendar")),
	},
	bump: {
		pkg: "@nivo/bump",
		component: "ResponsiveBump",
		loader: () => loadChart("ResponsiveBump", () => import("@nivo/bump")),
	},
	areaBump: {
		pkg: "@nivo/bump",
		component: "ResponsiveAreaBump",
		loader: () => loadChart("ResponsiveAreaBump", () => import("@nivo/bump")),
	},
	sankey: {
		pkg: "@nivo/sankey",
		component: "ResponsiveSankey",
		loader: () => loadChart("ResponsiveSankey", () => import("@nivo/sankey")),
	},
	stream: {
		pkg: "@nivo/stream",
		component: "ResponsiveStream",
		loader: () => loadChart("ResponsiveStream", () => import("@nivo/stream")),
	},
	waffle: {
		pkg: "@nivo/waffle",
		component: "ResponsiveWaffle",
		loader: () => loadChart("ResponsiveWaffle", () => import("@nivo/waffle")),
	},
	radialBar: {
		pkg: "@nivo/radial-bar",
		component: "ResponsiveRadialBar",
		loader: () =>
			loadChart("ResponsiveRadialBar", () => import("@nivo/radial-bar")),
	},
	chord: {
		pkg: "@nivo/chord",
		component: "ResponsiveChord",
		loader: () => loadChart("ResponsiveChord", () => import("@nivo/chord")),
	},
};

const DEFAULT_MARGIN = { top: 30, right: 30, bottom: 50, left: 60 };

interface NivoChartPreviewProps {
	input: ChartInput;
	height?: number;
}

function NivoChartPreview({ input, height = 350 }: NivoChartPreviewProps) {
	const containerRef = useRef<HTMLDivElement>(null);
	const [chartModule, setChartModule] = useState<ChartModule | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const { resolvedTheme } = useTheme();

	const { data, chartType, props } = useMemo(() => toNivoData(input), [input]);
	const isDark = resolvedTheme === "dark";
	const defaultTheme = useMemo(
		() => ({
			background: "transparent",
			text: { fill: isDark ? "#9ca3af" : "#6b7280" },
			fontSize: 11,
			axis: {
				domain: { line: { stroke: isDark ? "#374151" : "#d1d5db" } },
				ticks: {
					line: { stroke: isDark ? "#374151" : "#d1d5db" },
					text: { fill: isDark ? "#9ca3af" : "#6b7280" },
				},
				legend: { text: { fill: isDark ? "#9ca3af" : "#6b7280" } },
			},
			grid: { line: { stroke: isDark ? "#374151" : "#e5e7eb" } },
			legends: { text: { fill: isDark ? "#9ca3af" : "#6b7280" } },
			labels: { text: { fill: isDark ? "#f3f4f6" : "#111827" } },
			tooltip: {
				container: {
					background: isDark ? "#111827" : "#ffffff",
					color: isDark ? "#f9fafb" : "#111827",
					fontSize: 12,
					borderRadius: 4,
					boxShadow: "0 2px 8px rgba(0,0,0,0.2)",
				},
			},
		}),
		[isDark],
	);

	// Load the chart component dynamically
	useEffect(() => {
		const chartInfo = CHART_PACKAGES[chartType];
		if (!chartInfo) {
			setError(`Unknown chart type: ${chartType}`);
			setLoading(false);
			return;
		}

		setLoading(true);
		setError(null);

		const loadModule = async () => {
			const ChartComponent = await chartInfo.loader();
			if (ChartComponent) {
				setChartModule(() => ChartComponent);
			} else {
				setError(`Install with: bun add ${chartInfo.pkg}`);
			}
			setLoading(false);
		};

		loadModule();
	}, [chartType]);

	// Build props based on chart type
	const chartProps = useMemo(() => {
		const chartTheme =
			(props.theme as Record<string, unknown> | undefined) ?? undefined;
		const baseProps: Record<string, unknown> = {
			data,
			theme: chartTheme ?? defaultTheme,
			margin: DEFAULT_MARGIN,
			animate: input.config.animate !== false,
		};

		// Add chart-type specific defaults
		switch (chartType) {
			case "bar":
				return {
					...baseProps,
					padding: 0.3,
					enableLabel: true,
					labelTextColor: {
						from: "color",
						modifiers: [[isDark ? "brighter" : "darker", isDark ? 1.8 : 1.2]],
					},
					labelSkipWidth: 12,
					labelSkipHeight: 12,
					enableGridY: true,
					axisBottom: { tickRotation: 0 },
					axisLeft: {},
					...props,
				};
			case "line":
				return {
					...baseProps,
					xScale: { type: "point" },
					yScale: { type: "linear", min: "auto", max: "auto" },
					curve: "monotoneX",
					lineWidth: 2,
					enablePoints: true,
					pointSize: 8,
					pointBorderWidth: 2,
					enableSlices: "x",
					enableCrosshair: true,
					axisBottom: {},
					axisLeft: {},
					...props,
				};
			case "pie":
				return {
					...baseProps,
					innerRadius: 0.5,
					padAngle: 0.7,
					cornerRadius: 3,
					activeOuterRadiusOffset: 8,
					borderWidth: 1,
					arcLinkLabelsSkipAngle: 10,
					arcLinkLabelsThickness: 2,
					arcLabelsSkipAngle: 10,
					...props,
				};
			case "radar":
				return {
					...baseProps,
					gridShape: "circular",
					gridLabelOffset: 36,
					dotSize: 10,
					dotBorderWidth: 2,
					motionConfig: "wobbly",
					...props,
				};
			case "heatmap":
				return {
					...baseProps,
					axisTop: { tickRotation: -45 },
					axisLeft: {},
					...props,
				};
			case "scatter":
				return {
					...baseProps,
					xScale: { type: "linear", min: "auto", max: "auto" },
					yScale: { type: "linear", min: "auto", max: "auto" },
					nodeSize: 10,
					axisBottom: {},
					axisLeft: {},
					...props,
				};
			case "funnel":
				return {
					...baseProps,
					direction: "vertical",
					shapeBlending: 0.66,
					borderWidth: 20,
					labelColor: { from: "color", modifiers: [["darker", 3]] },
					...props,
				};
			default:
				return { ...baseProps, ...props };
		}
	}, [data, chartType, props, input.config.animate, defaultTheme, isDark]);

	if (loading) {
		return (
			<div
				className="w-full flex items-center justify-center text-muted-foreground"
				style={{ height: input.config.height || height }}
			>
				Loading chart...
			</div>
		);
	}

	if (error) {
		return (
			<div
				className="w-full flex flex-col items-center justify-center text-destructive p-4"
				style={{ height: input.config.height || height }}
			>
				<div className="text-sm">{error}</div>
				<div className="text-xs text-muted-foreground mt-2">
					Available types: {Object.keys(CHART_PACKAGES).join(", ")}
				</div>
			</div>
		);
	}

	const ChartComponent = chartModule;
	const title = input.config.title;

	return (
		<div className="w-full">
			{title ? (
				<div className="px-1 pb-2 text-center text-sm font-medium text-foreground">
					{title}
				</div>
			) : null}
			<div
				ref={containerRef}
				className="w-full overflow-hidden rounded-md"
				style={{ height: input.config.height || height }}
			>
				{ChartComponent && <ChartComponent {...chartProps} />}
			</div>
		</div>
	);
}

export default NivoChartPreview;
