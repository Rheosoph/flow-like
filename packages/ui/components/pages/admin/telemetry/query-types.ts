export type ITelemetryQueryDataset =
	| "events"
	| "errors"
	| "spans"
	| "performance"
	| "sessions"
	| "llm";

export type ITelemetryQueryMetricType =
	| "count"
	| "count_distinct"
	| "sum"
	| "avg"
	| "min"
	| "max"
	| "p50"
	| "p75"
	| "p95"
	| "p99";

export type ITelemetryQueryFilterOp =
	| "eq"
	| "neq"
	| "contains"
	| "gt"
	| "lt"
	| "gte"
	| "lte"
	| "in";

export type ITelemetryQueryInterval = "minute" | "hour" | "day" | "none";

export type ITelemetryQueryFieldKind = "string" | "number" | "bool";

export type ITelemetryQueryValue = string | number | boolean | null;

export interface ITelemetryQueryMetric {
	type: ITelemetryQueryMetricType;
	field?: string | null;
}

export interface ITelemetryQueryFilter {
	field: string;
	op: ITelemetryQueryFilterOp;
	value: ITelemetryQueryValue | ITelemetryQueryValue[];
}

export interface ITelemetryQueryRequest {
	dataset: ITelemetryQueryDataset;
	metric: ITelemetryQueryMetric;
	filters?: ITelemetryQueryFilter[];
	breakdown?: string | null;
	interval?: ITelemetryQueryInterval;
	hours: number;
}

export interface ITelemetryQueryResponse {
	columns: string[];
	rows: ITelemetryQueryValue[][];
	interval: string;
	total: number;
	/** Set when the server hit its row cap and dropped part of the result. */
	truncated?: boolean;
}

export interface ITelemetrySavedQuery {
	id: string;
	name: string;
	definition: ITelemetryQueryRequest;
	createdAt: string;
	updatedAt: string;
}

export interface ITelemetrySavedQueriesResponse {
	savedQueries: ITelemetrySavedQuery[];
}

export type ITelemetryDashboardTileWidth = "half" | "full";

export type ITelemetryQueryView = "chart" | "table";

export interface ITelemetryDashboardTile {
	id: string;
	savedQueryId: string;
	title: string;
	width: ITelemetryDashboardTileWidth;
	view: ITelemetryQueryView;
}

/**
 * Server bounds on a dashboard, mirrored from
 * `packages/api/src/routes/admin/telemetry/dashboards.rs`. Exceeding either one
 * is a 400, so the editor enforces them before the PATCH goes out.
 */
export const TELEMETRY_DASHBOARD_MAX_TILES = 24;
export const TELEMETRY_DASHBOARD_MAX_TILE_TITLE = 120;

export interface ITelemetryDashboard {
	id: string;
	name: string;
	tiles: ITelemetryDashboardTile[];
	createdAt: string;
	updatedAt: string;
}

export interface ITelemetryDashboardsResponse {
	dashboards: ITelemetryDashboard[];
}

export interface ITelemetryQueryField {
	readonly field: string;
	readonly label: string;
	readonly kind: ITelemetryQueryFieldKind;
}

export interface ITelemetryQueryDatasetSpec {
	readonly dataset: ITelemetryQueryDataset;
	readonly label: string;
	readonly description: string;
	readonly fields: readonly ITelemetryQueryField[];
}

/**
 * Mirror of the server-side per-dataset field allowlist. The UI only ever offers
 * these logical names; the API resolves each one to a constant physical column.
 * Adding a field here without adding it server-side yields a 400, never SQL text.
 */
