import { writeFile } from "node:fs/promises";
import { z } from "zod";
import {
	instantiateAppCapability,
	listAppCapabilities,
} from "../lib/app-build/capabilities";
import {
	approvalParametersSchema,
	crudParametersSchema,
} from "../lib/app-build/capability-contract";
import { compileAppSpec } from "../lib/app-build/compiler";
import {
	appBuildContractGuide,
	appSpecSchema,
} from "../lib/app-build/contract";
import { INTERACTIVE_APP_BUILD_CAPABILITIES } from "../lib/app-build/interactive-scenario-adapter";
import { appBuildStateSchema } from "../lib/app-build/state-contract";

const base = {
	namespace: "native.template-ns",
	page_name: "NATIVE_PAGE_NAME",
	route: "/NATIVE_ROUTE",
};
const inputs = {
	"crud.foundation": {
		...base,
		entity_name: "NATIVE_ENTITY_NAME",
		table_name: "native_table_name",
		fields: [{ name: "native_field", type: "string" }],
		sample_record: { native_field: "NATIVE_RECORD_VALUE" },
		sample_update: { native_field: "NATIVE_UPDATE_VALUE" },
	},
	"approval.foundation": {
		...base,
		request_name: "NATIVE_REQUEST_NAME",
		requests_table_name: "native_requests_table_name",
		audit_table_name: "native_audit_table_name",
		request_fields: [{ name: "native_field", type: "string" }],
		sample_request: { native_field: "NATIVE_REQUEST_VALUE" },
		requester_id: "NATIVE_REQUESTER_ID",
		approver_id: "NATIVE_APPROVER_ID",
	},
};
const fragments = Object.fromEntries(
	Object.entries(inputs).map(([id, parameters]) => [
		id,
		instantiateAppCapability(id, "1.0.0", parameters),
	]),
);
const spec = appBuildContractGuide().example;
const ordering = {
	schema_version: 1,
	name: "Native Unicode 🦀 é",
	requirements: [
		{ id: "coverage", description: "Preserve ordering and values." },
	],
	resources: ["alpha_a", "alpha-a", "alpha.a", "alpha0"].map((key, index) => ({
		key,
		kind: "board",
		depends_on: index === 0 ? ["alpha.a", "alpha-a"] : [],
		requirement_ids: ["coverage"],
		config: { name: `${key} Ä/é`, instruction: "Return the input." },
	})),
	scenarios: [],
};
function setPath(input: unknown, path: (string | number)[], value: unknown) {
	let parent = input as Record<string | number, unknown>;
	for (const key of path.slice(0, -1)) {
		parent = parent[key] as Record<string | number, unknown>;
	}
	parent[path[path.length - 1]] = value;
}
const numbers = structuredClone(spec);
setPath(numbers, ["resources", 2, "config", "config"], {
	message: "🦀 é Ä",
	rates: [1.5, 0.000001, 1e-7, 1e21, -0],
});
const invalid = [
	{ path: ["resources", 0, "depends_on"], value: ["missing"] },
	{ path: ["resources", 0, "depends_on"], value: [spec.resources[1].key] },
	{ path: ["resources", 0, "physical_id"], value: "model-authored" },
	{
		path: ["resources", 0, "config", "columns", 0],
		value: { name: "embedding", type: "vector" },
	},
	{ path: ["scenarios", 0, "assertions", 0, "status_path"], value: "/status" },
	{
		path: ["scenarios", 0, "steps", 0, "arguments"],
		value: { value: { $scenario_ref: { source: "step", step_id: "submit" } } },
	},
].map(({ path, value }) => {
	const input = structuredClone(spec);
	setPath(input, path, value);
	return input;
});
const recipeInputs = [
	{
		id: "crud.foundation",
		version: "1.0.0",
		parameters: {
			namespace: "stock.catalog",
			entity_name: "Crème supplies 🦀",
			table_name: "supplies",
			page_name: "Stock",
			route: "/stock",
			fields: [
				{ name: "name", type: "string" },
				{ name: "quantity", type: "int64" },
				{ name: "note", type: "string", nullable: true },
			],
			sample_record: { name: "Tea", quantity: 2, note: null },
			sample_update: { quantity: 4, name: "Coffee" },
		},
	},
	{
		id: "approval.foundation",
		version: "1.0.0",
		parameters: {
			namespace: "travel.requests",
			request_name: "Travel request",
			requests_table_name: "travel_requests",
			audit_table_name: "travel_audit",
			page_name: "Travel",
			route: "/travel",
			requester_id: "requester-1",
			approver_id: "reviewer-1",
			request_fields: [
				{ name: "destination", type: "string" },
				{ name: "budget", type: "float64" },
			],
			sample_request: { destination: "München", budget: 120.5 },
		},
	},
];
const data = {
	recipe_vectors: recipeInputs.map((input) => ({
		...input,
		output: instantiateAppCapability(input.id, input.version, input.parameters),
	})),
	guide: appBuildContractGuide(),
	host_capabilities: INTERACTIVE_APP_BUILD_CAPABILITIES,
	spec_schema: z.toJSONSchema(appSpecSchema, { io: "input" }),
	state_schema: z.toJSONSchema(appBuildStateSchema, { io: "input" }),
	capabilities: listAppCapabilities(),
	parameter_schemas: {
		"crud.foundation": z.toJSONSchema(crudParametersSchema, { io: "input" }),
		"approval.foundation": z.toJSONSchema(approvalParametersSchema, {
			io: "input",
		}),
	},
	template_parameters: inputs,
	templates: fragments,
	invalid_vectors: invalid,
	compiler_vectors: [
		...[ordering, numbers].map((input) => ({
			input,
			app_id: "native_app",
			build_id: "native_build",
			output: compileAppSpec(input, {
				app_id: "native_app",
				build_id: "native_build",
			}),
		})),
		{
			input: spec,
			app_id: "native_app",
			build_id: "native_build",
			output: compileAppSpec(spec, {
				app_id: "native_app",
				build_id: "native_build",
			}),
		},
		...Object.values(fragments).map((fragment) => {
			const input = {
				schema_version: 1,
				name: "Native compiler parity",
				requirements: fragment.requirements,
				resources: fragment.resources,
				scenarios: fragment.scenarios,
			};
			return {
				input,
				app_id: "native_app",
				build_id: "native_build",
				output: compileAppSpec(input, {
					app_id: "native_app",
					build_id: "native_build",
				}),
			};
		}),
	],
};
await writeFile(
	new URL(
		"../lib/flowpilot/generated/app-build-contracts.json",
		import.meta.url,
	),
	`${JSON.stringify(data, null, 2)}\n`,
);
