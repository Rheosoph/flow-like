import type { GraphOverlay } from "../../state/backend-state/graph-state";
import type {
	ExecuteSqlPayload,
	QueryColumn,
	SavedQuery,
} from "../../state/backend-state/query-state";

export const HOME_DATA_VISUALIZATIONS = [
	["stat", "Single metric"],
	["metricstrip", "Metric strip"],
	["progress", "Target progress"],
	["gauge", "Target gauge"],
	["bullet", "Bullet chart"],
	["bar", "Column chart"],
	["horizontal", "Ranked bars"],
	["stacked", "Stacked columns"],
	["percentstacked", "100% stacked columns"],
	["line", "Line chart"],
	["area", "Area chart"],
	["donut", "Donut"],
	["pie", "Pie"],
	["scatter", "Scatter plot"],
	["histogram", "Histogram"],
	["heatmap", "Heatmap"],
	["calendar", "Calendar heatmap"],
	["treemap", "Treemap"],
	["boxplot", "Box plot"],
	["funnel", "Funnel"],
	["waterfall", "Waterfall"],
	["sankey", "Sankey"],
	["pivot", "Pivot table"],
	["table", "Table"],
	["list", "Record list"],
	["cards", "Record cards"],
	["kanban", "Board by status"],
	["record", "Record detail"],
	["timeline", "Record timeline"],
	["recordcalendar", "Record calendar"],
	["comparison", "Object comparison"],
	["graph", "Relationship graph"],
] as const;
export type HomeDataVisualization =
	(typeof HOME_DATA_VISUALIZATIONS)[number][0];
export const HOME_DATA_AGGREGATIONS = [
	"count",
	"distinct",
	"sum",
	"avg",
	"min",
	"max",
	"median",
] as const;
export type HomeDataAggregation = (typeof HOME_DATA_AGGREGATIONS)[number];
export interface HomeDataMeasure {
	id?: string;
	aggregation: HomeDataAggregation;
	field: string;
	label: string;
}
export function homeDataMeasureTitle(measure: HomeDataMeasure): string {
	if (measure.label.trim()) return measure.label;
	const name = {
		count: "Count",
		distinct: "Distinct count",
		sum: "Sum",
		avg: "Average",
		min: "Minimum",
		max: "Maximum",
		median: "Median",
	}[measure.aggregation];
	return measure.aggregation !== "count" && measure.field
		? `${name} of ${measure.field}`
		: name;
}
export function updateHomeDataMeasure(
	measure: HomeDataMeasure,
	changes: Partial<HomeDataMeasure>,
): HomeDataMeasure {
	const automatic =
		!measure.label.trim() ||
		measure.label === homeDataMeasureTitle({ ...measure, label: "" });
	return {
		...measure,
		...changes,
		...(automatic && changes.label === undefined ? { label: "" } : {}),
	};
}
export const HOME_DATA_FILTER_OPERATORS = [
	"eq",
	"neq",
	"gt",
	"gte",
	"lt",
	"lte",
	"contains",
	"empty",
	"not_empty",
] as const;
export interface HomeDataFilter {
	id?: string;
	field: string;
	operator: (typeof HOME_DATA_FILTER_OPERATORS)[number];
	value: string;
	valueType: "text" | "number" | "boolean" | "viewer";
}
export interface HomeDataConfig {
	sourceKind: "table" | "ontology" | "query";
	appId: string;
	scope: "project" | "personal";
	table: string;
	ontologyId: string;
	objectType: string;
	queryId: string;
	queryParams: Record<string, unknown>;
	visualization: HomeDataVisualization;
	mode: "aggregate" | "records";
	measures: HomeDataMeasure[];
	groupBy: string;
	seriesBy: string;
	timeBucket: "none" | "day" | "week" | "month" | "quarter" | "year";
	filters: HomeDataFilter[];
	dateField: string;
	dateRange: "all" | "7d" | "30d" | "90d" | "year";
	sortBy: string;
	sortDirection: "asc" | "desc";
	limit: number;
	refreshSeconds: number;
	fields: string[];
	xField: string;
	yField: string;
	binWidth: number;
	format: "number" | "currency" | "percent";
	currency: string;
	decimals: number;
	target: number | null;
	categoryOrder: string[];
	baseline: number;
}
const RECORD_VIEWS = new Set([
	"table",
	"list",
	"cards",
	"kanban",
	"record",
	"graph",
	"scatter",
	"timeline",
	"recordcalendar",
	"comparison",
]);
export const DEFAULT_HOME_DATA_CONFIG: HomeDataConfig = {
	sourceKind: "table",
	appId: "",
	scope: "project",
	table: "",
	ontologyId: "",
	objectType: "",
	queryId: "",
	queryParams: {},
	visualization: "bar",
	mode: "aggregate",
	measures: [
		{ id: "measure-count", aggregation: "count", field: "", label: "Count" },
	],
	groupBy: "",
	seriesBy: "",
	timeBucket: "none",
	filters: [],
	dateField: "",
	dateRange: "all",
	sortBy: "value",
	sortDirection: "desc",
	limit: 20,
	refreshSeconds: 0,
	fields: [],
	xField: "",
	yField: "",
	binWidth: 10,
	format: "number",
	currency: "USD",
	decimals: 2,
	target: null,
	categoryOrder: [],
	baseline: 0,
};
const string = (value: unknown, fallback = "") =>
	typeof value === "string" ? value : fallback;
