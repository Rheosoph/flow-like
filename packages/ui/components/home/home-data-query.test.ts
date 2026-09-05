import { describe, expect, test } from "bun:test";
import type { GraphOverlay } from "../../state/backend-state/graph-state";
import type { SavedQuery } from "../../state/backend-state/query-state";
import {
	homeDataChartSeries,
	homeDataNetwork,
	homeDataNumber,
} from "./home-data-presentation";
import {
	buildHomeDataQuery,
	extractHomeQueryParameters,
	homeDataColumns,
	homeDataMeasureTitle,
	homeSavedQuerySql,
	normalizeHomeDataConfig,
	quoteHomeDataIdentifier,
	resolveHomeQueryParams,
	updateHomeDataMeasure,
} from "./home-data-query";

const base = (overrides: Record<string, unknown> = {}) =>
	normalizeHomeDataConfig({ appId: "app", table: "invoices", ...overrides });
const overlay: GraphOverlay = {
	id: "sales",
	name: "Sales",
	nodes: [
		{
			label: "Invoice",
			table: "invoice rows",
			id_column: "invoice_id",
			property_columns: [{ name: "amount", data_type: "Float64" }],
			style: { color: "", icon: "", size: { mode: "fixed" } },
		},
	],
	edges: [],
	object_views: [],
	actions: [],
	exposed: false,
	bindings_enabled: true,
	default_limit: 100,
	created_at: "",
	updated_at: "",
};
const savedQuery: SavedQuery = {
	id: "q",
	app_id: "app",
	name: "Mine",
	kind: "query",
	surface: "native",
	sql: "SELECT * FROM invoices WHERE owner = $owner;",
	created_at: "",
	updated_at: "",
};

