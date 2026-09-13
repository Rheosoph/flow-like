import { describe, expect, mock, test } from "bun:test";
import type { IBackendState } from "../state/backend-state";
import type { ExecuteSqlResult } from "../state/backend-state/query-state";
import {
	type NativeWidgetChartDefinition,
	type NativeWidgetPageDefinition,
	nativeWidgetScope,
	nativeWidgetShell,
	normalizeNativeWidgetDefinition,
	readNativeWidgetDefinitions,
	saveNativeWidgetDefinitions,
} from "./native-widget";
import { isNativeWidgetContent } from "./native-widget-content";
import {
	loadNativeWidgetChart,
	nativeChartFromResult,
} from "./native-widget-data";

const now = new Date("2026-09-13T10:00:00Z");
const page = (changes: Record<string, unknown> = {}) =>
	normalizeNativeWidgetDefinition({
		id: "page-one",
		kind: "page",
		title: "Orders",
		appId: "app",
		path: "/orders",
		queryParams: [],
		updatedAt: now.toISOString(),
		...changes,
	}) as NativeWidgetPageDefinition;
const chart = (data: Record<string, unknown> = {}) =>
	normalizeNativeWidgetDefinition({
		id: "chart-one",
		kind: "chart",
		title: "Sales",
		path: "/sales",
		updatedAt: now.toISOString(),
		data: { appId: "app", table: "invoices", visualization: "bar", ...data },
	}) as NativeWidgetChartDefinition;
const result = (rows: ExecuteSqlResult["rows"]): ExecuteSqlResult => ({
	columns: [],
	rows,
	row_count: rows.length,
	truncated: false,
});
function backend(rows: ExecuteSqlResult["rows"] = [{ __measure_0: 0 }]) {
	const executeSql = mock(async () => result(rows));
	const getSchema = mock(
		async (_appId: string, _table: string, _personal: boolean) => ({
			fields: [
				{ name: "amount", data_type: "Float64", nullable: true },
				{ name: "owner", data_type: "Utf8", nullable: true },
				{ name: "status", data_type: "Utf8", nullable: true },
			],
		}),
	);
	const getOverlay = mock(async () => ({
		id: "sales",
		nodes: [
			{
				label: "Invoice",
				table: "invoice rows",
				id_column: "invoice_id",
				property_columns: [],
			},
		],
	}));
	const getSavedQuery = mock(async () => ({
		id: "mine",
		surface: "native",
		kind: "query",
		sql: "SELECT * FROM invoices WHERE owner = $owner;",
	}));
	const getProfile = mock(async () => ({ apps: [{ app_id: "app" }] }));
	const value = {
		userState: { getProfile },
		isLocalOnly: mock(async () => false),
		dbState: { getSchema },
		graphState: { getOverlay },
		queryState: { executeSql, getSavedQuery },
	};
	return { ...value, value: value as unknown as IBackendState };
}