export const TELEMETRY_QUERY_DATASETS: readonly ITelemetryQueryDatasetSpec[] = [
	{
		dataset: "events",
		label: "Events",
		description: "Anonymous product events reported by opted-in installs.",
		fields: [
			{ field: "name", label: "Event name", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "app_version", label: "App version", kind: "string" },
			{ field: "platform", label: "Platform", kind: "string" },
			{ field: "country", label: "Country", kind: "string" },
		],
	},
	{
		dataset: "errors",
		label: "Errors",
		description: "Individual error and crash events behind grouped issues.",
		fields: [
			{ field: "issue_id", label: "Issue", kind: "string" },
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "platform", label: "Platform", kind: "string" },
			{ field: "app_version", label: "App version", kind: "string" },
			{ field: "release", label: "Release", kind: "string" },
			{ field: "kind", label: "Kind", kind: "string" },
			{ field: "title", label: "Title", kind: "string" },
			{ field: "culprit", label: "Culprit", kind: "string" },
			{ field: "level", label: "Level", kind: "string" },
			{ field: "country", label: "Country", kind: "string" },
		],
	},
	{
		dataset: "spans",
		label: "Spans",
		description: "Sampled trace spans with durations and status.",
		fields: [
			{ field: "trace_id", label: "Trace", kind: "string" },
			{ field: "name", label: "Span name", kind: "string" },
			{ field: "kind", label: "Span kind", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "release", label: "Release", kind: "string" },
			{ field: "platform", label: "Platform", kind: "string" },
			{ field: "status", label: "Status", kind: "string" },
			{ field: "duration_ms", label: "Duration (ms)", kind: "number" },
		],
	},
	{
		dataset: "performance",
		label: "Performance",
		description: "Web vitals and app-start samples per path.",
		fields: [
			{ field: "metric", label: "Metric", kind: "string" },
			{ field: "path", label: "Path", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "platform", label: "Platform", kind: "string" },
			{ field: "release", label: "Release", kind: "string" },
			{ field: "country", label: "Country", kind: "string" },
			{ field: "value", label: "Value", kind: "number" },
		],
	},
	{
		dataset: "sessions",
		label: "Sessions",
		description: "Session starts and outcomes used for release health.",
		fields: [
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "release", label: "Release", kind: "string" },
			{ field: "platform", label: "Platform", kind: "string" },
			{ field: "status", label: "Status", kind: "string" },
			{ field: "duration_ms", label: "Duration (ms)", kind: "number" },
		],
	},
	{
		dataset: "llm",
		label: "LLM calls",
		description: "Model invocations with latency, tokens and error kinds.",
		fields: [
			{ field: "provider", label: "Provider", kind: "string" },
			{ field: "model", label: "Model", kind: "string" },
			{ field: "operation", label: "Operation", kind: "string" },
			{ field: "status", label: "Status", kind: "string" },
			{ field: "error_kind", label: "Error kind", kind: "string" },
			{ field: "source", label: "Source", kind: "string" },
			{ field: "release", label: "Release", kind: "string" },
			{ field: "anon_id", label: "Install", kind: "string" },
			{ field: "streamed", label: "Streamed", kind: "bool" },
			{ field: "duration_ms", label: "Duration (ms)", kind: "number" },
			{ field: "prompt_tokens", label: "Prompt tokens", kind: "number" },
			{
				field: "completion_tokens",
				label: "Completion tokens",
				kind: "number",
			},
			{ field: "total_tokens", label: "Total tokens", kind: "number" },
			{ field: "tool_calls", label: "Tool calls", kind: "number" },
		],
	},
];

export const TELEMETRY_QUERY_METRIC_TYPES: readonly {
	value: ITelemetryQueryMetricType;
	label: string;
}[] = [
	{ value: "count", label: "Count" },
	{ value: "count_distinct", label: "Distinct count" },
	{ value: "sum", label: "Sum" },
	{ value: "avg", label: "Average" },
	{ value: "min", label: "Minimum" },
	{ value: "max", label: "Maximum" },
	{ value: "p50", label: "p50" },
	{ value: "p75", label: "p75" },
	{ value: "p95", label: "p95" },
	{ value: "p99", label: "p99" },
];

export const TELEMETRY_QUERY_FILTER_OPS: readonly {
	value: ITelemetryQueryFilterOp;
	label: string;
}[] = [
	{ value: "eq", label: "is" },
	{ value: "neq", label: "is not" },
	{ value: "contains", label: "contains" },
	{ value: "in", label: "is one of" },
	{ value: "gt", label: ">" },
	{ value: "gte", label: "≥" },
	{ value: "lt", label: "<" },
	{ value: "lte", label: "≤" },
];

export const TELEMETRY_QUERY_INTERVALS: readonly {
	value: ITelemetryQueryInterval;
	label: string;
}[] = [
	{ value: "none", label: "No time buckets" },
	{ value: "minute", label: "Per minute" },
	{ value: "hour", label: "Per hour" },
	{ value: "day", label: "Per day" },
];

export const TELEMETRY_QUERY_HOUR_OPTIONS: readonly {
	value: number;
	label: string;
}[] = [
	{ value: 1, label: "Last hour" },
	{ value: 6, label: "Last 6 hours" },
	{ value: 24, label: "Last 24 hours" },
	{ value: 72, label: "Last 3 days" },
	{ value: 168, label: "Last 7 days" },
	{ value: 720, label: "Last 30 days" },
	{ value: 2160, label: "Last 90 days" },
];

