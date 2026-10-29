export const KNOWN_HOME_WIDGET_TYPES = [
	"ai-usage",
	"app-collection",
	"app-collection-feature",
	"app-embed",
	"app-ranking",
	"app-spotlight",
	"categories",
	"data",
	"executions-by-app",
	"flowpilot",
	"greeting",
	"information",
	"model-spotlight",
	"models",
	"needs-attention",
	"notifications",
	"packages",
	"quick-actions",
	"quick-links",
	"recent-runs",
	"run-activity",
	"run-stats",
	"saved-forecast",
	"schedules",
	"section-heading",
	"workspace-pulse",
] as const;

export type KnownHomeWidgetType = (typeof KNOWN_HOME_WIDGET_TYPES)[number];

export type HomeWidgetConfigReferenceKind =
	| "app_event_id"
	| "app_route"
	| "data_field"
	| "model_hub"
	| "model_id"
	| "ontology_id"
	| "ontology_object_type"
	| "profile_app_id"
	| "profile_app_ids"
	| "safe_home_url"
	| "saved_query_id"
	| "saved_query_parameters"
	| "table_name";

export interface HomeWidgetConfigReferenceContract {
	kind: HomeWidgetConfigReferenceKind;
	provenance:
		| "app_interface"
		| "current_profile"
		| "home_data_source"
		| "local_safety_check"
		| "manual_profile_model";
	validation:
		| "local"
		| "reference"
		| "reference_when_schema_available"
		| "unverified";
	discover_with?: string;
	inspect_with?: string;
	note?: string;
	app_field?: string;
	scope_field?: string;
	source_field?: string;
	when?: HomeWidgetConfigCondition;
}

export type ContractScalar = string | number | boolean | null;

export interface HomeWidgetConfigCondition {
	field?: string;
	equals?: ContractScalar;
	in?: readonly ContractScalar[];
	not_equals?: ContractScalar;
	not_in?: readonly ContractScalar[];
	all?: readonly HomeWidgetConfigCondition[];
}

export interface HomeWidgetConfigRequirement {
	code: string;
	message: string;
	severity?: "error" | "warning";
	when?: HomeWidgetConfigCondition;
	required?: readonly string[];
	non_empty?: readonly string[];
	any_non_empty?: readonly string[];
	positive?: readonly string[];
	empty?: readonly string[];
	equals?: Readonly<Record<string, ContractScalar>>;
	numeric_strings?: readonly string[];
	allowed_values?: Readonly<Record<string, readonly ContractScalar[]>>;
}

export interface HomeWidgetObjectContract {
	additional_properties: boolean;
	fields: Readonly<Record<string, HomeWidgetConfigFieldContract>>;
	required?: readonly string[];
	requirements?: readonly HomeWidgetConfigRequirement[];
	unique_by?: string;
}

export interface HomeWidgetConfigFieldContract {
	type:
		| "boolean"
		| "number"
		| "object"
		| "object_list"
		| "string"
		| "string_list";
	description?: string;
	when?: HomeWidgetConfigCondition;
	default?: unknown;
	enum?: readonly ContractScalar[];
	legacy_values?: readonly {
		value: ContractScalar;
		maps_to: ContractScalar;
	}[];
	minimum?: number;
	exclusive_minimum?: number;
	maximum?: number;
	integer?: boolean;
	nullable?: boolean;
	min_items?: number;
	max_items?: number;
	unique_items?: boolean;
	item_enum?: readonly string[];
	item?: HomeWidgetObjectContract;
	pattern?: string;
	string_format?: "date-time" | "image-url" | "route" | "storage-image-path";
	reference?: HomeWidgetConfigReferenceContract;
	ranges?: readonly { minimum: number; maximum: number }[];
}

export interface HomeWidgetConfigContract extends HomeWidgetObjectContract {
	description: string;
	relations?: readonly Record<string, unknown>[];
}

export interface HomeWidgetConfigIssue {
	severity: "error" | "warning";
	code: string;
	path: string;
	message: string;
}
