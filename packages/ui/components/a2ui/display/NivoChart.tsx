"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Component,
	type ErrorInfo,
	type ReactNode,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	CHAT_CHART_PALETTE_VARS,
	getNivoChartTheme,
} from "../../../lib/chart-theme";
import { cn } from "../../../lib/utils";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { toEventContextValue } from "../event-context";
import { useElementRef } from "../hooks/use-element-ref";
import type {
	BarChartStyle,
	BoundValue,
	CalendarChartStyle,
	ChordChartStyle,
	FunnelChartStyle,
	HeatmapChartStyle,
	LineChartStyle,
	NivoChartComponent,
	PieChartStyle,
	RadarChartStyle,
	SankeyChartStyle,
	ScatterChartStyle,
	TreemapChartStyle,
} from "../types";
import { NIVO_SAMPLE_DATA } from "./nivo-data";

/** Charts never dispatched actions before, so `*` and `actions[0]` are not
 * inherited by their new click event. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

// Error boundary to catch chart rendering errors
interface ChartErrorBoundaryProps {
	children: ReactNode;
	chartType: string;
	onError?: (error: Error) => void;
}

interface ChartErrorBoundaryState {
	hasError: boolean;
	error: Error | null;
}

class ChartErrorBoundary extends Component<
	ChartErrorBoundaryProps,
	ChartErrorBoundaryState
> {
	constructor(props: ChartErrorBoundaryProps) {
		super(props);
		this.state = { hasError: false, error: null };
	}

	static getDerivedStateFromError(error: Error): ChartErrorBoundaryState {
		return { hasError: true, error };
	}

	componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
		console.error(
			`Chart render error (${this.props.chartType}):`,
			error,
			errorInfo,
		);
		this.props.onError?.(error);
	}

	render(): ReactNode {
		if (this.state.hasError) {
			return (
				<ChartErrorFallback
					chartType={this.props.chartType}
					error={this.state.error?.message ?? "Unknown error"}
				/>
			);
		}
		return this.props.children;
	}
}

// Silhouette/placeholder for failed charts
function ChartErrorFallback({
	chartType,
	error,
}: { chartType: string; error: string }) {
	const { t } = useTranslation("common");
	return (
		<div className="w-full h-full flex flex-col items-center justify-center bg-muted/30 rounded-lg border border-dashed border-muted-foreground/30 p-4 gap-3">
			{/* Chart silhouette based on type */}
			<div className="w-full max-w-[200px] h-[120px] relative opacity-30">
				<ChartSilhouette type={chartType} />
			</div>
			<div className="text-center space-y-1">
				<p className="text-sm font-medium text-muted-foreground">
					{t(
						"failedToRenderCharttypeChart",
						"Failed to render {{chartType}} chart",
						{ chartType },
					)}
				</p>
				<p className="text-xs text-muted-foreground/70 max-w-[280px]">
					{error.length > 100 ? `${error.slice(0, 100)}...` : error}
				</p>
				<p className="text-xs text-muted-foreground/50 mt-2">
					{t(
						"checkDataFormatMatchesChartRequirements",
						"Check data format matches chart requirements",
					)}
				</p>
			</div>
		</div>
	);
}

