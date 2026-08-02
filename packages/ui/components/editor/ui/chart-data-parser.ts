/**
 * Chart Data Parser
 *
 * Parses code block content into chart-ready data formats.
 * Supports:
 * - CSV mode: Simple tabular data with optional YAML/frontmatter config
 * - JSON mode: Full Plotly or Nivo JSON configurations
 */

// ============================================================================
// TYPES
// ============================================================================

export type ChartMode = "csv" | "json";

export type NivoChartType =
	| "bar"
	| "line"
	| "pie"
	| "radar"
	| "heatmap"
	| "scatter"
	| "funnel"
	| "treemap"
	| "sunburst"
	| "calendar"
	| "sankey"
	| "chord"
	| "stream"
	| "waffle"
	| "bump"
	| "areaBump"
	| "radialBar";

export type PlotlyChartType =
	| "bar"
	| "line"
	| "scatter"
	| "pie"
	| "area"
	| "histogram"
	| "heatmap"
	| "box"
	| "violin";

export interface CSVConfig {
	/** Chart type (e.g., "bar", "line", "pie") */
	type?: string;
	/** Chart title */
	title?: string;
	/** X-axis label */
	xLabel?: string;
	/** Y-axis label */
	yLabel?: string;
	/** Color scheme (for Nivo) or color array */
	colors?: string | string[];
	/** Chart height in pixels */
	height?: number;
	/** Whether to show legend */
	showLegend?: boolean;
	/** Legend position */
	legendPosition?: "top" | "bottom" | "left" | "right";
	/** Whether to stack bars/areas */
	stacked?: boolean;
	/** Orientation for bar charts */
	layout?: "vertical" | "horizontal";
	/** Enable animation */
	animate?: boolean;
}

export interface CSVData {
	headers: string[];
	rows: (string | number)[][];
}

export interface ChartInput {
	mode: ChartMode;
	config: CSVConfig;
	/** Parsed CSV data (for CSV mode) */
	csvData?: CSVData;
	/** Raw JSON object (for JSON mode) */
	jsonData?: Record<string, unknown>;
}

const DEFAULT_NIVO_MARGIN = { top: 30, right: 30, bottom: 50, left: 60 };

/** Nivo derives these from a continuous scale, so a categorical palette throws. */
const CONTINUOUS_COLOR_CHART_TYPES = new Set(["heatmap"]);

export function isContinuousColorChart(chartType: string): boolean {
	return CONTINUOUS_COLOR_CHART_TYPES.has(chartType);
}

/**
 * Coerces a cell to a chart value. Anything non-numeric becomes `null`, which
 * Nivo renders as a gap — feeding it through would reach the scales as `NaN`
 * and emit an unparseable SVG path.
 */
function toChartNumber(value: unknown): number | null {
	if (typeof value === "number") return Number.isFinite(value) ? value : null;
	if (typeof value !== "string") return null;
	const trimmed = value.trim();
	if (!trimmed) return null;
	const parsed = Number(trimmed);
	return Number.isFinite(parsed) ? parsed : null;
}

function nivoLegend(
	position: NonNullable<CSVConfig["legendPosition"]>,
): Record<string, unknown> {
	switch (position) {
		case "top":
			return {
				anchor: "top",
				direction: "row",
				justify: false,
				translateY: -24,
				itemsSpacing: 8,
				itemWidth: 90,
				itemHeight: 18,
				itemDirection: "left-to-right",
				symbolSize: 12,
				symbolShape: "circle",
			};
		case "left":
			return {
				anchor: "left",
				direction: "column",
				justify: false,
				translateX: -112,
				itemsSpacing: 6,
				itemWidth: 100,
				itemHeight: 18,
				itemDirection: "left-to-right",
				symbolSize: 12,
				symbolShape: "circle",
			};
		case "right":
			return {
				anchor: "right",
				direction: "column",
				justify: false,
				translateX: 112,
				itemsSpacing: 6,
				itemWidth: 100,
				itemHeight: 18,
				itemDirection: "left-to-right",
				symbolSize: 12,
				symbolShape: "circle",
			};
		case "bottom":
			return {
				anchor: "bottom",
				direction: "row",
				justify: false,
				translateY: 64,
				itemsSpacing: 8,
				itemWidth: 90,
				itemHeight: 18,
				itemDirection: "left-to-right",
				symbolSize: 12,
				symbolShape: "circle",
			};
	}
}

