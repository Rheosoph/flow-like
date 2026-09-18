import { describe, expect, test } from "vitest";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IBackendState } from "../../../state/backend-state";
import {
	buildHomeDataQuery,
	normalizeHomeDataConfig,
} from "../../home/home-data-query";
import type { IHomeLayout, IHomeWidget } from "../../home/types";
import {
	validateHomeLayoutCandidate,
	validateHomeLayoutReferences,
} from "./home-tools";

const layout = (...widgets: IHomeWidget[]): IHomeLayout => ({
	version: 1,
	widgets,
});

const widget = (
	type: string,
	config: Record<string, unknown> = {},
	id = "widget",
): IHomeWidget => ({
	id,
	type,
	title: id,
	size: { columns: 6, rows: 6 },
	appearance: { variant: "card", accent: "neutral" },
	config,
});

type ReferenceBackend = Pick<
	IBackendState,
	"dbState" | "eventState" | "graphState" | "queryState"
>;

const profileOptions = { profileAppIds: new Set(["app"]) };

describe("Home runtime reference parity", () => {
	test("validates the selected image app only for storage sources", async () => {
		const backend = {} as ReferenceBackend;
		const config = {
			mode: "image",
			imageSource: "storage",
			imageAppId: "missing-app",
			imagePath: "media/banner.png",
		};
		expect(
			await validateHomeLayoutReferences(
				backend,
				layout(widget("information", config)),
				profileOptions,
			),
		).toMatchObject([
			{
				code: "home_app_not_in_profile",
				path: "$.widgets[0].config.imageAppId",
			},
		]);
		for (const changes of [
			{ imageAppId: "app" },
			{ imageSource: "url", imageUrl: "/logo.png" },
		])
			expect(
				await validateHomeLayoutReferences(
					backend,
					layout(widget("information", { ...config, ...changes })),
					profileOptions,
				),
			).toEqual([]);
	});

	const event = (id: string, overrides: Partial<IEvent> = {}) =>
		({
			id,
			event_type: "generic_form",
			active: true,
			default_page_id: null,
			route: "/reports",
			...overrides,
		}) as IEvent;
	const validateEmbed = (
		events: ReturnType<typeof event>[],
		config: Record<string, unknown>,
	) =>
		validateHomeLayoutReferences(
			{
				eventState: { getEvents: async () => events },
			} as unknown as ReferenceBackend,
			layout(widget("app-embed", { appId: "app", ...config })),
			profileOptions,
		);

	test("canonical route variants resolve to the same embeddable Event", async () => {
		for (const route of [
			"/reports/",
			"/reports#overview",
			"/reports/?tab=summary#overview",
		]) {
			expect(
				await validateEmbed([event("reports", { route: " reports/ " })], {
					target: "route",
					route,
				}),
			).toEqual([]);
		}
	});

	test("headless route targets fail while page-backed Events remain embeddable", async () => {
		for (const event_type of ["api", "cron", "mcp"]) {
			const target = event("headless", { event_type });
			expect(
				await validateEmbed([target], { target: "route", route: "/reports" }),
			).toMatchObject([{ code: "home_app_route_missing" }]);
			expect(
				await validateEmbed([{ ...target, default_page_id: "page" }], {
					target: "route",
					route: "/reports",
				}),
			).toEqual([]);
		}
	});

	test("root and landing targets require a usable default route or an available fallback", async () => {
		for (const config of [
			{ target: "landing" },
			{ target: "route", route: "/" },
		]) {
			for (const events of [
				[],
				[event("headless-no-default", { event_type: "api" })],
				[event("inactive", { route: "/", active: false })],
				[event("headless", { route: "/", event_type: "api" })],
				[
					event("blank-page", {
						route: "/",
						event_type: "api",
						default_page_id: "  ",
					}),
				],
			]) {
				expect(await validateEmbed(events, config)).toMatchObject([
					{ code: "home_app_route_missing" },
				]);
			}
			expect(
				await validateEmbed([event("root", { route: "/" })], config),
			).toEqual([]);
			expect(
				await validateEmbed(
					[event("default", { route: null, is_default: true })],
					config,
				),
			).toEqual([]);
			expect(await validateEmbed([event("no-default")], config)).toMatchObject([
				{ severity: "warning", code: "home_app_landing_fallback" },
			]);
		}
	});

	test("validation honors first canonical route and explicit-root precedence", async () => {
		expect(
			await validateEmbed(
				[
					event("first", { active: false }),
					event("second", { route: "/reports/" }),
				],
				{ target: "route", route: "/reports" },
			),
		).toMatchObject([{ code: "home_app_route_missing" }]);
		expect(
			await validateEmbed(
				[
					event("default", { route: null, is_default: true }),
					event("explicit", { route: "/", event_type: "api" }),
				],
				{ target: "landing" },
			),
		).toMatchObject([{ code: "home_app_route_missing" }]);
		expect(
			await validateEmbed([event("default", { route: "/" })], {
				target: "route",
				route: "/missing",
			}),
		).toMatchObject([{ code: "home_app_route_missing" }]);
	});

	const dataBackend = {
		dbState: {
			listTables: async () => ["orders"],
			getSchema: async () => ({ fields: [{ name: "id", data_type: "Utf8" }] }),
		},
	} as unknown as ReferenceBackend;
	const recordsConfig = {
		appId: "app",
		sourceKind: "table",
		table: "orders",
		mode: "records",
		fields: [],
		groupBy: "removed_group",
		xField: "removed_x",
		yField: "removed_y",
	};

	test("SELECT * records ignore stale settings their visualization does not consume", async () => {
		for (const visualization of [
			"table",
			"list",
			"cards",
			"record",
			"comparison",
		]) {
			const config = { ...recordsConfig, visualization };
			expect(
				buildHomeDataQuery(normalizeHomeDataConfig(config), {}).sql,
			).toContain('SELECT * FROM "orders"');
			expect(
				await validateHomeLayoutReferences(
					dataBackend,
					layout(widget("data", config)),
					profileOptions,
				),
			).toEqual([]);
		}
	});

	test("records validate active visualization fields and explicit query projections", async () => {
		for (const [visualization, suffixes] of [
			["kanban", ["groupBy"]],
			["graph", ["xField", "yField"]],
			["scatter", ["xField", "yField"]],
			["timeline", ["xField"]],
			["recordcalendar", ["xField"]],
		] as const) {
			const issues = await validateHomeLayoutReferences(
				dataBackend,
				layout(widget("data", { ...recordsConfig, visualization })),
				profileOptions,
			);
			expect(issues.map((entry) => entry.path)).toEqual(
				suffixes.map((suffix) => `$.widgets[0].config.${suffix}`),
			);
		}
		const config = { ...recordsConfig, visualization: "table", fields: ["id"] };
		expect(() =>
			buildHomeDataQuery(normalizeHomeDataConfig(config), {
				columns: [{ name: "id", type_name: "Utf8", position: 0 }],
			}),
		).toThrow("removed_group");
		const issues = await validateHomeLayoutReferences(
			dataBackend,
			layout(widget("data", config)),
			profileOptions,
		);
		expect(issues.map((entry) => entry.path)).toEqual(
			["groupBy", "xField", "yField"].map(
				(suffix) => `$.widgets[0].config.${suffix}`,
			),
		);
	});

	test("sort, date, and filter columns remain active even for SELECT *", async () => {
		const config = {
			...recordsConfig,
			visualization: "table",
			sortBy: "missing_sort",
			dateRange: "7d",
			dateField: "missing_date",
			filters: [{ field: "group", operator: "empty" }],
		};
		const issues = await validateHomeLayoutReferences(
			dataBackend,
			layout(widget("data", config)),
			profileOptions,
		);
		expect(issues.map((entry) => entry.path)).toEqual(
			["dateField", "sortBy", "filters[0].field"].map(
				(suffix) => `$.widgets[0].config.${suffix}`,
			),
		);
	});
});

