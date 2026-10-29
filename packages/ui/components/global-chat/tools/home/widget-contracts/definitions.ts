import { APP_CATEGORY_ORDER } from "../../../../../lib/category-meta";
import { HOME_DATA_WIDGET_CONFIG_CONTRACT } from "./data";

import {
	appDiscoveryFields,
	boolean,
	count,
	emptyContract,
	imageUrl,
	informationItem,
	linkItem,
	number,
	optionalProfileApp,
	profileApp,
	profileApps,
	reference,
	safeUrl,
	strings,
	text,
} from "./fields";
import {
	ACTIVITY_DAYS,
	APP_CATEGORIES,
	APP_RENDERINGS,
	APP_SOURCES,
	INFORMATION_MODES,
	MODEL_RENDERINGS,
	NOTIFICATION_TYPES,
	PACKAGE_RENDERINGS,
	QUICK_ACTION_IDS,
	RUN_STAT_METRICS,
} from "./options";
import type { HomeWidgetConfigContract, KnownHomeWidgetType } from "./types";

export const HOME_WIDGET_CONFIG_CONTRACTS = {
	"ai-usage": emptyContract("Shows account AI and embedding usage totals."),
	"app-collection": {
		description: "A filtered or hand-picked app collection.",
		additional_properties: false,
		fields: {
			source: text("Which app inventory supplies the collection.", {
				default: "library",
				enum: APP_SOURCES,
			}),
			appIds: profileApps(undefined, {
				field: "source",
				equals: "manual",
			}),
			query: text("Case-insensitive app search text."),
			category: text("Exact app category filter.", { enum: APP_CATEGORIES }),
			tag: text("Exact app tag filter."),
			limit: count(8),
			rendering: text("Collection card presentation.", {
				default: "standard",
				enum: APP_RENDERINGS,
				legacy_values: [{ value: "spotlight", maps_to: "editorial" }],
			}),
			showUpdated: boolean("Show app update dates.", false),
			maxColumns: number("Maximum card columns. Zero uses automatic layout.", {
				default: 0,
				enum: [0, 1, 2, 3, 4, 6],
			}),
		},
		requirements: [
			{
				severity: "warning",
				code: "app_collection_manual_empty",
				message:
					"A manual app collection should contain at least one profile app.",
				when: { field: "source", equals: "manual" },
				non_empty: ["appIds"],
			},
		],
	},
	"app-collection-feature": {
		description: "An editorial feature for a selected app collection.",
		additional_properties: false,
		fields: {
			...appDiscoveryFields("new", 6),
			limit: count(3, 6),
			headline: text("Feature headline."),
			description: text("Feature supporting copy."),
			linkLabel: text("Optional destination label."),
			href: safeUrl("Optional destination override."),
		},
		requirements: [
			{
				severity: "warning",
				code: "app_feature_manual_empty",
				message:
					"A manual app feature should contain at least one profile app.",
				when: { field: "source", equals: "manual" },
				any_non_empty: ["appId", "appIds"],
			},
		],
	},
	"app-embed": {
		description: "An embedded app landing page, route, or Event interface.",
		additional_properties: false,
		fields: {
			target: text("Embedded app destination kind.", {
				default: "landing",
				enum: ["landing", "route", "event"],
			}),
			appId: text("App from the current profile.", {
				reference: reference("profile_app_id", "list_apps"),
			}),
			route: text("App route beginning with a single slash.", {
				default: "/",
				string_format: "route",
				reference: reference("app_route", "list_apps", {
					app_field: "appId",
					when: { field: "target", equals: "route" },
				}),
			}),
			eventId: text("Embeddable Event interface id.", {
				reference: {
					...reference("app_event_id", "list_apps", {
						app_field: "appId",
						when: { field: "target", equals: "event" },
					}),
					inspect_with: "describe_app_interface",
				},
			}),
			query: text("URL-encoded query parameters without routing keys."),
		},
		requirements: [
			{
				code: "app_embed_missing_app",
				message: "Choose an app from the current profile.",
				non_empty: ["appId"],
			},
			{
				code: "app_embed_missing_event",
				message: "Choose an embeddable Event interface.",
				when: { field: "target", equals: "event" },
				non_empty: ["eventId"],
			},
		],
	},
	"app-ranking": {
		description: "A ranked app list from a live or manual source.",
		additional_properties: false,
		fields: {
			...appDiscoveryFields("popular", 10),
			limit: count(6, 10),
			linkLabel: text("Optional destination label."),
			href: safeUrl("Optional destination override."),
		},
		requirements: [
			{
				severity: "warning",
				code: "app_ranking_manual_empty",
				message:
					"A manual app ranking should contain at least one profile app.",
				when: { field: "source", equals: "manual" },
				any_non_empty: ["appId", "appIds"],
			},
		],
	},
	"app-spotlight": {
		description: "A hero or compact feature for one app.",
		additional_properties: false,
		fields: {
			...appDiscoveryFields("new", 1),
			mode: text("Spotlight size.", {
				default: "hero",
				enum: ["hero", "compact"],
			}),
		},
		requirements: [
			{
				severity: "warning",
				code: "app_spotlight_manual_empty",
				message: "A manual app spotlight should identify one profile app.",
				when: { field: "source", equals: "manual" },
				any_non_empty: ["appId", "appIds"],
			},
		],
	},
	categories: {
		description: "App category chips or tiles.",
		additional_properties: false,
		fields: {
			rendering: text("Category presentation.", {
				default: "chips",
				enum: ["chips", "grid"],
			}),
			categories: strings("Categories to show. Empty means all categories.", {
				unique_items: true,
				item_enum: APP_CATEGORY_ORDER,
			}),
		},
	},
	data: HOME_DATA_WIDGET_CONFIG_CONTRACT,
	"executions-by-app": {
		description: "Recent execution counts grouped by app.",
		additional_properties: false,
		fields: {
			days: number("UTC time window.", { default: 7, enum: ACTIVITY_DAYS }),
			appId: optionalProfileApp,
			limit: count(5),
		},
	},
	flowpilot: {
		description: "A FlowPilot entry point.",
		additional_properties: false,
		fields: {
			mode: text("FlowPilot presentation.", {
				default: "bar",
				enum: ["orb", "bar", "card", "hero"],
			}),
			suggestions: boolean("Show prompt suggestions."),
			emphasis: boolean("Emphasize the prompt with the accent color.", false),
			placeholder: text("Prompt input placeholder."),
		},
	},
	greeting: {
		description: "A personalized greeting.",
		additional_properties: false,
		fields: {
			mode: text("Greeting presentation.", {
				default: "standard",
				enum: ["standard", "masthead"],
			}),
			name: text("Optional name override."),
			subtitle: text("Optional welcome message."),
		},
	},
	information: {
		description: "Markdown, structured guidance, media, or an announcement.",
		additional_properties: false,
		fields: {
			mode: text("Information presentation.", {
				default: "markdown",
				enum: INFORMATION_MODES,
			}),
			body: text("Markdown body."),
			items: {
				type: "object_list",
				description:
					"Items for FAQ, checklist, feed, steps, resources, and facts modes.",
				item: informationItem,
			},
			layout: text("Resource presentation.", {
				default: "cards",
				enum: ["cards", "list"],
			}),
			date: text("Milestone date or date-time.", {
				string_format: "date-time",
			}),
			eyebrow: text("Short label above the content."),
			imageSource: text("Image source for image and story presentations.", {
				default: "url",
				enum: ["url", "storage"],
			}),
			imageUrl: imageUrl("HTTP(S) or relative image URL.", {
				field: "imageSource",
				not_equals: "storage",
			}),
			imageAppId: profileApp("App that owns the selected storage image.", {
				field: "imageSource",
				equals: "storage",
			}),
			imagePath: text(
				"App-relative image file path, such as media/banner.png. Home requests download URLs using the viewer's existing app permissions. Save the path instead of a signed URL.",
				{
					when: { field: "imageSource", equals: "storage" },
					string_format: "storage-image-path",
				},
			),
			imageAlt: text("Accessible image description."),
			actionLabel: text("Call-to-action label."),
			actionHref: safeUrl("Call-to-action destination."),
			attribution: text("Quote attribution."),
		},
		requirements: [
			{
				code: "information_storage_image_app_missing",
				message: "Choose an app for the storage image.",
				when: { field: "imageSource", equals: "storage" },
				non_empty: ["imageAppId"],
			},
			{
				code: "information_storage_image_path_missing",
				message: "Choose an image file from the app's storage.",
				when: { field: "imageSource", equals: "storage" },
				non_empty: ["imagePath"],
			},
			{
				severity: "warning",
				code: "information_countdown_empty",
				message: "A countdown should include a valid milestone date.",
				when: { field: "mode", equals: "countdown" },
				non_empty: ["date"],
			},
		],
	},
	"model-spotlight": {
		description: "A feature for one profile or catalog model.",
		additional_properties: false,
		fields: {
			source: text("Model inventory.", {
				default: "explore",
				enum: ["explore", "profile", "manual"],
			}),
			modelId: text("Exact model id for manual selection.", {
				reference: reference("model_id", undefined, {
					when: { field: "source", equals: "manual" },
				}),
			}),
			modelHub: text("Optional model hub paired with modelId.", {
				reference: reference("model_hub", undefined, {
					when: { field: "source", equals: "manual" },
				}),
			}),
			query: text("Case-insensitive model search text."),
			eyebrow: text("Short label above the widget heading."),
		},
		requirements: [
			{
				code: "model_spotlight_manual_empty",
				message: "A manual model spotlight needs an exact model id.",
				when: { field: "source", equals: "manual" },
				non_empty: ["modelId"],
			},
		],
	},
	models: {
		description: "A profile or catalog model collection.",
		additional_properties: false,
		fields: {
			source: text("Model inventory.", {
				default: "profile",
				enum: ["profile", "explore"],
			}),
			query: text("Case-insensitive model search text."),
			limit: count(6),
			rendering: text("Model card presentation.", {
				default: "standard",
				enum: MODEL_RENDERINGS,
			}),
		},
	},
	"needs-attention": {
		description:
			"Unread workflow notifications and Error or Fatal execution records.",
		additional_properties: false,
		fields: {
			notificationType: text("Notification filter.", {
				default: "WORKFLOW",
				enum: NOTIFICATION_TYPES,
			}),
			limit: count(8),
			days: number("Execution lookback in UTC.", {
				default: 7,
				enum: ACTIVITY_DAYS,
			}),
			appId: optionalProfileApp,
		},
	},
	notifications: {
		description: "Personal notifications.",
		additional_properties: false,
		fields: {
			notificationType: text("Notification filter.", {
				default: "all",
				enum: NOTIFICATION_TYPES,
			}),
			unread: boolean("Only show unread notifications.", false),
			limit: count(8),
		},
	},
	packages: {
		description: "A package-store collection.",
		additional_properties: false,
		fields: {
			query: text("Package search text."),
			category: text("Optional package category."),
			limit: count(6),
			sort: text("Package sort order.", {
				default: "downloads",
				enum: ["downloads", "created_at", "updated_at", "relevance"],
			}),
			rendering: text("Package card presentation.", {
				default: "standard",
				enum: PACKAGE_RENDERINGS,
				legacy_values: [
					{ value: "list", maps_to: "compact" },
					{ value: "grid", maps_to: "standard" },
				],
			}),
		},
	},
	"quick-actions": {
		description: "Built-in Home actions.",
		additional_properties: false,
		fields: {
			actions: strings("Ordered built-in action ids.", {
				unique_items: true,
				item_enum: QUICK_ACTION_IDS,
			}),
			layout: text("Action presentation.", {
				default: "cards",
				enum: ["cards", "toolbar"],
			}),
		},
	},
	"quick-links": {
		description: "Internal or external destination links.",
		additional_properties: false,
		fields: {
			rendering: text("Link presentation.", {
				default: "grid",
				enum: ["grid", "list"],
			}),
			links: {
				type: "object_list",
				description: "Ordered destination links.",
				item: linkItem,
			},
		},
	},
	"recent-runs": {
		description: "Recent execution records.",
		additional_properties: false,
		fields: { appId: optionalProfileApp, limit: count(8) },
	},
	"run-activity": {
		description: "Execution activity by UTC day.",
		additional_properties: false,
		fields: {
			days: number("UTC time window.", { default: 7, enum: ACTIVITY_DAYS }),
			appId: optionalProfileApp,
		},
	},
	"run-stats": {
		description: "Recorded execution or AI usage statistics.",
		additional_properties: false,
		fields: {
			metric: text("Statistic to display.", {
				default: "overview",
				enum: RUN_STAT_METRICS,
			}),
			days: number("UTC time window. Applies to the period statistics.", {
				default: 7,
				enum: ACTIVITY_DAYS,
			}),
			appId: profileApp("Optional app filter from the current profile.", {
				field: "metric",
				in: ["errors", "duration"],
			}),
		},
	},
	"saved-forecast": {
		description:
			"A saved prediction and its uncertainty, shown without running the model again.",
		additional_properties: false,
		fields: {
			appId: profileApp("App that holds the saved prediction."),
			forecastId: text("Saved prediction id in that app."),
			scope: text("Where the prediction is saved.", {
				default: "project",
				enum: ["project", "personal"],
			}),
			view: text("What the widget shows.", {
				default: "forecast",
				enum: ["forecast", "evaluation"],
			}),
			seriesId: text("Optional series of the prediction to show."),
			target: text("Optional target column to show."),
		},
		requirements: [
			{
				code: "saved_forecast_missing_source",
				message: "Choose an app and a saved prediction.",
				non_empty: ["appId", "forecastId"],
			},
		],
	},
	schedules: {
		description: "Upcoming active app schedules.",
		additional_properties: false,
		fields: { appIds: profileApps(), limit: count(8) },
	},
	"section-heading": {
		description: "A section title with an optional destination.",
		additional_properties: false,
		fields: {
			linkLabel: text("Optional destination label."),
			href: safeUrl("Optional destination."),
		},
	},
	"workspace-pulse": {
		description: "Profile app and account activity summary.",
		additional_properties: false,
		fields: {
			mode: text("Workspace summary presentation.", {
				default: "card",
				enum: ["card", "strip", "attention"],
			}),
			days: number("UTC activity period.", { default: 7, enum: ACTIVITY_DAYS }),
			showAttention: boolean("Include Error or Fatal records.", true),
		},
	},
} satisfies Record<KnownHomeWidgetType, HomeWidgetConfigContract>;
