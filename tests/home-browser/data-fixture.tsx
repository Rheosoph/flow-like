import { useEffect, useRef, useState } from "react";
import { HomeDataWidget } from "../../packages/ui/components/home/data-widget";
import { HomeDataWidgetSettings } from "../../packages/ui/components/home/data-widget-settings";
import { HomeEditor } from "../../packages/ui/components/home/home-editor";
import type {
	IHomeLayout,
	IHomeWidget,
} from "../../packages/ui/components/home/types";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import type {
	ExecuteSqlPayload,
	QueryColumn,
} from "../../packages/ui/state/backend-state/query-state";

const DAY = 86_400_000;
const ownerSubs = [
	"3f9c2a71-8d4e-4b6a-9c1f-5e2d7a8b0c14",
	"b2e8d4f6-1a3c-4e5b-8d7f-9a0b1c2d3e4f",
	"e7a1c3d5-2b4f-4a6c-8e0d-1f3a5b7c9d2e",
];
/** The last sub stays unknown to the directory, so its id is what a cell shows. */
const people = new Map([
	[ownerSubs[0], "Mara Lindqvist"],
	[ownerSubs[1], "Jonas Weber"],
]);
const firstSeenDays = [0, 0, 1, 3, 3, 4, 6, 6, 8, 9];
const sourceRows = [
	["Sales", "Succeeded", 120, "2026-09-01", 12],
	["Sales", "Failed", 30, "2026-09-01", 36],
	["Sales", "Succeeded", 80, "2026-09-02", 18],
	["Support", "Succeeded", 70, "2026-09-02", 22],
	["Support", "Failed", 20, "2026-09-03", 48],
	["Support", "Succeeded", 90, "2026-09-03", 15],
	["Finance", "Succeeded", 180, "2026-09-04", 27],
	["Finance", "Failed", 40, "2026-09-04", 60],
	["Finance", "Succeeded", 100, "2026-09-05", 20],
	["Sales", "Succeeded", 110, "2026-09-05", 16],
].map(([department, status, amount, date, latency], index) => {
	const lastSuccess = 1789584356974 - index * 61_234_567;
	return {
		order_id: `order-${index}`,
		name: `Invoice ${index + 1}`,
		department,
		status,
		amount,
		date,
		latency,
		owner: index < 8 ? "fixture-user" : "other-user",
		owner_sub: ownerSubs[index % ownerSubs.length],
		first_seen_at:
			Date.UTC(2026, 7, 28) +
			firstSeenDays[index] * DAY +
			(7 + index) * 3_600_000,
		started_at: lastSuccess - 5_667 - index * 1_250,
		last_success_at: lastSuccess,
		net_amount: Number(amount) * 0.81,
		relevance_score: Number((0.94 - index * 0.061).toFixed(3)),
	};
});
const columnTypes: Record<string, string> = {
	amount: "Float64",
	latency: "Float64",
	net_amount: "Float64",
	relevance_score: "Float64",
	started_at: "Int64",
	last_success_at: "Int64",
	first_seen_at: 'Timestamp(ms, "UTC")',
};
const columns: QueryColumn[] = Object.keys(sourceRows[0]).map(
	(name, position) => ({
		name,
		position,
		type_name: columnTypes[name] ?? "Utf8",
	}),
);
const schema = {
	fields: columns.map((column) => ({
		name: column.name,
		data_type: column.type_name,
	})),
};
const ontology = {
	id: "qa-ontology",
	name: "Order operations",
	nodes: [
		{
			label: "Order",
			table: "orders",
			id_column: "order_id",
			display_column: "name",
			property_columns: columns.map((column) => ({
				name: column.name,
				data_type: column.type_name,
			})),
			style: { color: "#6284ff", icon: "", size: { mode: "fixed" } },
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
const saved = {
	id: "qa-query",
	app_id: "qa-data",
	name: "Orders owned by a user",
	kind: "query",
	surface: "native",
	sql: "SELECT * FROM orders WHERE owner = $owner; -- fixture query",
	param_schema: {
		properties: { owner: { type: "string", title: "Owner" } },
		required: ["owner"],
	},
	created_at: "",
	updated_at: "",
};
function widget(
	id: string,
	visualization: string,
	overrides: Record<string, unknown> = {},
): IHomeWidget {
	return {
		id,
		type: "data",
		title: id,
		size: { columns: 6, rows: 4 },
		appearance: { variant: "card", accent: "blue" },
		config: {
			appId: "qa-data",
			sourceKind: "table",
			table: "orders",
			visualization,
			groupBy: "department",
			measures: [{ aggregation: "sum", field: "amount", label: "Amount" }],
			limit: 50,
			...overrides,
		},
	};
}
const samples = [
	widget("stat", "stat", { groupBy: "", target: 1000 }),
	widget("bar", "bar"),
	widget("stacked", "stacked", { seriesBy: "status" }),
	widget("donut", "donut"),
	widget("pivot", "pivot", { seriesBy: "status" }),
	widget("calendar", "calendar", {
		groupBy: "date",
		timeBucket: "day",
		sortBy: "group",
		sortDirection: "asc",
	}),
	widget("sankey", "sankey", { seriesBy: "status" }),
	widget("boxplot", "boxplot", { yField: "latency" }),
	widget("percentstacked", "percentstacked", { seriesBy: "status", limit: 2 }),
	widget("records", "recordcalendar", {
		xField: "date",
		fields: ["name", "date", "amount"],
	}),
	widget("runs", "list", {
		mode: "records",
		fields: ["name", "last_success_at", "owner_sub", "relevance_score"],
	}),
	widget("run", "record", {
		mode: "records",
		fields: [
			"name",
			"started_at",
			"last_success_at",
			"owner_sub",
			"net_amount",
		],
	}),
	widget("activity", "area", {
		groupBy: "first_seen_at",
		timeBucket: "day",
		sortBy: "group",
		sortDirection: "asc",
		measures: [{ aggregation: "sum", field: "net_amount", label: "" }],
	}),
	widget("successes", "line", {
		groupBy: "last_success_at",
		timeBucket: "day",
		sortBy: "group",
		sortDirection: "asc",
		measures: [{ aggregation: "count", field: "", label: "" }],
	}),
];

const allSamples = [
	...samples,
	widget("metricstrip", "metricstrip", {
		groupBy: "",
		measures: [
			{ aggregation: "sum", field: "amount", label: "Revenue" },
			{ aggregation: "count", field: "", label: "Orders" },
			{ aggregation: "avg", field: "latency", label: "Latency" },
			{ aggregation: "max", field: "last_success_at", label: "" },
		],
	}),
	...(["progress", "gauge", "bullet"] as const).map((view) =>
		widget(view, view, { groupBy: "", target: 1000 }),
	),
	widget("horizontal", "horizontal"),
	widget("line", "line", {
		groupBy: "date",
		sortBy: "group",
		sortDirection: "asc",
	}),
	widget("area", "area", {
		groupBy: "date",
		sortBy: "group",
		sortDirection: "asc",
	}),
	widget("pie", "pie"),
	widget("scatter", "scatter", { xField: "amount", yField: "latency" }),
	widget("histogram", "histogram", {
		groupBy: "latency",
		binWidth: 10,
		measures: [{ aggregation: "count", field: "", label: "Orders" }],
	}),
	widget("heatmap", "heatmap", { seriesBy: "status" }),
	widget("treemap", "treemap"),
	widget("funnel", "funnel"),
	widget("waterfall", "waterfall"),
	...(
		["table", "list", "cards", "kanban", "record", "comparison"] as const
	).map((view) =>
		widget(view, view, {
			mode: "records",
			groupBy: "status",
			fields: ["name", "amount", "department", "status"],
		}),
	),
	widget("timeline", "timeline", {
		xField: "date",
		fields: ["name", "amount", "date"],
	}),
	widget("graph", "graph", {
		xField: "department",
		yField: "status",
		fields: ["department", "status"],
	}),
];

function aggregateFixture(payload: ExecuteSqlPayload) {
	let rows = sourceRows as Record<string, unknown>[];
	const owner = payload.params?.owner ?? payload.params?.__home_filter_0;
	if (owner !== undefined && payload.sql.includes('"owner"'))
		rows = rows.filter((row) => row.owner === owner);
	if (payload.params?.owner !== undefined)
		rows = rows.filter((row) => row.owner === payload.params?.owner);
	if (!payload.sql.includes('AS "__measure_0"')) {
		const selection = /^SELECT\s+(.+?)\s+FROM/is.exec(payload.sql)?.[1];
		const fields =
			selection && selection !== "*"
				? [...selection.matchAll(/"([^"]+)"/g)].map((match) => match[1])
				: columns.map((column) => column.name);
		return {
			rows: rows.map((row) =>
				Object.fromEntries(fields.map((field) => [field, row[field]])),
			),
			columns: fields.flatMap((field) =>
				columns.filter((column) => column.name === field),
			),
		};
	}
	const histogram = /FLOOR/.test(payload.sql);
	const binWidth = Number(payload.params?.__home_bin_width ?? 10);
	const truncation = /DATE_TRUNC\('(\w+)',\s*(.+?)\) AS "__group"/s.exec(
		payload.sql,
	);
	const group = truncation
		? (/"([^"]+)"/.exec(truncation[2])?.[1] ?? "")
		: (/"([^"]+)" AS "__group"/.exec(payload.sql)?.[1] ??
			(histogram ? "latency" : ""));
	// DataFusion truncates a cast column to nanoseconds and the epoch CASE to microseconds.
	const [groupType, groupScale] = !truncation
		? [histogram ? "Float64" : (columnTypes[group] ?? "Utf8"), 1]
		: /to_timestamp_micros/.test(truncation[2])
			? ["Timestamp(µs)", 1_000]
			: ["Timestamp(ns)", 1_000_000];
	const groupValue = (row: Record<string, unknown>) => {
		if (!group) return null;
		if (histogram) return Math.floor(Number(row[group]) / binWidth) * binWidth;
		if (!truncation) return row[group];
		const bucket = truncateFixtureInstant(row[group], truncation[1]);
		return bucket === null ? null : bucket * groupScale;
	};
	const series = /"([^"]+)" AS "__series"/.exec(payload.sql)?.[1] ?? "";
	const grouped = new Map<string, Record<string, unknown>[]>();
	for (const row of rows) {
		const key = JSON.stringify([groupValue(row), series ? row[series] : null]);
		const list = grouped.get(key) ?? [];
		list.push(row);
		grouped.set(key, list);
	}
	const matches = [
		...payload.sql.matchAll(
			/(SUM|COUNT|AVG|MIN|MAX|MEDIAN)\((?:DISTINCT )?(?:"([^"]+)"|\*)\) AS "__measure_([0-9]+)"/g,
		),
	];
	const measureTypes = new Map(
		matches.map(([, aggregation, field, index]) => [
			`__measure_${index}`,
			aggregation === "COUNT"
				? "Int64"
				: ["MIN", "MAX"].includes(aggregation) && field
					? (columnTypes[field] ?? "Utf8")
					: "Float64",
		]),
	);
	const result = [...grouped.values()].map((items) => {
		const row: Record<string, unknown> = {};
		if (group) row.__group = groupValue(items[0]);
		if (series) row.__series = items[0][series];
		for (const [, aggregation, field, index] of matches) {
			const values = items
				.map((item) => Number(item[field]))
				.sort((a, b) => a - b);
			row[`__measure_${index}`] =
				aggregation === "COUNT"
					? items.length
					: aggregation === "AVG"
						? values.reduce((sum, value) => sum + value, 0) / values.length
						: aggregation === "MIN"
							? values[0]
							: aggregation === "MAX"
								? values[values.length - 1]
								: aggregation === "MEDIAN"
									? values[Math.floor(values.length / 2)]
									: values.reduce((sum, value) => sum + value, 0);
		}
		if (payload.sql.includes('AS "__q1"')) {
			const values = items
				.map((item) => Number(item.latency))
				.sort((a, b) => a - b);
			Object.assign(row, {
				__min: values[0],
				__q1: values[Math.floor((values.length - 1) * 0.25)],
				__q3: values[Math.ceil((values.length - 1) * 0.75)],
				__max: values[values.length - 1],
			});
		}
		return row;
	});
	if (payload.sql.includes('AS "__share"'))
		for (const row of result)
			row.__share =
				Number(row.__measure_0) /
				result
					.filter((other) => other.__group === row.__group)
					.reduce((sum, item) => sum + Number(item.__measure_0), 0);
	result.sort((a, b) =>
		payload.sql.includes('ORDER BY "__group" ASC')
			? typeof a.__group === "number" && typeof b.__group === "number"
				? a.__group - b.__group
				: String(a.__group).localeCompare(String(b.__group))
			: Number(b.__measure_0) - Number(a.__measure_0),
	);
	return {
		rows: result,
		columns: Object.keys(result[0] ?? {}).map((name, position) => ({
			name,
			position,
			type_name:
				name === "__group"
					? groupType
					: name === "__series"
						? (columnTypes[series] ?? "Utf8")
						: (measureTypes.get(name) ?? "Float64"),
		})),
	};
}

