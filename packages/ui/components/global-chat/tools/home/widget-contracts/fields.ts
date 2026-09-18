import {
	HOME_DATA_AGGREGATIONS,
	HOME_DATA_FILTER_OPERATORS,
} from "../../../../home/home-data-query";
import { APP_CATEGORIES, DISCOVERY_APP_SOURCES } from "./options";
import type {
	HomeWidgetConfigCondition,
	HomeWidgetConfigContract,
	HomeWidgetConfigFieldContract,
	HomeWidgetConfigReferenceContract,
	HomeWidgetConfigReferenceKind,
	HomeWidgetObjectContract,
} from "./types";

export const reference = (
	kind: HomeWidgetConfigReferenceKind,
	discoverWith?: string,
	dependencies: Pick<
		HomeWidgetConfigReferenceContract,
		"app_field" | "scope_field" | "source_field" | "when"
	> = {},
): HomeWidgetConfigReferenceContract => {
	const details: Record<
		HomeWidgetConfigReferenceKind,
		Pick<
			HomeWidgetConfigReferenceContract,
			"note" | "provenance" | "validation"
		>
	> = {
		app_event_id: {
			provenance: "app_interface",
			validation: "reference",
			note: "Choose an active Event with a nonempty default_page_id or event_type simple_chat, generic_form, or quick_action.",
		},
		app_route: { provenance: "app_interface", validation: "reference" },
		data_field: {
			provenance: "home_data_source",
			validation: "reference_when_schema_available",
			note: "Saved-query result fields cannot be verified unless the source exposes a result schema.",
		},
		model_hub: {
			provenance: "manual_profile_model",
			validation: "unverified",
			note: "No Home model inventory tool currently resolves exact model references. Prefer source plus query unless the exact reference is already known.",
		},
		model_id: {
			provenance: "manual_profile_model",
			validation: "unverified",
			note: "No Home model inventory tool currently resolves exact model references. Prefer source plus query unless the exact reference is already known.",
		},
		ontology_id: {
			provenance: "home_data_source",
			validation: "reference",
		},
		ontology_object_type: {
			provenance: "home_data_source",
			validation: "reference",
		},
		profile_app_id: {
			provenance: "current_profile",
			validation: "reference",
		},
		profile_app_ids: {
			provenance: "current_profile",
			validation: "reference",
		},
		safe_home_url: {
			provenance: "local_safety_check",
			validation: "local",
		},
		saved_query_id: {
			provenance: "home_data_source",
			validation: "reference",
		},
		saved_query_parameters: {
			provenance: "home_data_source",
			validation: "reference",
		},
		table_name: {
			provenance: "home_data_source",
			validation: "reference",
		},
	};
	return {
		kind,
		...details[kind],
		...(discoverWith ? { discover_with: discoverWith } : {}),
		...dependencies,
	};
};

export const text = (
	description: string,
	options: Partial<HomeWidgetConfigFieldContract> = {},
): HomeWidgetConfigFieldContract => ({
	type: "string",
	description,
	...options,
});

export const number = (
	description: string,
	options: Partial<HomeWidgetConfigFieldContract> = {},
): HomeWidgetConfigFieldContract => ({
	type: "number",
	description,
	...options,
});

export const boolean = (
	description: string,
	defaultValue?: boolean,
): HomeWidgetConfigFieldContract => ({
	type: "boolean",
	description,
	...(defaultValue === undefined ? {} : { default: defaultValue }),
});

export const strings = (
	description: string,
	options: Partial<HomeWidgetConfigFieldContract> = {},
): HomeWidgetConfigFieldContract => ({
	type: "string_list",
	description,
	...options,
});

export const count = (defaultValue: number, maximum = 50) =>
	number("Maximum items to show.", {
		default: defaultValue,
		integer: true,
		minimum: 1,
		maximum,
	});

export const profileApp = (
	description: string,
	when?: HomeWidgetConfigCondition,
) =>
	text(description, {
		default: "",
		reference: reference("profile_app_id", "list_apps", { when }),
	});

export const optionalProfileApp = profileApp(
	"Optional app filter from the current profile.",
);

