import { describe, expect, test } from "vitest";
import { HOME_WIDGET_PRESETS } from "../../home/catalog";
import {
	HOME_APP_RENDERINGS,
	HOME_MODEL_RENDERINGS,
	HOME_PACKAGE_RENDERINGS,
} from "../../home/home-content/config";
import {
	HOME_DATA_AGGREGATIONS,
	HOME_DATA_FILTER_OPERATORS,
	HOME_DATA_VISUALIZATIONS,
} from "../../home/home-data-query";
import {
	HOME_WIDGET_CONFIG_CONTRACTS,
	KNOWN_HOME_WIDGET_TYPES,
	validateKnownHomeWidgetConfig,
} from "./home-widget-contracts";

const issueCodes = (type: string, config: unknown) =>
	validateKnownHomeWidgetConfig(type, config).map((entry) => entry.code);

describe("Home widget config contracts", () => {
	test("cover every widget implementation in the shipped catalog", () => {
		const catalogTypes = [
			...new Set(HOME_WIDGET_PRESETS.map((preset) => preset.type)),
		].sort();
		expect([...KNOWN_HOME_WIDGET_TYPES].sort()).toEqual(catalogTypes);
		expect(Object.keys(HOME_WIDGET_CONFIG_CONTRACTS).sort()).toEqual(
			catalogTypes,
		);
		expect(() => JSON.stringify(HOME_WIDGET_CONFIG_CONTRACTS)).not.toThrow();
		for (const contract of Object.values(HOME_WIDGET_CONFIG_CONTRACTS)) {
			expect(contract.additional_properties).toBe(false);
		}
	});

	test("publishes renderer-owned data and rendering options", () => {
		const data = HOME_WIDGET_CONFIG_CONTRACTS.data.fields;
		expect(data.visualization.enum).toEqual(
			HOME_DATA_VISUALIZATIONS.map(([id]) => id),
		);
		expect(data.measures.item?.fields.aggregation.enum).toEqual(
			HOME_DATA_AGGREGATIONS,
		);
		expect(data.filters.item?.fields.operator.enum).toEqual(
			HOME_DATA_FILTER_OPERATORS,
		);
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["app-collection"].fields.rendering.enum,
		).toEqual(HOME_APP_RENDERINGS.map(([id]) => id));
		expect(HOME_WIDGET_CONFIG_CONTRACTS.models.fields.rendering.enum).toEqual(
			HOME_MODEL_RENDERINGS.map(([id]) => id),
		);
		expect(HOME_WIDGET_CONFIG_CONTRACTS.packages.fields.rendering.enum).toEqual(
			HOME_PACKAGE_RENDERINGS.map(([id]) => id),
		);
	});

	test("rejects known-widget values the renderer would otherwise ignore or replace", () => {
		const flowpilotIssues = validateKnownHomeWidgetConfig("flowpilot", {
			mode: "banner",
			surprise: true,
		});
		expect(flowpilotIssues.map((entry) => entry.code)).toEqual(
			expect.arrayContaining([
				"home_widget_config_option_invalid",
				"home_widget_config_key_unknown",
			]),
		);
		expect(
			flowpilotIssues.find(
				(entry) => entry.code === "home_widget_config_key_unknown",
			)?.severity,
		).toBe("warning");
		const legacyRendering = validateKnownHomeWidgetConfig("app-collection", {
			rendering: "spotlight",
		});
		expect(legacyRendering).toContainEqual(
			expect.objectContaining({
				severity: "warning",
				code: "home_widget_config_legacy_value",
			}),
		);
		expect(legacyRendering).not.toContainEqual(
			expect.objectContaining({
				severity: "error",
				code: "home_widget_config_option_invalid",
			}),
		);
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS.packages.fields.rendering.legacy_values,
		).toEqual([
			{ value: "list", maps_to: "compact" },
			{ value: "grid", maps_to: "standard" },
		]);
		expect(
			issueCodes("app-collection", {
				source: "manual",
				appIds: ["app-a", "app-a"],
				limit: 100,
				rendering: "tiles",
			}),
		).toEqual(
			expect.arrayContaining([
				"home_widget_config_list_duplicate",
				"home_widget_config_number_too_large",
				"home_widget_config_option_invalid",
			]),
		);
		expect(
			issueCodes("information", {
				mode: "faq",
				items: [
					{ id: "same", title: "One" },
					{ id: "same", title: "Two" },
				],
			}),
		).toContain("home_widget_config_item_id_duplicate");
		expect(
			issueCodes("quick-links", {
				links: [{ title: "Bad", href: "javascript:alert(1)" }],
			}),
		).toContain("unsafe_home_url");
		expect(
			issueCodes("information", { mode: "image", imageUrl: "mailto:a@b.c" }),
		).toContain("home_widget_config_image_url_invalid");
		expect(
			issueCodes("information", {
				mode: "image",
				imageUrl: "javascript:alert(1)",
			}),
		).toContain("home_widget_config_image_url_invalid");
	});

	test("describes and enforces reference-bearing conditional requirements", () => {
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["app-embed"].fields.eventId.reference,
		).toMatchObject({
			kind: "app_event_id",
			discover_with: "list_apps",
			inspect_with: "describe_app_interface",
			app_field: "appId",
			when: { field: "target", equals: "event" },
			note: "Choose an active Event with a nonempty default_page_id or event_type simple_chat, generic_form, or quick_action.",
		});
		expect(
			issueCodes("app-embed", {
				target: "event",
				appId: "app-a",
				eventId: "",
				query: "route=stolen",
			}),
		).toEqual(
			expect.arrayContaining([
				"app_embed_missing_event",
				"app_embed_query_key_reserved",
			]),
		);
		expect(
			issueCodes("app-embed", {
				target: "route",
				appId: "app-a",
				route: "/reports?eventId=stolen",
			}),
		).toContain("app_embed_query_key_reserved");
		expect(
			issueCodes("model-spotlight", { source: "manual", modelId: "" }),
		).toContain("model_spotlight_manual_empty");
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["model-spotlight"].fields.modelId.reference,
		).toMatchObject({
			provenance: "manual_profile_model",
			validation: "unverified",
			when: { field: "source", equals: "manual" },
		});
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["model-spotlight"].fields.modelId.reference,
		).not.toHaveProperty("discover_with");
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["app-collection"].fields.appIds.reference
				?.when,
		).toEqual({ field: "source", equals: "manual" });
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["app-spotlight"].fields.appIds.reference
				?.when,
		).toEqual({
			all: [
				{ field: "source", equals: "manual" },
				{ field: "appId", equals: "" },
			],
		});
		expect(
			issueCodes("app-spotlight", {
				source: "manual",
				appIds: ["app-a"],
			}),
		).not.toContain("app_spotlight_manual_empty");
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS["run-stats"].fields.appId.reference?.when,
		).toEqual({ field: "metric", in: ["errors", "duration"] });
		expect(
			issueCodes("saved-forecast", { appId: "app-a", forecastId: "" }),
		).toContain("saved_forecast_missing_source");
		expect(
			issueCodes("saved-forecast", {
				appId: "app-a",
				forecastId: "forecast-1",
				scope: "personal",
				view: "evaluation",
			}),
		).toEqual([]);
		expect(
			issueCodes("saved-forecast", {
				appId: "app-a",
				forecastId: "forecast-1",
				view: "chart",
			}),
		).not.toEqual([]);
	});

	test("accepts URL images and advertises app storage image references", () => {
		for (const imageUrl of [
			"https://images.example.com/banner.png",
			"/logo.png",
		])
			expect(
				validateKnownHomeWidgetConfig("information", {
					mode: "image",
					imageUrl,
				}),
			).toEqual([]);
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS.information.fields.imageSource,
		).toMatchObject({
			default: "url",
			enum: ["url", "storage"],
		});
		expect(
			HOME_WIDGET_CONFIG_CONTRACTS.information.fields.imageAppId.reference,
		).toMatchObject({
			kind: "profile_app_id",
			discover_with: "list_apps",
			when: { field: "imageSource", equals: "storage" },
		});
		for (const mode of ["image", "story"])
			expect(
				validateKnownHomeWidgetConfig("information", {
					mode,
					imageSource: "storage",
					imageAppId: "app-a",
					imagePath: "media/team photo.png",
					imageAlt: "The support team",
					imageUrl: "javascript:stale-hidden-value",
				}),
			).toEqual([]);
		expect(
			validateKnownHomeWidgetConfig("information", {
				mode: "image",
				imageSource: "url",
				imageUrl: "/logo.png",
				imagePath: "../stale-hidden-value",
			}),
		).toEqual([]);
	});

	test("requires a complete storage image reference and rejects malformed paths", () => {
		expect(
			issueCodes("information", { mode: "image", imageSource: "storage" }),
		).toEqual([
			"information_storage_image_app_missing",
			"information_storage_image_path_missing",
		]);
		for (const imagePath of [
			"https://images.example.com/banner.png",
			"/banner.png",
			"../banner.png",
			"media/../banner.png",
			"media/",
			"media\\banner.png",
		])
			expect(
				issueCodes("information", {
					mode: "image",
					imageSource: "storage",
					imageAppId: "app-a",
					imagePath,
				}),
			).toContain("home_widget_config_storage_image_path_invalid");
		expect(
			issueCodes("information", { mode: "image", imageSource: "upload" }),
		).toContain("home_widget_config_option_invalid");
	});

	test("rejects incomplete data presentations and malformed nested config", () => {
		const base = {
			appId: "app-a",
			sourceKind: "table",
			table: "orders",
			visualization: "stat",
			mode: "aggregate",
			measures: [{ aggregation: "count", field: "", label: "Count" }],
		};
		expect(validateKnownHomeWidgetConfig("data", base)).toEqual([]);

		expect(
			issueCodes("data", {
				...base,
				visualization: "calendar",
				groupBy: "created_at",
				timeBucket: "month",
				seriesBy: "region",
				filters: [
					{
						field: "amount",
						operator: "gte",
						valueType: "number",
						value: "not-a-number",
					},
				],
				queryParams: { __home_start: "reserved" },
			}),
		).toEqual(
			expect.arrayContaining([
				"data_calendar_fields_invalid",
				"data_filter_number_invalid",
				"data_query_parameter_reserved",
			]),
		);
		expect(
			issueCodes("data", {
				...base,
				visualization: "graph",
				mode: "aggregate",
			}),
		).toEqual(
			expect.arrayContaining([
				"data_xy_fields_missing",
				"data_mode_incompatible",
			]),
		);
		expect(
			issueCodes("data", {
				...base,
				visualization: "progress",
				target: 0,
			}),
		).toContain("data_target_missing");
		expect(
			issueCodes("data", {
				...base,
				filters: [
					{
						field: "archived_at",
						operator: "empty",
						valueType: "number",
						value: "",
					},
				],
			}),
		).not.toContain("data_filter_number_invalid");
		expect(
			issueCodes("data", {
				...base,
				filters: [
					{
						field: "active",
						operator: "eq",
						valueType: "boolean",
						value: "yes",
					},
				],
			}),
		).toContain("data_filter_boolean_invalid");
	});

	test("does not claim ownership of future widget config", () => {
		expect(
			validateKnownHomeWidgetConfig("future-widget", { anything: true }),
		).toEqual([]);
	});
});