function nivoMargin(config: CSVConfig): Record<string, number> {
	const margin = { ...DEFAULT_NIVO_MARGIN };
	if (config.xLabel) margin.bottom = 68;
	if (config.yLabel) margin.left = 78;

	if (config.showLegend || config.legendPosition) {
		switch (config.legendPosition ?? "bottom") {
			case "top":
				margin.top = Math.max(margin.top, 64);
				break;
			case "bottom":
				margin.bottom = Math.max(margin.bottom, 88);
				break;
			case "left":
				margin.left = Math.max(margin.left, 136);
				break;
			case "right":
				margin.right = Math.max(margin.right, 136);
				break;
		}
	}
	return margin;
}

function plotlyTitle(text: string): { text: string } {
	return { text };
}

export function normalizePlotlyTitle(value: unknown): Record<string, unknown> {
	if (typeof value === "string") return plotlyTitle(value);
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

function plotlyLegend(
	position: NonNullable<CSVConfig["legendPosition"]>,
): Record<string, unknown> {
	switch (position) {
		case "top":
			return {
				orientation: "h",
				x: 0.5,
				xanchor: "center",
				y: 1.08,
				yanchor: "bottom",
			};
		case "left":
			return {
				orientation: "v",
				x: -0.08,
				xanchor: "right",
				y: 0.5,
				yanchor: "middle",
			};
		case "right":
			return {
				orientation: "v",
				x: 1.04,
				xanchor: "left",
				y: 0.5,
				yanchor: "middle",
			};
		case "bottom":
			return {
				orientation: "h",
				x: 0.5,
				xanchor: "center",
				y: -0.16,
				yanchor: "top",
			};
	}
}

// ============================================================================
// CSV PARSING
// ============================================================================

const FENCE_LINE = /^\s*(?:`{3,}|~{3,})\s*\w*\s*$/;

/**
 * Drops fence markers that survived markdown deserialization — a leaked closing
 * fence would otherwise parse as a data row and show up as an axis category.
 */
function stripFenceLines(content: string): string {
	return content
		.split("\n")
		.filter((line) => !FENCE_LINE.test(line))
		.join("\n");
}

/**
 * Parse CSV string into headers and rows
 */
function parseCSV(csvContent: string): CSVData {
	const lines = csvContent
		.trim()
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);

	if (lines.length === 0) {
		return { headers: [], rows: [] };
	}

	// First line is headers
	const headers = lines[0].split(",").map((h) => h.trim());
	const rows: (string | number)[][] = [];

	// Parse remaining lines
	for (let i = 1; i < lines.length; i++) {
		const cells = lines[i].split(",").map((cell) => {
			const trimmed = cell.trim();
			const num = Number(trimmed);
			return Number.isNaN(num) ? trimmed : num;
		});
		rows.push(cells);
	}

	return { headers, rows };
}

/**
 * Parse YAML-like config from frontmatter
 * Simple key: value parser, no full YAML support needed
 */
function parseConfig(configBlock: string): CSVConfig {
	const config: Record<string, string | number | boolean | string[]> = {};
	const lines = configBlock.trim().split("\n");

	for (const line of lines) {
		const colonIndex = line.indexOf(":");
		if (colonIndex === -1) continue;

		const key = line.slice(0, colonIndex).trim();
		let value: string | number | boolean | string[] = line
			.slice(colonIndex + 1)
			.trim();

		// Parse common values
		if (value === "true") value = true;
		else if (value === "false") value = false;
		else if (!Number.isNaN(Number(value))) value = Number(value);
		else if (value.startsWith("[") && value.endsWith("]")) {
			// Parse array: [a, b, c]
			value = value
				.slice(1, -1)
				.split(",")
				.map((v) => v.trim().replace(/^["']|["']$/g, ""));
		}

		config[key] = value;
	}

	return config as CSVConfig;
}

/**
 * Auto-detect chart type from CSV data
 */
function autoDetectChartType(data: CSVData): string {
	if (data.headers.length === 0 || data.rows.length === 0) {
		return "bar";
	}

	const numCols = data.headers.length;
	const numRows = data.rows.length;

	// If 2 columns and second is numeric, could be pie or bar
	if (numCols === 2) {
		const secondColNumeric = data.rows.every(
			(row) => typeof row[1] === "number",
		);
		if (secondColNumeric) {
			// Few categories = pie, many = bar
			return numRows <= 6 ? "pie" : "bar";
		}
	}

	// If 3+ columns with numeric values, likely grouped bar or line
	if (numCols >= 3) {
		const hasTimeLikeFirst = data.rows.some((row) => {
			const val = String(row[0]).toLowerCase();
			return (
				val.includes("jan") ||
				val.includes("feb") ||
				val.includes("q1") ||
				val.includes("2024") ||
				/^\d{4}/.test(val)
			);
		});
		return hasTimeLikeFirst ? "line" : "bar";
	}

	return "bar";
}

// ============================================================================
// DATA TRANSFORMATIONS
// ============================================================================

/**
 * Transform CSV data to Nivo bar format
 */
export function csvToNivoBar(data: CSVData): unknown[] {
	const [indexKey, ...valueKeys] = data.headers;
	return data.rows.map((row) => {
		const item: Record<string, string | number> = { [indexKey]: row[0] };
		valueKeys.forEach((key, i) => {
			item[key] = toChartNumber(row[i + 1]) ?? 0;
		});
		return item;
	});
}

/**
 * Transform CSV data to Nivo line format
 */
export function csvToNivoLine(data: CSVData): unknown[] {
	const [xKey, ...seriesKeys] = data.headers;
	return seriesKeys.map((seriesId) => ({
		id: seriesId,
		data: data.rows.map((row) => {
			const xIndex = 0;
			const yIndex = data.headers.indexOf(seriesId);
			return {
				x: row[xIndex],
				y: toChartNumber(row[yIndex]),
			};
		}),
	}));
}

/**
 * Transform CSV data to Nivo pie format
 */
export function csvToNivoPie(data: CSVData): unknown[] {
	return data.rows.map((row) => ({
		id: String(row[0]),
		label: String(row[0]),
		value: toChartNumber(row[1]) ?? 0,
	}));
}

/**
 * Transform CSV data to Nivo radar format
 */
export function csvToNivoRadar(data: CSVData): unknown[] {
	const [indexKey, ...valueKeys] = data.headers;
	return data.rows.map((row) => {
		const item: Record<string, string | number> = { [indexKey]: row[0] };
		valueKeys.forEach((key, i) => {
			item[key] = toChartNumber(row[i + 1]) ?? 0;
		});
		return item;
	});
}

/**
 * Transform CSV data to Nivo heatmap format
 */
export function csvToNivoHeatmap(data: CSVData): unknown[] {
	const [rowLabel, ...colLabels] = data.headers;
	return data.rows.map((row) => ({
		id: String(row[0]),
		data: colLabels.map((col, i) => ({
			x: col,
			y: toChartNumber(row[i + 1]),
		})),
	}));
}

/**
 * Transform CSV data to Nivo funnel format
 */
export function csvToNivoFunnel(data: CSVData): unknown[] {
	return data.rows.map((row, index) => ({
		id: `step_${index}`,
		value: toChartNumber(row[1]) ?? 0,
		label: String(row[0]),
	}));
}

/**
 * Transform CSV to Nivo scatter format
 */
export function csvToNivoScatter(data: CSVData): unknown[] {
	// Assumes columns: group, x, y
	const groups = new Map<string, { x: number; y: number }[]>();

	for (const row of data.rows) {
		const group = String(row[0]);
		const x = toChartNumber(row[1]);
		const y = toChartNumber(row[2]);
		if (x === null || y === null) continue;

		const points = groups.get(group);
		if (points) points.push({ x, y });
		else groups.set(group, [{ x, y }]);
	}

	return Array.from(groups.entries()).map(([id, dataPoints]) => ({
		id,
		data: dataPoints,
	}));
}

/**
 * Transform CSV to Plotly format
 */
export function csvToPlotly(
	data: CSVData,
	chartType: string,
	stacked?: boolean,
): { data: unknown[]; layout: Record<string, unknown> } {
	const [xKey, ...yKeys] = data.headers;
	const x = data.rows.map((row) => row[0]);

	const plotlyType = chartType === "area" ? "scatter" : chartType;

	const traces = yKeys.map((yKey, i) => {
		const y = data.rows.map((row) => row[i + 1]);
		const trace: Record<string, unknown> = {
			name: yKey,
			x,
			y,
			type: plotlyType,
		};

		if (chartType === "area") {
			trace.mode = "lines";
			// `tonexty` fills to the previous trace's raw values, which only reads
			// as a stack when the series are already cumulative. `stackgroup` makes
			// Plotly do the accumulation; unstacked areas each fill to zero.
			if (stacked === false) trace.fill = "tozeroy";
			else trace.stackgroup = "one";
		}
		if (chartType === "line") {
			trace.mode = "lines+markers";
		}
		if (chartType === "scatter") {
			trace.mode = "markers";
		}

		return trace;
	});

	// For pie charts, restructure
	if (chartType === "pie") {
		return {
			data: [
				{
					type: "pie",
					labels: x,
					values: data.rows.map((row) => row[1]),
				},
			],
			layout: {},
		};
	}

	return {
		data: traces,
		layout: {
			xaxis: { title: plotlyTitle(xKey) },
			barmode: chartType === "bar" ? "group" : undefined,
		},
	};
}

// ============================================================================
// MAIN PARSER
// ============================================================================

/**
 * Parse chart code block content into ChartInput
 */
export function parseChartData(
	content: string,
	language: "nivo" | "plotly",
): ChartInput {
	const trimmed = stripFenceLines(content).trim();

	// Check if it's JSON mode
	if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
		try {
			const jsonData = JSON.parse(trimmed);
			return {
				mode: "json",
				config: {},
				jsonData,
			};
		} catch {
			throw new Error("Invalid JSON in chart code block");
		}
	}

	// CSV mode - check for frontmatter config
	let config: CSVConfig = {};
	let csvContent = trimmed;

	// Check for YAML-like frontmatter separated by ---
	const frontmatterMatch = trimmed.match(/^([\s\S]*?)\n---\n([\s\S]*)$/);
	if (frontmatterMatch) {
		config = parseConfig(frontmatterMatch[1]);
		csvContent = frontmatterMatch[2];
	}

	const csvData = parseCSV(csvContent);

	// Auto-detect chart type if not specified
	if (!config.type) {
		config.type = autoDetectChartType(csvData);
	}

	return {
		mode: "csv",
		config,
		csvData,
	};
}

/**
 * Maps a configured colour to what the chart's scale accepts. Continuous charts
 * need a `{ type: "sequential" }` config; a bare array or scheme name throws.
 * Returns `null` when the configuration cannot produce a valid scale, so the
 * renderer's themed default applies instead.
 */
export function normalizeNivoColors(
	chartType: string,
	colors: string | string[],
): unknown {
	if (!isContinuousColorChart(chartType)) {
		return Array.isArray(colors) ? colors : { scheme: colors };
	}
	if (!Array.isArray(colors)) return { type: "sequential", scheme: colors };

	const stops = colors.filter(
		(color) => typeof color === "string" && color.trim().length > 0,
	);
	if (stops.length < 2) return null;
	return { type: "sequential", colors: [stops[0], stops[stops.length - 1]] };
}

function isContinuousColorConfig(value: unknown): boolean {
	if (typeof value === "function") return true;
	if (!value || typeof value !== "object" || Array.isArray(value)) return false;
	const type = (value as { type?: unknown }).type;
	return type === "sequential" || type === "diverging" || type === "quantize";
}

/**
 * Hand-written JSON blocks routinely carry a categorical `colors` array, which
 * a continuous chart throws on. Convert what can be converted, drop the rest so
 * the renderer's themed default takes over.
 */
function sanitizeContinuousColors(
	chartType: string,
	props: Record<string, unknown>,
): Record<string, unknown> {
	if (!isContinuousColorChart(chartType) || !("colors" in props)) return props;

	const colors = props.colors;
	if (isContinuousColorConfig(colors)) return props;

	// Omit rather than blank the key: `colors: undefined` would still override
	// the renderer's default once the props are spread over it.
	const { colors: _dropped, ...withoutColors } = props;
	const normalized =
		typeof colors === "string" || Array.isArray(colors)
			? normalizeNivoColors(chartType, colors as string | string[])
			: null;

	return normalized ? { ...withoutColors, colors: normalized } : withoutColors;
}

function isRenderablePoint(point: unknown): boolean {
	if (!point || typeof point !== "object") return false;
	const { x, y } = point as { x?: unknown; y?: unknown };
	return x !== null && x !== undefined && y !== null && y !== undefined;
}

/**
 * Whether Nivo can draw the dataset. Series with no plottable point make its
 * path generators return `null`, which reaches the DOM as `d="null"` — so a
 * partially streamed or empty code block must render a placeholder instead.
 */
export function hasRenderableNivoData(data: unknown): boolean {
	if (Array.isArray(data)) {
		if (data.length === 0) return false;
		const series = data.filter(
			(entry): entry is { data: unknown[] } =>
				Boolean(entry) &&
				typeof entry === "object" &&
				Array.isArray((entry as { data?: unknown }).data),
		);
		if (series.length === 0) return true;
		return series.some((entry) => entry.data.some(isRenderablePoint));
	}
	if (data && typeof data === "object") {
		const nodes = (data as { nodes?: unknown }).nodes;
		return Array.isArray(nodes) ? nodes.length > 0 : true;
	}
	return false;
}

/**
 * Transform ChartInput to Nivo data format
 */
export function toNivoData(input: ChartInput): {
	data: unknown;
	chartType: string;
	props: Record<string, unknown>;
} {
	if (input.mode === "json" && input.jsonData) {
		// JSON mode - pass through with type extraction
		const chartType = (input.jsonData.chartType as string) || "bar";
		const { chartType: _, ...rest } = input.jsonData;
		return {
			data: rest.data ?? input.jsonData,
			chartType,
			props: sanitizeContinuousColors(chartType, rest),
		};
	}

	// CSV mode
	const chartType = input.config.type || "bar";
	const csvData = input.csvData;
	if (!csvData) throw new Error("CSV chart data is missing.");
	const props: Record<string, unknown> = {};

	let data: unknown;

	switch (chartType) {
		case "line":
		case "bump":
		case "areaBump":
			data = csvToNivoLine(csvData);
			break;
		case "pie":
		case "sunburst":
			data = csvToNivoPie(csvData);
			break;
		case "radar":
			data = csvToNivoRadar(csvData);
			props.indexBy = csvData.headers[0];
			props.keys = csvData.headers.slice(1);
			break;
		case "heatmap":
			data = csvToNivoHeatmap(csvData);
			break;
		case "funnel":
			data = csvToNivoFunnel(csvData);
			break;
		case "scatter":
			data = csvToNivoScatter(csvData);
			break;
		default:
			data = csvToNivoBar(csvData);
			props.indexBy = csvData.headers[0];
			props.keys = csvData.headers.slice(1);
			if (input.config.layout === "horizontal") {
				props.layout = "horizontal";
			}
			if (input.config.stacked !== undefined) {
				props.groupMode = input.config.stacked ? "stacked" : "grouped";
			}
			break;
	}

	// Apply common config
	if (input.config.colors) {
		const colors = normalizeNivoColors(chartType, input.config.colors);
		if (colors) props.colors = colors;
	}
	if (input.config.xLabel) {
		props.axisBottom = {
			legend: input.config.xLabel,
			legendOffset: 44,
			legendPosition: "middle",
		};
	}
	if (input.config.yLabel) {
		props.axisLeft = {
			legend: input.config.yLabel,
			legendOffset: -52,
			legendPosition: "middle",
		};
	}
	if (input.config.showLegend === false) {
		props.legends = [];
	} else if (input.config.showLegend || input.config.legendPosition) {
		props.legends = [nivoLegend(input.config.legendPosition ?? "bottom")];
	}
	if (input.config.animate !== undefined) {
		props.animate = input.config.animate;
	}
	props.margin = nivoMargin(input.config);

	return { data, chartType, props };
}

/**
 * Transform ChartInput to Plotly data format
 */
export function toPlotlyData(input: ChartInput): {
	data: unknown[];
	layout: Record<string, unknown>;
	config: Record<string, unknown>;
} {
	if (input.mode === "json" && input.jsonData) {
		// JSON mode - Plotly native format
		return {
			data: (input.jsonData.data as unknown[]) || [],
			layout: (input.jsonData.layout as Record<string, unknown>) || {},
			config: (input.jsonData.config as Record<string, unknown>) || {},
		};
	}

	// CSV mode
	const chartType = input.config.type || "bar";
	const csvData = input.csvData;
	if (!csvData) throw new Error("CSV chart data is missing.");
	const result = csvToPlotly(csvData, chartType, input.config.stacked);

	// Apply config
	const layout: Record<string, unknown> = {
		...result.layout,
		paper_bgcolor: "transparent",
		plot_bgcolor: "transparent",
		font: { color: "#888" },
		margin: { t: 40, r: 20, b: 40, l: 50 },
	};

	if (input.config.title) {
		layout.title = plotlyTitle(input.config.title);
	}
	if (input.config.xLabel) {
		layout.xaxis = {
			...((layout.xaxis as object) || {}),
			title: plotlyTitle(input.config.xLabel),
		};
	}
	if (input.config.yLabel) {
		layout.yaxis = { title: plotlyTitle(input.config.yLabel) };
	}
	if (input.config.showLegend !== undefined) {
		layout.showlegend = input.config.showLegend;
	}
	if (input.config.legendPosition) {
		layout.legend = plotlyLegend(input.config.legendPosition);
		const margin = {
			...((layout.margin as Record<string, number>) || {}),
		};
		switch (input.config.legendPosition) {
			case "top":
				margin.t = Math.max(margin.t ?? 0, 76);
				break;
			case "bottom":
				margin.b = Math.max(margin.b ?? 0, 76);
				break;
			case "left":
				margin.l = Math.max(margin.l ?? 0, 112);
				break;
			case "right":
				margin.r = Math.max(margin.r ?? 0, 112);
				break;
		}
		layout.margin = margin;
	}
	if (chartType === "bar" && input.config.stacked) {
		layout.barmode = "stack";
	}
	if (chartType === "bar" && input.config.layout === "horizontal") {
		for (const trace of result.data as Array<Record<string, unknown>>) {
			const x = trace.x;
			trace.x = trace.y;
			trace.y = x;
			trace.orientation = "h";
		}
	}
	if (input.config.height) {
		layout.height = input.config.height;
	}

	return {
		data: result.data,
		layout,
		config: {
			responsive: true,
			// Hover-only: a pinned toolbar covers the title and the top of the plot.
			displayModeBar: "hover",
			displaylogo: false,
			modeBarButtonsToRemove: ["lasso2d", "select2d"],
		},
	};
}
