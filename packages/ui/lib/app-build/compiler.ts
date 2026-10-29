import { z } from "zod";

import {
	appSpecSchema,
	type AppBehaviorScenario,
	type AppResource,
	type AppResourceKind,
	type AppSpec,
} from "./contract";
import { appBuildFingerprint, reserveAppResourceId } from "./fingerprint";

export type AppSpecIssueCode =
	| "invalid_schema"
	| "duplicate_identifier"
	| "unknown_requirement"
	| "uncovered_requirement"
	| "unknown_dependency"
	| "duplicate_dependency"
	| "duplicate_requirement_reference"
	| "self_dependency"
	| "dependency_cycle"
	| "missing_symbolic_dependency"
	| "wrong_resource_kind"
	| "invalid_event_target"
	| "missing_page_event"
	| "duplicate_page_event"
	| "duplicate_route"
	| "duplicate_table_name"
	| "invalid_scenario_target";

export interface AppSpecIssue {
	readonly code: AppSpecIssueCode;
	readonly path: readonly (string | number)[];
	readonly message: string;
}

export type AppSpecValidation =
	| { readonly ok: true; readonly spec: AppSpec }
	| { readonly ok: false; readonly issues: readonly AppSpecIssue[] };

export type CompiledAppResource = AppResource & {
	readonly physical_id: string;
	readonly desired_fingerprint: string;
	readonly dependents: readonly string[];
};

export interface CompiledAppScenario extends AppBehaviorScenario {
	readonly desired_fingerprint: string;
}

export interface RequirementCoverage {
	readonly requirement_id: string;
	readonly resource_keys: readonly string[];
	readonly scenario_ids: readonly string[];
}

export interface CompiledAppSpec {
	readonly schema_version: 1;
	readonly app_id: string;
	readonly build_id: string;
	readonly spec: AppSpec;
	readonly spec_fingerprint: string;
	readonly resources: readonly CompiledAppResource[];
	readonly scenarios: readonly CompiledAppScenario[];
	readonly topological_order: readonly string[];
	readonly waves: readonly (readonly string[])[];
	readonly requirement_coverage: readonly RequirementCoverage[];
}

export class AppSpecCompileError extends Error {
	readonly issues: readonly AppSpecIssue[];

	constructor(issues: readonly AppSpecIssue[]) {
		super(issues.map((issue) => issue.message).join("\n"));
		this.name = "AppSpecCompileError";
		this.issues = issues;
	}
}

function issue(
	code: AppSpecIssueCode,
	path: readonly (string | number)[],
	message: string,
): AppSpecIssue {
	return { code, path, message };
}