// SVG silhouettes for different chart types
function ChartSilhouette({ type }: { type: string }) {
	const { t } = useTranslation("common");
	const fill = "currentColor";
	const stroke = "currentColor";

	switch (type) {
		case "bar":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<rect
						x="10"
						y="30"
						width="15"
						height="25"
						fill={fill}
						opacity="0.6"
						rx="2"
					/>
					<rect
						x="30"
						y="15"
						width="15"
						height="40"
						fill={fill}
						opacity="0.8"
						rx="2"
					/>
					<rect
						x="50"
						y="25"
						width="15"
						height="30"
						fill={fill}
						opacity="0.5"
						rx="2"
					/>
					<rect
						x="70"
						y="10"
						width="15"
						height="45"
						fill={fill}
						opacity="0.7"
						rx="2"
					/>
				</svg>
			);
		case "line":
		case `areaBump`:
		case "bump":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<polyline
						points="5,45 25,30 45,40 65,15 95,25"
						fill="none"
						stroke={stroke}
						strokeWidth="3"
						opacity="0.7"
					/>
					<polyline
						points="5,50 25,45 45,50 65,35 95,40"
						fill="none"
						stroke={stroke}
						strokeWidth="2"
						opacity="0.4"
					/>
				</svg>
			);
		case "pie":
		case "sunburst":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<circle
						cx="50"
						cy="30"
						r="25"
						fill="none"
						stroke={stroke}
						strokeWidth="15"
						strokeDasharray="40 100"
						opacity="0.6"
					/>
					<circle
						cx="50"
						cy="30"
						r="25"
						fill="none"
						stroke={stroke}
						strokeWidth="15"
						strokeDasharray="30 100"
						strokeDashoffset="-40"
						opacity="0.4"
					/>
					<circle
						cx="50"
						cy="30"
						r="25"
						fill="none"
						stroke={stroke}
						strokeWidth="15"
						strokeDasharray="30 100"
						strokeDashoffset="-70"
						opacity="0.8"
					/>
				</svg>
			);
		case "radar":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<polygon
						points="50,5 85,25 75,55 25,55 15,25"
						fill="none"
						stroke={stroke}
						strokeWidth="1"
						opacity="0.3"
					/>
					<polygon
						points="50,15 70,28 65,48 35,48 30,28"
						fill={fill}
						opacity="0.3"
					/>
				</svg>
			);
		case "scatter":
		case "swarmplot":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<circle cx="20" cy="40" r="4" fill={fill} opacity="0.6" />
					<circle cx="35" cy="25" r="5" fill={fill} opacity="0.7" />
					<circle cx="50" cy="35" r="3" fill={fill} opacity="0.5" />
					<circle cx="65" cy="15" r="6" fill={fill} opacity="0.8" />
					<circle cx="80" cy="30" r="4" fill={fill} opacity="0.6" />
				</svg>
			);
		case "heatmap":
		case "calendar":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					{[0, 1, 2, 3].map((row) =>
						[0, 1, 2, 3, 4].map((col) => (
							<rect
								key={`${row}-${col}`}
								x={10 + col * 18}
								y={5 + row * 14}
								width="14"
								height="10"
								fill={fill}
								opacity={0.2 + Math.random() * 0.6}
								rx="1"
							/>
						)),
					)}
				</svg>
			);
		case "treemap":
		case "circlePacking":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<rect
						x="5"
						y="5"
						width="45"
						height="50"
						fill={fill}
						opacity="0.5"
						rx="2"
					/>
					<rect
						x="55"
						y="5"
						width="40"
						height="25"
						fill={fill}
						opacity="0.7"
						rx="2"
					/>
					<rect
						x="55"
						y="35"
						width="20"
						height="20"
						fill={fill}
						opacity="0.4"
						rx="2"
					/>
					<rect
						x="78"
						y="35"
						width="17"
						height="20"
						fill={fill}
						opacity="0.6"
						rx="2"
					/>
				</svg>
			);
		case "funnel":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<polygon points="10,5 90,5 75,20 25,20" fill={fill} opacity="0.8" />
					<polygon points="25,22 75,22 65,37 35,37" fill={fill} opacity="0.6" />
					<polygon points="35,39 65,39 55,55 45,55" fill={fill} opacity="0.4" />
				</svg>
			);
		case "sankey":
		case "chord":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<rect
						x="5"
						y="10"
						width="8"
						height="15"
						fill={fill}
						opacity="0.7"
						rx="1"
					/>
					<rect
						x="5"
						y="35"
						width="8"
						height="15"
						fill={fill}
						opacity="0.5"
						rx="1"
					/>
					<rect
						x="87"
						y="15"
						width="8"
						height="30"
						fill={fill}
						opacity="0.6"
						rx="1"
					/>
					<path
						d="M13,17 Q50,10 87,25"
						fill="none"
						stroke={stroke}
						strokeWidth="4"
						opacity="0.3"
					/>
					<path
						d="M13,42 Q50,50 87,35"
						fill="none"
						stroke={stroke}
						strokeWidth="6"
						opacity="0.4"
					/>
				</svg>
			);
		case "waffle":
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					{[0, 1, 2, 3, 4].map((row) =>
						[0, 1, 2, 3, 4, 5, 6, 7].map((col) => (
							<rect
								key={`${row}-${col}`}
								x={8 + col * 11}
								y={5 + row * 11}
								width="9"
								height="9"
								fill={fill}
								opacity={row * 8 + col < 25 ? 0.7 : 0.2}
								rx="1"
							/>
						)),
					)}
				</svg>
			);
		default:
			// Generic chart placeholder
			return (
				<svg
					viewBox="0 0 100 60"
					className="w-full h-full text-muted-foreground"
				>
					<rect
						x="10"
						y="10"
						width="80"
						height="40"
						fill="none"
						stroke={stroke}
						strokeWidth="2"
						strokeDasharray="4"
						opacity="0.4"
						rx="4"
					/>
					<text
						x="50"
						y="35"
						textAnchor="middle"
						fontSize="10"
						fill={fill}
						opacity="0.5"
					>
						?
					</text>
				</svg>
			);
	}
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

/**
 * NIVO CHART PACKAGES - Install as needed:
 *
 * bun add @nivo/core                    # Required
 * bun add @nivo/bar                     # Bar charts
 * bun add @nivo/line                    # Line charts
 * bun add @nivo/pie                     # Pie/Donut charts
 * bun add @nivo/radar                   # Radar/Spider charts
 * bun add @nivo/heatmap                 # Heatmaps
 * bun add @nivo/scatterplot             # Scatter plots
 * bun add @nivo/funnel                  # Funnel charts
 * bun add @nivo/treemap                 # Treemaps
 * bun add @nivo/sunburst                # Sunburst charts
 * bun add @nivo/calendar                # Calendar heatmaps
 * bun add @nivo/bump                    # Bump & Area Bump charts
 * bun add @nivo/circle-packing          # Circle packing
 * bun add @nivo/network                 # Network graphs
 * bun add @nivo/sankey                  # Sankey diagrams
 * bun add @nivo/stream                  # Stream charts
 * bun add @nivo/swarmplot               # Swarm plots
 * bun add @nivo/voronoi                 # Voronoi diagrams
 * bun add @nivo/waffle                  # Waffle charts
 * bun add @nivo/marimekko               # Marimekko charts
 * bun add @nivo/parallel-coordinates    # Parallel coordinates
 * bun add @nivo/radial-bar              # Radial bar charts
 * bun add @nivo/boxplot                 # Box plots
 * bun add @nivo/bullet                  # Bullet charts
 * bun add @nivo/chord                   # Chord diagrams
 *
 * Install all: bun add @nivo/core @nivo/bar @nivo/line @nivo/pie @nivo/radar @nivo/heatmap @nivo/scatterplot @nivo/funnel @nivo/treemap @nivo/sunburst @nivo/calendar @nivo/bump @nivo/circle-packing @nivo/network @nivo/sankey @nivo/stream @nivo/swarmplot @nivo/voronoi @nivo/waffle @nivo/marimekko @nivo/parallel-coordinates @nivo/radial-bar @nivo/boxplot @nivo/bullet @nivo/chord
 */

