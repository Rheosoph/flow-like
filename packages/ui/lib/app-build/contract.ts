import { z } from "zod";

import {
	APP_BEHAVIOR_SCENARIO_SCHEMA,
	APP_SCENARIO_TOOLS,
	appBehaviorScenarioSchema,
	type AppBehaviorScenario,
} from "./scenario-contract";

export const APP_SPEC_SCHEMA_VERSION = 1 as const;
/** Leaves room for build receipts and evidence under the host's 1 MiB checkpoint cap. */
export const MAX_APP_SPEC_BYTES = 512 * 1024;
export const APP_RESOURCE_KINDS = [
	"board",
	"page",
	"widget",
	"table",
	"event",
] as const;

export type AppResourceKind = (typeof APP_RESOURCE_KINDS)[number];
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | JsonObject;
export type JsonObject = { [key: string]: JsonValue };

export function serializedJsonByteLength(value: unknown): number {
	const serialized = JSON.stringify(value);
	if (serialized === undefined) {
		throw new TypeError("Expected a JSON-serializable value.");
	}
	return new TextEncoder().encode(serialized).byteLength;
}

export const logicalKeySchema = z
	.string()
	.trim()
	.min(1)
	.max(128)
	.regex(
		/^[a-z][a-z0-9._-]*$/,
		"must be a stable lowercase identifier beginning with a letter",
	);

// Entry selectors resolve against persisted node IDs and names, not logical resource keys.
const entrySelectorSchema = z
	.string()
	.min(1)
	.max(128)
	.refine(
		(value) => value.trim().length > 0,
		"must name an exact persisted workflow entry",
	);

export const jsonValueSchema: z.ZodType<JsonValue> = z.lazy(() =>
	z.union([
		z.string(),
		z.number().finite(),
		z.boolean(),
		z.null(),
		z.array(jsonValueSchema),
		z.record(jsonValueSchema),
	]),
);

export const jsonObjectSchema: z.ZodType<JsonObject> =
	z.record(jsonValueSchema);

const shortText = z.string().trim().min(1).max(512);
const instruction = z.string().trim().min(1).max(40_000);
const requirementIds = z.array(logicalKeySchema).max(128);
const dependencies = z.array(logicalKeySchema).max(128);

export const tableColumnTypeSchema = z.enum([
	"string",
	"boolean",
	"int8",
	"int16",
	"int32",
	"int64",
	"uint8",
	"uint16",
	"uint32",
	"uint64",
	"float32",
	"float64",
	"binary",
	"date32",
	"timestamp:ms:UTC",
	"vector",
]);

export const tableColumnSchema = z
	.object({
		name: z.string().trim().min(1).max(128),
		type: tableColumnTypeSchema,
		nullable: z.boolean().optional(),
		vector_size: z.number().int().positive().max(65_536).optional(),
	})
	.strict()
	.superRefine((column, context) => {
		if (column.type === "vector" && column.vector_size === undefined) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "vector columns require vector_size",
				path: ["vector_size"],
			});
		}
		if (column.type !== "vector" && column.vector_size !== undefined) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "vector_size is only valid for vector columns",
				path: ["vector_size"],
			});
		}
	});

const resourceBase = {
	key: logicalKeySchema,
	depends_on: dependencies,
	requirement_ids: requirementIds,
};

export const boardResourceSchema = z
	.object({
		...resourceBase,
		kind: z.literal("board"),
		config: z
			.object({
				name: shortText,
				instruction,
			})
			.strict(),
	})
	.strict();

export const pageResourceSchema = z
	.object({
		...resourceBase,
		kind: z.literal("page"),
		config: z
			.object({
				name: shortText,
				route: z.string().trim().min(1).max(256).startsWith("/"),
				board: logicalKeySchema,
				instruction,
				on_load_entry: entrySelectorSchema.optional(),
				on_unload_entry: entrySelectorSchema.optional(),
				on_interval_entry: entrySelectorSchema.optional(),
				interval_seconds: z.number().int().positive().max(86_400).optional(),
			})
			.strict()
			.superRefine((config, context) => {
				if (
					(config.on_interval_entry === undefined) !==
					(config.interval_seconds === undefined)
				) {
					context.addIssue({
						code: z.ZodIssueCode.custom,
						message:
							"on_interval_entry and interval_seconds must be provided together",
						path: [
							config.on_interval_entry === undefined
								? "on_interval_entry"
								: "interval_seconds",
						],
					});
				}
			}),
	})
	.strict();

export const widgetResourceSchema = z
	.object({
		...resourceBase,
		kind: z.literal("widget"),
		config: z
			.object({
				name: shortText,
				instruction,
			})
			.strict(),
	})
	.strict();

export const tableResourceSchema = z
	.object({
		...resourceBase,
		kind: z.literal("table"),
		config: z
			.object({
				name: z.string().trim().min(1).max(128),
				columns: z.array(tableColumnSchema).min(1).max(256),
			})
			.strict(),
	})
	.strict();

export const appEventTypeSchema = z.enum([
	"page",
	"quick_action",
	"api",
	"cron",
	"daemon",
	"geolocation",
	"deeplink",
	"rest",
	"mcp",
	"generic_form",
	"simple_chat",
	"discord",
	"telegram",
]);

export const eventResourceSchema = z
	.object({
		...resourceBase,
		kind: z.literal("event"),
		config: z
			.object({
				name: shortText,
				event_type: appEventTypeSchema,
				board: logicalKeySchema.optional(),
				page: logicalKeySchema.optional(),
				entry_node: entrySelectorSchema.optional(),
				route: z.string().trim().min(1).max(256).startsWith("/").optional(),
				config: jsonObjectSchema.optional(),
			})
			.strict(),
	})
	.strict();