describe("native widget configuration", () => {
	test("does not split emoji at a native string limit", () => {
		expect(page({ title: `${"a".repeat(119)}🙂` }).title).toBe("a".repeat(119));
		const value = nativeChartFromResult(
			chart({ groupBy: "status" }),
			result([{ __group: `${"a".repeat(99)}🙂`, __measure_0: 1 }]),
		);
		expect(value.chart.points[0].label).toBe("a".repeat(99));
	});
	test("preserves encoded paths and repeated query values without changing the target app", () => {
		const definition = page({
			path: "/orders?tag=one%20two&tag=%26",
			queryParams: [
				{ name: "appId", value: "different" },
				{ name: "q", value: "東京 + café" },
			],
		});
		expect(definition.path).toBe("/orders");
		expect(definition.queryParams).toEqual([
			{ name: "tag", value: "one two" },
			{ name: "tag", value: "&" },
			{ name: "appId", value: "different" },
			{ name: "q", value: "東京 + café" },
		]);
		expect(definition.appId).toBe("app");
		expect(() => page({ path: "https://example.com" })).toThrow();
		expect(() => page({ path: "/../settings" })).toThrow();
	});
	test("normalizes scalar charts and requires a usable target", () => {
		const definition = chart({
			visualization: "progress",
			target: 100,
			mode: "records",
			groupBy: "status",
			seriesBy: "owner",
			limit: 500,
			refreshSeconds: 60,
			measures: [
				{ aggregation: "count" },
				{ aggregation: "sum", field: "amount" },
			],
		});
		expect(definition.data).toMatchObject({
			mode: "aggregate",
			groupBy: "",
			seriesBy: "",
			limit: 120,
			refreshSeconds: 0,
		});
		expect(definition.data.measures).toHaveLength(1);
		expect(() => chart({ visualization: "gauge", target: 0 })).toThrow(
			"positive target",
		);
		expect(() => chart({ visualization: "heatmap" })).toThrow("supported");
	});
	test("isolates definitions by viewer, profile and hub and keeps valid siblings", () => {
		const values = new Map<string, string>();
		const storage = {
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => {
				values.set(key, value);
			},
		};
		const scope = nativeWidgetScope(
			{ profile: { id: "profile-a", hub: "https://a.example" } } as Parameters<
				typeof nativeWidgetScope
			>[0],
			"alice",
		);
		const other = nativeWidgetScope(
			{ profile: { id: "profile-a", hub: "https://a.example" } } as Parameters<
				typeof nativeWidgetScope
			>[0],
			"bob",
		);
		saveNativeWidgetDefinitions(scope, [page()], storage);
		expect(readNativeWidgetDefinitions(scope, storage)).toHaveLength(1);
		expect(readNativeWidgetDefinitions(other, storage)).toEqual([]);
		expect(
			nativeWidgetScope(
				{ profile: { id: "profile-b" } } as Parameters<
					typeof nativeWidgetScope
				>[0],
				"alice",
			),
		).not.toBe(scope);
		const key = [...values.keys()][0];
		values.set(
			key,
			JSON.stringify([
				page(),
				{ ...page(), id: "bad", path: "//external.test" },
				page(),
			]),
		);
		expect(
			readNativeWidgetDefinitions(scope, storage).map((item) => item.id),
		).toEqual(["page-one"]);
		expect(() =>
			saveNativeWidgetDefinitions(
				scope,
				Array.from({ length: 13 }, (_, index) => page({ id: `p-${index}` })),
				storage,
			),
		).toThrow("12");
	});
});

describe("native chart queries", () => {
	test("aggregates a personal table with the current viewer filter and preserves zero", async () => {
		const api = backend();
		const definition = chart({
			scope: "personal",
			visualization: "stat",
			filters: [{ field: "owner", operator: "eq", valueType: "viewer" }],
		});
		const widget = await loadNativeWidgetChart(api.value, definition, {
			viewerId: "alice",
			now,
		});
		expect(api.dbState.getSchema.mock.calls[0]).toEqual([
			"app",
			"invoices",
			true,
		]);
		const [appId, query, personal] = api.queryState.executeSql.mock
			.calls[0] as unknown as [
			string,
			{ sql: string; params: Record<string, unknown> },
			boolean,
		];
		expect(appId).toBe("app");
		expect(personal).toBe(true);
		expect(query.sql).toContain("COUNT(*)");
		expect(query.params.__home_filter_0).toBe("alice");
		expect(widget.state).toBe("ready");
		expect(widget.chart?.value).toBe(0);
		expect(isNativeWidgetContent(widget, definition, now.getTime())).toBe(true);
	});
	test("uses ontology object identity and the overlay query surface", async () => {
		const api = backend();
		await loadNativeWidgetChart(
			api.value,
			chart({
				sourceKind: "ontology",
				ontologyId: "sales",
				objectType: "Invoice",
			}),
			{ viewerId: "alice", now },
		);
		const [, query] = api.queryState.executeSql.mock.calls[0] as unknown as [
			string,
			{ sql: string; surface: string; overlay_id: string },
		];
		expect(query.surface).toBe("overlay");
		expect(query.overlay_id).toBe("sales");
		expect(query.sql).toContain('COUNT(DISTINCT "invoice_id")');
	});
	test("binds saved-query parameters and aggregates the saved source", async () => {
		const api = backend();
		await loadNativeWidgetChart(
			api.value,
			chart({
				sourceKind: "query",
				queryId: "mine",
				queryParams: { owner: "$viewer.id" },
			}),
			{ viewerId: "alice", now },
		);
		const [, query] = api.queryState.executeSql.mock.calls[0] as unknown as [
			string,
			{ sql: string; params: Record<string, unknown> },
		];
		expect(query.sql).toContain("FROM (");
		expect(Object.values(query.params)).toContain("alice");
	});
	test("stops before reading data when access is missing or the request was cancelled", async () => {
		const api = backend();
		await expect(
			loadNativeWidgetChart(api.value, chart(), { now }),
		).rejects.toThrow("Sign in");
		api.userState.getProfile.mockResolvedValue({ apps: [] });
		await expect(
			loadNativeWidgetChart(api.value, chart(), { viewerId: "alice", now }),
		).rejects.toThrow("selected profile");
		const abort = new AbortController();
		abort.abort();
		await expect(
			loadNativeWidgetChart(api.value, chart(), {
				viewerId: "alice",
				signal: abort.signal,
				now,
			}),
		).rejects.toThrow("cancelled");
		expect(api.queryState.executeSql).not.toHaveBeenCalled();
	});
	test("drops invalid slices, keeps negative bars, and bounds groups and Unicode labels", () => {
		const rows = [
			{ __group: "Zero", __measure_0: 0 },
			{ __group: "Loss", __measure_0: -10 },
			{ __group: "Null", __measure_0: null },
			{ __group: "Bad", __measure_0: "no" },
		];
		const bars = nativeChartFromResult(
			chart({ groupBy: "status" }),
			result(rows),
		);
		expect(bars.chart.points.map((point) => point.value)).toEqual([0, -10]);
		const pie = nativeChartFromResult(
			chart({ visualization: "pie", groupBy: "status" }),
			result(rows),
		);
		expect(pie.chart.points.map((point) => point.value)).toEqual([0]);
		expect(pie.warnings).toHaveLength(1);
		const definition = chart({ groupBy: "status" });
		const many = nativeChartFromResult(
			definition,
			result(
				Array.from({ length: 200 }, (_, index) => ({
					__group: "東".repeat(400),
					__measure_0: index,
				})),
			),
		);
		expect(many.chart.points).toHaveLength(120);
		expect(many.warnings.length).toBeGreaterThan(0);
		expect(
			isNativeWidgetContent(
				{ ...nativeWidgetShell(definition, now), state: "ready", ...many },
				definition,
				now.getTime(),
			),
		).toBe(true);
	});
});

