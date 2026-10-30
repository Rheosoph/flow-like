import {
	APP_CAPABILITY_FRAGMENT_SCHEMA,
	type AppCapabilityFragment,
	type ApprovalFoundationParameters,
	type CapabilityDefinition,
	type CrudFoundationParameters,
	approvalParametersSchema,
	baseColumns,
	crudParametersSchema,
	escapeJsonPointerSegment,
	interfaceScenario,
	requirement,
	runtimeScenarioAssertions,
} from "./capability-contract";
import type { AppBehaviorScenario, AppResource, JsonObject } from "./contract";
import { APP_BEHAVIOR_SCENARIO_SCHEMA } from "./scenario-contract";

function instantiateCrud(
	parameters: CrudFoundationParameters,
): Omit<AppCapabilityFragment, "capability"> {
	const ns = parameters.namespace;
	const tableKey = `${ns}.records`;
	const boardKey = `${ns}.workflow`;
	const pageKey = `${ns}.page`;
	const pageEventKey = `${ns}.page_event`;
	const commandEventKey = `${ns}.command_event`;
	const requirementIds = {
		list: `${ns}.list`,
		create: `${ns}.create`,
		update: `${ns}.update`,
		delete: `${ns}.delete`,
		interface: `${ns}.interface`,
		runtime: `${ns}.runtime`,
	};
	const requirements = [
		requirement(
			requirementIds.list,
			`List persisted ${parameters.entity_name} records.`,
		),
		requirement(
			requirementIds.create,
			`Validate and create one ${parameters.entity_name}.`,
		),
		requirement(
			requirementIds.update,
			`Update an existing ${parameters.entity_name} by id.`,
		),
		requirement(
			requirementIds.delete,
			`Delete an existing ${parameters.entity_name} by id.`,
		),
		requirement(
			requirementIds.interface,
			"Expose the CRUD workflow through an active page and headless Event.",
		),
		requirement(
			requirementIds.runtime,
			"Exercise CRUD only inside a host-attested isolated runtime.",
		),
	];
	const allCrudRequirements = [
		requirementIds.list,
		requirementIds.create,
		requirementIds.update,
		requirementIds.delete,
	];
	const resources: AppResource[] = [
		{
			key: tableKey,
			kind: "table",
			depends_on: [],
			requirement_ids: allCrudRequirements,
			config: {
				name: parameters.table_name,
				columns: [...baseColumns(), ...parameters.fields],
			},
		},
		{
			key: boardKey,
			kind: "board",
			depends_on: [tableKey],
			requirement_ids: [...allCrudRequirements, requirementIds.runtime],
			config: {
				name: `${parameters.page_name} workflow`,
				instruction: `Use the board specialist and catalog discovery to implement one persisted CRUD command entry named ${ns.replace(/[^a-z0-9_]/g, "_")}_command. Accept operation list/create/read/update/delete, validate payloads, read writes back from table ${parameters.table_name}, return explicit not-found/conflict outcomes, and make duplicate command delivery idempotent. Return the source run id plus domain output that the isolated host can read back as: create/read/update { record }, delete { deleted_id, exists_after_delete }, and list { records, probe_present }. Do not assume catalog node type names.`,
			},
		},
		{
			key: pageKey,
			kind: "page",
			depends_on: [boardKey],
			requirement_ids: [...allCrudRequirements, requirementIds.interface],
			config: {
				name: parameters.page_name,
				route: parameters.route,
				board: boardKey,
				instruction: `Use the UI specialist to build a ${parameters.entity_name} list plus create/edit/delete controls. Wire the real persisted page and component ids to ${boardKey}; include loading, empty, validation, conflict, and error states.`,
			},
		},
		{
			key: pageEventKey,
			kind: "event",
			depends_on: [pageKey],
			requirement_ids: [requirementIds.interface],
			config: {
				name: `${parameters.page_name} page`,
				event_type: "page",
				page: pageKey,
				route: parameters.route,
			},
		},
		{
			key: commandEventKey,
			kind: "event",
			depends_on: [boardKey],
			requirement_ids: [
				...allCrudRequirements,
				requirementIds.interface,
				requirementIds.runtime,
			],
			config: {
				name: `${parameters.page_name} command`,
				event_type: "quick_action",
				board: boardKey,
				entry_node: `${ns.replace(/[^a-z0-9_]/g, "_")}_command`,
			},
		},
	];
	const probeId = `__flowpilot_${ns.replace(/[^a-z0-9_]/g, "_")}_probe__`;
	const operations = [
		{
			id: "create",
			payload: {
				operation: "create",
				record: { id: probeId, ...parameters.sample_record },
			},
		},
		{ id: "read", payload: { operation: "read", id: probeId } },
		{
			id: "update",
			payload: {
				operation: "update",
				id: probeId,
				changes: parameters.sample_update,
			},
		},
		{ id: "delete", payload: { operation: "delete", id: probeId } },
		{ id: "list", payload: { operation: "list", limit: 5 } },
	] as const;
	const scenarios: AppBehaviorScenario[] = [
		interfaceScenario(ns, commandEventKey, requirementIds.interface),
		{
			schema: APP_BEHAVIOR_SCENARIO_SCHEMA,
			id: `${ns}.crud_lifecycle`,
			description:
				"Run the CRUD lifecycle against disposable data in one isolated runtime.",
			requirement_ids: [...allCrudRequirements, requirementIds.runtime],
			target: { resource_key: commandEventKey, kind: "event" },
			timeout_ms: 120_000,
			steps: operations.map((operation) => ({
				id: operation.id,
				tool: "call_app_event" as const,
				arguments: { payload: operation.payload },
			})),
			assertions: [
				...runtimeScenarioAssertions("create", [
					{
						id: "record_id_matches",
						path: "/record/id",
						operator: "equals",
						expected: probeId,
					},
				]),
				...runtimeScenarioAssertions("read", [
					{
						id: "record_id_matches",
						path: "/record/id",
						operator: "equals",
						expected: probeId,
					},
				]),
				...runtimeScenarioAssertions("update", [
					{
						id: "record_id_matches",
						path: "/record/id",
						operator: "equals",
						expected: probeId,
					},
					...Object.entries(parameters.sample_update)
						.sort(([left], [right]) => left.localeCompare(right))
						.map(([field, expected]) => ({
							id: `record_${field.replace(/[^a-z0-9_-]/gi, "_").toLowerCase()}_updated`,
							path: `/record/${escapeJsonPointerSegment(field)}`,
							operator: "equals" as const,
							expected,
						})),
				]),
				...runtimeScenarioAssertions("delete", [
					{
						id: "deleted_id_matches",
						path: "/deleted_id",
						operator: "equals",
						expected: probeId,
					},
					{
						id: "record_absent",
						path: "/exists_after_delete",
						operator: "equals",
						expected: false,
					},
				]),
				...runtimeScenarioAssertions("list", [
					{
						id: "probe_absent",
						path: "/probe_present",
						operator: "equals",
						expected: false,
					},
					{
						id: "records_returned",
						path: "/records",
						operator: "exists",
					},
				]),
			],
		},
	];
	return {
		schema: APP_CAPABILITY_FRAGMENT_SCHEMA,
		generation: {
			mode: "specialist",
			brief: `Generate ${parameters.entity_name} CRUD FlowScript and A2UI from this exact resource contract. Preserve logical keys and let the host resolve physical ids.`,
			exact_template_source: false,
			evidence_limitations: [
				"This recipe contains no pre-certified FlowScript or catalog node names.",
				"Runtime scenarios remain outstanding until a host executes them in attested isolation.",
			],
		},
		requirements,
		resources,
		scenarios,
	};
}

