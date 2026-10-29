import { z } from "zod";

export const APP_BEHAVIOR_SCENARIO_SCHEMA =
	"flowpilot.app-behavior-scenario/v1" as const;

export const APP_SCENARIO_READ_ONLY_TOOLS = [
	"describe_app_interface",
	"query_execution_logs",
] as const;

export const APP_SCENARIO_RUNTIME_TOOLS = [
	"call_app_event",
	"call_app_chat",
	"interact_app_page",
] as const;

export const APP_SCENARIO_TOOLS = [
	...APP_SCENARIO_READ_ONLY_TOOLS,
	...APP_SCENARIO_RUNTIME_TOOLS,
] as const;

export type AppScenarioReadOnlyTool =
	(typeof APP_SCENARIO_READ_ONLY_TOOLS)[number];
export type AppScenarioRuntimeTool =
	(typeof APP_SCENARIO_RUNTIME_TOOLS)[number];
export type AppScenarioTool = (typeof APP_SCENARIO_TOOLS)[number];

export type ScenarioJsonPrimitive = string | number | boolean | null;

export interface AppScenarioReference {
	readonly $scenario_ref: {
		readonly source: "app" | "target" | "step";
		readonly step_id?: string;
		/** RFC 6901 JSON Pointer. An empty pointer selects the whole source value. */
		readonly path?: string;
	};
}

export interface ScenarioJsonArray extends ReadonlyArray<ScenarioJsonValue> {}

export interface ScenarioJsonObject {
	readonly [key: string]: ScenarioJsonValue;
}

export type ScenarioJsonValue =
	| ScenarioJsonPrimitive
	| AppScenarioReference
	| ScenarioJsonArray
	| ScenarioJsonObject;

export interface AppScenarioTarget {
	readonly resource_key: string;
	readonly kind: "app" | "chat" | "event" | "page";
}

export interface AppScenarioStep {
	readonly id: string;
	readonly tool: AppScenarioTool;
	readonly arguments: ScenarioJsonObject;
}

export type AppScenarioJsonAssertionOperator =
	| "exists"
	| "equals"
	| "not_equals"
	| "contains"
	| "length_at_least";

export interface AppScenarioJsonAssertion {
	readonly id: string;
	readonly kind: "json";
	readonly step_id: string;
	/** RFC 6901 JSON Pointer into the named step's fresh observation. */
	readonly path: string;
	readonly operator: AppScenarioJsonAssertionOperator;
	readonly expected?: ScenarioJsonValue;
}

/**
 * Assert host-observed domain state after a runtime invocation. The state is supplied by the
 * trusted adapter from the isolated runtime, never by scenario-authored tool arguments.
 */
export interface AppScenarioStateAssertion {
	readonly id: string;
	readonly kind: "state";
	readonly step_id: string;
	/** RFC 6901 JSON Pointer into the named step's host-observed state. */
	readonly path: string;
	readonly operator: AppScenarioJsonAssertionOperator;
	readonly expected?: ScenarioJsonValue;
}

export interface AppScenarioRunOutcomeAssertion {
	readonly id: string;
	readonly kind: "run_outcome";
	readonly step_id: string;
	/** Pointer to a non-empty run id, proving that this invocation started a run. */
	readonly run_id_path: string;
	/** Optional pointer to a terminal status in the same observation. */
	readonly status_path?: string;
	readonly allowed_statuses?: readonly string[];
}

export type AppScenarioAssertion =
	| AppScenarioJsonAssertion
	| AppScenarioStateAssertion
	| AppScenarioRunOutcomeAssertion;

export interface AppBehaviorScenario {
	readonly schema: typeof APP_BEHAVIOR_SCENARIO_SCHEMA;
	readonly id: string;
	readonly description: string;
	readonly requirement_ids: readonly string[];
	readonly target: AppScenarioTarget;
	/** The runner clamps this to its own ceiling. */
	readonly timeout_ms?: number;
	readonly steps: readonly AppScenarioStep[];
	readonly assertions: readonly AppScenarioAssertion[];
}

