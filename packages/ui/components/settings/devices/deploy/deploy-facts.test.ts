import { describe, expect, test } from "bun:test";
import type {
	DeploymentVariable,
	PlacementConfiguration,
} from "../../../../lib/device-management/deployment";
import {
	type DeployDraft,
	type PlanApp,
	type PlanFacts,
	makePlan,
	resolvePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	eventVariables,
	limitsOutOfRange,
	mergeEventVariables,
	portsInUse,
	runsNewest,
	unresolvedOverrides,
	variableUsers,
} from "./deploy-facts";

const NOW = 1_790_769_600;

const variable = (
	id: string,
	name: string,
	dataType = "String",
	secret = false,
	valueType = "Normal",
): DeploymentVariable => ({
	id,
	name,
	data_type: dataType,
	value_type: valueType,
	secret,
});

const GREETING = variable("var_greeting", "Greeting");
const API_KEY = variable("var_api_key", "API key", "String", true);
const COMPACT = variable("var_compact", "Compact", "Boolean");

const APP: PlanApp = {
	id: "app_notes",
	name: "Field Notes",
	visibility: "Private",
	events: [
		{
			id: "evt_page",
			name: "Notes page",
			active: true,
			event_type: "http",
			event_version: [2, 0, 0],
			board_version: [3, 0, 0],
		},
		{
			id: "evt_sync",
			name: "Sync",
			active: true,
			event_type: "http",
			event_version: [1, 0, 0],
			board_version: [3, 0, 0],
		},
	],
	variables: {
		evt_page: [GREETING, API_KEY, COMPACT],
		evt_sync: [GREETING],
	},
};

function configuration(
	variables: Record<string, unknown>,
	secrets: Record<string, string> = {},
): PlacementConfiguration {
	return {
		placement_id: "notes",
		project_id: APP.id,
		deployment_id: "dep-1",
		config_revision: 3,
		config: {
			id: "notes",
			project_id: APP.id,
			deployment_id: "dep-1",
			revision: "a".repeat(64),
			source: "online",
			project_path: "projects/app_notes",
			events: [
				{
					event_id: "evt_page",
					event_version: [2, 0, 0],
					board_version: [3, 0, 0],
				},
			],
			max_replicas: 1,
			variables,
			secret_overrides: secrets,
		},
	};
}

/** An update of `notes` on one device, its service serving the page. */
function updatePlan(change: (draft: DeployDraft) => DeployDraft = (d) => d) {
	const draft = makePlan({
		scope: { kind: "app", appId: APP.id },
		route: { deviceIds: ["d1"], serviceId: "notes" },
		app: APP,
		deploymentId: "dep-2",
		now: NOW,
		updateEvents: ["evt_page"],
	});
	const facts: PlanFacts = {
		app: APP,
		platform: "desktop",
		now: NOW,
		devices: {
			d1: {
				id: "d1",
				name: "studio",
				gate: null,
				locked: false,
				services: [
					{ serviceId: "notes", projectId: APP.id, events: ["evt_page"] },
				],
			},
		},
	};
	return resolvePlan(change(draft), facts);
}

const issues = (
	plan: ReturnType<typeof updatePlan>,
	existing: PlacementConfiguration,
	previous: Record<string, readonly DeploymentVariable[]> = {},
) =>
	unresolvedOverrides(plan, {
		configurations: { d1: [existing] },
		previous,
	}).map((row) => `${row.variableId}:${row.issue}`);