type ChartModule = React.ComponentType<Record<string, unknown>>;

type ChartInfo = {
	pkg: string;
	component: string;
	loader: () => Promise<ChartModule | null>;
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const loadChart = async (
	component: string,
	importFn: () => Promise<any>,
): Promise<ChartModule | null> => {
	try {
		const mod = await importFn();
		return (mod[component] as ChartModule) ?? null;
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
	circlePacking: {
		pkg: "@nivo/circle-packing",
		component: "ResponsiveCirclePacking",
		loader: () =>
			loadChart(
				"ResponsiveCirclePacking",
				() => import("@nivo/circle-packing"),
			),
	},
	network: {
		pkg: "@nivo/network",
		component: "ResponsiveNetwork",
		loader: () => loadChart("ResponsiveNetwork", () => import("@nivo/network")),
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
	swarmplot: {
		pkg: "@nivo/swarmplot",
		component: "ResponsiveSwarmPlot",
		loader: () =>
			loadChart("ResponsiveSwarmPlot", () => import("@nivo/swarmplot")),
	},
	voronoi: {
		pkg: "@nivo/voronoi",
		component: "ResponsiveVoronoi",
		loader: () => loadChart("ResponsiveVoronoi", () => import("@nivo/voronoi")),
	},
	waffle: {
		pkg: "@nivo/waffle",
		component: "ResponsiveWaffle",
		loader: () => loadChart("ResponsiveWaffle", () => import("@nivo/waffle")),
	},
	marimekko: {
		pkg: "@nivo/marimekko",
		component: "ResponsiveMarimekko",
		loader: () =>
			loadChart("ResponsiveMarimekko", () => import("@nivo/marimekko")),
	},
	parallelCoordinates: {
		pkg: "@nivo/parallel-coordinates",
		component: "ResponsiveParallelCoordinates",
		loader: () =>
			loadChart(
				"ResponsiveParallelCoordinates",
				() => import("@nivo/parallel-coordinates"),
			),
	},
	radialBar: {
		pkg: "@nivo/radial-bar",
		component: "ResponsiveRadialBar",
		loader: () =>
			loadChart("ResponsiveRadialBar", () => import("@nivo/radial-bar")),
	},
	boxplot: {
		pkg: "@nivo/boxplot",
		component: "ResponsiveBoxPlot",
		loader: () => loadChart("ResponsiveBoxPlot", () => import("@nivo/boxplot")),
	},
	bullet: {
		pkg: "@nivo/bullet",
		component: "ResponsiveBullet",
		loader: () => loadChart("ResponsiveBullet", () => import("@nivo/bullet")),
	},
	chord: {
		pkg: "@nivo/chord",
		component: "ResponsiveChord",
		loader: () => loadChart("ResponsiveChord", () => import("@nivo/chord")),
	},
};

export function A2UINivoChart({
	elementRef,
	component,
	style,
	componentId,
}: ComponentProps<NivoChartComponent>) {
	const { t } = useTranslation("common");
	const triggerEvent = useComponentEventTrigger(componentId);
	const containerRef = useRef<HTMLDivElement>(null);
	const rootRef = useElementRef(elementRef, containerRef);
	const [chartModule, setChartModule] = useState<ChartModule | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });

	const chartType = useResolved<string>(component.chartType) ?? "bar";
	const title = useResolved<string>(component.title);
	const height = useResolved<string>(component.height) ?? "400px";
	const rawData = useResolved<unknown>(component.data);
	const rawConfig = useResolved<Record<string, unknown>>(component.config);
	const colors = useResolved<string | string[]>(component.colors);
	const animate = useResolved<boolean>(component.animate) ?? true;
	const showLegend = useResolved<boolean>(component.showLegend) ?? true;
	const legendPosition =
		useResolved<string>(component.legendPosition) ?? "bottom";
	const indexBy = useResolved<string>(component.indexBy);
	const keys = useResolved<string[]>(component.keys);
	const margin = useResolved<{
		top?: number;
		right?: number;
		bottom?: number;
		left?: number;
	}>(component.margin);
	const axisBottom = useResolved<Record<string, unknown>>(component.axisBottom);
	const axisLeft = useResolved<Record<string, unknown>>(component.axisLeft);
	const axisTop = useResolved<Record<string, unknown>>(component.axisTop);
	const axisRight = useResolved<Record<string, unknown>>(component.axisRight);

	// Chart-specific styles
	const barStyle = useResolved<BarChartStyle>(component.barStyle);
	const lineStyle = useResolved<LineChartStyle>(component.lineStyle);
	const pieStyle = useResolved<PieChartStyle>(component.pieStyle);
	const radarStyle = useResolved<RadarChartStyle>(component.radarStyle);
	const heatmapStyle = useResolved<HeatmapChartStyle>(component.heatmapStyle);
	const scatterStyle = useResolved<ScatterChartStyle>(component.scatterStyle);
	const funnelStyle = useResolved<FunnelChartStyle>(component.funnelStyle);
	const treemapStyle = useResolved<TreemapChartStyle>(component.treemapStyle);
	const sankeyStyle = useResolved<SankeyChartStyle>(component.sankeyStyle);
	const calendarStyle = useResolved<CalendarChartStyle>(
		component.calendarStyle,
	);
	const chordStyle = useResolved<ChordChartStyle>(component.chordStyle);

	useEffect(() => {
		const loadModule = async () => {
			setLoading(true);
			setError(null);
			const chartInfo = CHART_PACKAGES[chartType];
			if (!chartInfo) {
				setError(`Unknown chart type: ${chartType}`);
				setLoading(false);
				return;
			}
			const ChartComponent = await chartInfo.loader();
			if (ChartComponent) setChartModule(() => ChartComponent);
			else setError(`Install with: bun add ${chartInfo.pkg}`);
			setLoading(false);
		};
		loadModule();
	}, [chartType]);

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;
		const observer = new ResizeObserver((entries) => {
			const entry = entries[0];
			if (entry)
				setContainerSize({
					width: entry.contentRect.width,
					height: entry.contentRect.height,
				});
		});
		observer.observe(container);
		return () => observer.disconnect();
	}, []);

	const data = useMemo(() => {
		const fallbackData = NIVO_SAMPLE_DATA[chartType] ?? NIVO_SAMPLE_DATA.bar;

		// Parse raw data if string
		let parsedData: unknown;
		if (!rawData) {
			return fallbackData;
		}
		if (typeof rawData === "string") {
			try {
				parsedData = JSON.parse(rawData);
			} catch {
				return fallbackData;
			}
		} else {
			parsedData = rawData;
		}

		// Validate data format matches chart type expectations
		const isValidForChartType = (): boolean => {
			if (!parsedData) return false;

			switch (chartType) {
				case "pie":
				case "waffle":
				case "funnel":
					// Expects array of { id, value, ... }
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						"id" in parsedData[0] &&
						"value" in parsedData[0]
					);

				case "line":
				case "bump":
				case `areaBump`:
				case "radialBar":
					// Expects array of { id, data: [{ x, y }] }
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						"id" in parsedData[0] &&
						"data" in parsedData[0] &&
						Array.isArray(parsedData[0].data)
					);

				case "heatmap":
				case "scatter":
					// Expects array of { id, data: [...] }
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						"id" in parsedData[0] &&
						"data" in parsedData[0]
					);

				case "treemap":
				case "sunburst":
				case "circlePacking":
					// Expects hierarchical { id, children/value }
					return (
						typeof parsedData === "object" &&
						parsedData !== null &&
						!Array.isArray(parsedData) &&
						"id" in parsedData &&
						("children" in parsedData || "value" in parsedData)
					);

				case "sankey":
				case "network":
					// Expects { nodes: [], links: [] }
					return (
						typeof parsedData === "object" &&
						parsedData !== null &&
						!Array.isArray(parsedData) &&
						"nodes" in parsedData &&
						"links" in parsedData
					);

				case "chord":
					// Expects matrix (array of arrays) or { data: matrix, keys: [] }
					if (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						Array.isArray(parsedData[0])
					) {
						return true; // Matrix format
					}
					return (
						typeof parsedData === "object" &&
						parsedData !== null &&
						!Array.isArray(parsedData) &&
						("data" in parsedData || "matrix" in parsedData)
					);

				case "calendar":
					// Expects array of { day, value }
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						"day" in parsedData[0] &&
						"value" in parsedData[0]
					);

				case "bar":
				case "stream":
					// Expects array of objects with keys for values
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null
					);

				case "radar":
					// Expects array of objects with indexBy field
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null
					);

				case "swarmplot":
				case "boxplot":
					// Expects array with group/value
					return Array.isArray(parsedData) && parsedData.length > 0;

				case "voronoi":
					// Expects array of { x, y } or point data
					return Array.isArray(parsedData) && parsedData.length > 0;

				case "bullet":
					// Expects array with ranges/measures
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						"ranges" in parsedData[0]
					);

				case "parallelCoordinates":
					// Expects array of objects
					return Array.isArray(parsedData) && parsedData.length > 0;

				case "marimekko":
					// Expects flat objects with dimension properties (statement, participation, etc.)
					return (
						Array.isArray(parsedData) &&
						parsedData.length > 0 &&
						typeof parsedData[0] === "object" &&
						parsedData[0] !== null &&
						("statement" in parsedData[0] || "id" in parsedData[0])
					);

				default:
					return true;
			}
		};

		return isValidForChartType() ? parsedData : fallbackData;
	}, [rawData, chartType]);

	const theme = useMemo(() => getNivoChartTheme(), []);

	const colorScheme = useMemo(() => {
		if (Array.isArray(colors)) return colors;
		if (typeof colors === "string") return { scheme: colors };
		return [...CHAT_CHART_PALETTE_VARS];
	}, [colors]);

	// Responsive margins based on container size
	const isSmallContainer = containerSize.width > 0 && containerSize.width < 400;
	const chartsWithSmallMargins = [
		"funnel",
		"pie",
		"treemap",
		"sunburst",
		"circlePacking",
		"waffle",
		"chord",
		"voronoi",
		"network",
	];
	const isSmallMarginChart = chartsWithSmallMargins.includes(chartType);
	const defaultMargin = useMemo(() => {
		const baseMargin = isSmallMarginChart ? 5 : isSmallContainer ? 20 : 30;
		const bottomMargin = isSmallMarginChart ? 5 : isSmallContainer ? 30 : 50;
		const leftMargin = isSmallMarginChart ? 5 : isSmallContainer ? 35 : 50;
		return {
			top: margin?.top ?? baseMargin,
			right: margin?.right ?? baseMargin,
			bottom: margin?.bottom ?? bottomMargin,
			left: margin?.left ?? leftMargin,
		};
	}, [margin, isSmallMarginChart, isSmallContainer]);

	const legends = useMemo(() => {
		if (!showLegend) return [];
		const anchorMap: Record<string, string> = {
			top: "top",
			bottom: "bottom",
			left: "left",
			right: "right",
		};
		const directionMap: Record<string, string> = {
			top: "row",
			bottom: "row",
			left: "column",
			right: "column",
		};
		return [
			{
				anchor: anchorMap[legendPosition] ?? "bottom",
				direction: directionMap[legendPosition] ?? "row",
				translateY: legendPosition === "bottom" ? 50 : 0,
				translateX: legendPosition === "right" ? 100 : 0,
				itemWidth: 80,
				itemHeight: 20,
				symbolSize: 12,
				symbolShape: "circle",
			},
		];
	}, [showLegend, legendPosition]);

	const chartProps = useMemo(() => {
		const baseProps: Record<string, unknown> = {
			data,
			theme,
			animate,
			motionConfig: "gentle",
			margin: defaultMargin,
			colors: colorScheme,
		};
		const chartsWithLegends = [
			"bar",
			"line",
			"pie",
			"radar",
			"heatmap",
			"scatter",
			"stream",
			"waffle",
		];
		if (chartsWithLegends.includes(chartType)) baseProps.legends = legends;

		switch (chartType) {
			case "bar":
				return {
					...baseProps,
					indexBy: indexBy ?? "category",
					keys: keys ?? ["value"],
					layout: barStyle?.layout ?? "vertical",
					groupMode: barStyle?.groupMode ?? "grouped",
					padding: barStyle?.padding ?? 0.3,
					innerPadding: barStyle?.innerPadding ?? 0,
					borderRadius: barStyle?.borderRadius ?? 0,
					borderWidth: barStyle?.borderWidth ?? 0,
					enableLabel: barStyle?.enableLabel ?? true,
					labelSkipWidth: barStyle?.labelSkipWidth ?? 12,
					labelSkipHeight: barStyle?.labelSkipHeight ?? 12,
					enableGridX: barStyle?.enableGridX ?? false,
					enableGridY: barStyle?.enableGridY ?? true,
					axisBottom: axisBottom ?? { tickRotation: 0 },
					axisLeft: axisLeft ?? {},
					axisTop: axisTop ?? null,
					axisRight: axisRight ?? null,
				};
			case "line":
				return {
					...baseProps,
					xScale: { type: "point" },
					yScale: { type: "linear", min: "auto", max: "auto" },
					curve: lineStyle?.curve ?? "monotoneX",
					lineWidth: lineStyle?.lineWidth ?? 2,
					enableArea: lineStyle?.enableArea ?? false,
					areaOpacity: lineStyle?.areaOpacity ?? 0.2,
					enablePoints: lineStyle?.enablePoints ?? true,
					pointSize: lineStyle?.pointSize ?? 10,
					pointBorderWidth: lineStyle?.pointBorderWidth ?? 2,
					enableSlices: lineStyle?.enableSlices ?? "x",
					enableCrosshair: lineStyle?.enableCrosshair ?? true,
					enableGridX: lineStyle?.enableGridX ?? false,
					enableGridY: lineStyle?.enableGridY ?? true,
					axisBottom: axisBottom ?? { tickRotation: 0 },
					axisLeft: axisLeft ?? {},
					useMesh: true,
				};
			case "pie":
				return {
					...baseProps,
					innerRadius: pieStyle?.innerRadius ?? 0.5,
					padAngle: pieStyle?.padAngle ?? 0.7,
					cornerRadius: pieStyle?.cornerRadius ?? 3,
					startAngle: pieStyle?.startAngle ?? 0,
					endAngle: pieStyle?.endAngle ?? 360,
					sortByValue: pieStyle?.sortByValue ?? false,
					enableArcLabels: pieStyle?.enableArcLabels ?? true,
					enableArcLinkLabels: pieStyle?.enableArcLinkLabels ?? true,
					arcLabelsSkipAngle: pieStyle?.arcLabelsSkipAngle ?? 10,
					arcLinkLabelsSkipAngle: pieStyle?.arcLinkLabelsSkipAngle ?? 10,
					activeOuterRadiusOffset: pieStyle?.activeOuterRadiusOffset ?? 8,
					borderWidth: 1,
					arcLinkLabelsTextColor: "var(--fl-chat-chart-text-muted, #99a3b1)",
					arcLabelsTextColor: "var(--fl-chat-chart-text, #e8ebf0)",
				};
			case "radar":
				return {
					...baseProps,
					keys: keys ?? ["A", "B"],
					indexBy: indexBy ?? "taste",
					gridShape: radarStyle?.gridShape ?? "circular",
					gridLevels: radarStyle?.gridLevels ?? 5,
					gridLabelOffset: radarStyle?.gridLabelOffset ?? 20,
					dotSize: radarStyle?.dotSize ?? 10,
					dotBorderWidth: radarStyle?.dotBorderWidth ?? 2,
					enableDots: radarStyle?.enableDots ?? true,
					enableDotLabel: radarStyle?.enableDotLabel ?? false,
					fillOpacity: radarStyle?.fillOpacity ?? 0.25,
					borderWidth: radarStyle?.borderWidth ?? 2,
				};
			case "heatmap": {
				// Heatmap needs special color config - remove categorical colors and use heatmap colors
				const { colors: _colors, ...heatmapBaseProps } = baseProps;
				return {
					...heatmapBaseProps,
					forceSquare: heatmapStyle?.forceSquare ?? false,
					sizeVariation: heatmapStyle?.sizeVariation ?? 0,
					cellOpacity: heatmapStyle?.cellOpacity ?? 1,
					cellBorderWidth: heatmapStyle?.cellBorderWidth ?? 0,
					enableLabels: heatmapStyle?.enableLabels ?? true,
					labelTextColor: heatmapStyle?.labelTextColor ?? {
						from: "color",
						modifiers: [["darker", 3]],
					},
					cellBorderColor: { from: "color", modifiers: [["darker", 0.4]] },
					axisTop: axisTop ?? { tickRotation: -45 },
					axisRight: axisRight ?? null,
					axisBottom: axisBottom ?? null,
					axisLeft: axisLeft ?? {},
					hoverTarget: "cell",
				};
			}
			case "scatter":
				return {
					...baseProps,
					xScale: { type: "linear", min: "auto", max: "auto" },
					yScale: { type: "linear", min: "auto", max: "auto" },
					nodeSize: scatterStyle?.nodeSize ?? 10,
					enableGridX: scatterStyle?.enableGridX ?? true,
					enableGridY: scatterStyle?.enableGridY ?? true,
					useMesh: scatterStyle?.useMesh ?? true,
					debugMesh: scatterStyle?.debugMesh ?? false,
					axisBottom: axisBottom ?? {},
					axisLeft: axisLeft ?? {},
				};
			case "funnel": {
				// Funnel doesn't need legends, remove them
				const { legends: _legends, ...funnelBaseProps } = baseProps;
				return {
					...funnelBaseProps,
					direction: funnelStyle?.direction ?? "vertical",
					interpolation: funnelStyle?.interpolation ?? "smooth",
					spacing: funnelStyle?.spacing ?? 0,
					shapeBlending: funnelStyle?.shapeBlending ?? 0.8,
					enableLabel: funnelStyle?.enableLabel ?? true,
					labelColor: { from: "color", modifiers: [["darker", 3]] },
					borderWidth: funnelStyle?.borderWidth ?? 0,
					borderOpacity: funnelStyle?.borderOpacity ?? 0.5,
					beforeSeparatorLength: funnelStyle?.beforeSeparatorLength ?? 0,
					afterSeparatorLength: funnelStyle?.afterSeparatorLength ?? 0,
					currentPartSizeExtension: funnelStyle?.currentPartSizeExtension ?? 0,
					motionConfig: "wobbly",
				};
			}
			case "treemap": {
				return {
					...baseProps,
					identity: "id",
					value: "value",
					tile: treemapStyle?.tile ?? "squarify",
					leavesOnly: treemapStyle?.leavesOnly ?? false,
					innerPadding: treemapStyle?.innerPadding ?? 3,
					outerPadding: treemapStyle?.outerPadding ?? 3,
					enableLabel: treemapStyle?.enableLabel ?? true,
					enableParentLabel: treemapStyle?.enableParentLabel ?? true,
					labelSkipSize: treemapStyle?.labelSkipSize ?? 12,
					label: "id",
					parentLabelPosition: "left",
					parentLabelTextColor: { from: "color", modifiers: [["darker", 2]] },
					labelTextColor: { from: "color", modifiers: [["darker", 1.2]] },
					borderWidth: 1,
					borderColor: { from: "color", modifiers: [["darker", 0.1]] },
				};
			}
			case "sunburst": {
				return {
					...baseProps,
					id: "id",
					value: "value",
					cornerRadius: 2,
					borderWidth: 1,
					borderColor: { from: "color", modifiers: [["darker", 0.3]] },
					enableArcLabels: true,
					arcLabelsSkipAngle: 10,
					arcLabelsTextColor: { from: "color", modifiers: [["darker", 1.4]] },
					childColor: { from: "color", modifiers: [["brighter", 0.1]] },
				};
			}
			case "calendar": {
				// Calendar doesn't use the standard colors prop
				const {
					colors: _colors,
					legends: _legends,
					...calendarBaseProps
				} = baseProps;
				return {
					...calendarBaseProps,
					from: "2024-01-01",
					to: "2024-12-31",
					direction: calendarStyle?.direction ?? "horizontal",
					emptyColor:
						calendarStyle?.emptyColor ?? "var(--fl-chat-chart-empty, #242a33)",
					colors: [
						"var(--fl-chat-chart-3, #2fb8c6)",
						"var(--fl-chat-chart-6, #3fb27f)",
						"var(--fl-chat-chart-4, #f0a93c)",
						"var(--fl-chat-chart-1, #fb562d)",
					],
					yearSpacing: calendarStyle?.yearSpacing ?? 40,
					yearLegendOffset: calendarStyle?.yearLegendOffset ?? 10,
					monthSpacing: calendarStyle?.monthSpacing ?? 0,
					monthBorderWidth: calendarStyle?.monthBorderWidth ?? 2,
					monthBorderColor: "var(--fl-chat-chart-axis, #39404a)",
					daySpacing: calendarStyle?.daySpacing ?? 0,
					dayBorderWidth: calendarStyle?.dayBorderWidth ?? 2,
					dayBorderColor: "var(--fl-chat-surface-background, transparent)",
				};
			}
			case "bump":
			case `areaBump`:
				return {
					...baseProps,
					xPadding: 0.5,
					axisTop: null,
					axisBottom: axisBottom ?? {},
					axisLeft: axisLeft ?? {},
					pointSize: 10,
					activePointSize: 16,
					pointBorderWidth: 3,
				};
			case "circlePacking": {
				return {
					...baseProps,
					id: "id",
					value: "value",
					padding: 4,
					leavesOnly: false,
					labelsSkipRadius: 16,
					labelsTextColor: { from: "color", modifiers: [["darker", 2]] },
					borderWidth: 1,
					borderColor: { from: "color", modifiers: [["darker", 0.3]] },
					childColor: { from: "color", modifiers: [["brighter", 0.4]] },
				};
			}
			case "network": {
				return {
					...baseProps,
					linkDistance: 150,
					centeringStrength: 0.3,
					repulsivity: 6,
					nodeSize: 20,
					activeNodeSize: 28,
					inactiveNodeSize: 12,
					linkThickness: 2,
					linkColor: { from: "source.color", modifiers: [["opacity", 0.4]] },
					nodeBorderWidth: 1,
					nodeBorderColor: { from: "color", modifiers: [["darker", 0.8]] },
				};
			}
			case "sankey":
				return {
					...baseProps,
					layout: sankeyStyle?.layout ?? "horizontal",
					align: sankeyStyle?.align ?? "justify",
					nodeOpacity: sankeyStyle?.nodeOpacity ?? 1,
					nodeThickness: sankeyStyle?.nodeThickness ?? 18,
					nodeInnerPadding: sankeyStyle?.nodeInnerPadding ?? 3,
					nodeSpacing: sankeyStyle?.nodeSpacing ?? 24,
					linkOpacity: sankeyStyle?.linkOpacity ?? 0.5,
					linkBlendMode: sankeyStyle?.linkBlendMode ?? "multiply",
					enableLinkGradient: sankeyStyle?.enableLinkGradient ?? true,
					enableLabels: sankeyStyle?.enableLabels ?? true,
					labelPosition: sankeyStyle?.labelPosition ?? "outside",
				};
			case "stream":
				return {
					...baseProps,
					keys: keys ?? ["Raoul", "Josiane", "Marcel"],
					offsetType: "silhouette",
					order: "none",
					axisBottom: axisBottom ?? {},
					axisLeft: null,
					enableGridX: true,
				};
			case "swarmplot":
				return {
					...baseProps,
					groups: ["group A", "group B", "group C"],
					identity: "id",
					value: "price",
					valueScale: { type: "linear", min: "auto", max: "auto" },
					size: 10,
					spacing: 2,
					axisBottom: axisBottom ?? {},
					axisLeft: axisLeft ?? {},
				};
			case "voronoi": {
				const { colors: _colors, ...voronoiBaseProps } = baseProps;
				return {
					...voronoiBaseProps,
					xDomain: [0, 100],
					yDomain: [0, 100],
					enableLinks: true,
					linkLineWidth: 1,
					linkLineColor: "var(--fl-chat-chart-grid, #2b313a)",
					enableCells: true,
					cellLineWidth: 2,
					cellLineColor: "var(--fl-chat-chart-axis, #39404a)",
					enablePoints: true,
					pointSize: 8,
					pointColor: "var(--fl-chat-chart-1, #fb562d)",
				};
			}
			case "waffle":
				return {
					...baseProps,
					total: 100,
					rows: 10,
					columns: 10,
					padding: 1,
					borderWidth: 0,
					motionStagger: 2,
				};
			case "marimekko":
				return {
					...baseProps,
					id: "statement",
					value: "participation",
					dimensions: [
						{ id: "strongly agree", value: "stronglyAgree" },
						{ id: "agree", value: "agree" },
						{ id: "disagree", value: "disagree" },
						{ id: "strongly disagree", value: "stronglyDisagree" },
					],
					innerPadding: 9,
					axisLeft: axisLeft ?? {},
					axisBottom: axisBottom ?? { tickRotation: -45 },
				};
			case "parallelCoordinates": {
				return {
					...baseProps,
					variables: [
						{ id: "temp", value: "temp", min: "auto", max: "auto" },
						{ id: "cost", value: "cost", min: "auto", max: "auto" },
						{ id: "volume", value: "volume", min: "auto", max: "auto" },
					],
					lineWidth: 2,
					lineOpacity: 0.5,
					axesPlan: "foreground",
					axesTicksPosition: "after",
				};
			}
			case "radialBar":
				return {
					...baseProps,
					valueFormat: ">-.2f",
					startAngle: 0,
					endAngle: 360,
					innerRadius: 0.2,
					padding: 0.3,
					cornerRadius: 2,
				};
			case "boxplot":
				return {
					...baseProps,
					groupBy: "group",
					quantiles: [0.1, 0.25, 0.5, 0.75, 0.9],
					whiskerWidth: 0.5,
					borderWidth: 2,
					medianWidth: 4,
				};
			case "bullet": {
				// Bullet has its own color system (rangeColors, measureColors, markerColors)
				const {
					colors: _colors,
					legends: _legends,
					...bulletBaseProps
				} = baseProps;
				return {
					...bulletBaseProps,
					layout: "horizontal",
					titlePosition: "before",
					titleAlign: "start",
					measureSize: 0.5,
					markerSize: 0.6,
					spacing: 46,
					rangeColors: "seq:cool",
					measureColors: "seq:red_purple",
					markerColors: "seq:red_purple",
				};
			}
			case "chord": {
				// Chord uses 'data' for the matrix and needs 'keys' prop
				const { legends: _legends, ...chordBaseProps } = baseProps;
				return {
					...chordBaseProps,
					keys: keys ?? ["John", "Raoul", "Jane", "Marcel", "Ibrahim"],
					padAngle: chordStyle?.padAngle ?? 0.02,
					innerRadiusRatio: chordStyle?.innerRadiusRatio ?? 0.96,
					innerRadiusOffset: chordStyle?.innerRadiusOffset ?? 0,
					arcOpacity: chordStyle?.arcOpacity ?? 1,
					arcBorderWidth: chordStyle?.arcBorderWidth ?? 1,
					ribbonOpacity: chordStyle?.ribbonOpacity ?? 0.5,
					ribbonBorderWidth: chordStyle?.ribbonBorderWidth ?? 1,
					enableLabel: chordStyle?.enableLabel ?? true,
					labelOffset: chordStyle?.labelOffset ?? 12,
					labelRotation: chordStyle?.labelRotation ?? 0,
				};
			}
			default:
				return baseProps;
		}
	}, [
		data,
		chartType,
		theme,
		animate,
		defaultMargin,
		colorScheme,
		legends,
		indexBy,
		keys,
		axisBottom,
		axisLeft,
		axisTop,
		axisRight,
		barStyle,
		lineStyle,
		pieStyle,
		radarStyle,
		heatmapStyle,
		scatterStyle,
		funnelStyle,
		treemapStyle,
		sankeyStyle,
		calendarStyle,
		chordStyle,
	]);

	const finalProps = useMemo(() => {
		const merged = rawConfig ? { ...chartProps, ...rawConfig } : chartProps;
		return {
			...merged,
			// Every nivo chart calls `onClick(datum, event)`, but the datum shape
			// is per-chart and carries live scales and parent back-references.
			onClick: (datum: unknown) => {
				void triggerEvent(
					"pointClick",
					component,
					{ point: toEventContextValue(datum), chartType },
					EXACT_ONLY,
				);
			},
		};
	}, [chartProps, chartType, component, rawConfig, triggerEvent]);

	if (loading) {
		return (
			<div
				ref={rootRef}
				className={cn(
					"w-full flex flex-col items-center justify-center text-muted-foreground",
					resolveStyle(style),
				)}
				style={{ ...resolveInlineStyle(style), height }}
			>
				{t("loadingChart", "Loading chart...")}
			</div>
		);
	}

	if (error) {
		return (
			<div
				ref={rootRef}
				className={cn(
					"w-full flex flex-col items-center justify-center text-destructive p-4",
					resolveStyle(style),
				)}
				style={{ ...resolveInlineStyle(style), height }}
			>
				<div className="text-sm">{error}</div>
				<div className="text-xs text-muted-foreground mt-2">
					{t("available", "Available:")}{" "}
					{Object.keys(CHART_PACKAGES).join(", ")}
				</div>
			</div>
		);
	}

	const ChartComponent = chartModule;

	return (
		<div
			ref={rootRef}
			className={cn(
				"w-full flex flex-col overflow-hidden",
				resolveStyle(style),
			)}
			style={{ ...resolveInlineStyle(style), height }}
		>
			{title && (
				<div className="text-center text-sm font-medium text-foreground mb-2 shrink-0">
					{title}
				</div>
			)}
			<div className="flex-1 min-h-0 w-full min-w-0" style={{ height: "100%" }}>
				{ChartComponent && (
					<ChartErrorBoundary chartType={chartType}>
						<ChartComponent {...finalProps} />
					</ChartErrorBoundary>
				)}
			</div>
		</div>
	);
}