const IDENTIFIER_PATTERN = /^[a-z][a-z0-9._-]{0,127}$/;
const MAX_SCENARIO_STEPS = 24;
const MAX_SCENARIO_ASSERTIONS = 64;

function isJsonPointer(value: string): boolean {
	if (value === "") return true;
	if (!value.startsWith("/")) return false;
	for (let index = 0; index < value.length; index += 1) {
		if (value[index] !== "~") continue;
		const escapeCode = value[index + 1];
		if (escapeCode !== "0" && escapeCode !== "1") return false;
		index += 1;
	}
	return true;
}

const identifierSchema = z
	.string()
	.min(1)
	.max(128)
	.regex(IDENTIFIER_PATTERN, "must be a stable lowercase identifier");

const jsonPointerSchema = z
	.string()
	.max(512)
	.refine(isJsonPointer, "must be an RFC 6901 JSON Pointer");

export const appScenarioReferenceSchema = z
	.object({
		$scenario_ref: z
			.object({
				source: z.enum(["app", "target", "step"]),
				step_id: identifierSchema.optional(),
				path: jsonPointerSchema.optional(),
			})
			.strict()
			.superRefine((reference, context) => {
				if (reference.source === "step" && !reference.step_id) {
					context.addIssue({
						code: z.ZodIssueCode.custom,
						message: "step references require step_id",
						path: ["step_id"],
					});
				}
				if (reference.source !== "step" && reference.step_id) {
					context.addIssue({
						code: z.ZodIssueCode.custom,
						message: "step_id is only valid for step references",
						path: ["step_id"],
					});
				}
			}),
	})
	.strict();

export const scenarioJsonValueSchema: z.ZodType<ScenarioJsonValue> = z.lazy(
	() =>
		z.union([
			z.string(),
			z.number().finite(),
			z.boolean(),
			z.null(),
			appScenarioReferenceSchema,
			z.array(scenarioJsonValueSchema),
			z.record(z.string(), scenarioJsonValueSchema),
		]),
);

export const scenarioJsonObjectSchema = z.record(z.string(), scenarioJsonValueSchema);

function valueAssertionSchema<K extends "json" | "state">(kind: K) {
	return z
		.object({
			id: identifierSchema,
			kind: z.literal(kind),
			step_id: identifierSchema,
			path: jsonPointerSchema,
			operator: z.enum([
				"exists",
				"equals",
				"not_equals",
				"contains",
				"length_at_least",
			]),
			expected: scenarioJsonValueSchema.optional(),
		})
		.strict()
		.superRefine((assertion, context) => {
			const hasExpected = Object.hasOwn(assertion, "expected");
			if (assertion.operator === "exists" && hasExpected) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: "exists assertions do not accept expected",
					path: ["expected"],
				});
				return;
			}
			if (assertion.operator !== "exists" && !hasExpected) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: `${assertion.operator} assertions require expected`,
					path: ["expected"],
				});
			}
			if (
				assertion.operator === "length_at_least" &&
				(!Number.isSafeInteger(assertion.expected) ||
					Number(assertion.expected) < 0)
			) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: "length_at_least expected must be a non-negative integer",
					path: ["expected"],
				});
			}
		});
}

const jsonAssertionSchema = valueAssertionSchema("json");
const stateAssertionSchema = valueAssertionSchema("state");

const runOutcomeAssertionSchema = z
	.object({
		id: identifierSchema,
		kind: z.literal("run_outcome"),
		step_id: identifierSchema,
		run_id_path: jsonPointerSchema,
		status_path: jsonPointerSchema.optional(),
		allowed_statuses: z
			.array(z.string().min(1).max(64))
			.min(1)
			.max(16)
			.optional(),
	})
	.strict()
	.superRefine((assertion, context) => {
		if (assertion.run_id_path === "") {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "run_id_path must select a field, not the whole response",
				path: ["run_id_path"],
			});
		}
		if (assertion.status_path === "/status") {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message:
					"root /status is a transport acknowledgement and cannot prove terminal workflow success",
				path: ["status_path"],
			});
		}
		if (assertion.allowed_statuses && !assertion.status_path) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "allowed_statuses requires status_path",
				path: ["allowed_statuses"],
			});
		}
	});