describe("stored values an update can't carry over", () => {
	test("values that still fit need nothing", () => {
		expect(
			issues(
				updatePlan(),
				configuration(
					{ var_greeting: "Hello", var_compact: true },
					{
						var_api_key: "variable-1",
					},
				),
			),
		).toEqual([]);
	});

	test("a value for an event the update no longer serves is outside", () => {
		const rows = unresolvedOverrides(updatePlan(), {
			configurations: { d1: [configuration({ var_gone: "x" })] },
			previous: {},
		});
		expect(rows).toEqual([
			expect.objectContaining({
				deviceId: "d1",
				device: "studio",
				serviceId: "notes",
				variableId: "var_gone",
				name: "var_gone",
				issue: "outside",
				secret: false,
			}),
		]);
	});

	test("a plain value whose type no longer accepts it is invalid", () => {
		expect(issues(updatePlan(), configuration({ var_compact: "yes" }))).toEqual(
			["var_compact:invalid"],
		);
	});

	test("plain turned secret, or secret turned plain, needs a replacement", () => {
		expect(
			issues(
				updatePlan(),
				configuration({ var_api_key: "in the clear" }, { var_greeting: "v-1" }),
			).sort(),
		).toEqual(["var_api_key:kind_changed", "var_greeting:kind_changed"]);
	});

	test("a stored secret written for another type is a mismatch", () => {
		const existing = configuration({}, { var_api_key: "variable-1" });
		const before = variable(
			"var_api_key",
			"API key",
			"String",
			true,
			"HashMap",
		);
		const rows = unresolvedOverrides(updatePlan(), {
			configurations: { d1: [existing] },
			previous: { "d1/notes": [before] },
		});
		expect(rows).toHaveLength(1);
		expect(rows[0]).toMatchObject({
			variableId: "var_api_key",
			issue: "type_changed",
			secret: true,
			previous: before,
			current: API_KEY,
		});
		// The same definition as before is no mismatch.
		expect(issues(updatePlan(), existing, { "d1/notes": [API_KEY] })).toEqual(
			[],
		);
	});

	test("a new value or an explicit removal resolves it", () => {
		const existing = configuration(
			{ var_compact: "yes", var_gone: "x" },
			{ var_api_key: "variable-1" },
		);
		const previous = {
			"d1/notes": [variable("var_api_key", "API key", "Integer", true)],
		};
		const typed = updatePlan((draft) => ({
			...draft,
			vars: { var_compact: "true" },
			secrets: { var_api_key: "a new secret" },
			edited: ["var_compact", "var_api_key"],
			targets: draft.targets.map((target) => ({
				...target,
				over: { removeOverrides: ["var_gone"] },
			})),
		}));
		expect(issues(typed, existing, previous)).toEqual([]);
		// An empty secret field keeps the stored one, so the mismatch stays.
		const empty = updatePlan((draft) => ({
			...draft,
			secrets: { var_api_key: "" },
			edited: ["var_api_key"],
		}));
		expect(issues(empty, existing, previous)).toContain(
			"var_api_key:type_changed",
		);
	});

	test("nothing is claimed before the definitions or the configuration are known", () => {
		const plan = updatePlan();
		expect(
			unresolvedOverrides(plan, { configurations: {}, previous: {} }),
		).toEqual([]);
		const blind = { ...plan, app: { ...APP, variables: undefined } };
		expect(
			unresolvedOverrides(blind, {
				configurations: { d1: [configuration({ var_gone: "x" })] },
				previous: {},
			}),
		).toEqual([]);
	});
});

describe("definitions", () => {
	test("each variable is listed once, by name, with the events that use it", () => {
		const events = ["evt_page", "evt_sync"];
		expect(eventVariables(APP, events).map((row) => row.name)).toEqual([
			"API key",
			"Compact",
			"Greeting",
		]);
		expect(variableUsers(APP, events, "var_greeting")).toEqual(events);
		expect(variableUsers(APP, events, "var_compact")).toEqual(["evt_page"]);
		expect(eventVariables(null, events)).toEqual([]);
	});

	test("the newest source wins: installed, then the device's check, then the approved bundle", () => {
		const installed = { evt_page: [GREETING], evt_old: [COMPACT] };
		const checks = {
			d1: { refusals: {}, variables: { evt_page: [GREETING, COMPACT] } },
		};
		const approved = { evt_page: [API_KEY] };
		expect(mergeEventVariables(installed, checks, undefined)).toEqual({
			evt_page: [GREETING, COMPACT],
			evt_old: [COMPACT],
		});
		expect(mergeEventVariables(installed, checks, approved)?.evt_page).toEqual([
			API_KEY,
		]);
		expect(mergeEventVariables(undefined, {}, undefined)).toBeUndefined();
	});
});

