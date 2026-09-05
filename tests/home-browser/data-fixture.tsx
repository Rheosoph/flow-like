import { useEffect, useRef, useState } from "react";
import { HomeDataWidget } from "../../packages/ui/components/home/data-widget";
import { HomeDataWidgetSettings } from "../../packages/ui/components/home/data-widget-settings";
import type { IHomeWidget } from "../../packages/ui/components/home/types";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import type {
	ExecuteSqlPayload,
	QueryColumn,
} from "../../packages/ui/state/backend-state/query-state";

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
].map(([department, status, amount, date, latency], index) => ({
	order_id: `order-${index}`,
	name: `Invoice ${index + 1}`,
	department,
	status,
	amount,
	date,
	latency,
	owner: index < 8 ? "fixture-user" : "other-user",
}));
const columns: QueryColumn[] = Object.keys(sourceRows[0]).map(
	(name, position) => ({
		name,
		position,
		type_name: ["amount", "latency"].includes(name) ? "Float64" : "Utf8",
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
];

function aggregateFixture(payload: ExecuteSqlPayload) {
	let rows = sourceRows as Record<string, unknown>[];
	const owner = payload.params?.owner ?? payload.params?.__home_filter_0;
	if (owner !== undefined && payload.sql.includes('"owner"'))
		rows = rows.filter((row) => row.owner === owner);
	if (payload.params?.owner !== undefined)
		rows = rows.filter((row) => row.owner === payload.params?.owner);
	if (!payload.sql.includes('AS "__measure_0"')) return { rows, columns };
	const group =
		/"([^"]+)" AS "__group"/.exec(payload.sql)?.[1] ??
		(/DATE_TRUNC/.test(payload.sql) ? "date" : "");
	const series = /"([^"]+)" AS "__series"/.exec(payload.sql)?.[1] ?? "";
	const grouped = new Map<string, Record<string, unknown>[]>();
	for (const row of rows) {
		const key = JSON.stringify([
			group ? row[group] : null,
			series ? row[series] : null,
		]);
		const list = grouped.get(key) ?? [];
		list.push(row);
		grouped.set(key, list);
	}
	const result = [...grouped.values()].map((items) => {
		const row: Record<string, unknown> = {};
		if (group) row.__group = items[0][group];
		if (series) row.__series = items[0][series];
		const matches = [
			...payload.sql.matchAll(
				/(SUM|COUNT|AVG|MIN|MAX|MEDIAN)\((?:DISTINCT )?(?:"([^"]+)"|\*)\) AS "__measure_([0-9]+)"/g,
			),
		];
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
			? String(a.__group).localeCompare(String(b.__group))
			: Number(b.__measure_0) - Number(a.__measure_0),
	);
	return {
		rows: result,
		columns: Object.keys(result[0] ?? {}).map((name, position) => ({
			name,
			position,
			type_name: name === "__group" || name === "__series" ? "Utf8" : "Float64",
		})),
	};
}

export default function DataFixture() {
	const backend = useBackend();
	const initial = useRef(backend);
	const [ready, setReady] = useState(false);
	const [scenario, setScenario] = useState("populated");
	const scenarioRef = useRef(scenario);
	scenarioRef.current = scenario;
	const [lastQuery, setLastQuery] = useState("");
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
						<option value="empty">Empty</option>
						<option value="error">Access error</option>
					</select>
				</label>
			</header>
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit,minmax(min(100%,360px),1fr))",
					gap: 16,
				}}
			>
				{samples.map((item) => (
					<section
						key={`${scenario}:${item.id}`}
						data-testid={`data-${item.id}`}
						className="flex min-w-0 flex-col rounded-xl border bg-card p-4"
						style={{ height: 360 }}
					>
						<h2 className="mb-3 shrink-0 font-semibold">{item.id}</h2>
						<div className="min-h-0 flex-1">
							<HomeDataWidget widget={item} />
						</div>
					</section>
				))}
			</div>
			<section className="mt-6 rounded-xl border p-4">
				<h2 className="mb-4 text-lg font-semibold">Configure a real widget</h2>
				<div
					style={{
						display: "grid",
						gridTemplateColumns: "repeat(auto-fit,minmax(min(100%,360px),1fr))",
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
							style={{ height: 360 }}
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
		</main>
	);
}