describe("native persisted content boundary", () => {
	const definition = page();
	const content = () => ({
		...nativeWidgetShell(definition, now),
		state: "ready",
		page: { id: "title", kind: "text", text: "Private content" },
	});
	test("accepts bounded content only in its app and lifetime", () => {
		expect(isNativeWidgetContent(content(), definition, now.getTime())).toBe(
			true,
		);
		expect(
			isNativeWidgetContent(
				content(),
				page({ appId: "another" }),
				now.getTime(),
			),
		).toBe(false);
		expect(
			isNativeWidgetContent(
				content(),
				definition,
				now.getTime() + 8 * 86_400_000,
			),
		).toBe(false);
		expect(
			isNativeWidgetContent(
				{ ...content(), staleAt: "2030-01-01" },
				definition,
				now.getTime(),
			),
		).toBe(false);
	});
	test("rejects raw media and privileged actions, including hidden error-state payloads", () => {
		expect(
			isNativeWidgetContent(
				{
					...content(),
					page: {
						id: "image",
						kind: "image",
						image: { url: "https://signed.example/private" },
					},
				},
				definition,
				now.getTime(),
			),
		).toBe(false);
		expect(
			isNativeWidgetContent(
				{
					...content(),
					action: { kind: "run_event", appId: "app", eventId: "event" },
				},
				definition,
				now.getTime(),
			),
		).toBe(false);
		expect(
			isNativeWidgetContent(
				{
					...content(),
					state: "error",
					page: {
						id: "button",
						kind: "link",
						action: { kind: "open_app", appId: "other" },
					},
				},
				definition,
				now.getTime(),
			),
		).toBe(false);
	});
	test("rejects malicious or excessive trees before previewing", () => {
		expect(
			isNativeWidgetContent(
				{
					...content(),
					page: {
						id: "root",
						kind: "column",
						children: [
							{ id: "same", kind: "text" },
							{ id: "same", kind: "text" },
						],
					},
				},
				definition,
				now.getTime(),
			),
		).toBe(false);
		let node: Record<string, unknown> = { id: "leaf", kind: "text" };
		for (let index = 0; index < 8; index++)
			node = { id: `level-${index}`, kind: "column", children: [node] };
		expect(
			isNativeWidgetContent(
				{ ...content(), page: node },
				definition,
				now.getTime(),
			),
		).toBe(false);
		expect(
			isNativeWidgetContent(
				{
					...content(),
					page: { ...content().page, fetch: "https://external.example" },
				},
				definition,
				now.getTime(),
			),
		).toBe(false);
	});
});