describe("a service that already runs the newest version", () => {
	const page = {
		event_id: "evt_page",
		event_version: [2, 0, 0] as [number, number, number],
		board_version: [3, 0, 0] as [number, number, number],
	};

	test("every served event is pinned to what the app has now", () => {
		expect(runsNewest(APP.events, [page])).toBe(true);
	});

	test("an older event or flow version is behind", () => {
		expect(
			runsNewest(APP.events, [{ ...page, event_version: [1, 9, 0] }]),
		).toBe(false);
		expect(
			runsNewest(APP.events, [{ ...page, board_version: [2, 4, 1] }]),
		).toBe(false);
	});

	test("unknown counts as behind: no event list, an event the app dropped, a flow that isn't pinned", () => {
		expect(runsNewest(APP.events, null)).toBe(false);
		expect(runsNewest(APP.events, [])).toBe(false);
		expect(runsNewest(APP.events, [{ ...page, event_id: "evt_gone" }])).toBe(
			false,
		);
		const [first, ...rest] = APP.events;
		expect(
			runsNewest([{ ...first, board_version: null }, ...rest], [page]),
		).toBe(false);
	});
});

describe("sandbox limits", () => {
	const sandboxed = (change: (draft: DeployDraft) => DeployDraft) => {
		const draft = makePlan({
			scope: { kind: "app", appId: APP.id },
			route: { deviceIds: ["d1"], mode: "new" },
			app: APP,
			deploymentId: "dep-3",
			now: NOW,
		});
		return resolvePlan(change(draft), {
			app: APP,
			platform: "desktop",
			now: NOW,
			devices: {
				d1: {
					id: "d1",
					name: "edge",
					gate: null,
					locked: false,
					services: [],
					isolation: "required",
				},
			},
		});
	};
	const limits = (patch: Partial<NonNullable<DeployDraft["isolation"]>>) =>
		sandboxed((draft) =>
			draft.isolation
				? { ...draft, isolation: { ...draft.isolation, ...patch } }
				: draft,
		);

	test("the defaults are in range", () => {
		expect(limitsOutOfRange(sandboxed((draft) => draft))).toBe(false);
	});

	test("the lib's bounds decide: memory, processes, CPU and disk", () => {
		expect(limitsOutOfRange(limits({ memoryBytes: 8 * 1024 ** 2 }))).toBe(true);
		expect(limitsOutOfRange(limits({ memoryBytes: 64 * 1024 ** 2 }))).toBe(
			false,
		);
		expect(limitsOutOfRange(limits({ maxProcesses: 8 }))).toBe(true);
		expect(limitsOutOfRange(limits({ cpuMillis: 0 }))).toBe(true);
		expect(limitsOutOfRange(limits({ diskBytes: 1024 }))).toBe(true);
	});

	test("a service that runs as the agent has no limits to check", () => {
		expect(
			limitsOutOfRange(
				sandboxed((draft) =>
					draft.isolation
						? {
								...draft,
								isolation: {
									...draft.isolation,
									profile: "trusted_process",
									memoryBytes: 1,
								},
							}
						: draft,
				),
			),
		).toBe(false);
	});
});

test("ports in use come from the services that listen", () => {
	const listening = configuration({});
	listening.config.hosting = {
		host: "127.0.0.1",
		port: 8090,
		max_in_flight: 64,
		request_timeout_secs: 300,
		auth_secret: "token",
	};
	expect(portsInUse([listening, configuration({})])).toEqual([
		{ port: 8090, serviceId: "notes" },
	]);
});