export const TELEMETRY_QUERY_MAX_HOURS = 2160;
export const TELEMETRY_QUERY_MAX_FILTERS = 8;
export const TELEMETRY_QUERY_MAX_BREAKDOWN_ROWS = 50;
export const TELEMETRY_QUERY_MAX_ROWS = 5000;
export const TELEMETRY_QUERY_TABLE_PREVIEW_ROWS = 500;

const NUMERIC_ONLY_OPS: readonly ITelemetryQueryFilterOp[] = [
	"gt",
	"gte",
	"lt",
	"lte",
];

const AGGREGATE_METRIC_TYPES: readonly ITelemetryQueryMetricType[] = [
	"sum",
	"avg",
	"min",
	"max",
	"p50",
	"p75",
	"p95",
	"p99",
];

export function telemetryDatasetSpec(
	dataset: string,
): ITelemetryQueryDatasetSpec | undefined {
	return TELEMETRY_QUERY_DATASETS.find((spec) => spec.dataset === dataset);
}

export function telemetryFieldSpec(
	dataset: string,
	field: string | null | undefined,
): ITelemetryQueryField | undefined {
	if (!field) return undefined;
	return telemetryDatasetSpec(dataset)?.fields.find((f) => f.field === field);
}

export function telemetryNumericFields(
	dataset: string,
): readonly ITelemetryQueryField[] {
	return (telemetryDatasetSpec(dataset)?.fields ?? []).filter(
		(f) => f.kind === "number",
	);
}

/** The server groups on a text column only, so numeric and boolean fields are
 * not offerable as a breakdown. */
export function telemetryBreakdownFields(
	dataset: string,
): readonly ITelemetryQueryField[] {
	return (telemetryDatasetSpec(dataset)?.fields ?? []).filter(
		(f) => f.kind === "string",
	);
}

export function telemetryMetricNeedsNumericField(
	type: ITelemetryQueryMetricType,
): boolean {
	return AGGREGATE_METRIC_TYPES.includes(type);
}

export function telemetryMetricNeedsField(
	type: ITelemetryQueryMetricType,
): boolean {
	return type === "count_distinct" || telemetryMetricNeedsNumericField(type);
}

export function telemetryFilterOpsForKind(
	kind: ITelemetryQueryFieldKind | undefined,
): readonly { value: ITelemetryQueryFilterOp; label: string }[] {
	if (kind === "number") {
		return TELEMETRY_QUERY_FILTER_OPS.filter((op) => op.value !== "contains");
	}
	if (kind === "bool") {
		return TELEMETRY_QUERY_FILTER_OPS.filter(
			(op) => op.value === "eq" || op.value === "neq",
		);
	}
	return TELEMETRY_QUERY_FILTER_OPS.filter(
		(op) => !NUMERIC_ONLY_OPS.includes(op.value),
	);
}

export function defaultTelemetryQuery(): ITelemetryQueryRequest {
	return {
		dataset: "events",
		metric: { type: "count" },
		filters: [],
		breakdown: null,
		interval: "hour",
		hours: 24,
	};
}

export function telemetryQueryFilterValueToText(
	value: ITelemetryQueryValue | ITelemetryQueryValue[],
): string {
	if (Array.isArray(value)) return value.map((v) => String(v ?? "")).join(", ");
	if (value === null || value === undefined) return "";
	return String(value);
}

export function telemetryQueryFilterValueFromText(
	kind: ITelemetryQueryFieldKind | undefined,
	op: ITelemetryQueryFilterOp,
	text: string,
): ITelemetryQueryValue | ITelemetryQueryValue[] {
	if (op === "in") {
		return text
			.split(",")
			.map((part) => part.trim())
			.filter((part) => part.length > 0)
			.map((part) => (kind === "number" ? Number(part) : part));
	}
	if (kind === "number") return text.trim() === "" ? "" : Number(text);
	if (kind === "bool") return text === "true";
	return text;
}