const object = (value: unknown): Record<string, unknown> =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
const bounded = (value: unknown, fallback: number, min: number, max: number) =>
	typeof value === "number" && Number.isFinite(value)
		? Math.min(max, Math.max(min, Math.floor(value)))
		: fallback;
function option<T extends string>(
	value: unknown,
	options: readonly T[],
	fallback: T,
): T {
	return options.includes(value as T) ? (value as T) : fallback;
}
export function normalizeHomeDataConfig(
	raw: Record<string, unknown>,
): HomeDataConfig {
	const defaults = DEFAULT_HOME_DATA_CONFIG;
	const visualization = option(
		raw.visualization,
		HOME_DATA_VISUALIZATIONS.map(([id]) => id),
		defaults.visualization,
	);
	const measures = Array.isArray(raw.measures)
		? raw.measures.slice(0, 6).map((value, index) => {
				const item = object(value);
				return {
					id: string(item.id, `measure-${index}`),
					aggregation: option(
						item.aggregation,
						HOME_DATA_AGGREGATIONS,
						"count",
					),
					field: string(item.field),
					label: string(item.label),
				};
			})
		: defaults.measures;
	return {
		sourceKind: option(raw.sourceKind, ["table", "ontology", "query"], "table"),
		appId: string(raw.appId),
		scope: option(raw.scope, ["project", "personal"], "project"),
		table: string(raw.table),
		ontologyId: string(raw.ontologyId),
		objectType: string(raw.objectType),
		queryId: string(raw.queryId),
		queryParams: object(raw.queryParams),
		visualization,
		mode:
			RECORD_VIEWS.has(visualization) &&
			(!["table", "list", "cards"].includes(visualization) ||
				raw.mode !== "aggregate")
				? "records"
				: "aggregate",
		measures: measures.length ? measures : defaults.measures,
		groupBy: string(raw.groupBy),
		seriesBy: string(raw.seriesBy),
		timeBucket: option(
			raw.timeBucket,
			["none", "day", "week", "month", "quarter", "year"],
			"none",
		),
		filters: Array.isArray(raw.filters)
			? raw.filters.slice(0, 12).map((value, index) => {
					const filter = object(value);
					return {
						id: string(filter.id, `filter-${index}`),
						field: string(filter.field),
						operator: option(filter.operator, HOME_DATA_FILTER_OPERATORS, "eq"),
						value: string(filter.value),
						valueType: option(
							filter.valueType,
							["text", "number", "boolean", "viewer"],
							"text",
						),
					};
				})
			: [],
		dateField: string(raw.dateField),
		dateRange: option(
			raw.dateRange,
			["all", "7d", "30d", "90d", "year"],
			"all",
		),
		sortBy: string(raw.sortBy, "value"),
		sortDirection: option(raw.sortDirection, ["asc", "desc"], "desc"),
		limit: bounded(raw.limit, 20, 1, 500),
		refreshSeconds:
			raw.refreshSeconds === 0 ? 0 : bounded(raw.refreshSeconds, 0, 30, 3600),
		fields: Array.isArray(raw.fields)
			? raw.fields
					.filter((field): field is string => typeof field === "string")
					.slice(0, 20)
			: [],
		xField: string(raw.xField),
		yField: string(raw.yField),
		binWidth:
			typeof raw.binWidth === "number" &&
			Number.isFinite(raw.binWidth) &&
			raw.binWidth > 0
				? raw.binWidth
				: 10,
		format: option(raw.format, ["number", "currency", "percent"], "number"),
		currency: /^[A-Z]{3}$/.test(string(raw.currency))
			? String(raw.currency)
			: "USD",
		decimals: bounded(raw.decimals, 2, 0, 6),
		target:
			typeof raw.target === "number" && Number.isFinite(raw.target)
				? raw.target
				: null,
		categoryOrder: Array.isArray(raw.categoryOrder)
			? raw.categoryOrder
					.filter((value): value is string => typeof value === "string")
					.slice(0, 100)
			: [],
		baseline:
			typeof raw.baseline === "number" && Number.isFinite(raw.baseline)
				? raw.baseline
				: 0,
	};
}

