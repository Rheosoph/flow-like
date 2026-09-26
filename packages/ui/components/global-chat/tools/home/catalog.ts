import { HOME_WIDGET_PRESETS } from "../../../home/catalog";
import { HOME_ACCENTS } from "../../../home/home-appearance";
import {
	HOME_APP_RENDERINGS,
	HOME_MODEL_RENDERINGS,
	HOME_PACKAGE_RENDERINGS,
} from "../../../home/home-content/config";
import {
	HOME_DATA_AGGREGATIONS,
	HOME_DATA_FILTER_OPERATORS,
	HOME_DATA_VISUALIZATIONS,
} from "../../../home/home-data-query";
import {
	MAX_HOME_LAYOUT_BYTES,
	MAX_HOME_WIDGETS,
	MAX_HOME_WIDGET_CLASS_NAME_BYTES,
	responsiveHomeColumns,
} from "../../../home/home-layout";
import { DATA_SOURCE_KINDS, HOME_VARIANTS, stringArg } from "./shared";
import { HOME_WIDGET_CONFIG_CONTRACTS } from "./widget-contracts/definitions";

const HOME_CATEGORIES = new Set(
	HOME_WIDGET_PRESETS.map((preset) => preset.category),
);

const HOME_DATA_MODES = ["aggregate", "records"] as const;

const HOME_DATA_SCOPES = ["project", "personal"] as const;

const HOME_DATA_TIME_BUCKETS = [
	"none",
	"day",
	"week",
	"month",
	"quarter",
	"year",
] as const;

const HOME_DATA_DATE_RANGES = ["all", "7d", "30d", "90d", "year"] as const;

const HOME_DATA_SORT_DIRECTIONS = ["asc", "desc"] as const;

const HOME_DATA_FORMATS = ["number", "currency", "percent"] as const;

const HOME_DATA_VALUE_TYPES = ["text", "number", "boolean", "viewer"] as const;

/** Return compact creation templates for the exact widget implementations in this client. */
export function getHomeWidgetCatalog(args: Record<string, unknown>) {
	const category = stringArg(args, "category").toLowerCase();
	const type = stringArg(args, "type").toLowerCase();
	const query = stringArg(args, "query").toLowerCase();
	if (category && !HOME_CATEGORIES.has(category as never)) {
		return {
			status: "error",
			code: "home_widget_category_invalid",
			message: `Unknown category '${category}'.`,
			categories: [...HOME_CATEGORIES],
		};
	}
	const matches = HOME_WIDGET_PRESETS.filter((preset) => {
		if (category && preset.category !== category) return false;
		if (type && preset.type.toLowerCase() !== type) return false;
		if (!query) return true;
		return [preset.id, preset.name, preset.description, preset.type]
			.join(" ")
			.toLowerCase()
			.includes(query);
	});
	const matchedTypes = [...new Set(matches.map((preset) => preset.type))];
	return {
		status: "ok",
		total: matches.length,
		widget_type_total: matchedTypes.length,
		widget_types: Object.fromEntries(
			matchedTypes.map((widgetType) => [
				widgetType,
				{
					type: widgetType,
					...structuredClone(
						HOME_WIDGET_CONFIG_CONTRACTS[
							widgetType as keyof typeof HOME_WIDGET_CONFIG_CONTRACTS
						],
					),
				},
			]),
		),
		layout_contract: {
			version: 1,
			max_widgets: MAX_HOME_WIDGETS,
			max_bytes: MAX_HOME_LAYOUT_BYTES,
			grid_columns: {
				mobile: responsiveHomeColumns(0),
				tablet: responsiveHomeColumns(600),
				desktop: responsiveHomeColumns(1050),
			},
			breakpoints: {
				mobile_max_exclusive: 600,
				desktop_min: 1050,
			},
			height_modes: ["auto", "content", "fixed"],
			class_name: {
				field: "appearance.className",
				optional: true,
				max_bytes: MAX_HOME_WIDGET_CLASS_NAME_BYTES,
				applies_to:
					"Widget root surface, after variant and accent; conflicting background, border, radius, shadow, and text color classes win.",
				syntax:
					"Space-separated Tailwind CSS v4 utility classes compiled at runtime (v4 names such as bg-linear-to-br; arbitrary values work). Style descendants with variants such as [&_h2]:text-lg.",
				ignored:
					"The layout controls position, grid span, height, and self-alignment, so those utilities have no effect. Do not use z-index or sibling variants (~, +).",
				prefer:
					"Theme tokens such as bg-card, text-primary, border-primary/30.",
			},
		},
		categories: [...HOME_CATEGORIES],
		accents: Object.keys(HOME_ACCENTS),
		surface_variants: [...HOME_VARIANTS],
		visualizations: HOME_DATA_VISUALIZATIONS.map(([id, name]) => ({
			id,
			name,
		})),
		data_options: {
			source_kinds: DATA_SOURCE_KINDS,
			scopes: [...HOME_DATA_SCOPES],
			modes: [...HOME_DATA_MODES],
			aggregations: [...HOME_DATA_AGGREGATIONS],
			filter_operators: [...HOME_DATA_FILTER_OPERATORS],
			time_buckets: [...HOME_DATA_TIME_BUCKETS],
			date_ranges: [...HOME_DATA_DATE_RANGES],
			sort_directions: [...HOME_DATA_SORT_DIRECTIONS],
			formats: [...HOME_DATA_FORMATS],
			filter_value_types: [...HOME_DATA_VALUE_TYPES],
		},
		renderings: {
			apps: HOME_APP_RENDERINGS.map(([id, name]) => ({ id, name })),
			models: HOME_MODEL_RENDERINGS.map(([id, name]) => ({ id, name })),
			packages: HOME_PACKAGE_RENDERINGS.map(([id, name]) => ({ id, name })),
			quick_links: ["grid", "list"],
		},
		presets: matches.map((preset) => ({
			preset_id: preset.id,
			name: preset.name,
			description: preset.description,
			category: preset.category,
			widget: {
				type: preset.type,
				title: preset.name,
				description: "",
				size: { columns: preset.columns, rows: preset.rows },
				appearance: {
					variant: preset.variant ?? "card",
					accent: preset.accent ?? "neutral",
				},
				config: structuredClone(preset.config),
			},
		})),
		note: "Give every widget a unique id when placing it in a layout.",
	};
}