describe("home data query boundaries", () => {
	test("aggregates on the source before limiting the result", () => {
		const query = buildHomeDataQuery(
			base({
				groupBy: "status",
				measures: [{ aggregation: "sum", field: "amount", label: "Amount" }],
				limit: 7,
			}),
		);
		expect(query.sql).toContain(
			'SUM("amount") AS "__measure_0" FROM "invoices"',
		);
		expect(query.sql).toContain('GROUP BY "status"');
		expect(query.sql).toEndWith('ORDER BY "__measure_0" DESC NULLS LAST');
		expect(query.sql).not.toContain("LIMIT");
		expect(query.limit).toBe(7);
	});
	test("quotes table and column names as complete identifiers", () => {
		expect(quoteHomeDataIdentifier('name"; DROP TABLE invoices; --')).toBe(
			'"name""; DROP TABLE invoices; --"',
		);
		const query = buildHomeDataQuery(
			base({ table: "schema.table", groupBy: 'x" OR true --' }),
		);
		expect(query.sql).toContain('FROM "schema.table"');
		expect(query.sql).toContain('"x"" OR true --"');
		expect(() => quoteHomeDataIdentifier("\0")).toThrow();
	});
	test("binds filters separately, including literal wildcard characters", () => {
		const payload = "x' OR true -- %_";
		const query = buildHomeDataQuery(
			base({
				filters: [
					{
						field: "name",
						operator: "contains",
						value: payload,
						valueType: "text",
					},
				],
			}),
		);
		expect(query.sql).not.toContain(payload);
		expect(query.sql).toContain(
			'strpos(CAST("name" AS VARCHAR), CAST($__home_filter_0 AS VARCHAR)) > 0',
		);
		expect(query.params?.__home_filter_0).toBe(payload);
	});
	test("resolves current-user filters from each authenticated viewer", () => {
		const config = base({
			filters: [{ field: "owner", operator: "eq", valueType: "viewer" }],
		});
		expect(
			buildHomeDataQuery(config, { viewerId: "alice" }).params?.__home_filter_0,
		).toBe("alice");
		expect(
			buildHomeDataQuery(config, { viewerId: "bob" }).params?.__home_filter_0,
		).toBe("bob");
		expect(() => buildHomeDataQuery(config)).toThrow("Sign in");
	});
	test("retains personal scope and clamps refresh, rows and measures", () => {
		const config = base({
			scope: "personal",
			limit: 999_999,
			refreshSeconds: 1,
			measures: Array.from({ length: 50 }, () => ({ aggregation: "count" })),
		});
		expect(config.scope).toBe("personal");
		expect(config.limit).toBe(500);
		expect(config.refreshSeconds).toBe(30);
		expect(config.measures).toHaveLength(6);
		expect(
			base({ limit: Number.NaN, refreshSeconds: Number.POSITIVE_INFINITY })
				.limit,
		).toBe(20);
	});
	test("uses distinct object identity for ontology counts without joining edges", () => {
		const query = buildHomeDataQuery(
			base({
				sourceKind: "ontology",
				ontologyId: "sales",
				objectType: "Invoice",
			}),
			{ overlay },
		);
		expect(query.surface).toBe("overlay");
		expect(query.overlay_id).toBe("sales");
		expect(query.sql).toContain('COUNT(DISTINCT "invoice_id")');
		expect(query.sql).toContain('FROM "invoice rows"');
		expect(query.sql).not.toContain("JOIN");
		expect(() =>
			buildHomeDataQuery(
				base({
					sourceKind: "ontology",
					ontologyId: "other",
					objectType: "Invoice",
				}),
				{ overlay },
			),
		).toThrow("unavailable");
	});
	test("fails clearly when source schema no longer includes a selected field", () => {
		expect(() =>
			buildHomeDataQuery(base({ groupBy: "deleted" }), {
				columns: [{ name: "id", type_name: "Int64", position: 0 }],
			}),
		).toThrow("no longer available");
	});
	test("wraps saved queries with bound parameters before aggregating", () => {
		const config = base({
			sourceKind: "query",
			queryId: "q",
			queryParams: { owner: "$viewer.id" },
		});
		const query = buildHomeDataQuery(config, { savedQuery, viewerId: "alice" });
		expect(query.sql).toContain(
			'FROM (\nSELECT * FROM invoices WHERE owner = $owner\n) AS "__home_source"',
		);
		expect(query.params?.owner).toBe("alice");
		expect(() =>
			resolveHomeQueryParams(
				base({ queryParams: { __home_filter_0: "override" } }),
			),
		).toThrow("cannot start");
	});
	test("finds real saved-query parameters without inspecting literals or comments", () => {
		expect(
			extractHomeQueryParameters(
				`SELECT '$literal', "$identifier", $$ $body $$, $tag$ $body2 $tag$, $1, $owner /* $comment /* $nested */ */ -- $line\n WHERE id = $owner`,
			),
		).toEqual(["1", "owner"]);
		expect(homeSavedQuerySql("SELECT ';' AS value; -- final comment")).toBe(
			"SELECT ';' AS value -- final comment",
		);
	});
	test("rejects missing or reserved saved-query parameters before adding widget bindings", () => {
		expect(() =>
			buildHomeDataQuery(base({ sourceKind: "query", queryId: "q" }), {
				savedQuery,
			}),
		).toThrow("Enter a value");
		expect(() =>
			buildHomeDataQuery(base({ sourceKind: "query", queryId: "q" }), {
				savedQuery: { ...savedQuery, sql: "SELECT $__home_filter_0" },
			}),
		).toThrow("cannot start");
	});
	test("bounds date windows explicitly and groups dates at the source", () => {
		const query = buildHomeDataQuery(
			base({
				groupBy: "created_at",
				timeBucket: "month",
				dateRange: "7d",
				dateField: "created_at",
				sortBy: "group",
				sortDirection: "asc",
			}),
			{ now: new Date("2026-09-05T12:00:00Z") },
		);
		expect(query.sql).toContain(
			`DATE_TRUNC('month', CAST("created_at" AS TIMESTAMP))`,
		);
		expect(query.params?.__home_start).toBe("2026-08-29T12:00:00.000Z");
		expect(query.params?.__home_end).toBe("2026-09-05T12:00:00.000Z");
		expect(query.sql).toContain('ORDER BY "__group" ASC');
	});
	test("histogram bins are calculated before aggregation with bound width", () => {
		const query = buildHomeDataQuery(
			base({ visualization: "histogram", groupBy: "amount", binWidth: 25 }),
		);
		expect(query.sql).toContain(
			'FLOOR(CAST("amount" AS DOUBLE) / $__home_bin_width) * $__home_bin_width',
		);
		expect(query.sql).toContain("COUNT(*)");
		expect(query.params?.__home_bin_width).toBe(25);
	});
	test("percentage charts compute denominators across complete groups before result limits", () => {
		const query = buildHomeDataQuery(
			base({
				visualization: "percentstacked",
				groupBy: "department",
				seriesBy: "status",
				limit: 2,
			}),
		);
		expect(query.sql).toContain(
			'SUM("__measure_0") OVER (PARTITION BY "__group")',
		);
		expect(query.sql).toContain('AS "__share"');
		expect(query.sql).not.toContain("LIMIT");
		expect(query.limit).toBe(2);
		expect(() =>
			buildHomeDataQuery(base({ visualization: "percentstacked" })),
		).toThrow("group and a series");
	});
	test("box plots request source statistics instead of deriving quartiles from preview rows", () => {
		const query = buildHomeDataQuery(
			base({
				visualization: "boxplot",
				groupBy: "department",
				yField: "salary",
				limit: 10,
			}),
		);
		expect(query.sql).toContain(
			'APPROX_PERCENTILE_CONT(CAST("salary" AS DOUBLE), 0.25) AS "__q1"',
		);
		expect(query.sql).toContain('MEDIAN("salary") AS "__measure_0"');
		expect(query.sql).toContain('MIN("salary") AS "__min"');
		expect(query.sql).toContain('MAX("salary") AS "__max"');
		expect(query.sql).not.toContain("LIMIT");
	});
	test("record presentations consistently request records even when a preset has aggregate mode", () => {
		for (const visualization of [
			"timeline",
			"recordcalendar",
			"comparison",
			"kanban",
			"graph",
		]) {
			const config = base({ visualization, mode: "aggregate" });
			expect(config.mode).toBe("records");
			expect(buildHomeDataQuery(config).sql).not.toContain("COUNT");
		}
	});
	test("record queries select configured fields plus fields needed by the visualization", () => {
		const query = buildHomeDataQuery(
			base({
				visualization: "kanban",
				fields: ["name"],
				groupBy: "status",
				sortBy: "created_at",
			}),
		);
		expect(query.sql).toStartWith('SELECT "name", "status" FROM "invoices"');
		expect(query.sql).not.toContain("COUNT");
		expect(query.sql).toContain('ORDER BY "created_at"');
	});
	test("rejects invalid typed filter values", () => {
		expect(() =>
			buildHomeDataQuery(
				base({
					filters: [
						{
							field: "amount",
							operator: "gt",
							value: "invalid",
							valueType: "number",
						},
					],
				}),
			),
		).toThrow("numeric");
		expect(() =>
			buildHomeDataQuery(
				base({
					filters: [
						{
							field: "active",
							operator: "eq",
							value: "yes",
							valueType: "boolean",
						},
					],
				}),
			),
		).toThrow("true or false");
	});
	test("reads both Arrow schema type shapes", () => {
		expect(
			homeDataColumns({
				fields: [
					{ name: "value", data_type: "Float64" },
					{ name: "time", type: { Timestamp: ["Millisecond", null] } },
				],
			}).map((field) => field.type_name),
		).toEqual(["Float64", '{"Timestamp":["Millisecond",null]}']);
	});
});