/** Epoch milliseconds of the UTC bucket start, as DATE_TRUNC would place it. */
function truncateFixtureInstant(value: unknown, bucket: string): number | null {
	const time = typeof value === "number" ? value : Date.parse(String(value));
	if (!Number.isFinite(time)) return null;
	const date = new Date(time);
	const year = date.getUTCFullYear();
	const month = date.getUTCMonth();
	if (bucket === "year") return Date.UTC(year, 0, 1);
	if (bucket === "quarter") return Date.UTC(year, month - (month % 3), 1);
	if (bucket === "month") return Date.UTC(year, month, 1);
	const day = Date.UTC(year, month, date.getUTCDate());
	return bucket === "week" ? day - ((date.getUTCDay() + 6) % 7) * DAY : day;
}

export default function DataFixture() {
	const backend = useBackend();
	const initial = useRef(backend);
	const [ready, setReady] = useState(false);
	const [scenario, setScenario] = useState("populated");
	const scenarioRef = useRef(scenario);
	scenarioRef.current = scenario;
	const [lastQuery, setLastQuery] = useState("");
	const expanded = new URLSearchParams(window.location.search).has("all");
	const editor = new URLSearchParams(window.location.search).has("editor");
	const [layout, setLayout] = useState<IHomeLayout>({
		version: 1,
		title: "Data overview",
		widgets: allSamples,
	});
	const visibleLayout =
		scenario === "unconfigured"
			? {
					...layout,
					widgets: layout.widgets.map((item) => ({
						...item,
						config: { ...item.config, appId: "" },
					})),
				}
			: layout;
	const [editable, setEditable] = useState(widget("builder", "bar"));
	useEffect(() => {
		const original = initial.current;
		const app = { id: "qa-data", name: "Fixture data" };
		useBackendStore.getState().setBackend({
			...original,
			profile: {
				id: "qa-data-profile",
				hub: "fixture.invalid",
				secure: true,
				name: "Data QA",
			},
			appState: {
				...original.appState,
				getApps: async () => [[app, { name: "Fixture data" }]],
			},
			dbState: {
				...original.dbState,
				listTables: async () => ["orders"],
				listTablesUser: async () => ["orders"],
				getSchema: async () => schema,
			},
			userState: {
				...original.userState,
				lookupUser: async (id: string) => {
					const name = people.get(id);
					if (!name) throw new Error(`No fixture account has the id ${id}.`);
					return { id, name, created_at: "2026-01-01T00:00:00Z" };
				},
				lookupUsers: async (ids: string[]) =>
					ids.flatMap((id) => {
						const name = people.get(id);
						return name
							? [{ id, name, created_at: "2026-01-01T00:00:00Z" }]
							: [];
					}),
			},
			graphState: {
				...original.graphState,
				listOverlays: async () => [ontology],
				getOverlay: async () => ontology,
			},
			queryState: {
				...original.queryState,
				listSavedQueries: async () => [saved],
				getSavedQuery: async () => saved,
				executeSql: async (
					_appId: string,
					payload: ExecuteSqlPayload,
					personal: boolean,
				) => {
					setLastQuery(JSON.stringify({ ...payload, personal }, null, 2));
					await new Promise((resolve) => setTimeout(resolve, 70));
					if (scenarioRef.current === "error")
						throw new Error(
							"Fixture access denied. This source is unavailable to the viewer.",
						);
					if (payload.sql.includes("WHERE false"))
						return { columns, rows: [], row_count: 0, truncated: false };
					const result = aggregateFixture(payload);
					const rows =
						scenarioRef.current === "empty"
							? []
							: result.rows.slice(0, payload.limit ?? 50);
					return {
						...result,
						rows,
						row_count: rows.length,
						truncated:
							scenarioRef.current !== "empty" &&
							result.rows.length > rows.length,
					};
				},
			},
		} as unknown as typeof original);
		setReady(true);
	}, []);
	if (!ready) return <p>Preparing fixture backend…</p>;
	return (
		<main className="min-h-screen bg-background p-5 text-foreground">
			<header className="mb-5 flex flex-wrap items-center justify-between gap-4">
				<div>
					<h1 className="text-xl font-semibold">
						Data widgets · local fixture
					</h1>
					<p className="text-xs text-muted-foreground">
						Production renderers with deterministic mock workbench results. No
						remote data.
					</p>
				</div>
				<label className="flex items-center gap-2 text-sm">
					Scenario
					<select
						aria-label="Scenario"
						className="rounded border bg-background p-2"
						value={scenario}
						onChange={(event) => setScenario(event.target.value)}
					>
						<option value="populated">Populated</option>
						{editor && <option value="unconfigured">Unconfigured</option>}
						<option value="empty">Empty</option>
						<option value="error">Access error</option>
					</select>
				</label>
			</header>
			{editor ? (
				<div data-testid="data-production-editor">
					<HomeEditor
						key={scenario}
						layout={visibleLayout}
						defaultLayout={layout}
						onSave={async (next) => setLayout(next)}
						onReset={async () => {}}
					/>
				</div>
			) : (
				<>
					<div
						style={{
							display: "grid",
							gridTemplateColumns:
								"repeat(auto-fit,minmax(min(100%,360px),1fr))",
							gap: 16,
						}}
					>
						{(expanded ? allSamples : samples).map((item) => (
							<section
								key={`${scenario}:${item.id}`}
								data-testid={`data-${item.id}`}
								className="flex min-w-0 flex-col rounded-xl border bg-card p-4"
								style={{
									height:
										scenario === "populated"
											? ["stat", "metricstrip", "progress", "bullet"].includes(
													String(item.config.visualization),
												)
												? 190
												: 300
											: undefined,
								}}
							>
								<h2 className="mb-3 shrink-0 text-sm font-semibold">
									{item.id}
								</h2>
								<div className="min-h-0 flex-1">
									<HomeDataWidget widget={item} />
								</div>
							</section>
						))}
					</div>
					<section className="mt-6 rounded-xl border p-4">
						<h2 className="mb-4 text-lg font-semibold">
							Configure a real widget
						</h2>
						<div
							style={{
								display: "grid",
								gridTemplateColumns:
									"repeat(auto-fit,minmax(min(100%,360px),1fr))",
								gap: 24,
							}}
						>
							<div data-testid="data-settings">
								<HomeDataWidgetSettings
									widget={editable}
									onChange={(config) =>
										setEditable((previous) => ({ ...previous, config }))
									}
								/>
							</div>
							<div>
								<div
									data-testid="data-builder-preview"
									className="rounded-xl border p-4"
									style={{
										height:
											scenario === "populated"
												? [
														"stat",
														"metricstrip",
														"progress",
														"bullet",
													].includes(String(editable.config.visualization))
													? 190
													: 300
												: undefined,
									}}
								>
									<HomeDataWidget key={scenario} widget={editable} />
								</div>
								<details className="mt-4" open>
									<summary>Last workbench request</summary>
									<pre
										data-testid="data-last-query"
										className="mt-2 overflow-auto whitespace-pre-wrap break-all text-xs"
									>
										{lastQuery}
									</pre>
								</details>
							</div>
						</div>
					</section>
				</>
			)}
		</main>
	);
}