export function validateTelemetryQuery(
	request: ITelemetryQueryRequest,
): string[] {
	const errors: string[] = [];
	const spec = telemetryDatasetSpec(request.dataset);
	if (!spec) {
		return [`Unknown dataset "${request.dataset}".`];
	}

	const metricType = request.metric?.type;
	if (!TELEMETRY_QUERY_METRIC_TYPES.some((m) => m.value === metricType)) {
		errors.push(`Unknown metric "${String(metricType)}".`);
	} else if (telemetryMetricNeedsField(metricType)) {
		const field = telemetryFieldSpec(request.dataset, request.metric.field);
		if (!field) {
			errors.push(`The ${metricType} metric needs a field from this dataset.`);
		} else if (
			telemetryMetricNeedsNumericField(metricType) &&
			field.kind !== "number"
		) {
			errors.push(`"${field.label}" is not numeric — pick a numeric field.`);
		}
	}

	const filters = request.filters ?? [];
	if (filters.length > TELEMETRY_QUERY_MAX_FILTERS) {
		errors.push(`At most ${TELEMETRY_QUERY_MAX_FILTERS} filters are allowed.`);
	}
	filters.forEach((filter, index) => {
		const position = index + 1;
		const field = telemetryFieldSpec(request.dataset, filter.field);
		if (!field) {
			errors.push(
				`Filter ${position} uses a field not allowed on this dataset.`,
			);
			return;
		}
		if (!TELEMETRY_QUERY_FILTER_OPS.some((op) => op.value === filter.op)) {
			errors.push(`Filter ${position} uses an unknown operator.`);
			return;
		}
		if (
			!telemetryFilterOpsForKind(field.kind).some(
				(op) => op.value === filter.op,
			)
		) {
			errors.push(
				`Filter ${position}: "${field.label}" does not support that operator.`,
			);
			return;
		}
		if (filter.op === "in") {
			const values = Array.isArray(filter.value) ? filter.value : [];
			if (values.length === 0) {
				errors.push(`Filter ${position} needs at least one value.`);
			}
			return;
		}
		if (field.kind === "number") {
			if (typeof filter.value !== "number" || !Number.isFinite(filter.value)) {
				errors.push(`Filter ${position} needs a number.`);
			}
			return;
		}
		if (field.kind === "bool") return;
		if (typeof filter.value !== "string" || filter.value.trim() === "") {
			errors.push(`Filter ${position} needs a value.`);
		}
	});

	if (request.breakdown) {
		const breakdown = telemetryFieldSpec(request.dataset, request.breakdown);
		if (!breakdown) {
			errors.push("The breakdown field is not allowed on this dataset.");
		} else if (breakdown.kind !== "string") {
			errors.push(
				`"${breakdown.label}" is not a text field — pick one to group by.`,
			);
		}
	}

	const interval = request.interval ?? "none";
	if (!TELEMETRY_QUERY_INTERVALS.some((i) => i.value === interval)) {
		errors.push(`Unknown interval "${interval}".`);
	}

	if (
		!Number.isFinite(request.hours) ||
		request.hours < 1 ||
		request.hours > TELEMETRY_QUERY_MAX_HOURS
	) {
		errors.push(
			`Time range must be between 1 and ${TELEMETRY_QUERY_MAX_HOURS} hours.`,
		);
	}

	return errors;
}

const BUCKETS_PER_HOUR: Record<ITelemetryQueryInterval, number> = {
	minute: 60,
	hour: 1,
	day: 1 / 24,
	none: 1,
};

/**
 * Upper bound on the rows a query can return: one row per time bucket, times the
 * breakdown groups the server keeps. Used to warn before the row cap silently
 * eats part of the range.
 */
export function estimateTelemetryQueryRows(
	request: ITelemetryQueryRequest,
): number {
	const interval = request.interval ?? "none";
	const hours = Number.isFinite(request.hours) ? Math.max(1, request.hours) : 1;
	const buckets =
		interval === "none"
			? 1
			: Math.max(1, Math.ceil(hours * BUCKETS_PER_HOUR[interval]));
	const groups = request.breakdown ? TELEMETRY_QUERY_MAX_BREAKDOWN_ROWS : 1;
	return buckets * groups;
}

/** Warning text when interval × time range × breakdown will overrun the cap. */
export function telemetryQueryCapWarning(
	request: ITelemetryQueryRequest,
): string | null {
	const estimate = estimateTelemetryQueryRows(request);
	if (estimate <= TELEMETRY_QUERY_MAX_ROWS) return null;
	return `This combination can produce about ${estimate.toLocaleString()} rows. The server caps a query at ${TELEMETRY_QUERY_MAX_ROWS.toLocaleString()} rows, so part of the range would be missing — pick a coarser interval or a shorter time range.`;
}

/**
 * The server reports `truncated` exactly — it fetches one row past the cap — so
 * the flag wins whenever it is present. Only a deployment that predates the flag
 * falls back to the heuristic, where a full page of rows is treated as capped.
 */
export function telemetryQueryTruncated(
	response: ITelemetryQueryResponse | undefined,
): boolean {
	if (!response) return false;
	if (typeof response.truncated === "boolean") return response.truncated;
	return response.rows.length >= TELEMETRY_QUERY_MAX_ROWS;
}