describe("home data presentation preserves meaning", () => {
	test("blank or structured data never becomes an invented numeric zero", () => {
		for (const value of [
			" ",
			"\n\t",
			[],
			[1],
			{},
			true,
			false,
			null,
			undefined,
		])
			expect(homeDataNumber(value)).toBeNull();
		expect(homeDataNumber("0")).toBe(0);
		expect(homeDataNumber(" -12.5 ")).toBe(-12.5);
	});
	test("measure edits update generated labels and preserve authored labels", () => {
		const count = { aggregation: "count" as const, field: "", label: "Count" };
		const sum = updateHomeDataMeasure(count, {
			aggregation: "sum",
			field: "amount",
		});
		expect(homeDataMeasureTitle(sum)).toBe("Sum of amount");
		const custom = updateHomeDataMeasure(
			{ ...sum, label: "Revenue" },
			{ field: "net_amount" },
		);
		expect(homeDataMeasureTitle(custom)).toBe("Revenue");
		expect(
			homeDataMeasureTitle(updateHomeDataMeasure(custom, { label: "" })),
		).toBe("Sum of net_amount");
	});
	test("a truncated percentage chart retains the source denominator", () => {
		const result = homeDataChartSeries(
			[{ __group: "A", __series: "First", __measure_0: 20, __share: 0.2 }],
			base({
				visualization: "percentstacked",
				groupBy: "group",
				seriesBy: "status",
			}),
		);
		expect(result.points).toEqual([{ name: "A", value_0: 0.2 }]);
	});
	test("pivots series without summing a limited result or inventing missing values", () => {
		const result = homeDataChartSeries(
			[
				{ __group: "Mon", __series: "Succeeded", __measure_0: 8 },
				{ __group: "Mon", __series: "Failed", __measure_0: 2 },
				{ __group: "Tue", __series: "Succeeded", __measure_0: 11 },
			],
			base({ groupBy: "day", seriesBy: "status" }),
		);
		expect(result.points).toEqual([
			{ name: "Mon", value_0: 8, value_1: 2 },
			{ name: "Tue", value_0: 11 },
		]);
		expect(result.series).toHaveLength(2);
		expect(homeDataNumber(null)).toBeNull();
		expect(homeDataNumber(0)).toBe(0);
	});
	test("deduplicates relationship pairs while preserving direction", () => {
		const graph = homeDataNetwork(
			[
				{ from: "a", to: "b" },
				{ from: "a", to: "b" },
				{ from: "b", to: "a" },
				{ from: null, to: "c" },
			],
			"from",
			"to",
		);
		expect(graph.nodes).toEqual([{ id: "a" }, { id: "b" }]);
		expect(graph.links).toEqual([
			{ source: "a", target: "b" },
			{ source: "b", target: "a" },
		]);
	});
});