function instantiateApproval(
	parameters: ApprovalFoundationParameters,
): Omit<AppCapabilityFragment, "capability"> {
	const ns = parameters.namespace;
	const requestsKey = `${ns}.requests`;
	const auditKey = `${ns}.audit`;
	const boardKey = `${ns}.workflow`;
	const pageKey = `${ns}.page`;
	const pageEventKey = `${ns}.page_event`;
	const commandEventKey = `${ns}.command_event`;
	const requirementIds = {
		submit: `${ns}.submit`,
		decide: `${ns}.decide`,
		audit: `${ns}.audit`,
		authorize: `${ns}.authorize`,
		interface: `${ns}.interface`,
		runtime: `${ns}.runtime`,
	};
	const requirements = [
		requirement(
			requirementIds.submit,
			`Submit a pending ${parameters.request_name}.`,
		),
		requirement(
			requirementIds.decide,
			"Approve or reject a pending request exactly once.",
		),
		requirement(
			requirementIds.audit,
			"Append an immutable audit row for every attempted transition.",
		),
		requirement(
			requirementIds.authorize,
			"Require an explicit authorized actor for decisions.",
		),
		requirement(
			requirementIds.interface,
			"Expose the approval queue through an active page and command Event.",
		),
		requirement(
			requirementIds.runtime,
			"Exercise approval only inside a host-attested isolated runtime.",
		),
	];
	const workflowRequirements = Object.values(requirementIds);
	const resources: AppResource[] = [
		{
			key: requestsKey,
			kind: "table",
			depends_on: [],
			requirement_ids: [
				requirementIds.submit,
				requirementIds.decide,
				requirementIds.authorize,
			],
			config: {
				name: parameters.requests_table_name,
				columns: [
					{ name: "id", type: "string", nullable: false },
					{ name: "status", type: "string", nullable: false },
					{ name: "requester_id", type: "string", nullable: false },
					{ name: "submitted_at", type: "timestamp:ms:UTC", nullable: false },
					{ name: "updated_at", type: "timestamp:ms:UTC", nullable: false },
					{ name: "version", type: "uint64", nullable: false },
					...parameters.request_fields,
				],
			},
		},
		{
			key: auditKey,
			kind: "table",
			depends_on: [],
			requirement_ids: [requirementIds.audit],
			config: {
				name: parameters.audit_table_name,
				columns: [
					{ name: "id", type: "string", nullable: false },
					{ name: "request_id", type: "string", nullable: false },
					{ name: "action", type: "string", nullable: false },
					{ name: "actor_id", type: "string", nullable: false },
					{ name: "idempotency_key", type: "string", nullable: false },
					{ name: "occurred_at", type: "timestamp:ms:UTC", nullable: false },
				],
			},
		},
		{
			key: boardKey,
			kind: "board",
			depends_on: [requestsKey, auditKey],
			requirement_ids: workflowRequirements,
			config: {
				name: `${parameters.page_name} workflow`,
				instruction: `Use the board specialist and catalog discovery to implement ${ns.replace(/[^a-z0-9_]/g, "_")}_command with submit/list/approve/reject operations. Enforce pending -> approved|rejected only, authorized actor input, optimistic version checks, idempotency keys, and immutable audit writes for accepted and refused transitions. Read persisted state after writes. Return the source run id plus domain output that the isolated host can read back as { request: { id, status }, audit_action, idempotent_replay, duplicate_audit_appended }. Do not assume catalog node type names.`,
			},
		},
		{
			key: pageKey,
			kind: "page",
			depends_on: [boardKey],
			requirement_ids: [
				requirementIds.decide,
				requirementIds.authorize,
				requirementIds.interface,
			],
			config: {
				name: parameters.page_name,
				route: parameters.route,
				board: boardKey,
				instruction:
					"Use the UI specialist to build a pending-request queue, detail view, and approve/reject controls. Show actor, decision confirmation, stale-version, already-decided, unauthorized, empty, loading, and error states.",
			},
		},
		{
			key: pageEventKey,
			kind: "event",
			depends_on: [pageKey],
			requirement_ids: [requirementIds.interface],
			config: {
				name: `${parameters.page_name} page`,
				event_type: "page",
				page: pageKey,
				route: parameters.route,
			},
		},
		{
			key: commandEventKey,
			kind: "event",
			depends_on: [boardKey],
			requirement_ids: workflowRequirements,
			config: {
				name: `${parameters.page_name} command`,
				event_type: "quick_action",
				board: boardKey,
				entry_node: `${ns.replace(/[^a-z0-9_]/g, "_")}_command`,
			},
		},
	];
	const probeId = `__flowpilot_${ns.replace(/[^a-z0-9_]/g, "_")}_probe__`;
	const operations = [
		{
			id: "submit",
			payload: {
				operation: "submit",
				request: {
					id: probeId,
					requester_id: parameters.requester_id,
					...parameters.sample_request,
				},
				idempotency_key: `${probeId}:submit`,
			},
		},
		{
			id: "approve",
			payload: {
				operation: "approve",
				id: probeId,
				actor_id: parameters.approver_id,
				expected_version: 1,
				idempotency_key: `${probeId}:approve`,
			},
		},
		{
			id: "approve_replay",
			payload: {
				operation: "approve",
				id: probeId,
				actor_id: parameters.approver_id,
				expected_version: 1,
				idempotency_key: `${probeId}:approve`,
			},
		},
	] as const;
	const scenarios: AppBehaviorScenario[] = [
		interfaceScenario(ns, commandEventKey, requirementIds.interface),
		{
			schema: APP_BEHAVIOR_SCENARIO_SCHEMA,
			id: `${ns}.approval_lifecycle`,
			description:
				"Submit, approve, and replay one decision inside an isolated runtime.",
			requirement_ids: [
				requirementIds.submit,
				requirementIds.decide,
				requirementIds.audit,
				requirementIds.authorize,
				requirementIds.runtime,
			],
			target: { resource_key: commandEventKey, kind: "event" },
			timeout_ms: 120_000,
			steps: operations.map((operation) => ({
				id: operation.id,
				tool: "call_app_event" as const,
				arguments: { payload: operation.payload },
			})),
			assertions: [
				...runtimeScenarioAssertions("submit", [
					{
						id: "request_id_matches",
						path: "/request/id",
						operator: "equals",
						expected: probeId,
					},
					{
						id: "request_pending",
						path: "/request/status",
						operator: "equals",
						expected: "pending",
					},
					{
						id: "submit_audited",
						path: "/audit_action",
						operator: "equals",
						expected: "submit",
					},
				]),
				...runtimeScenarioAssertions("approve", [
					{
						id: "request_id_matches",
						path: "/request/id",
						operator: "equals",
						expected: probeId,
					},
					{
						id: "request_approved",
						path: "/request/status",
						operator: "equals",
						expected: "approved",
					},
					{
						id: "approval_audited",
						path: "/audit_action",
						operator: "equals",
						expected: "approve",
					},
				]),
				...runtimeScenarioAssertions("approve_replay", [
					{
						id: "request_still_approved",
						path: "/request/status",
						operator: "equals",
						expected: "approved",
					},
					{
						id: "idempotent_replay",
						path: "/idempotent_replay",
						operator: "equals",
						expected: true,
					},
					{
						id: "no_duplicate_audit",
						path: "/duplicate_audit_appended",
						operator: "equals",
						expected: false,
					},
				]),
			],
		},
	];
	return {
		schema: APP_CAPABILITY_FRAGMENT_SCHEMA,
		generation: {
			mode: "specialist",
			brief: `Generate the ${parameters.request_name} approval state machine and queue UI from this exact resource contract. Preserve logical keys and let the host resolve physical ids.`,
			exact_template_source: false,
			evidence_limitations: [
				"This recipe defines state-machine invariants but contains no pre-certified FlowScript.",
				"Authorization must be connected to the app's real identity/role source during specialist generation.",
				"Runtime scenarios remain outstanding until a host executes them in attested isolation.",
			],
		},
		requirements,
		resources,
		scenarios,
	};
}