export const appResourceSchema = z.discriminatedUnion("kind", [
	boardResourceSchema,
	pageResourceSchema,
	widgetResourceSchema,
	tableResourceSchema,
	eventResourceSchema,
]);

export const appRequirementSchema = z
	.object({
		id: logicalKeySchema,
		description: z.string().trim().min(1).max(2_000),
	})
	.strict();

export const appSpecSchema = z
	.object({
		schema_version: z.literal(APP_SPEC_SCHEMA_VERSION),
		name: z.string().trim().min(1).max(256),
		description: z.string().trim().min(1).max(4_000).optional(),
		requirements: z.array(appRequirementSchema).min(1).max(256),
		resources: z.array(appResourceSchema).min(1).max(256),
		scenarios: z.array(appBehaviorScenarioSchema).max(64),
	})
	.strict()
	.superRefine((spec, context) => {
		const actual = serializedJsonByteLength(spec);
		if (actual > MAX_APP_SPEC_BYTES) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: `AppSpec is ${actual} bytes; the maximum is ${MAX_APP_SPEC_BYTES} bytes so durable build state retains space for receipts and evidence.`,
				path: [],
			});
		}
	});

export type AppRequirement = z.infer<typeof appRequirementSchema>;
export type AppResource = z.infer<typeof appResourceSchema>;
export type BoardResource = z.infer<typeof boardResourceSchema>;
export type PageResource = z.infer<typeof pageResourceSchema>;
export type WidgetResource = z.infer<typeof widgetResourceSchema>;
export type TableResource = z.infer<typeof tableResourceSchema>;
export type EventResource = z.infer<typeof eventResourceSchema>;
export type AppSpec = z.infer<typeof appSpecSchema>;
export type { AppBehaviorScenario };

export function parseAppSpec(input: unknown): AppSpec {
	return appSpecSchema.parse(input);
}

/** JSON-serializable contract returned by the lightweight `app_build` capabilities operation. */
export function appBuildContractGuide() {
	return {
		contract: "flowpilot.app-spec/v1",
		schema_version: APP_SPEC_SCHEMA_VERSION,
		additional_properties: false,
		required_fields: [
			"schema_version",
			"name",
			"requirements",
			"resources",
			"scenarios",
		],
		resource_shape: {
			required_fields: [
				"key",
				"kind",
				"depends_on",
				"requirement_ids",
				"config",
			],
			kinds: {
				board: { config: ["name", "instruction"] },
				page: {
					config: [
						"name",
						"route",
						"board",
						"instruction",
						"on_load_entry?",
						"on_unload_entry?",
						"on_interval_entry?",
						"interval_seconds?",
					],
				},
				widget: { config: ["name", "instruction"] },
				table: {
					config: ["name", "columns"],
					column: ["name", "type", "nullable?", "vector_size?"],
				},
				event: {
					config: [
						"name",
						"event_type",
						"board?",
						"page?",
						"entry_node?",
						"route?",
						"config?",
					],
				},
			},
		},
		scenario_shape: {
			schema: APP_BEHAVIOR_SCENARIO_SCHEMA,
			tools: [...APP_SCENARIO_TOOLS],
			required_fields: [
				"schema",
				"id",
				"description",
				"requirement_ids",
				"target",
				"steps",
				"assertions",
			],
		},
		invariants: [
			"Every resource, dependency, requirement, scenario, step, and assertion identifier is unique in its scope.",
			"Every symbolic resource reference exists, has the expected kind, and appears in depends_on.",
			"The resource graph is acyclic.",
			"Every requirement is covered by at least one resource or behavioral scenario.",
			"Promotion requires every requirement to be covered by at least one passed host-certified behavioral scenario.",
			"A page Event targets a page; every other Event targets a board and exact entry_node.",
			"Page lifecycle entries select exact runnable entries on the page's board; on_interval_entry and interval_seconds appear together.",
			"Physical resource ids are host-reserved and must never be included in AppSpec.",
		],
		example: {
			schema_version: 1,
			name: "Issue inbox",
			description: "Collect and review incoming issues.",
			requirements: [{ id: "capture", description: "Save a submitted issue." }],
			resources: [
				{
					key: "issues",
					kind: "table",
					depends_on: [],
					requirement_ids: ["capture"],
					config: {
						name: "issues",
						columns: [
							{ name: "id", type: "string", nullable: false },
							{ name: "title", type: "string", nullable: false },
						],
					},
				},
				{
					key: "submit_issue",
					kind: "board",
					depends_on: ["issues"],
					requirement_ids: ["capture"],
					config: {
						name: "Submit issue",
						instruction: "Validate and insert one issue into issues.",
					},
				},
				{
					key: "submit_issue_event",
					kind: "event",
					depends_on: ["submit_issue"],
					requirement_ids: ["capture"],
					config: {
						name: "Submit issue",
						event_type: "quick_action",
						board: "submit_issue",
						entry_node: "submit",
					},
				},
			],
			scenarios: [
				{
					schema: APP_BEHAVIOR_SCENARIO_SCHEMA,
					id: "submit_persists_issue",
					description:
						"A valid submission starts a run and persists the issue.",
					requirement_ids: ["capture"],
					target: { resource_key: "submit_issue_event", kind: "event" },
					steps: [
						{
							id: "submit",
							tool: "call_app_event",
							arguments: { payload: { title: "Example issue" } },
						},
					],
					assertions: [
						{
							id: "run_started",
							kind: "run_outcome",
							step_id: "submit",
							run_id_path: "/run_id",
						},
						{
							id: "row_persisted",
							kind: "state",
							step_id: "submit",
							path: "/issues/0/title",
							operator: "equals",
							expected: "Example issue",
						},
					],
				},
			],
		},
	} as const;
}