export function normalizeTelemetryQuery(
	request: ITelemetryQueryRequest,
): ITelemetryQueryRequest {
	const filters = (request.filters ?? []).filter((filter) =>
		Boolean(telemetryFieldSpec(request.dataset, filter.field)),
	);
	return {
		dataset: request.dataset,
		metric: {
			type: request.metric.type,
			field: telemetryMetricNeedsField(request.metric.type)
				? (request.metric.field ?? null)
				: null,
		},
		filters,
		breakdown:
			telemetryFieldSpec(request.dataset, request.breakdown)?.kind === "string"
				? request.breakdown
				: null,
		interval: request.interval ?? "none",
		hours: request.hours,
	};
}

export function describeTelemetryQuery(
	request: ITelemetryQueryRequest,
): string {
	const spec = telemetryDatasetSpec(request.dataset);
	const metric = TELEMETRY_QUERY_METRIC_TYPES.find(
		(m) => m.value === request.metric.type,
	);
	const metricField = telemetryFieldSpec(request.dataset, request.metric.field);
	const breakdown = telemetryFieldSpec(request.dataset, request.breakdown);
	const parts = [
		`${metric?.label ?? request.metric.type}${metricField ? ` of ${metricField.label}` : ""}`,
		`on ${spec?.label ?? request.dataset}`,
	];
	if (breakdown) parts.push(`by ${breakdown.label}`);
	const hours = TELEMETRY_QUERY_HOUR_OPTIONS.find(
		(h) => h.value === request.hours,
	);
	parts.push(hours ? hours.label.toLowerCase() : `last ${request.hours}h`);
	return parts.join(" · ");
}

export type ITelemetryQueryLayoutKind =
	| "timeseries"
	| "timeseries_breakdown"
	| "breakdown"
	| "scalar";

export interface ITelemetryQueryLayout {
	kind: ITelemetryQueryLayoutKind;
	tsIndex: number;
	breakdownIndex: number;
	valueIndex: number;
}

/**
 * Column contract: the metric value is always the last column, a time bucket is
 * the first column whenever interval != none, and a breakdown key sits between
 * them. Positions are derived rather than matched by name so a server-side
 * rename cannot silently blank the chart.
 */
export function telemetryQueryLayout(
	request: ITelemetryQueryRequest,
	response: ITelemetryQueryResponse | undefined,
): ITelemetryQueryLayout {
	const columnCount = response?.columns.length ?? 0;
	const valueIndex = Math.max(0, columnCount - 1);
	const timed = (request.interval ?? "none") !== "none";
	const tsIndex = timed && columnCount >= 2 ? 0 : -1;
	const hasBreakdown = timed ? columnCount >= 3 : columnCount >= 2;
	const breakdownIndex = hasBreakdown ? (timed ? 1 : 0) : -1;
	if (tsIndex >= 0 && breakdownIndex >= 0)
		return {
			kind: "timeseries_breakdown",
			tsIndex,
			breakdownIndex,
			valueIndex,
		};
	if (tsIndex >= 0)
		return { kind: "timeseries", tsIndex, breakdownIndex: -1, valueIndex };
	if (breakdownIndex >= 0)
		return { kind: "breakdown", tsIndex: -1, breakdownIndex, valueIndex };
	return { kind: "scalar", tsIndex: -1, breakdownIndex: -1, valueIndex };
}

export function formatTelemetryQueryValue(value: ITelemetryQueryValue): string {
	if (value === null || value === undefined) return "—";
	if (typeof value === "number") {
		if (!Number.isFinite(value)) return "—";
		if (Number.isInteger(value)) return value.toLocaleString();
		return value.toLocaleString(undefined, { maximumFractionDigits: 3 });
	}
	if (typeof value === "boolean") return value ? "true" : "false";
	return value;
}

function csvCell(value: ITelemetryQueryValue): string {
	if (value === null || value === undefined) return "";
	const text = String(value);
	if (/[",\n\r]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
	return text;
}

export function telemetryRowsToCsv(
	columns: readonly string[],
	rows: readonly ITelemetryQueryValue[][],
): string {
	const header = columns.map((column) => csvCell(column)).join(",");
	const body = rows.map((row) => row.map((cell) => csvCell(cell)).join(","));
	return [header, ...body].join("\n");
}

/** A capped export is named as such so the file still says so once it leaves the app. */
export function telemetryQueryFileName(
	name: string,
	truncated = false,
): string {
	const slug = name
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "");
	const suffix = truncated ? `-capped-at-${TELEMETRY_QUERY_MAX_ROWS}-rows` : "";
	return `${slug || "telemetry-query"}${suffix}.csv`;
}