export function quoteHomeDataIdentifier(identifier: string): string {
	if (!identifier || identifier.includes("\0"))
		throw new Error("Choose a valid field or data source.");
	return `"${identifier.replaceAll('"', '""')}"`;
}

/** Mask literals and comments before inspecting saved SQL, preserving positions. */
function homeSqlCode(sql: string): string {
	const characters = sql.split("");
	const blank = (start: number, end: number) => {
		for (let i = start; i < end; i++)
			if (characters[i] !== "\n") characters[i] = " ";
	};
	let i = 0;
	while (i < sql.length) {
		const start = i;
		if (sql.startsWith("--", i)) {
			while (i < sql.length && sql[i] !== "\n") i++;
			blank(start, i);
		} else if (sql.startsWith("/*", i)) {
			i += 2;
			let depth = 1;
			while (i < sql.length && depth > 0) {
				if (sql.startsWith("/*", i)) {
					depth++;
					i += 2;
				} else if (sql.startsWith("*/", i)) {
					depth--;
					i += 2;
				} else i++;
			}
			blank(start, i);
		} else if (sql[i] === "'" || sql[i] === '"') {
			const quote = sql[i++];
			const escaped =
				quote === "'" &&
				/[Ee]/.test(sql[start - 1] ?? "") &&
				!/[A-Za-z0-9_]/.test(sql[start - 2] ?? "");
			while (i < sql.length) {
				if (escaped && sql[i] === "\\") {
					i += 2;
					continue;
				}
				if (sql[i++] === quote) {
					if (sql[i] !== quote) break;
					i++;
				}
			}
			blank(start, Math.min(i, sql.length));
		} else if (sql[i] === "$") {
			const delimiter = /^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/.exec(
				sql.slice(i),
			)?.[0];
			if (!delimiter) {
				i++;
				continue;
			}
			const end = sql.indexOf(delimiter, i + delimiter.length);
			i = end < 0 ? sql.length : end + delimiter.length;
			blank(start, i);
		} else i++;
	}
	return characters.join("");
}
export function homeSavedQuerySql(sql: string): string {
	const code = homeSqlCode(sql).trimEnd();
	return code.endsWith(";")
		? `${sql.slice(0, code.length - 1)}${sql.slice(code.length)}`
		: sql;
}
export function extractHomeQueryParameters(sql: string): string[] {
	return [
		...new Set(
			[...homeSqlCode(sql).matchAll(/\$([A-Za-z_][A-Za-z0-9_]*|[0-9]+)/g)].map(
				(match) => match[1],
			),
		),
	];
}
export function homeDataColumns(schema: unknown): QueryColumn[] {
	const fields = object(schema).fields;
	if (!Array.isArray(fields)) return [];
	return fields.flatMap((value, position) => {
		const field = object(value);
		if (typeof field.name !== "string") return [];
		const type = field.type ?? field.data_type;
		return [
			{
				name: field.name,
				type_name:
					typeof type === "string" ? type : type ? JSON.stringify(type) : "",
				position,
			},
		];
	});
}
export function homeOntologyColumns(
	overlay: GraphOverlay,
	objectType: string,
): QueryColumn[] {
	const node = overlay.nodes.find((node) => node.label === objectType);
	if (!node) return [];
	const fields = new Map(
		node.property_columns.map((property) => [
			property.name,
			property.data_type,
		]),
	);
	if (!fields.has(node.id_column)) fields.set(node.id_column, "");
	if (node.display_column && !fields.has(node.display_column))
		fields.set(node.display_column, "Utf8");
	return [...fields].map(([name, type_name], position) => ({
		name,
		type_name,
		position,
	}));
}
export function resolveHomeQueryParams(
	config: Pick<HomeDataConfig, "queryParams">,
	viewerId?: string,
): Record<string, unknown> {
	return Object.fromEntries(
		Object.entries(config.queryParams).map(([key, value]) => {
			if (key.startsWith("__home_"))
				throw new Error(
					"Saved query parameter names cannot start with __home_.",
				);
			if (value !== "$viewer.id") return [key, value];
			if (!viewerId)
				throw new Error("Sign in to show data filtered to the current user.");
			return [key, viewerId];
		}),
	);
}