export const profileApps = (
	maximum?: number,
	when?: HomeWidgetConfigCondition,
) =>
	strings("Ordered app ids from the current profile.", {
		unique_items: true,
		...(maximum === undefined ? {} : { max_items: maximum }),
		reference: reference("profile_app_ids", "list_apps", { when }),
	});

export const safeUrl = (description: string) =>
	text(description, {
		reference: reference("safe_home_url"),
	});

export const imageUrl = (
	description: string,
	when?: HomeWidgetConfigCondition,
) =>
	text(description, {
		string_format: "image-url",
		when,
		reference: reference("safe_home_url", undefined, { when }),
	});

export const informationItem: HomeWidgetObjectContract = {
	additional_properties: false,
	fields: {
		id: text("Optional stable item id."),
		title: text("Item heading."),
		body: text("Optional Markdown body."),
		href: safeUrl("Optional item destination."),
		label: text("Optional short label, date, or category."),
		checked: boolean("Checklist completion state.", false),
	},
	required: ["title"],
	unique_by: "id",
};

export const linkItem: HomeWidgetObjectContract = {
	additional_properties: false,
	fields: {
		id: text("Optional stable link id."),
		title: text("Link title."),
		description: text("Optional link description."),
		href: safeUrl("Link destination."),
	},
	required: ["title", "href"],
	unique_by: "id",
};

export const measureItem: HomeWidgetObjectContract = {
	additional_properties: false,
	fields: {
		id: text("Optional stable measure id."),
		aggregation: text("Aggregation to calculate.", {
			enum: HOME_DATA_AGGREGATIONS,
		}),
		field: text("Source field used by non-count aggregations.", {
			reference: reference("data_field", "list_home_data_sources", {
				source_field: "sourceKind",
			}),
		}),
		label: text("Optional display label."),
	},
	required: ["aggregation"],
	requirements: [
		{
			code: "data_measure_field_missing",
			message: "Choose a source field for every aggregation except count.",
			when: { field: "aggregation", not_equals: "count" },
			non_empty: ["field"],
		},
	],
	unique_by: "id",
};

export const filterItem: HomeWidgetObjectContract = {
	additional_properties: false,
	fields: {
		id: text("Optional stable filter id."),
		field: text("Source field to filter.", {
			reference: reference("data_field", "list_home_data_sources", {
				source_field: "sourceKind",
			}),
		}),
		operator: text("Filter comparison.", { enum: HOME_DATA_FILTER_OPERATORS }),
		value: text("Comparison value. Empty and not_empty ignore this field."),
		valueType: text("How the comparison value is converted.", {
			enum: ["text", "number", "boolean", "viewer"],
		}),
	},
	required: ["field", "operator", "valueType"],
	requirements: [
		{
			code: "data_filter_number_invalid",
			message: "A numeric filter needs a finite numeric value.",
			when: {
				all: [
					{ field: "valueType", equals: "number" },
					{ field: "operator", not_in: ["empty", "not_empty"] },
				],
			},
			numeric_strings: ["value"],
		},
		{
			code: "data_filter_boolean_invalid",
			message: "A boolean filter value must be true or false.",
			when: {
				all: [
					{ field: "valueType", equals: "boolean" },
					{ field: "operator", not_in: ["empty", "not_empty"] },
				],
			},
			allowed_values: { value: ["true", "false"] },
		},
	],
	unique_by: "id",
};

export const appDiscoveryFields = (
	defaultSource: string,
	limitMaximum: number,
) =>
	({
		source: text("Which app inventory supplies the widget.", {
			default: defaultSource,
			enum: DISCOVERY_APP_SOURCES,
		}),
		appId: profileApp("Exact app used by a single-app manual selection.", {
			field: "source",
			equals: "manual",
		}),
		appIds: profileApps(limitMaximum, {
			all: [
				{ field: "source", equals: "manual" },
				{ field: "appId", equals: "" },
			],
		}),
		query: text("Case-insensitive app search text."),
		category: text("Exact app category filter.", { enum: APP_CATEGORIES }),
		tag: text("Exact app tag filter."),
		eyebrow: text("Short label above the widget heading."),
	}) satisfies Record<string, HomeWidgetConfigFieldContract>;

export const emptyContract = (
	description: string,
): HomeWidgetConfigContract => ({
	description,
	additional_properties: false,
	fields: {},
});