const rawAppBehaviorScenarioSchema = z
	.object({
		schema: z.literal(APP_BEHAVIOR_SCENARIO_SCHEMA),
		id: identifierSchema,
		description: z.string().trim().min(1).max(2_000),
		requirement_ids: z.array(identifierSchema).min(1).max(64),
		target: z
			.object({
				resource_key: identifierSchema,
				kind: z.enum(["app", "chat", "event", "page"]),
			})
			.strict(),
		timeout_ms: z
			.number()
			.int()
			.min(100)
			.max(10 * 60_000)
			.optional(),
		steps: z
			.array(
				z
					.object({
						id: identifierSchema,
						tool: z.enum(APP_SCENARIO_TOOLS),
						arguments: scenarioJsonObjectSchema,
					})
					.strict(),
			)
			.min(1)
			.max(MAX_SCENARIO_STEPS),
		assertions: z
			.array(
				z.union([
					jsonAssertionSchema,
					stateAssertionSchema,
					runOutcomeAssertionSchema,
				]),
			)
			.min(1, "a behavioral scenario must contain at least one assertion")
			.max(MAX_SCENARIO_ASSERTIONS),
	})
	.strict()
	.superRefine((scenario, context) => {
		const stepIndexes = new Map<string, number>();
		for (const [index, step] of scenario.steps.entries()) {
			if (stepIndexes.has(step.id)) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: `duplicate step id ${step.id}`,
					path: ["steps", index, "id"],
				});
			} else {
				stepIndexes.set(step.id, index);
			}
		}

		const assertionIds = new Set<string>();
		for (const [index, assertion] of scenario.assertions.entries()) {
			if (assertionIds.has(assertion.id)) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: `duplicate assertion id ${assertion.id}`,
					path: ["assertions", index, "id"],
				});
			}
			assertionIds.add(assertion.id);
			if (!stepIndexes.has(assertion.step_id)) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: `assertion references unknown step ${assertion.step_id}`,
					path: ["assertions", index, "step_id"],
				});
			}
			const referencedStep =
				scenario.steps[stepIndexes.get(assertion.step_id) ?? -1];
			if (
				(assertion.kind === "state" || assertion.kind === "run_outcome") &&
				referencedStep &&
				!APP_SCENARIO_RUNTIME_TOOLS.includes(
					referencedStep.tool as AppScenarioRuntimeTool,
				)
			) {
				context.addIssue({
					code: z.ZodIssueCode.custom,
					message: `${assertion.kind} assertions require a runtime step`,
					path: ["assertions", index, "step_id"],
				});
			}
		}

		const visitReferences = (
			value: ScenarioJsonValue,
			stepIndex: number,
			path: readonly (string | number)[],
		) => {
			if (!value || typeof value !== "object") return;
			const reference = !Array.isArray(value)
				? (value as Partial<AppScenarioReference>).$scenario_ref
				: undefined;
			if (reference !== undefined && reference.source === "step") {
				const referencedIndex = stepIndexes.get(reference.step_id ?? "");
				if (referencedIndex === undefined || referencedIndex >= stepIndex) {
					context.addIssue({
						code: z.ZodIssueCode.custom,
						message:
							"step references must name an earlier step in the same scenario",
						path: [...path, "$scenario_ref", "step_id"],
					});
				}
				return;
			}
			if (Array.isArray(value)) {
				for (const [index, entry] of value.entries()) {
					visitReferences(entry, stepIndex, [...path, index]);
				}
				return;
			}
			for (const [key, entry] of Object.entries(value)) {
				visitReferences(entry, stepIndex, [...path, key]);
			}
		};

		for (const [index, step] of scenario.steps.entries()) {
			visitReferences(step.arguments, index, ["steps", index, "arguments"]);
		}
	});

export const appBehaviorScenarioSchema =
	rawAppBehaviorScenarioSchema as unknown as z.ZodType<AppBehaviorScenario>;