const CRUD_CONTENT: JsonObject = {
	id: "crud.foundation",
	version: "1.0.0",
	parameter_contract: {
		required: [
			"namespace",
			"entity_name",
			"table_name",
			"page_name",
			"route",
			"fields",
			"sample_record",
			"sample_update",
		],
		field_types: "AppSpec table column types",
	},
	resources: ["table", "board", "page", "page_event", "command_event"],
	behavior: ["interface_contract", "isolated_crud_lifecycle"],
	generation: "specialist",
	evidence: "host_terminal_outcomes_plus_domain_state",
	certification: "scaffold_only",
};

const APPROVAL_CONTENT: JsonObject = {
	id: "approval.foundation",
	version: "1.0.0",
	parameter_contract: {
		required: [
			"namespace",
			"request_name",
			"requests_table_name",
			"audit_table_name",
			"page_name",
			"route",
			"request_fields",
			"sample_request",
			"requester_id",
			"approver_id",
		],
		transitions: ["pending->approved", "pending->rejected"],
	},
	resources: [
		"requests_table",
		"audit_table",
		"board",
		"page",
		"page_event",
		"command_event",
	],
	behavior: ["interface_contract", "isolated_approval_lifecycle"],
	generation: "specialist",
	evidence: "host_terminal_outcomes_plus_domain_state",
	certification: "scaffold_only",
};

export const crudDefinition: CapabilityDefinition<"crud.foundation"> = {
	id: "crud.foundation",
	version: "1.0.0",
	title: "CRUD foundation",
	description:
		"A persisted list/create/read/update/delete workflow with page and headless interfaces.",
	parameters: CRUD_CONTENT.parameter_contract as JsonObject,
	content: CRUD_CONTENT,
	fingerprint: "fp1:10dc499c28ecf74e204ea76cb0b300cc",
	schema: crudParametersSchema,
	instantiate: instantiateCrud,
};

export const approvalDefinition: CapabilityDefinition<"approval.foundation"> = {
	id: "approval.foundation",
	version: "1.0.0",
	title: "Approval foundation",
	description:
		"A guarded pending/approved/rejected state machine with an immutable audit trail.",
	parameters: APPROVAL_CONTENT.parameter_contract as JsonObject,
	content: APPROVAL_CONTENT,
	fingerprint: "fp1:d277dd4b78b6c507674f536caad14891",
	schema: approvalParametersSchema,
	instantiate: instantiateApproval,
};

export const appCapabilityDefinitions = [
	crudDefinition,
	approvalDefinition,
] as const;