export interface HomeDataSourceContext {
	overlay?: GraphOverlay;
	savedQuery?: SavedQuery;
	columns?: QueryColumn[];
	viewerId?: string;
	now?: Date;
}
export function buildHomeDataQuery(
	config: HomeDataConfig,
	context: HomeDataSourceContext = {},
): ExecuteSqlPayload {
	if (!config.appId)
		throw new Error("Choose an app in this widget's settings.");
	const quote = quoteHomeDataIdentifier;
	let source: string;
	let surface: ExecuteSqlPayload["surface"] = "native";
	let overlayId: string | undefined;
	let identity: string | undefined;
	const params = resolveHomeQueryParams(config, context.viewerId);
	if (config.sourceKind === "query") {
		const saved = context.savedQuery;
		if (!saved || saved.id !== config.queryId)
			throw new Error("Choose an available saved query.");
		for (const parameter of extractHomeQueryParameters(saved.sql)) {
			if (parameter.startsWith("__home_"))
				throw new Error(
					"Saved query parameter names cannot start with __home_.",
				);
			if (!Object.hasOwn(params, parameter))
				throw new Error(
					`Enter a value for the saved query parameter “${parameter}”.`,
				);
		}
		const sql = homeSavedQuerySql(saved.sql);
		source = `(\n${sql}\n) AS ${quote("__home_source")}`;
		surface = saved.surface;
		overlayId = saved.overlay_id;
	} else if (config.sourceKind === "ontology") {
		const overlay = context.overlay;
		if (!overlay || overlay.id !== config.ontologyId)
			throw new Error("The selected ontology is unavailable.");
		const node = overlay.nodes.find((node) => node.label === config.objectType);
		if (!node) throw new Error("Choose an available ontology object type.");
		source = quote(node.table);
		identity = node.id_column;
		surface = "overlay";
		overlayId = overlay.id;
	} else source = quote(config.table);
	const knownColumns = context.columns?.length
		? new Set(context.columns.map((column) => column.name))
		: undefined;
	const field = (name: string) => {
		if (knownColumns && !knownColumns.has(name))
			throw new Error(
				`The field “${name}” is no longer available. Update this widget's settings.`,
			);
		return quote(name);
	};
	const conditions: string[] = [];
	for (const [index, filter] of config.filters.entries()) {
		if (!filter.field) continue;
		const column = field(filter.field);
		if (filter.operator === "empty" || filter.operator === "not_empty") {
			conditions.push(
				`${column} IS ${filter.operator === "not_empty" ? "NOT " : ""}NULL`,
			);
			continue;
		}
		const key = `__home_filter_${index}`;
		let value: unknown = filter.value;
		if (filter.valueType === "viewer") {
			if (!context.viewerId)
				throw new Error("Sign in to show data filtered to the current user.");
			value = context.viewerId;
		} else if (filter.valueType === "number") {
			value = Number(filter.value);
			if (!filter.value.trim() || !Number.isFinite(value))
				throw new Error(`Enter a numeric filter value for ${filter.field}.`);
		} else if (filter.valueType === "boolean") {
			if (filter.value !== "true" && filter.value !== "false")
				throw new Error(`Choose true or false for ${filter.field}.`);
			value = filter.value === "true";
		}
		params[key] = value;
		if (filter.operator === "contains") {
			conditions.push(
				`strpos(CAST(${column} AS VARCHAR), CAST($${key} AS VARCHAR)) > 0`,
			);
		} else {
			const operators = {
				eq: "=",
				neq: "<>",
				gt: ">",
				gte: ">=",
				lt: "<",
				lte: "<=",
			} as const;
			conditions.push(`${column} ${operators[filter.operator]} $${key}`);
		}
	}
	if (config.dateRange !== "all") {
		if (!config.dateField)
			throw new Error("Choose the date field for this time range.");
		const now = context.now ?? new Date();
		const start = new Date(now);
		if (config.dateRange === "year") {
			start.setUTCFullYear(start.getUTCFullYear(), 0, 1);
			start.setUTCHours(0, 0, 0, 0);
		} else
			start.setTime(
				start.getTime() - Number(config.dateRange.slice(0, -1)) * 86_400_000,
			);
		params.__home_start = start.toISOString();
		params.__home_end = now.toISOString();
		conditions.push(
			`CAST(${field(config.dateField)} AS TIMESTAMP) >= CAST($__home_start AS TIMESTAMP)`,
		);
		conditions.push(
			`CAST(${field(config.dateField)} AS TIMESTAMP) <= CAST($__home_end AS TIMESTAMP)`,
		);
	}
	const where = conditions.length ? ` WHERE ${conditions.join(" AND ")}` : "";
	const limit = Math.min(500, Math.max(1, Math.floor(config.limit)));
	let sql: string;
	if (config.mode === "records" && config.visualization !== "histogram") {
		const columns = config.fields.length
			? [
					...new Set(
						[
							...config.fields,
							config.groupBy,
							config.xField,
							config.yField,
						].filter(Boolean),
					),
				]
					.map(field)
					.join(", ")
			: "*";
		const sort =
			config.sortBy && config.sortBy !== "value" && config.sortBy !== "group"
				? ` ORDER BY ${field(config.sortBy)} ${config.sortDirection.toUpperCase()} NULLS LAST`
				: "";
		sql = `SELECT ${columns} FROM ${source}${where}${sort}`;
	} else {
		const groups: string[] = [];
		const select: string[] = [];
		let group = config.groupBy ? field(config.groupBy) : "";
		if (config.visualization === "histogram") {
			if (!config.groupBy)
				throw new Error("Choose a numeric field to create histogram bins.");
			params.__home_bin_width = config.binWidth;
			group = `FLOOR(CAST(${group} AS DOUBLE) / $__home_bin_width) * $__home_bin_width`;
		} else if (group && config.timeBucket !== "none")
			group = `DATE_TRUNC('${config.timeBucket}', CAST(${group} AS TIMESTAMP))`;
		if (group) {
			groups.push(group);
			select.push(`${group} AS ${quote("__group")}`);
		}
		if (config.seriesBy) {
			const series = field(config.seriesBy);
			groups.push(series);
			select.push(`${series} AS ${quote("__series")}`);
		}
		if (config.visualization === "boxplot") {
			if (!config.yField)
				throw new Error(
					"Choose a numeric distribution field in widget settings.",
				);
			const value = field(config.yField);
			select.push(
				`MIN(${value}) AS "__min"`,
				`APPROX_PERCENTILE_CONT(CAST(${value} AS DOUBLE), 0.25) AS "__q1"`,
				`MEDIAN(${value}) AS "__measure_0"`,
				`APPROX_PERCENTILE_CONT(CAST(${value} AS DOUBLE), 0.75) AS "__q3"`,
				`MAX(${value}) AS "__max"`,
			);
		} else
			config.measures.forEach((measure, index) => {
				let expression: string;
				if (measure.aggregation === "count")
					expression = identity
						? `COUNT(DISTINCT ${field(identity)})`
						: "COUNT(*)";
				else if (measure.aggregation === "distinct")
					expression = `COUNT(DISTINCT ${field(measure.field)})`;
				else
					expression = `${measure.aggregation.toUpperCase()}(${field(measure.field)})`;
				select.push(`${expression} AS ${quote(`__measure_${index}`)}`);
			});
		const sort = config.sortBy === "group" && group ? "__group" : "__measure_0";
		sql = `SELECT ${select.join(", ")} FROM ${source}${where}${groups.length ? ` GROUP BY ${groups.join(", ")}` : ""} ORDER BY ${quote(sort)} ${config.sortDirection.toUpperCase()} NULLS LAST`;
		if (config.visualization === "percentstacked") {
			if (!config.groupBy || !config.seriesBy)
				throw new Error(
					"Choose a group and a series column for a percentage breakdown.",
				);
			sql = `SELECT *, CAST("__measure_0" AS DOUBLE) / NULLIF(SUM("__measure_0") OVER (PARTITION BY "__group"), 0) AS "__share" FROM (\n${sql}\n) AS "__home_shares" ORDER BY ${quote(sort)} ${config.sortDirection.toUpperCase()} NULLS LAST`;
		}
	}
	return { sql, params, surface, overlay_id: overlayId, limit };
}

export function formatHomeDataValue(
	value: unknown,
	config: Pick<HomeDataConfig, "format" | "currency" | "decimals">,
): string {
	if (value === null || value === undefined) return "No value";
	if (typeof value === "object") return JSON.stringify(value);
	const numeric =
		typeof value === "number"
			? value
			: typeof value === "string" && value.trim()
				? Number(value)
				: Number.NaN;
	if (!Number.isFinite(numeric)) return String(value);
	return new Intl.NumberFormat(undefined, {
		style: config.format === "number" ? "decimal" : config.format,
		...(config.format === "currency" ? { currency: config.currency } : {}),
		maximumFractionDigits: config.decimals,
	}).format(numeric);
}