describe("Home future-value preservation", () => {
	test("unchanged future rendering and appearance values survive an unrelated edit or reorder", () => {
		const future = widget("app-collection", { rendering: "future-masonry" });
		future.appearance = { variant: "future-glass", accent: "future-magenta" };
		const current = layout(future, widget("greeting", {}, "greeting"));
		const candidate = layout(
			{ ...current.widgets[1], title: "Welcome" },
			{ ...future, title: "Apps" },
		);
		const result = validateHomeLayoutCandidate(candidate, current);
		expect(result.valid).toBe(true);
		expect(result.canonical_layout).toEqual(candidate);
		expect(
			result.issues.filter(
				(entry) => entry.code === "home_widget_future_value_preserved",
			),
		).toHaveLength(3);
		expect(validateHomeLayoutCandidate(candidate).valid).toBe(false);
	});

	test("new, changed, or relocated unsupported values still fail", () => {
		const future = widget("app-collection", { rendering: "future-masonry" });
		const current = layout(future);
		for (const candidate of [
			layout({ ...future, id: "new" }),
			layout({ ...future, type: "models" }),
			layout({ ...future, config: { rendering: "future-other" } }),
			layout({
				...future,
				config: { rendering: "standard", source: "future-masonry" },
			}),
		]) {
			expect(validateHomeLayoutCandidate(candidate, current).valid).toBe(false);
		}
		expect(
			validateHomeLayoutCandidate(
				layout({ ...future, config: { rendering: "standard" } }),
				current,
			).valid,
		).toBe(true);
	});

	test("nested future enum values are preserved at the same path", () => {
		const future = widget("data", {
			appId: "app",
			sourceKind: "table",
			table: "orders",
			visualization: "bar",
			measures: [{ aggregation: "future-quantile", field: "amount" }],
		});
		const current = layout(future);
		expect(
			validateHomeLayoutCandidate(
				layout({ ...future, title: "Revenue" }),
				current,
			).valid,
		).toBe(true);
		expect(
			validateHomeLayoutCandidate(
				layout({
					...future,
					config: {
						...future.config,
						measures: [
							{ aggregation: "sum", field: "amount" },
							{ aggregation: "future-quantile", field: "amount" },
						],
					},
				}),
				current,
			).valid,
		).toBe(false);
	});

	test("preservation never downgrades known incompatibilities or unsafe config", () => {
		const invalid = widget("quick-links", {
			links: [{ id: "bad", title: "Unsafe", href: "javascript:alert(1)" }],
		});
		invalid.appearance.variant = "solid";
		const result = validateHomeLayoutCandidate(
			layout(invalid),
			layout(invalid),
		);
		expect(result.valid).toBe(false);
		expect(
			result.issues
				.filter((entry) => entry.severity === "error")
				.map((entry) => entry.code),
		).toEqual(
			expect.arrayContaining(["unsupported_solid_variant", "unsafe_home_url"]),
		);
	});
});