function sortedUnique(values: readonly string[]): string[] {
	return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function normalizeSpec(spec: AppSpec): AppSpec {
	return {
		...spec,
		requirements: [...spec.requirements].sort((left, right) =>
			left.id.localeCompare(right.id),
		),
		resources: [...spec.resources]
			.map((resource) => ({
				...resource,
				depends_on: sortedUnique(resource.depends_on),
				requirement_ids: sortedUnique(resource.requirement_ids),
			}))
			.sort((left, right) => left.key.localeCompare(right.key)),
		scenarios: [...spec.scenarios]
			.map((scenario) => ({
				...scenario,
				requirement_ids: sortedUnique(scenario.requirement_ids),
			}))
			.sort((left, right) => left.id.localeCompare(right.id)),
	};
}

function duplicateIssues(
	values: readonly string[],
	path: readonly (string | number)[],
	label: string,
): AppSpecIssue[] {
	const seen = new Set<string>();
	const duplicate = new Set<string>();
	for (const value of values) {
		if (seen.has(value)) duplicate.add(value);
		seen.add(value);
	}
	return [...duplicate].map((value) =>
		issue(
			"duplicate_identifier",
			path,
			`${label} identifier '${value}' is duplicated.`,
		),
	);
}

function addTypedReferenceIssues(
	issues: AppSpecIssue[],
	resource: AppResource,
	resourceIndex: number,
	reference: string,
	expectedKind: AppResourceKind,
	resourceByKey: ReadonlyMap<string, AppResource>,
	field: string,
) {
	const target = resourceByKey.get(reference);
	if (!target) {
		issues.push(
			issue(
				"unknown_dependency",
				["resources", resourceIndex, "config", field],
				`Resource '${resource.key}' references unknown ${expectedKind} '${reference}'.`,
			),
		);
		return;
	}
	if (target.kind !== expectedKind) {
		issues.push(
			issue(
				"wrong_resource_kind",
				["resources", resourceIndex, "config", field],
				`Resource '${resource.key}' expects '${reference}' to be a ${expectedKind}, but it is a ${target.kind}.`,
			),
		);
	}
	if (!resource.depends_on.includes(reference)) {
		issues.push(
			issue(
				"missing_symbolic_dependency",
				["resources", resourceIndex, "depends_on"],
				`Resource '${resource.key}' must list symbolic ${expectedKind} reference '${reference}' in depends_on.`,
			),
		);
	}
}

function findCycle(
	resourceByKey: ReadonlyMap<string, AppResource>,
): readonly string[] | undefined {
	const visiting = new Set<string>();
	const visited = new Set<string>();
	const path: string[] = [];

	const visit = (key: string): readonly string[] | undefined => {
		if (visiting.has(key)) {
			const start = path.indexOf(key);
			return [...path.slice(start), key];
		}
		if (visited.has(key)) return undefined;
		visiting.add(key);
		path.push(key);
		for (const dependency of resourceByKey.get(key)?.depends_on ?? []) {
			if (!resourceByKey.has(dependency)) continue;
			const cycle = visit(dependency);
			if (cycle) return cycle;
		}
		path.pop();
		visiting.delete(key);
		visited.add(key);
		return undefined;
	};

	for (const key of [...resourceByKey.keys()].sort()) {
		const cycle = visit(key);
		if (cycle) return cycle;
	}
	return undefined;
}

function semanticIssues(spec: AppSpec): AppSpecIssue[] {
	const issues: AppSpecIssue[] = [];
	issues.push(
		...duplicateIssues(
			spec.requirements.map((requirement) => requirement.id),
			["requirements"],
			"Requirement",
		),
		...duplicateIssues(
			spec.resources.map((resource) => resource.key),
			["resources"],
			"Resource",
		),
		...duplicateIssues(
			spec.scenarios.map((scenario) => scenario.id),
			["scenarios"],
			"Scenario",
		),
	);

	const requirementIds = new Set(
		spec.requirements.map((requirement) => requirement.id),
	);
	const resourceByKey = new Map(
		spec.resources.map((resource) => [resource.key, resource] as const),
	);

	for (const [resourceIndex, resource] of spec.resources.entries()) {
		for (const requirementId of resource.requirement_ids) {
			if (!requirementIds.has(requirementId)) {
				issues.push(
					issue(
						"unknown_requirement",
						["resources", resourceIndex, "requirement_ids"],
						`Resource '${resource.key}' covers unknown requirement '${requirementId}'.`,
					),
				);
			}
		}
		if (
			new Set(resource.requirement_ids).size !== resource.requirement_ids.length
		) {
			issues.push(
				issue(
					"duplicate_requirement_reference",
					["resources", resourceIndex, "requirement_ids"],
					`Resource '${resource.key}' repeats a requirement reference.`,
				),
			);
		}
		if (new Set(resource.depends_on).size !== resource.depends_on.length) {
			issues.push(
				issue(
					"duplicate_dependency",
					["resources", resourceIndex, "depends_on"],
					`Resource '${resource.key}' repeats a dependency.`,
				),
			);
		}
		for (const dependency of resource.depends_on) {
			if (dependency === resource.key) {
				issues.push(
					issue(
						"self_dependency",
						["resources", resourceIndex, "depends_on"],
						`Resource '${resource.key}' cannot depend on itself.`,
					),
				);
			} else if (!resourceByKey.has(dependency)) {
				issues.push(
					issue(
						"unknown_dependency",
						["resources", resourceIndex, "depends_on"],
						`Resource '${resource.key}' depends on unknown resource '${dependency}'.`,
					),
				);
			}
		}

		if (resource.kind === "page") {
			addTypedReferenceIssues(
				issues,
				resource,
				resourceIndex,
				resource.config.board,
				"board",
				resourceByKey,
				"board",
			);
		}
		if (resource.kind === "event") {
			const isPage = resource.config.event_type === "page";
			if (isPage) {
				if (!resource.config.page || resource.config.entry_node) {
					issues.push(
						issue(
							"invalid_event_target",
							["resources", resourceIndex, "config"],
							`Page Event '${resource.key}' requires page and cannot set entry_node.`,
						),
					);
				}
			} else if (
				!resource.config.board ||
				!resource.config.entry_node ||
				resource.config.page
			) {
				issues.push(
					issue(
						"invalid_event_target",
						["resources", resourceIndex, "config"],
						`Workflow Event '${resource.key}' requires board and entry_node and cannot set page.`,
					),
				);
			}
			if (isPage && resource.config.page) {
				const page = resourceByKey.get(resource.config.page);
				if (page?.kind === "page") {
					if (!resource.config.route) {
						issues.push(
							issue(
								"invalid_event_target",
								["resources", resourceIndex, "config", "route"],
								`Page Event '${resource.key}' requires the target page route '${page.config.route}'.`,
							),
						);
					} else if (resource.config.route !== page.config.route) {
						issues.push(
							issue(
								"invalid_event_target",
								["resources", resourceIndex, "config", "route"],
								`Page Event '${resource.key}' route must equal target page '${resource.config.page}' route '${page.config.route}'.`,
							),
						);
					}
					if (
						resource.config.board &&
						resource.config.board !== page.config.board
					) {
						issues.push(
							issue(
								"invalid_event_target",
								["resources", resourceIndex, "config", "board"],
								`Page Event '${resource.key}' board must equal its target page board '${page.config.board}'.`,
							),
						);
					}
				}
			}
			if (resource.config.board) {
				addTypedReferenceIssues(
					issues,
					resource,
					resourceIndex,
					resource.config.board,
					"board",
					resourceByKey,
					"board",
				);
			}
			if (resource.config.page) {
				addTypedReferenceIssues(
					issues,
					resource,
					resourceIndex,
					resource.config.page,
					"page",
					resourceByKey,
					"page",
				);
			}
		}
	}

	for (const [pageIndex, page] of spec.resources.entries()) {
		if (page.kind !== "page") continue;
		const pageEvents = spec.resources.filter(
			(resource) =>
				resource.kind === "event" &&
				resource.config.event_type === "page" &&
				resource.config.page === page.key,
		);
		if (pageEvents.length === 0) {
			issues.push(
				issue(
					"missing_page_event",
					["resources", pageIndex],
					`Page '${page.key}' requires exactly one page Event so it can be reached through its route.`,
				),
			);
		} else if (pageEvents.length > 1) {
			issues.push(
				issue(
					"duplicate_page_event",
					["resources", pageIndex],
					`Page '${page.key}' is targeted by ${pageEvents.length} page Events.`,
				),
			);
		}
	}

	for (const [scenarioIndex, scenario] of spec.scenarios.entries()) {
		for (const requirementId of scenario.requirement_ids) {
			if (!requirementIds.has(requirementId)) {
				issues.push(
					issue(
						"unknown_requirement",
						["scenarios", scenarioIndex, "requirement_ids"],
						`Scenario '${scenario.id}' covers unknown requirement '${requirementId}'.`,
					),
				);
			}
		}
		if (
			new Set(scenario.requirement_ids).size !== scenario.requirement_ids.length
		) {
			issues.push(
				issue(
					"duplicate_requirement_reference",
					["scenarios", scenarioIndex, "requirement_ids"],
					`Scenario '${scenario.id}' repeats a requirement reference.`,
				),
			);
		}
		const target = resourceByKey.get(scenario.target.resource_key);
		const validTarget =
			(scenario.target.kind === "app" &&
				scenario.target.resource_key === "app") ||
			(scenario.target.kind === "page" && target?.kind === "page") ||
			(scenario.target.kind === "event" && target?.kind === "event") ||
			(scenario.target.kind === "chat" &&
				target?.kind === "event" &&
				target.config.event_type === "simple_chat");
		if (!validTarget) {
			issues.push(
				issue(
					"invalid_scenario_target",
					["scenarios", scenarioIndex, "target"],
					`Scenario '${scenario.id}' has no compatible ${scenario.target.kind} target '${scenario.target.resource_key}'.`,
				),
			);
		}
	}

	for (const requirement of spec.requirements) {
		const resourceCoverage = spec.resources.some((resource) =>
			resource.requirement_ids.includes(requirement.id),
		);
		const scenarioCoverage = spec.scenarios.some((scenario) =>
			scenario.requirement_ids.includes(requirement.id),
		);
		if (!resourceCoverage && !scenarioCoverage) {
			issues.push(
				issue(
					"uncovered_requirement",
					["requirements"],
					`Requirement '${requirement.id}' is not covered by a resource or scenario.`,
				),
			);
		}
	}

	const pageRoutes = new Map<string, string>();
	const eventRoutes = new Map<string, string>();
	const tableNames = new Map<string, string>();
	for (const resource of spec.resources) {
		if (resource.kind !== "page") continue;
		const previous = pageRoutes.get(resource.config.route);
		if (!previous) pageRoutes.set(resource.config.route, resource.key);
	}
	for (const [index, resource] of spec.resources.entries()) {
		if (resource.kind === "page") {
			const previousIndex = spec.resources.findIndex(
				(candidate) =>
					candidate.kind === "page" &&
					candidate.config.route === resource.config.route,
			);
			if (previousIndex !== index) {
				const previous = spec.resources[previousIndex];
				issues.push(
					issue(
						"duplicate_route",
						["resources", index, "config", "route"],
						`Pages '${previous?.key}' and '${resource.key}' both claim route '${resource.config.route}'.`,
					),
				);
			}
		}
		if (resource.kind === "event" && resource.config.route) {
			const previous = eventRoutes.get(resource.config.route);
			if (previous) {
				issues.push(
					issue(
						"duplicate_route",
						["resources", index, "config", "route"],
						`Events '${previous}' and '${resource.key}' both claim route '${resource.config.route}'.`,
					),
				);
			}
			eventRoutes.set(resource.config.route, resource.key);
			const pageAtRoute = pageRoutes.get(resource.config.route);
			if (pageAtRoute && resource.config.page !== pageAtRoute) {
				issues.push(
					issue(
						"duplicate_route",
						["resources", index, "config", "route"],
						`Event '${resource.key}' collides with page '${pageAtRoute}' at '${resource.config.route}'.`,
					),
				);
			}
		}
		if (resource.kind === "table") {
			const previous = tableNames.get(resource.config.name);
			if (previous) {
				issues.push(
					issue(
						"duplicate_table_name",
						["resources", index, "config", "name"],
						`Tables '${previous}' and '${resource.key}' both use physical name '${resource.config.name}'.`,
					),
				);
			}
			tableNames.set(resource.config.name, resource.key);
			const columnNames = resource.config.columns.map((column) => column.name);
			issues.push(
				...duplicateIssues(
					columnNames,
					["resources", index, "config", "columns"],
					`Table '${resource.key}' column`,
				),
			);
		}
	}

	const cycle = findCycle(resourceByKey);
	if (cycle) {
		issues.push(
			issue(
				"dependency_cycle",
				["resources"],
				`Resource dependency cycle: ${cycle.join(" -> ")}.`,
			),
		);
	}
	return issues;
}

export function validateAppSpec(input: unknown): AppSpecValidation {
	const parsed = appSpecSchema.safeParse(input);
	if (!parsed.success) {
		return {
			ok: false,
			issues: parsed.error.issues.map((entry: z.ZodIssue) =>
				issue(
					"invalid_schema",
					entry.path.map((key) => (typeof key === "symbol" ? String(key) : key)),
					entry.message,
				),
			),
		};
	}
	const issues = semanticIssues(parsed.data);
	const spec = normalizeSpec(parsed.data);
	return issues.length > 0 ? { ok: false, issues } : { ok: true, spec };
}

function topologicalWaves(resources: readonly AppResource[]): string[][] {
	const pending = new Map(
		resources.map(
			(resource) => [resource.key, new Set(resource.depends_on)] as const,
		),
	);
	const waves: string[][] = [];
	while (pending.size > 0) {
		const ready = [...pending]
			.filter(([, dependencies]) => dependencies.size === 0)
			.map(([key]) => key)
			.sort((left, right) => left.localeCompare(right));
		if (ready.length === 0) break;
		waves.push(ready);
		for (const key of ready) pending.delete(key);
		for (const dependencies of pending.values()) {
			for (const key of ready) dependencies.delete(key);
		}
	}
	return waves;
}

export function compileAppSpec(
	input: unknown,
	context: { readonly app_id: string; readonly build_id: string },
): CompiledAppSpec {
	const validation = validateAppSpec(input);
	if (!validation.ok) throw new AppSpecCompileError(validation.issues);
	const { spec } = validation;
	const dependents = new Map<string, string[]>();
	for (const resource of spec.resources) dependents.set(resource.key, []);
	for (const resource of spec.resources) {
		for (const dependency of resource.depends_on) {
			dependents.get(dependency)?.push(resource.key);
		}
	}
	const resources = spec.resources.map((resource) => {
		const physical_id =
			resource.kind === "table"
				? resource.config.name
				: reserveAppResourceId(
						context.app_id,
						context.build_id,
						resource.kind,
						resource.key,
					);
		return {
			...resource,
			physical_id,
			desired_fingerprint: appBuildFingerprint("resource-desired", resource),
			dependents: (dependents.get(resource.key) ?? []).sort((left, right) =>
				left.localeCompare(right),
			),
		} as CompiledAppResource;
	});
	const scenarios = spec.scenarios.map((scenario) => ({
		...scenario,
		desired_fingerprint: appBuildFingerprint("scenario-desired", scenario),
	}));
	const waves = topologicalWaves(spec.resources);
	return {
		schema_version: 1,
		app_id: context.app_id,
		build_id: context.build_id,
		spec,
		spec_fingerprint: appBuildFingerprint("app-spec", spec),
		resources,
		scenarios,
		topological_order: waves.flat(),
		waves,
		requirement_coverage: spec.requirements.map((requirement) => ({
			requirement_id: requirement.id,
			resource_keys: resources
				.filter((resource) => resource.requirement_ids.includes(requirement.id))
				.map((resource) => resource.key),
			scenario_ids: scenarios
				.filter((scenario) => scenario.requirement_ids.includes(requirement.id))
				.map((scenario) => scenario.id),
		})),
	};
}
