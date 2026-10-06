import { expect, test } from "bun:test";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IEventState } from "../../state/backend-state/event-state";
import type { IEvent } from "../schema/flow/event";
import { botTokenKey } from "./bot-config";
import {
	DEPLOYMENT_CONFIG_BYTES,
	DEVICE_INELIGIBLE_CODES,
	type DeploymentEvent,
	DeploymentPublicationFailedError,
	DeploymentRejectedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentRolloutStatus,
	type DeploymentVariable,
	EVENT_KINDS,
	type EventIneligibleCode,
	type EventKind,
	type EventReadiness,
	type InstalledProject,
	type PlacementConfiguration,
	StaleDeploymentRevisionError,
	approvedOnlineCatalog,
	assertOfflineQueuesDrained,
	boardVariables,
	cancelDeploymentRollout,
	createDeploymentPlan,
	discoverOfflineEvents,
	discoverOnlineEvents,
	discoverOnlineVariables,
	discoverPreviousOfflineVariables,
	discoverPreviousOnlineVariables,
	eventEligibility,
	eventKind,
	executeDeploymentPlan,
	limitsInstances,
	mergeVariables,
	missingFeature,
	offlineWritesSchema,
	placementResourcesSchema,
	readDeploymentRollout,
	readExistingDeployment,
	removesOfflineBuffering,
	requiredFeatures,
	validateVariableValue,
	variableText,
	variableValue,
	waitForDeploymentRollout,
} from "./deployment";
import type { EventRoute } from "./event-route";
import type { AgentFeature } from "./model/types";
import type { ManagementCall } from "./telemetry";

const installed: InstalledProject = {
	project_id: "project",
	project_path: "/private/projects/project/revisions/abc",
	revision: "a".repeat(64),
	source: "offline",
	online_metadata_sha256: "a".repeat(64),
};
const event: DeploymentEvent = {
	id: "api",
	name: "API",
	event_type: "http",
	event_version: [1, 2, 3],
	board_version: [3, 2, 1],
	hosted: true,
	eligible: true,
};
const secret: DeploymentVariable = {
	id: "credential",
	name: "Credential",
	data_type: "String",
	value_type: "Normal",
	secret: true,
};

const time = { secs_since_epoch: 100, nanos_since_epoch: 0 };
const ROUTE_CONFIG = Array.from(
	new TextEncoder().encode(JSON.stringify({ path: "/api", method: "POST" })),
);
function approvedEvent(change: Partial<IEvent> = {}): IEvent {
	return {
		id: event.id,
		name: event.name,
		event_type: event.event_type,
		event_version: [1, 2, 3],
		board_id: "board",
		board_version: [3, 2, 1],
		active: true,
		config: ROUTE_CONFIG,
		created_at: time,
		updated_at: time,
		description: "",
		node_id: "entry",
		priority: 0,
		variables: {},
		...change,
	} as IEvent;
}
function approvedBoard(version: number[], variables: DeploymentVariable[]) {
	return {
		id: "board",
		version,
		layers: {},
		variables: Object.fromEntries(
			variables.map((variable) => [
				variable.id,
				{ ...variable, exposed: true, editable: true, default_value: null },
			]),
		),
	};
}
function approvedDocuments(
	events: IEvent[] = [approvedEvent()],
	variables: DeploymentVariable[] = [secret],
): Record<string, unknown> {
	return Object.fromEntries([
		["app", { id: "project" }],
		...events.map((value) => [
			`events/${value.id}/versions/${value.event_version.join("/")}`,
			value,
		]),
		...events.map((value) => [
			`boards/${value.board_id}/versions/${value.board_version?.join("/")}`,
			approvedBoard(value.board_version ?? [], variables),
		]),
	]);
}
function onlineInstalled(documents = approvedDocuments()): InstalledProject {
	return {
		...installed,
		source: "online",
		project_path: "/private/online/projects/project",
		revision: "c".repeat(64),
		online_catalog: approvedOnlineCatalog(documents),
	};
}

test("online discovery offers only the approved metadata installed on the device", () => {
	const project = onlineInstalled();
	const [selected] = discoverOnlineEvents(project);
	expect(selected).toEqual({
		...event,
		readiness_kind: "listener",
		rollout_supported: true,
		kind: "served",
		route: { method: "POST", path: "/api" },
	});
	expect(discoverOnlineVariables(project, selected)).toEqual([secret]);
	const changes: Partial<DeploymentEvent>[] = [
		{ event_version: [1, 2, 4] },
		{ board_version: [3, 2, 2] },
		{ id: "another-event" },
	];
	for (const change of changes)
		expect(() =>
			discoverOnlineVariables(project, { ...selected, ...change }),
		).toThrow("Prepare and install the project again");
	expect(() =>
		discoverOnlineEvents({ ...project, online_catalog: undefined }),
	).toThrow("Prepare and install this online project again");
	expect(() => discoverOnlineEvents(installed)).toThrow(
		"Prepare and install this online project again",
	);
});

test("approved metadata keeps eligibility rules and rejects forged keys or missing boards", () => {
	const catalog = approvedOnlineCatalog(
		approvedDocuments([
			approvedEvent(),
			approvedEvent({ id: "paused", active: false }),
			approvedEvent({ id: "worker", event_type: "daemon" }),
		]),
	);
	expect(
		catalog.events.map((value) => [
			value.id,
			value.eligible,
			value.readiness_kind,
		]),
	).toEqual([
		["api", true, "listener"],
		["paused", false, "listener"],
		["worker", true, "explicit"],
	]);
	const documents = approvedDocuments();
	expect(() =>
		approvedOnlineCatalog({
			...documents,
			"events/api/versions/1/2/4": documents["events/api/versions/1/2/3"],
		}),
	).toThrow("does not match its event identity");
	const { "boards/board/versions/3/2/1": _board, ...missing } = documents;
	expect(() => approvedOnlineCatalog(missing)).toThrow("missing board");
});

test("online plans pin only events from the installed approved metadata", () => {
	const online = {
		...input(),
		replicas: 1,
		installed: onlineInstalled(),
		resourceGrant: { grant_id: "grant", authz_version: 1 },
	};
	expect(createDeploymentPlan(online).config.events).toEqual([
		{ event_id: "api", event_version: [1, 2, 3], board_version: [3, 2, 1] },
	]);
	expect(() =>
		createDeploymentPlan({
			...online,
			events: [{ ...event, event_version: [1, 3, 0] }],
		}),
	).toThrow("not in the approved metadata installed on the device");
});

test("kept secret references must match the type they were written for", async () => {
	const base = await updateInput();
	const previous = [{ ...secret, value_type: "HashMap" }];
	expect(() =>
		createDeploymentPlan({ ...base, previousVariables: previous }),
	).toThrow(
		"Stored secret credential was written for String/HashMap, but the selected event expects String/Normal",
	);
	expect(
		createDeploymentPlan({ ...base, previousVariables: [secret] }).config
			.secret_overrides,
	).toEqual({ credential: "variable-stored" });
	const replaced = createDeploymentPlan({
		...base,
		previousVariables: previous,
		overrides: { ...base.overrides, credential: "new-value" },
	});
	expect(
		(replaced.config.secret_overrides as Record<string, string>).credential,
	).not.toBe("variable-stored");
	expect(
		createDeploymentPlan({
			...base,
			previousVariables: previous,
			removeOverrides: [...base.removeOverrides, "credential"],
		}).config.secret_overrides,
	).toEqual({});
});

test("earlier secret definitions come from the deployed revision or its published archive", async () => {
	const existing = await existingPlacement();
	const requests: unknown[] = [];
	const call: ManagementCall = async (command) => {
		const request = command.request as Record<string, unknown>;
		requests.push(request);
		return {
			operation_id: "describe",
			state: "completed",
			result: {
				project_id: "project",
				revision: request.revision,
				event_id: request.event_id,
				items: [{ ...secret, value_type: "HashMap" }, greeting],
				next: null,
			},
		};
	};
	expect(
		await discoverPreviousOfflineVariables(call, installed, existing),
	).toEqual([{ ...secret, value_type: "HashMap" }]);
	expect(requests).toEqual([
		{
			kind: "describe",
			project_id: "project",
			revision: "b".repeat(64),
			event_id: "api",
			after: null,
		},
	]);
	expect(
		await discoverPreviousOfflineVariables(
			call,
			{ ...installed, revision: "b".repeat(64) },
			existing,
		),
	).toEqual([]);
	expect(requests).toHaveLength(1);

	const reads: unknown[][] = [];
	const events = {
		getEventAuthoritative: async (...args: unknown[]) => {
			reads.push(args);
			return approvedEvent({
				event_version: [1, 0, 0],
				board_version: [1, 0, 0],
			});
		},
	} as unknown as IEventState;
	const boards = {
		getBoardAuthoritative: async (...args: unknown[]) => {
			reads.push(args);
			return approvedBoard([1, 0, 0], [{ ...secret, data_type: "Integer" }]);
		},
	} as unknown as IBoardState;
	const online = {
		...existing,
		config: { ...existing.config, source: "online" },
	} as PlacementConfiguration;
	expect(
		await discoverPreviousOnlineVariables(
			events,
			boards,
			onlineInstalled(),
			online,
		),
	).toEqual([{ ...secret, data_type: "Integer" }]);
	expect(reads).toEqual([
		["project", "api", [1, 0, 0]],
		["project", "board", [1, 0, 0]],
	]);
	const unchanged = {
		...online,
		config: {
			...online.config,
			events: [
				{ event_id: "api", event_version: [1, 2, 3], board_version: [3, 2, 1] },
			],
		},
	} as PlacementConfiguration;
	expect(
		await discoverPreviousOnlineVariables(
			events,
			boards,
			onlineInstalled(),
			unchanged,
		),
	).toEqual([secret]);
	expect(reads).toHaveLength(2);
});

test("health-checked updates stage isolated secrets before activation and preserve the previous revision", async () => {
	const plan = createDeploymentPlan({
		...(await updateInput()),
		healthChecked: true,
		overrides: { credential: "replacement" },
		serviceToken: "z".repeat(32),
	});
	expect(plan.steps.map((step) => step.command.type)).toEqual([
		"stage_rollout",
		"rollout_secret",
		"rollout_secret",
		"activate_rollout",
	]);
	expect(plan.rollout_id).toBe(plan.steps[0].id);
	expect(plan.steps[0].command).toMatchObject({
		expected_revision: 4,
		stabilization_seconds: 10,
		deadline_seconds: 120,
	});
	expect(
		plan.steps
			.slice(1)
			.every((step) => step.command.rollout_id === plan.rollout_id),
	).toBe(true);
	expect(JSON.stringify(plan.config)).not.toContain("replacement");
	expect(JSON.stringify(plan.config)).not.toContain("z".repeat(32));
	const calls: string[] = [];
	await executeDeploymentPlan(async (command, id) => {
		calls.push(String(command.type));
		return {
			operation_id: id ?? "read",
			state: command.type === "rollout" ? "completed" : "accepted",
			result: {
				rollout_id: plan.rollout_id,
				placement_id: plan.config.id,
				project_id: plan.config.project_id,
				...(command.type === "rollout_secret"
					? { name: command.name, secret: "completed" }
					: {
							state:
								command.type === "stage_rollout"
									? "staged"
									: command.type === "activate_rollout"
										? "validating"
										: "healthy",
						}),
			},
		};
	}, plan);
	expect(calls).toEqual([
		"stage_rollout",
		"rollout_secret",
		"rollout_secret",
		"activate_rollout",
		"rollout",
	]);
});

test("lost rollout activation replies resume from the journal without repeating mutations", async () => {
	const plan = createDeploymentPlan({
		...(await updateInput()),
		healthChecked: true,
	});
	const journal = new Map<string, Record<string, unknown>>();
	const mutations: string[] = [];
	let lose = true;
	const call: ManagementCall = async (command, id) => {
		if (command.type === "rollout")
			return {
				operation_id: "read",
				state: "completed",
				result: {
					rollout_id: plan.rollout_id,
					placement_id: plan.config.id,
					project_id: plan.config.project_id,
					state: "healthy",
				},
			};
		if (command.type === "operation")
			return {
				operation_id: String(command.operation_id),
				state: "accepted",
				result: journal.get(String(command.operation_id)) ?? {},
			};
		const result = {
			rollout_id: plan.rollout_id,
			placement_id: plan.config.id,
			project_id: plan.config.project_id,
			state: command.type === "stage_rollout" ? "staged" : "validating",
		};
		const operationId = requiredOperationId(id);
		journal.set(operationId, result);
		mutations.push(operationId);
		if (command.type === "activate_rollout" && lose) {
			lose = false;
			throw new Error("Connection lost");
		}
		return { operation_id: operationId, state: "accepted", result };
	};
	await expect(executeDeploymentPlan(call, plan)).rejects.toThrow(
		"Connection lost",
	);
	await executeDeploymentPlan(call, plan);
	expect(mutations).toEqual(plan.steps.map((step) => step.id));
});

test("rollout activation requires exact secret receipts and stops when its controller is locked", async () => {
	for (const cancel of [false, true]) {
		const plan = createDeploymentPlan({
			...(await updateInput()),
			healthChecked: true,
			overrides: { credential: "replacement" },
		});
		const abort = new AbortController();
		const commands: string[] = [];
		await expect(
			executeDeploymentPlan(
				async (command, id) => {
					commands.push(String(command.type));
					if (cancel) abort.abort(new Error("Locked"));
					return {
						operation_id: requiredOperationId(id),
						state: "accepted",
						result: {
							rollout_id: plan.rollout_id,
							placement_id: plan.config.id,
							project_id: plan.config.project_id,
							...(command.type === "stage_rollout"
								? { state: "staged" }
								: { name: "different-secret", secret: "completed" }),
						},
					};
				},
				plan,
				abort.signal,
			),
		).rejects.toThrow(cancel ? "Locked" : "staged secret");
		expect(commands).toEqual(
			cancel ? ["stage_rollout"] : ["stage_rollout", "rollout_secret"],
		);
	}
});

test("rollout observation rejects cross-project receipts and reports rollback as a terminal outcome", async () => {
	const scope = {
		rollout_id: crypto.randomUUID(),
		placement_id: "placement",
		project_id: "project",
	};
	for (const field of ["rollout_id", "placement_id", "project_id"] as const) {
		await expect(
			waitForDeploymentRollout(
				async () => ({
					operation_id: "read",
					state: "completed",
					result: {
						...scope,
						[field]: field === "rollout_id" ? crypto.randomUUID() : "other",
						state: "healthy",
					},
				}),
				scope,
			),
		).rejects.toThrow("another project or placement");
	}
	for (const state of ["rolled_back", "failed", "cancelled"]) {
		await expect(
			waitForDeploymentRollout(
				async () => ({
					operation_id: "read",
					state: "completed",
					result: { ...scope, state },
				}),
				scope,
			),
		).rejects.toBeInstanceOf(DeploymentRolloutEndedError);
	}
});

test("a definitive refusal to read rollout status ends retries while a retryable one keeps them", async () => {
	const scope = {
		rollout_id: crypto.randomUUID(),
		placement_id: "placement",
		project_id: "project",
	};
	const denied = await waitForDeploymentRollout(
		async () => rejected(undefined, "unauthorized", "Deploy access expired"),
		scope,
	).catch((error: unknown) => error);
	expect(denied).toBeInstanceOf(DeploymentRejectedError);
	expect(String(denied)).toContain(`rollout ${scope.rollout_id}`);
	expect(String(denied)).toContain("Deploy access expired");
	const failed = await waitForDeploymentRollout(
		async () => rejected(undefined, "failed", "Rollout store is unreadable"),
		scope,
	).catch((error: unknown) => error);
	expect(failed).toBeInstanceOf(Error);
	expect(failed).not.toBeInstanceOf(DeploymentRejectedError);
	expect(String(failed)).toContain("Rollout store is unreadable");
	expect(String(failed)).toContain("retry the same rollout");
});

test("a stopped or expired staged rollout ends retries before activation", async () => {
	for (const rejectSecret of [false, true]) {
		const plan = createDeploymentPlan({
			...(await updateInput()),
			healthChecked: true,
			overrides: rejectSecret ? { credential: "replacement" } : {},
		});
		const commands: string[] = [];
		await expect(
			executeDeploymentPlan(async (command, id) => {
				commands.push(String(command.type));
				return {
					operation_id: id ?? "read",
					state:
						command.type === "stage_rollout"
							? "accepted"
							: command.type === "rollout"
								? "completed"
								: "rejected",
					result: {
						rollout_id: plan.rollout_id,
						placement_id: plan.config.id,
						project_id: plan.config.project_id,
						state: command.type === "stage_rollout" ? "staged" : "cancelled",
					},
				};
			}, plan),
		).rejects.toBeInstanceOf(DeploymentRolloutEndedError);
		expect(commands).toEqual([
			"stage_rollout",
			rejectSecret ? "rollout_secret" : "activate_rollout",
			"rollout",
		]);
	}
});

test("staging rejections require review after Stop or a different active rollout without pretending the revision changed", async () => {
	for (const stopped of [true, false]) {
		const plan = createDeploymentPlan({
			...(await updateInput()),
			healthChecked: true,
		});
		await expect(
			executeDeploymentPlan(
				async (command, id) => ({
					operation_id: id ?? "read",
					state:
						command.type === "placement_configuration"
							? "completed"
							: "rejected",
					result:
						command.type === "placement_configuration"
							? {
									placement_id: plan.config.id,
									project_id: plan.config.project_id,
									deployment_id: plan.config.deployment_id,
									config_revision: plan.expected_revision,
									config: storedConfig,
									desired_state: stopped ? "stopped" : "running",
									...(stopped
										? {}
										: {
												rollout: {
													rollout_id: "different-rollout",
													placement_id: plan.config.id,
													project_id: plan.config.project_id,
													state: "staged",
												},
											}),
								}
							: {},
				}),
				plan,
			),
		).rejects.toBeInstanceOf(DeploymentReviewRequiredError);
	}
});

test("discard reads the exact rollout and handles activation races without stopping services", async () => {
	const scope = {
		rollout_id: "staged-update",
		placement_id: "placement",
		project_id: "project",
	};
	for (const race of [false, true]) {
		const commands: string[] = [];
		const status = await cancelDeploymentRollout(async (command, id) => {
			commands.push(String(command.type));
			return {
				operation_id: id ?? "read",
				state:
					command.type === "rollout"
						? "completed"
						: race
							? "rejected"
							: "accepted",
				result: {
					...scope,
					state:
						command.type === "cancel_rollout"
							? "cancelled"
							: commands.length === 1
								? "staged"
								: "activating",
				},
			};
		}, scope);
		expect(status.state).toBe(race ? "activating" : "cancelled");
		expect(commands).toEqual(
			race
				? ["rollout", "cancel_rollout", "rollout"]
				: ["rollout", "cancel_rollout"],
		);
	}
});

test("a refused discard of a still staged update reports the device's reason instead of an activation race", async () => {
	const scope = {
		rollout_id: "staged-update",
		placement_id: "placement",
		project_id: "project",
	};
	const refuse =
		(response: ReturnType<typeof rejected>): ManagementCall =>
		async (command, id) =>
			command.type === "rollout"
				? {
						operation_id: "read",
						state: "completed",
						result: { ...scope, state: "staged" },
					}
				: { ...response, operation_id: id ?? "read" };
	const denied = await cancelDeploymentRollout(
		refuse(rejected(undefined, "unauthorized", "Deploy access expired")),
		scope,
	).catch((error: unknown) => error);
	expect(denied).toBeInstanceOf(DeploymentRejectedError);
	expect(String(denied)).toContain("Deploy access expired");
	expect(String(denied)).toContain("does not allow this change");
	const busy = await cancelDeploymentRollout(
		refuse(rejected(undefined, "busy", "Rollout store is locked")),
		scope,
	).catch((error: unknown) => error);
	expect(busy).not.toBeInstanceOf(DeploymentRejectedError);
	expect(String(busy)).toContain("did not discard staged update");
	expect(String(busy)).toContain("Rollout store is locked");
});

test("device-supplied rejection codes never pick up inherited object members as hints", () => {
	for (const code of ["constructor", "__proto__", "to_string"])
		expect(
			new DeploymentRejectedError(
				{ code, error: "Refused.", retryable: false },
				"the update",
			).message,
		).toBe("The device rejected the update: Refused.");
});

test("automatic startup checks reject stopped placements, unsupported agents and workflow-owned listeners", async () => {
	const base = await updateInput();
	expect(() =>
		createDeploymentPlan({
			...base,
			existing: { ...base.existing, desired_state: "stopped" },
			healthChecked: true,
		}),
	).toThrow("running HTTP");
	expect(() =>
		createDeploymentPlan({
			...base,
			healthChecked: true,
			replicas: 1,
			existing: {
				...base.existing,
				config: { ...base.existing.config, max_replicas: 1 },
			},
			events: [{ ...event, event_type: "mcp", hosted: false }],
		}),
	).toThrow("running HTTP");
	// A schedule is checkable once it is armed; a row without the fact (an older agent) is not.
	const single = {
		...base,
		healthChecked: true,
		replicas: 1,
		existing: {
			...base.existing,
			config: { ...base.existing.config, max_replicas: 1 },
		},
	};
	const schedule = { ...event, event_type: "cron", hosted: false };
	expect(() => createDeploymentPlan({ ...single, events: [schedule] })).toThrow(
		"running HTTP",
	);
	expect(
		createDeploymentPlan({
			...single,
			events: [event, { ...schedule, id: "nightly", rollout_supported: true }],
		}).steps.map(({ command }) => command.type),
	).toEqual(["stage_rollout", "activate_rollout"]);
	expect(() =>
		createDeploymentPlan({
			...base,
			healthChecked: true,
			installed: { ...installed, source: "online" },
			existing: {
				...base.existing,
				config: { ...base.existing.config, source: "online" },
			},
		}),
	).toThrow("device support for its project source");
});

test("online staged updates preserve scoped approvals and cache identity while replacing pins and isolated secrets", async () => {
	const base = await updateInput();
	const online = {
		...installed,
		source: "online" as const,
		project_path: "/private/online/projects/project",
		revision: "release-2",
	};
	const resourceGrant = {
		grant_id: "online-grant",
		authz_version: 8,
		billing_grant_id: "project-model-allowance",
		billing_authz_version: 3,
	};
	const existing = await readExistingDeployment(
		readerFor({
			...base.existing,
			rollout_sources: ["offline", "online"],
			rollout: null,
			config: {
				...base.existing.config,
				source: online.source,
				project_path: online.project_path,
				resource_grant: resourceGrant,
			},
		}),
		base.placement,
		online.project_id,
	);
	const plan = createDeploymentPlan({
		...base,
		installed: online,
		existing,
		healthChecked: true,
		overrides: { greeting: "online device", credential: "new-online-secret" },
		serviceToken: "updated-online-access-".repeat(2),
	});
	expect(plan.steps.map(({ command }) => command.type)).toEqual([
		"stage_rollout",
		"rollout_secret",
		"rollout_secret",
		"activate_rollout",
	]);
	expect(plan.config).toMatchObject({
		id: base.placement,
		deployment_id: base.deployment,
		project_id: online.project_id,
		source: "online",
		project_path: online.project_path,
		revision: online.revision,
		resource_grant: resourceGrant,
		variables: { greeting: "online device" },
		events: [
			{
				event_id: event.id,
				event_version: event.event_version,
				board_version: event.board_version,
			},
		],
	});
	expect(plan.config.rollout_sources).toBeUndefined();
	expect(plan.expected_revision).toBe(existing.config_revision);
	expect(plan.config.secret_overrides).not.toEqual(
		existing.config.secret_overrides,
	);
	expect(JSON.stringify(plan.config)).not.toContain("new-online-secret");
	expect(JSON.stringify(plan.config)).not.toContain("updated-online-access");
	expect(() =>
		createDeploymentPlan({
			...base,
			installed: online,
			existing,
			healthChecked: true,
			resourceGrant: { ...resourceGrant, authz_version: 9 },
		}),
	).toThrow("resource and billing approvals");
	expect(() =>
		createDeploymentPlan({
			...base,
			installed: online,
			existing: {
				...existing,
				config: { ...existing.config, resource_grant: null },
			},
			healthChecked: true,
		}),
	).toThrow();
});

test("explicit rollout source capabilities can disable offline checks on an otherwise running agent", async () => {
	const base = await updateInput();
	const existing = await readExistingDeployment(
		readerFor({ ...base.existing, rollout_sources: [], rollout: null }),
		base.placement,
		installed.project_id,
	);
	expect(existing.rollout_sources).toEqual([]);
	expect(() =>
		createDeploymentPlan({ ...base, existing, healthChecked: true }),
	).toThrow("device support for its project source");
	expect(
		createDeploymentPlan({ ...base, existing }).steps[0].command.type,
	).toBe("apply");
});

test("configuration responses without authoritative desired state keep the manual update path", async () => {
	const base = {
		...(await updateInput()),
		existing: await existingPlacement(),
	};
	expect(base.existing.desired_state).toBeUndefined();
	expect(() => createDeploymentPlan({ ...base, healthChecked: true })).toThrow(
		"running HTTP",
	);
	const manual = createDeploymentPlan(base);
	expect(manual.rollout_id).toBeUndefined();
	expect(manual.steps[0].command.type).toBe("apply");
});

test("manual conversion to workflow listeners removes obsolete native hosting settings", async () => {
	const base = await updateInput();
	const plan = createDeploymentPlan({
		...base,
		replicas: 1,
		existing: {
			...base.existing,
			config: { ...base.existing.config, max_replicas: 1 },
		},
		events: [{ ...event, event_type: "mcp", hosted: false }],
	});
	expect(plan.config.hosting).toBeUndefined();
	expect(plan.rollout_id).toBeUndefined();
	expect(plan.steps.map((step) => step.command.type)).toEqual(["apply"]);
});

test("approved events with traffic variants, floating pins or unsupported types stay ineligible", () => {
	const changes: Partial<IEvent>[] = [
		{ event_type: "unsupported" },
		{
			canary: {
				board_id: "canary",
				node_id: "entry",
				weight: 1,
				created_at: time,
				updated_at: time,
				variables: {},
			},
		},
		{
			variants: [
				{
					name: "shadow",
					board_id: "shadow",
					node_id: "entry",
					mode: { Shadow: { sample_rate: 0.5 } },
					created_at: time,
					updated_at: time,
					variables: {},
				},
			],
		},
	];
	for (const change of changes) {
		const project = onlineInstalled(approvedDocuments([approvedEvent(change)]));
		const [selected] = discoverOnlineEvents(project);
		expect(selected.eligible).toBe(false);
		expect(() => discoverOnlineVariables(project, selected)).toThrow(
			"not in the approved metadata",
		);
	}
	expect(() =>
		approvedOnlineCatalog(
			approvedDocuments([approvedEvent({ board_version: null })]),
		),
	).toThrow("does not match its event identity");
});

test("a board with unconfigurable variables blocks only its own approved event", () => {
	const otherBoard = (variables: DeploymentVariable[], layers = {}) => ({
		...approvedDocuments(),
		"events/broken/versions/1/2/3": approvedEvent({
			id: "broken",
			board_id: "other",
		}),
		"boards/other/versions/3/2/1": {
			...approvedBoard([3, 2, 1], variables),
			id: "other",
			layers,
		},
	});
	const invalid = onlineInstalled(otherBoard([{ ...secret, id: "not an id" }]));
	const events = discoverOnlineEvents(invalid);
	expect(events.map((value) => [value.id, value.eligible])).toEqual([
		["api", true],
		["broken", false],
	]);
	expect(events[1]?.ineligible_reason).toContain(
		"Board other 3.2.1 cannot be deployed",
	);
	expect(events[1]?.ineligible_reason).toContain('"Credential"');
	expect(discoverOnlineVariables(invalid, events[0] ?? event)).toEqual([
		secret,
	]);
	const layered = discoverOnlineEvents(
		onlineInstalled(
			otherBoard([secret], {
				layer: {
					variables: {
						credential: { ...secret, data_type: "Integer", exposed: true },
					},
				},
			}),
		),
	);
	expect(layered[1]?.eligible).toBe(false);
	expect(layered[1]?.ineligible_reason).toContain(
		"define variable Credential differently",
	);
});

test("board variables are the ones a device may set, from the board and its layers", () => {
	const plain = { ...secret, id: "limit", name: "Limit", secret: false };
	const board = {
		variables: {
			credential: { ...secret, exposed: true },
			internal: { ...plain, id: "internal", exposed: false },
		},
		layers: {
			layer: {
				variables: {
					limit: { ...plain, exposed: false, runtime_configured: true },
					credential: { ...secret, exposed: true },
				},
			},
		},
	} as unknown as Parameters<typeof boardVariables>[0];
	expect(boardVariables(board)).toEqual([secret, plain]);
	expect(boardVariables({ variables: {}, layers: {} })).toEqual([]);
	expect(() =>
		boardVariables({
			variables: { bad: { ...secret, id: "not an id", exposed: true } },
			layers: {},
		} as unknown as Parameters<typeof boardVariables>[0]),
	).toThrow("cannot configure");
});

test("earlier online definitions propagate archive failures and refuse a different board snapshot", async () => {
	const existing = {
		...(await existingPlacement()),
		config: { ...(await existingPlacement()).config, source: "online" },
	} as PlacementConfiguration;
	const failure = new Error("Event read denied");
	const archived = approvedEvent({
		event_version: [1, 0, 0],
		board_version: [1, 0, 0],
	});
	const events = (read: () => Promise<IEvent>) =>
		({ getEventAuthoritative: read }) as unknown as IEventState;
	const boards = (change: Record<string, unknown>) =>
		({
			getBoardAuthoritative: async () => ({
				...approvedBoard([1, 0, 0], [secret]),
				...change,
			}),
		}) as unknown as IBoardState;
	await expect(
		discoverPreviousOnlineVariables(
			events(async () => {
				throw failure;
			}),
			boards({}),
			onlineInstalled(),
			existing,
		),
	).rejects.toBe(failure);
	for (const change of [{ id: "another-board" }, { version: [1, 0, 1] }])
		await expect(
			discoverPreviousOnlineVariables(
				events(async () => archived),
				boards(change),
				onlineInstalled(),
				existing,
			),
		).rejects.toThrow("board pins differ");
});
function input() {
	return {
		installed,
		placement: "api-on-device",
		deployment: "deployment",
		events: [event],
		variables: [secret],
		overrides: { credential: "workflow-secret" },
		host: "127.0.0.1",
		port: 8080,
		replicas: 2,
		serviceToken: "service-secret-".repeat(4),
	};
}

test("offline writes are opt-in, scoped, and validated before a deployment is sent", () => {
	const buffering = offlineWritesSchema.parse({
		tables: [
			{
				purpose: "storage",
				database: "db",
				table: "measurements",
				primary_key: "id",
			},
		],
		files: [{ purpose: "files", prefix: "exports" }],
	});
	expect(createDeploymentPlan(input()).config.offline_writes).toBeUndefined();
	expect(() =>
		createDeploymentPlan({ ...input(), offlineWrites: buffering }),
	).toThrow("only to online");
	const online = {
		...input(),
		replicas: 1,
		installed: { ...installed, source: "online" as const },
		resourceGrant: { grant_id: "grant", authz_version: 1 },
	};
	expect(
		createDeploymentPlan({ ...online, offlineWrites: buffering }).config
			.offline_writes,
	).toEqual(buffering);
	expect(() =>
		createDeploymentPlan({ ...online, replicas: 3, offlineWrites: buffering }),
	).toThrow(
		"requires a placement with one replica, but this placement allows 3",
	);
	for (const prefix of [
		"../escape",
		"db/table.lance",
		"files%2fother",
		"/absolute",
		"a//b",
	]) {
		expect(
			offlineWritesSchema.safeParse({
				...buffering,
				files: [{ purpose: "files", prefix }],
			}).success,
		).toBe(false);
	}
	for (const purpose of ["storage", "user"]) {
		expect(
			offlineWritesSchema.safeParse({
				...buffering,
				files: [{ purpose, prefix: "db" }],
			}).success,
		).toBe(false);
	}
	expect(
		offlineWritesSchema.safeParse({
			...buffering,
			tables: [{ ...buffering.tables[0], database: "arbitrary" }],
		}).success,
	).toBe(false);
	expect(
		offlineWritesSchema.safeParse({
			...buffering,
			tables: [buffering.tables[0], buffering.tables[0]],
		}).success,
	).toBe(false);
	expect(
		offlineWritesSchema.safeParse({ ...buffering, max_queue_bytes: 1 }).success,
	).toBe(false);
	expect(offlineWritesSchema.safeParse({ tables: [], files: [] }).success).toBe(
		false,
	);
});

test("deployment updates preserve buffering unless explicitly changed or disabled", async () => {
	const base = await updateInput();
	const buffering = offlineWritesSchema.parse({
		files: [{ purpose: "storage", prefix: "exports" }],
	});
	const online = {
		...base,
		replicas: 1,
		installed: { ...base.installed, source: "online" as const },
		existing: {
			...base.existing,
			config: {
				...base.existing.config,
				max_replicas: 1,
				source: "online" as const,
				offline_writes: buffering,
			},
		},
	};
	expect(createDeploymentPlan(online).config.offline_writes).toEqual(buffering);
	expect(
		createDeploymentPlan({ ...online, offlineWrites: null }).config
			.offline_writes,
	).toBeUndefined();
	expect(
		createDeploymentPlan({
			...online,
			offlineWrites: { ...buffering, max_operations: 123 },
		}).config.offline_writes,
	).toMatchObject({ max_operations: 123 });
});

function requiredOperationId(id: string | undefined): string {
	if (!id) throw new Error("Expected a mutation operation ID");
	return id;
}

test("placement metadata excludes private values and every operation remains stopped", () => {
	const plan = createDeploymentPlan(input());
	expect(JSON.stringify(plan.config)).not.toContain("workflow-secret");
	expect(JSON.stringify(plan.config)).not.toContain("service-secret-");
	expect(plan.steps[0].command.start).toBe(false);
	expect(plan.steps.map((step) => step.command.type)).toEqual([
		"apply",
		"set_secret",
		"set_secret",
	]);
	expect(plan.config.events).toEqual([
		{ event_id: "api", event_version: [1, 2, 3], board_version: [3, 2, 1] },
	]);
	expect(plan.steps[1].command.value).toBe('"workflow-secret"');
	expect(plan.steps[2].command.value).toBe(input().serviceToken);
});

test("token-free hosting requires an explicit choice and saves only workflow secrets", () => {
	const plan = createDeploymentPlan({
		...input(),
		serviceAuthentication: "none",
		serviceToken: "",
	});
	expect(plan.config.hosting).toMatchObject({ authentication: "none" });
	expect(plan.config.hosting).not.toHaveProperty("auth_secret");
	expect(plan.steps.map((step) => step.command.type)).toEqual([
		"apply",
		"set_secret",
	]);
	expect(plan.steps[1].command.value).toBe('"workflow-secret"');
	expect(() => createDeploymentPlan({ ...input(), serviceToken: "" })).toThrow(
		"service token",
	);
});

test("updates can remove token authentication, preserve that choice, and enable it again", async () => {
	const base = await updateInput();
	const removed = createDeploymentPlan({
		...base,
		serviceAuthentication: "none",
	});
	expect(removed.config.hosting).toMatchObject({ authentication: "none" });
	expect(removed.config.hosting).not.toHaveProperty("auth_secret");
	expect(removed.steps.map((step) => step.command.type)).toEqual(["apply"]);
	if (!base.existing.config.hosting) throw new Error("Expected hosted fixture");
	const { auth_secret: _authSecret, ...hosting } = base.existing.config.hosting;
	const existing = {
		...base.existing,
		config: {
			...base.existing.config,
			hosting: {
				...hosting,
				authentication: "none" as const,
			},
		},
	};
	expect(
		createDeploymentPlan({ ...base, existing }).config.hosting,
	).toMatchObject({
		authentication: "none",
	});
	expect(() =>
		createDeploymentPlan({
			...base,
			existing,
			serviceAuthentication: "token",
		}),
	).toThrow("service token");
	const protectedPlan = createDeploymentPlan({
		...base,
		existing,
		serviceAuthentication: "token",
		serviceToken: "new-service-token-".repeat(3),
	});
	expect(protectedPlan.config.hosting).toMatchObject({
		authentication: "token",
	});
	expect(protectedPlan.steps.map((step) => step.command.type)).toEqual([
		"apply",
		"set_secret",
	]);
	expect(protectedPlan.steps[1].command.value).toBe(
		"new-service-token-".repeat(3),
	);
	expect(() =>
		createDeploymentPlan({
			...base,
			existing: { ...existing, config: { ...existing.config, hosting } },
		}),
	).toThrow();
	expect(() =>
		createDeploymentPlan({
			...base,
			existing: {
				...existing,
				config: {
					...existing.config,
					hosting: {
						...existing.config.hosting,
						auth_secret: "leftover-token",
					},
				},
			},
		}),
	).toThrow();
});

test("selection forbids floating pins, replicated daemons, unknown overrides and missing online grant", () => {
	expect(() =>
		createDeploymentPlan({
			...input(),
			events: [{ ...event, eligible: false, board_version: null }],
		}),
	).toThrow();
	expect(() =>
		createDeploymentPlan({
			...input(),
			events: [{ ...event, hosted: false, event_type: "daemon" }],
		}),
	).toThrow();
	// Two instances would start every scheduled run twice.
	const schedule: DeploymentEvent = {
		...event,
		hosted: false,
		event_type: "cron",
		kind: "scheduled",
		readiness_kind: "explicit",
		rollout_supported: true,
	};
	expect(() =>
		createDeploymentPlan({ ...input(), events: [schedule] }),
	).toThrow("multiple replicas");
	expect(() =>
		createDeploymentPlan({
			...input(),
			events: [event, { ...schedule, id: "nightly" }],
		}),
	).toThrow("multiple replicas");
	const alone = createDeploymentPlan({
		...input(),
		replicas: 1,
		events: [schedule],
	});
	expect(alone.config.hosting).toBeUndefined();
	expect(alone.config.max_replicas).toBe(1);
	expect(alone.config.events).toEqual([
		{ event_id: "api", event_version: [1, 2, 3], board_version: [3, 2, 1] },
	]);
	expect(() =>
		createDeploymentPlan({ ...input(), overrides: { unknown: "secret" } }),
	).toThrow();
	expect(() =>
		createDeploymentPlan({
			...input(),
			installed: { ...installed, source: "online" },
		}),
	).toThrow();
	expect(() =>
		createDeploymentPlan({ ...input(), placement: "device" }),
	).toThrow();
	expect(() =>
		createDeploymentPlan({ ...input(), host: "example.com" }),
	).toThrow();
});
test("a placement pins one version per node package, as the device requires", () => {
	const pin = (version: string) => ({
		package_id: "tokenizer",
		version,
		wasm_sha256: "a".repeat(64),
		manifest_sha256: "b".repeat(64),
	});
	const withPins = (versions: string[]) => ({
		...input(),
		installed: {
			...installed,
			assets: { bit_pins: [], package_pins: versions.map(pin) },
		},
	});
	expect(createDeploymentPlan(withPins(["1.0.0"])).config.package_pins).toEqual(
		[pin("1.0.0")],
	);
	expect(() => createDeploymentPlan(withPins(["1.0.0", "2.0.0"]))).toThrow(
		"several versions of node package tokenizer",
	);
});
test("variable parsing keeps literal strings and validates primitives and containers", () => {
	expect(variableValue(secret, "${HOME}")).toBe("${HOME}");
	expect(variableValue({ ...secret, data_type: "Boolean" }, "true")).toBe(true);
	expect(() =>
		variableValue({ ...secret, data_type: "Boolean" }, '"true"'),
	).toThrow();
	expect(() =>
		variableValue({ ...secret, data_type: "Integer" }, "1.5"),
	).toThrow();
	expect(() =>
		variableValue({ ...secret, data_type: "Byte" }, "256"),
	).toThrow();
	expect(() =>
		variableValue({ ...secret, value_type: "Array" }, "{} "),
	).toThrow();
	expect(() =>
		variableValue({ ...secret, value_type: "HashMap" }, "[]"),
	).toThrow();
	expect(
		variableValue({ ...secret, value_type: "Array" }, '["a","b"]'),
	).toEqual(["a", "b"]);
	expect(mergeVariables([[secret], [{ ...secret }]])).toEqual([secret]);
	expect(() =>
		mergeVariables([[secret], [{ ...secret, secret: false }]]),
	).toThrow();
});
test("accepted Apply advances only after exact revision binding; secrets wait for completed publication", async () => {
	const plan = createDeploymentPlan(input());
	const calls: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command, id) => {
		calls.push(command);
		if (command.type === "apply")
			return {
				operation_id: requiredOperationId(id),
				state: "accepted",
				result: { placement_id: plan.config.id, config_revision: 1 },
			};
		if (command.type === "set_secret")
			return {
				operation_id: requiredOperationId(id),
				state: "accepted",
				result: {
					secret: "pending",
					placement_id: plan.config.id,
					name: command.name,
				},
			};
		const step = plan.steps.find((step) => step.id === command.operation_id);
		if (!step) throw new Error("Expected a known deployment operation");
		return {
			operation_id: step.id,
			state: "completed",
			result: {
				placement_id: plan.config.id,
				name: step.command.name,
				secret: "completed",
			},
		};
	};
	await executeDeploymentPlan(call, plan);
	expect(calls.map((value) => value.type)).toEqual([
		"apply",
		"set_secret",
		"operation",
		"set_secret",
		"operation",
	]);
	expect(calls.some((value) => value.type === "start")).toBe(false);
	const wrong: ManagementCall = async () => ({
		operation_id: "wrong",
		state: "accepted",
		result: { placement_id: "other", config_revision: 1 },
	});
	await expect(
		executeDeploymentPlan(wrong, createDeploymentPlan(input())),
	).rejects.toThrow();
});
test("a lost response retries by reading the existing journal before any further mutation", async () => {
	const plan = createDeploymentPlan({
		...input(),
		events: [{ ...event, event_type: "daemon", hosted: false }],
		replicas: 1,
	});
	let lost = true;
	const calls: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command, id) => {
		calls.push(command);
		if (command.type === "apply" && lost) {
			lost = false;
			throw new Error("response lost");
		}
		if (command.type === "operation")
			return {
				operation_id: command.operation_id as string,
				state: "accepted",
				result: { placement_id: plan.config.id, config_revision: 1 },
			};
		return {
			operation_id: requiredOperationId(id),
			state: "completed",
			result: {
				placement_id: plan.config.id,
				name: command.name,
				secret: "completed",
			},
		};
	};
	await expect(executeDeploymentPlan(call, plan)).rejects.toThrow(
		"response lost",
	);
	await executeDeploymentPlan(call, plan);
	expect(calls.map((value) => value.type)).toEqual([
		"apply",
		"operation",
		"set_secret",
	]);
});
test("offline discovery validates project, revision, order and pagination progress", async () => {
	let count = 0;
	const call: ManagementCall = async (command) => {
		const request = command.request as Record<string, unknown>;
		count++;
		expect(request.project_id).toBe("project");
		return {
			operation_id: "read",
			state: "completed",
			result: {
				project_id: "project",
				revision: installed.revision,
				event_id: null,
				items: [{ ...event, id: count === 1 ? "a" : "b" }],
				next: count === 1 ? "a" : null,
			},
		};
	};
	expect(
		(await discoverOfflineEvents(call, installed)).map((event) => event.id),
	).toEqual(["a", "b"]);
	const invalid: ManagementCall = async () => ({
		operation_id: "read",
		state: "completed",
		result: {
			project_id: "project",
			revision: installed.revision,
			event_id: null,
			items: [event],
			next: "wrong",
		},
	});
	await expect(discoverOfflineEvents(invalid, installed)).rejects.toThrow();
	const foreign: ManagementCall = async () => ({
		operation_id: "read",
		state: "completed",
		result: {
			project_id: "other",
			revision: installed.revision,
			event_id: null,
			items: [],
			next: null,
		},
	});
	await expect(discoverOfflineEvents(foreign, installed)).rejects.toThrow();
});

/* run-more-design §1.3: the literal a device sends for a schedule it can run. */
const SCHEDULE_ROW = JSON.parse(
	`{"id":"evt_report","name":"Nightly report","event_type":"cron","event_version":[0,0,3],"board_version":[0,0,7],
 "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
 "kind":"scheduled","schedule":{"expression":"0 0 2 * * *","timezone":"Europe/Berlin"},"ineligible_code":null}`,
);

test("offline discovery reads a schedule row and drops facts it does not know", async () => {
	const discover = (items: unknown[]) =>
		discoverOfflineEvents(
			async () => ({
				operation_id: "read",
				state: "completed",
				result: {
					project_id: "project",
					revision: installed.revision,
					event_id: null,
					items,
					next: null,
				},
			}),
			installed,
		);
	const [row] = await discover([SCHEDULE_ROW]);
	expect(row).toMatchObject({
		id: "evt_report",
		eligible: true,
		hosted: false,
		readiness_kind: "explicit",
		rollout_supported: true,
		kind: "scheduled",
		schedule: { expression: "0 0 2 * * *", timezone: "Europe/Berlin" },
	});
	expect(row.ineligible_code ?? null).toBeNull();
	const [refused] = await discover([
		{
			...SCHEDULE_ROW,
			eligible: false,
			rollout_supported: false,
			schedule: null,
			ineligible_code: "schedule_too_often",
			readiness_error:
				"Schedule */30 * * * * * runs more often than once a minute.",
		},
	]);
	expect([refused.ineligible_code, refused.readiness_error]).toEqual([
		"schedule_too_often",
		"Schedule */30 * * * * * runs more often than once a minute.",
	]);
	// An agent newer than this client: unknown values are dropped, the row and the discovery stay;
	// a kind this client does not know can't run here, whatever the device says.
	const [newer, older] = await discover([
		{
			...SCHEDULE_ROW,
			kind: "webhook",
			ineligible_code: "schedule_paused_elsewhere",
			schedule: { expression: 7 },
			readiness_error: 404,
			a_later_fact: true,
		},
		{ ...event, id: "older" },
	]);
	expect(newer).toMatchObject({ id: "evt_report", eligible: false });
	expect([
		newer.kind,
		newer.schedule,
		newer.ineligible_code,
		newer.readiness_error,
	]).toEqual([undefined, undefined, undefined, undefined]);
	expect("a_later_fact" in newer).toBe(false);
	expect(older).toEqual({ ...event, id: "older" });
	// `readiness_kind` stays a closed enum on every client, so a device never sends a fourth value.
	await expect(
		discover([{ ...SCHEDULE_ROW, readiness_kind: "armed" }]),
	).rejects.toThrow();
});
/* run-more-2-design §1.5: the literals a device sends for an Endpoint, a one-time schedule, a form and a bot. */
const ROUND_TWO_ROWS = [
	`{"id":"evt_helper","name":"Helper","event_type":"telegram","event_version":[0,0,1],"board_version":[0,0,3],
 "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
 "kind":"bot","ineligible_code":null}`,
	`{"id":"evt_notes_form","name":"New note","event_type":"generic_form","event_version":[1,0,0],"board_version":[3,0,1],
 "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
 "kind":"on_demand","ineligible_code":null}`,
	`{"id":"evt_once","name":"Migration","event_type":"cron","event_version":[0,0,4],"board_version":[0,0,7],
 "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
 "kind":"scheduled","once":{"date":"2026-09-24","time":"09:00","at":1790233200,"timezone":"Europe/Berlin"},"ineligible_code":null}`,
	`{"id":"evt_orders","name":"Orders","event_type":"api","event_version":[0,0,2],"board_version":[0,0,5],
 "hosted":true,"eligible":true,"readiness_kind":"listener","rollout_supported":true,"readiness_error":null,
 "kind":"served","route":{"method":"GET","path":"/orders"},"ineligible_code":null}`,
].map((row) => JSON.parse(row));

async function discoverRows(items: unknown[]) {
	return discoverOfflineEvents(
		async () => ({
			operation_id: "read",
			state: "completed",
			result: {
				project_id: "project",
				revision: installed.revision,
				event_id: null,
				items,
				next: null,
			},
		}),
		installed,
	);
}

test("offline discovery reads the rows of Endpoints, one-time schedules, forms and bots", async () => {
	const [helper, form, once, orders] = await discoverRows(ROUND_TWO_ROWS);
	expect([helper.kind, form.kind, once.kind, orders.kind]).toEqual([
		"bot",
		"on_demand",
		"scheduled",
		"served",
	]);
	expect([helper, form, once, orders].every((row) => row.eligible)).toBe(true);
	expect(orders.route).toEqual({ method: "GET", path: "/orders" });
	expect(once.once).toEqual({
		date: "2026-09-24",
		time: "09:00",
		at: 1790233200,
		timezone: "Europe/Berlin",
	});
	// 2026-09-24 09:00 in Berlin is 07:00 UTC, the instant the row carries.
	expect(
		eventEligibility(
			approvedEvent(
				cron({
					scheduled_for: { date: "2026-09-24", time: "09:00" },
					timezone: "Europe/Berlin",
				}),
			),
		).once?.at,
	).toBe(1790233200);
	// A one-time row has no `schedule`: a parser that needs an expression never meets one.
	expect("schedule" in once).toBe(false);
	const [refused] = await discoverRows([
		{
			...ROUND_TWO_ROWS[3],
			eligible: false,
			route: null,
			ineligible_code: "route_reserved",
			readiness_error: "HTTP event path is reserved by the service host",
		},
	]);
	expect([refused.ineligible_code, refused.route]).toEqual([
		"route_reserved",
		null,
	]);
	// A route or an instant a device can't send is dropped, the row stays.
	const [bent] = await discoverRows([
		{
			...ROUND_TWO_ROWS[3],
			route: { method: "TRACE", path: "/orders" },
		},
	]);
	expect([bent.eligible, bent.route]).toEqual([true, undefined]);
	const [late] = await discoverRows([
		{ ...ROUND_TWO_ROWS[2], once: { ...ROUND_TWO_ROWS[2].once, at: 1 } },
	]);
	expect(late.once).toBeUndefined();
	// An agent built without a part answers as today.
	const [older] = await discoverRows([
		{
			...ROUND_TWO_ROWS[0],
			eligible: false,
			readiness_kind: "unsupported",
			kind: undefined,
		},
	]);
	expect([older.eligible, older.kind, older.ineligible_code ?? null]).toEqual([
		false,
		undefined,
		null,
	]);
});

const withConfig = (event_type: string, config: unknown): Partial<IEvent> => ({
	event_type,
	config: configBytes(config),
});

test("each event needs the agent flags of its kind; a Page needs none", () => {
	const cases: [Partial<IEvent>, AgentFeature[]][] = [
		[withConfig("http", { path: "/orders", method: "get" }), []],
		[withConfig("http", { path: "orders" }), ["api_events"]],
		[withConfig("http", { path: "/orders" }), ["api_events"]],
		[
			withConfig("http", { path_suffix: "/a/b", method: "PUT" }),
			["api_events"],
		],
		[withConfig("api", API_DEFAULT), ["api_events"]],
		[{ event_type: "simple_chat" }, []],
		[{ event_type: "rest" }, []],
		[{ event_type: "daemon" }, []],
		[{ event_type: "cron", config: nightly }, ["scheduled_events"]],
		[
			cron({ scheduled_for: { date: "2026-12-24", time: "18:00" } }),
			["scheduled_events", "scheduled_once"],
		],
		[{ event_type: "quick_action" }, ["on_demand_events"]],
		[{ event_type: "generic_form" }, ["on_demand_events"]],
		[withConfig("telegram", TELEGRAM_DEFAULT), ["telegram_bots"]],
		[withConfig("discord", DISCORD_DEFAULT), ["discord_bots"]],
		[{ event_type: "telegram", default_page_id: "page" }, []],
		[{ event_type: "generic_form", default_page_id: "page" }, []],
		[{ event_type: "email" }, []],
	];
	for (const [change, features] of cases)
		expect([
			change.event_type,
			requiredFeatures(approvedEvent(change)),
		]).toEqual([change.event_type, features]);
	const once = approvedEvent(
		cron({ scheduled_for: { date: "2026-12-24", time: "18:00" } }),
	);
	expect(missingFeature(once, undefined)).toBe("unknown");
	expect(missingFeature(once, {})).toBe("scheduled_events");
	expect(missingFeature(once, { scheduled_events: 1 })).toBe("scheduled_once");
	expect(
		missingFeature(once, { scheduled_events: 1, scheduled_once: 1 }),
	).toBeNull();
	expect(missingFeature(approvedEvent(), undefined)).toBeNull();
});

test("person-started events do not keep a service at one instance; claimed ones do", () => {
	expect(EVENT_KINDS.filter(limitsInstances)).toEqual([
		"own_server",
		"background",
		"scheduled",
		"bot",
	]);
	for (const [event_type, kind] of [
		["http", "served"],
		["api", "served"],
		["simple_chat", "served"],
		["rest", "own_server"],
		["mcp", "own_server"],
		["daemon", "background"],
		["cron", "scheduled"],
		["quick_action", "on_demand"],
		["generic_form", "on_demand"],
		["telegram", "bot"],
		["discord", "bot"],
		["teams", null],
		["email", null],
		["deeplink", null],
	] as const)
		expect([event_type, eventKind({ event_type })]).toEqual([event_type, kind]);
	expect(eventKind({ event_type: "telegram", default_page_id: "p" })).toBe(
		"served",
	);
});

test("an online app's hub hands an Endpoint, form or bot to devices only when it lists the type", () => {
	const types = [
		["api", withConfig("api", API_DEFAULT)],
		["quick_action", { event_type: "quick_action" }],
		["generic_form", { event_type: "generic_form" }],
		["telegram", withConfig("telegram", TELEGRAM_DEFAULT)],
		["discord", withConfig("discord", DISCORD_DEFAULT)],
	] as const;
	for (const [type, change] of types) {
		const record = approvedEvent(change);
		expect(eventEligibility(record).code).toBeNull();
		expect(eventEligibility(record, { hubTypes: [] }).code).toBe("hub_type");
		expect(eventEligibility(record, { hubTypes: ["http", "cron"] }).code).toBe(
			"hub_type",
		);
		expect(eventEligibility(record, { hubTypes: [type] }).code).toBeNull();
		// A Page goes whatever its type.
		expect(
			eventEligibility({ ...record, default_page_id: "page" }, { hubTypes: [] })
				.code,
		).toBeNull();
	}
	for (const change of [
		{},
		{ event_type: "simple_chat" },
		{ event_type: "cron", config: nightly },
		cron({ scheduled_for: { date: "2026-12-24", time: "18:00" } }),
	])
		expect(eventEligibility(approvedEvent(change), { hubTypes: [] }).code).toBe(
			null,
		);
	// The event's own rule comes first, the hub after it, the bundle and the device last.
	expect(
		eventEligibility(approvedEvent(withConfig("discord", { intents: ["X"] })), {
			hubTypes: [],
		}).code,
	).toBe("bot_invalid");
	expect(
		eventEligibility(approvedEvent(withConfig("api", { path: "/ui" })), {
			hubTypes: [],
		}).code,
	).toBe("route_reserved");
	expect(
		eventEligibility(approvedEvent({ event_type: "generic_form" }), {
			hubTypes: [],
			ineligibleReason: "Board board 3.2.1 cannot be deployed",
		}).code,
	).toBe("hub_type");
});

test("the rule names what a device can't read, and what a runnable event shows", () => {
	const invalid = eventEligibility(
		approvedEvent(withConfig("http", { path: "/x", method: "TRACE" })),
	);
	expect([invalid.code, invalid.detail, invalid.rolloutSupported]).toEqual([
		"route_invalid",
		"TRACE",
		false,
	]);
	const reserved = eventEligibility(
		approvedEvent(withConfig("api", { path: "/channels/1" })),
	);
	expect([reserved.code, reserved.detail]).toEqual([
		"route_reserved",
		"/channels/1",
	]);
	const bot = eventEligibility(
		approvedEvent({
			...withConfig("telegram", { chat_whitelist: "all" }),
		}),
	);
	expect([bot.code, bot.detail, bot.bot]).toEqual([
		"bot_invalid",
		"chat_whitelist",
		undefined,
	]);
	const longId = eventEligibility({
		...approvedEvent(withConfig("telegram", TELEGRAM_DEFAULT)),
		id: "e".repeat(113),
	});
	expect([longId.code, longId.detail]).toEqual(["bot_invalid", "id"]);
	const endpoint = eventEligibility(
		approvedEvent(withConfig("api", API_DEFAULT)),
	);
	expect([endpoint.route, endpoint.rolloutSupported, endpoint.bot]).toEqual([
		{ method: "GET", path: "/cm1abc" },
		true,
		undefined,
	]);
	const telegram = eventEligibility(
		approvedEvent(withConfig("telegram", TELEGRAM_DEFAULT)),
	);
	expect(telegram.bot).toEqual({
		provider: "telegram",
		open: true,
		savedToken: false,
		prefix: "/",
		mentions: true,
	});
	expect(telegram.readiness).toBe("explicit");
	// A Discord bot's prefix is read like a Telegram bot's, and refused like it.
	const discord = eventEligibility(
		approvedEvent(withConfig("discord", DISCORD_DEFAULT)),
	);
	expect(discord.bot).toEqual({
		provider: "discord",
		open: true,
		savedToken: false,
		prefix: "!",
		mentions: true,
	});
	const longPrefix = eventEligibility(
		approvedEvent(
			withConfig("discord", {
				...DISCORD_DEFAULT,
				command_prefix: "x".repeat(17),
			}),
		),
	);
	expect([longPrefix.code, longPrefix.detail, longPrefix.bot]).toEqual([
		"bot_invalid",
		"command_prefix",
		undefined,
	]);
	const page = eventEligibility(
		approvedEvent({
			...withConfig("api", { path: "/services" }),
			default_page_id: "page",
		}),
	);
	expect([page.code, page.route]).toEqual([null, undefined]);
	// Whether its time has passed never makes a one-time schedule ineligible.
	const past = eventEligibility(
		approvedEvent(
			cron({ scheduled_for: { date: "2001-01-01", time: "00:00" } }),
		),
	);
	expect([past.eligible, past.once?.at, past.schedule]).toEqual([
		true,
		978307200,
		undefined,
	]);
	const gap = eventEligibility(
		approvedEvent(
			cron({
				scheduled_for: { date: "2027-03-28", time: "02:30" },
				timezone: "Europe/Berlin",
			}),
		),
	);
	expect([gap.code, gap.scheduleDetail]).toEqual([
		"schedule_invalid",
		{ code: "gap", date: "2027-03-28", time: "02:30", zone: "Europe/Berlin" },
	]);
});

const discovered = (change: Partial<DeploymentEvent>): DeploymentEvent => ({
	...event,
	...change,
});
const BOT: DeploymentEvent = discovered({
	id: "evt_helper",
	name: "Helper",
	event_type: "telegram",
	hosted: false,
	kind: "bot",
	readiness_kind: "explicit",
	rollout_supported: true,
});
const FORM: DeploymentEvent = discovered({
	id: "evt_notes_form",
	name: "New note",
	event_type: "generic_form",
	hosted: false,
	kind: "on_demand",
	readiness_kind: "explicit",
	rollout_supported: true,
});
const TOKEN = "123456789:AAH-secret_token_value_123";

test("hosting forms and quick actions requires an explicit opt-in and reuses service authentication", () => {
	const base = { ...input(), events: [FORM], replicas: 1 };
	expect(createDeploymentPlan(base).config.hosting).toBeUndefined();
	const hosted = createDeploymentPlan({ ...base, hostOnDemand: true });
	expect(hosted.config.hosting).toMatchObject({
		host: "127.0.0.1",
		port: 8080,
		auth_secret: "service-access",
	});
	expect(
		hosted.steps.some(
			(step) =>
				step.command.type === "set_secret" &&
				step.command.value === base.serviceToken,
		),
	).toBe(true);
	expect(() =>
		createDeploymentPlan({ ...base, hostOnDemand: true, serviceToken: "" }),
	).toThrow("service token");
	const publicPlan = createDeploymentPlan({
		...base,
		hostOnDemand: true,
		serviceToken: "",
		serviceAuthentication: "none",
	});
	expect(publicPlan.config.hosting).toMatchObject({ authentication: "none" });
	expect(publicPlan.config.hosting).not.toHaveProperty("auth_secret");
	const botOnly = createDeploymentPlan({
		...base,
		events: [BOT],
		hostOnDemand: true,
		overrides: { ...base.overrides, [botTokenKey(BOT.id)]: TOKEN },
	});
	expect(botOnly.config.hosting).toBeUndefined();
});

test("enabling a form listener stages its token before a health-checked update", async () => {
	const base = await updateInput();
	const { hosting: _hosting, ...unhosted } = base.existing.config;
	const token = "new-form-listener-access-token-0123456789";
	const plan = createDeploymentPlan({
		...base,
		events: [FORM],
		replicas: 1,
		existing: { ...base.existing, config: { ...unhosted, max_replicas: 1 } },
		hostOnDemand: true,
		healthChecked: true,
		serviceToken: token,
	});
	expect(plan.steps.map((step) => step.command.type)).toEqual([
		"stage_rollout",
		"rollout_secret",
		"activate_rollout",
	]);
	const hosting = plan.config.hosting as Record<string, unknown>;
	expect(hosting.auth_secret).toBeTruthy();
	expect(plan.steps[1].command).toMatchObject({
		name: hosting.auth_secret,
		value: token,
		rollout_id: plan.rollout_id,
	});
	expect(JSON.stringify(plan.config)).not.toContain(token);
});

test("an on-demand-only update preserves its explicitly configured Studio listener", async () => {
	const update = await updateInput();
	const hosting = update.existing.config.hosting;
	if (!hosting) throw new Error("Missing fixture hosting settings");
	const { auth_secret: _authSecret, ...publicHosting } = hosting;
	const plan = createDeploymentPlan({ ...update, events: [FORM] });
	expect(plan.config.hosting).toMatchObject({
		...update.existing.config.hosting,
		host: update.host,
		port: update.port,
	});
	expect(plan.config.max_replicas).toBe(2);
	expect(plan.steps.some((step) => step.command.type === "set_secret")).toBe(
		false,
	);
	const publicUpdate = {
		...update,
		existing: {
			...update.existing,
			config: {
				...update.existing.config,
				hosting: {
					...publicHosting,
					authentication: "none" as const,
				},
			},
		},
	};
	expect(
		createDeploymentPlan({ ...publicUpdate, events: [FORM] }).config.hosting,
	).toMatchObject({ authentication: "none" });
	expect(
		createDeploymentPlan({
			...update,
			existing: {
				...update.existing,
				config: { ...update.existing.config, max_replicas: 1 },
			},
			events: [BOT],
			replicas: 1,
			overrides: { ...update.overrides, [botTokenKey(BOT.id)]: TOKEN },
		}).config.hosting,
	).toBeUndefined();
});

test("a form beside a served event keeps its instances; a bot or an older row without a kind keeps one", () => {
	const plan = createDeploymentPlan({ ...input(), events: [event, FORM] });
	expect(plan.config.max_replicas).toBe(2);
	expect(() => createDeploymentPlan({ ...input(), events: [FORM] })).toThrow(
		"multiple replicas",
	);
	const alone = createDeploymentPlan({
		...input(),
		replicas: 1,
		events: [FORM],
	});
	expect(alone.config.hosting).toBeUndefined();
	expect(() =>
		createDeploymentPlan({
			...input(),
			events: [event, BOT],
			overrides: { [botTokenKey(BOT.id)]: TOKEN },
		}),
	).toThrow("multiple replicas");
	expect(() =>
		createDeploymentPlan({
			...input(),
			events: [event, { ...FORM, kind: undefined }],
		}),
	).toThrow("multiple replicas");
});

test("a bot's token is a secret setting of its event, checked and stored by reference only", () => {
	const single = { ...input(), replicas: 1, events: [BOT], variables: [] };
	const plan = createDeploymentPlan({
		...single,
		overrides: { [botTokenKey(BOT.id)]: ` ${TOKEN}\n` },
	});
	const references = plan.config.secret_overrides as Record<string, string>;
	expect(Object.keys(references)).toEqual(["event.evt_helper.bot_token"]);
	expect(references["event.evt_helper.bot_token"]).toMatch(/^variable-/);
	expect(JSON.stringify(plan.config)).not.toContain(TOKEN);
	const write = plan.steps.find((step) => step.command.type === "set_secret");
	expect(write?.command).toMatchObject({
		name: references["event.evt_helper.bot_token"],
		value: JSON.stringify(TOKEN),
	});
	expect(() => createDeploymentPlan({ ...single, overrides: {} })).toThrow(
		"Bot Helper needs its bot token. Enter it under Settings.",
	);
	expect(() =>
		createDeploymentPlan({
			...single,
			overrides: { [botTokenKey(BOT.id)]: "my token" },
		}),
	).toThrow("doesn't look like a Telegram bot token");
	// A flow variable that claims the reserved id never takes a value.
	expect(() =>
		createDeploymentPlan({
			...input(),
			variables: [{ ...secret, id: botTokenKey("api") }],
			overrides: { [botTokenKey("api")]: TOKEN },
		}),
	).toThrow("outside the selected events");
});

test("a stored bot token stays with its bot, and leaves when the bot leaves", async () => {
	const existing = await existingPlacement();
	const stored = {
		...existing,
		config: {
			...existing.config,
			max_replicas: 1,
			hosting: null,
			events: [
				{
					event_id: BOT.id,
					event_version: [1, 2, 3] as [number, number, number],
					board_version: [3, 2, 1] as [number, number, number],
				},
			],
			variables: {},
			secret_overrides: { [botTokenKey(BOT.id)]: "variable-token" },
		},
	};
	const kept = createDeploymentPlan({
		...input(),
		existing: stored,
		replicas: 1,
		events: [BOT, FORM],
		variables: [],
		overrides: {},
		serviceToken: "",
	});
	expect(kept.config.secret_overrides).toEqual({
		[botTokenKey(BOT.id)]: "variable-token",
	});
	expect(() =>
		createDeploymentPlan({
			...input(),
			existing: stored,
			replicas: 1,
			events: [FORM],
			variables: [],
			overrides: {},
			serviceToken: "",
		}),
	).toThrow(
		`Stored override ${botTokenKey(BOT.id)} is outside the selected events`,
	);
	const removed = createDeploymentPlan({
		...input(),
		existing: stored,
		replicas: 1,
		events: [FORM],
		variables: [],
		overrides: {},
		serviceToken: "",
		removeOverrides: [botTokenKey(BOT.id)],
	});
	expect(removed.config.secret_overrides).toEqual({});
});

const storedConfig = {
	id: "api-on-device",
	project_id: "project",
	deployment_id: "deployment",
	revision: "b".repeat(64),
	source: "offline",
	project_path: "/private/projects/project/revisions/old",
	events: [
		{ event_id: "api", event_version: [1, 0, 0], board_version: [1, 0, 0] },
	],
	artifact_pins: [{ kind: "widget", id: "chart", version: [1, 0, 0] }],
	package_pins: [],
	bit_pins: [],
	hosting: {
		host: "0.0.0.0",
		port: 9000,
		max_in_flight: 8,
		request_timeout_secs: 30,
		auth_secret: "service-access",
	},
	max_replicas: 2,
	variables: { greeting: "hello", retired: 1 },
	secret_overrides: { credential: "variable-stored" },
	resource_grant: { grant_id: "grant", authz_version: 3 },
	restart: { initial_backoff_secs: 5, max_backoff_secs: 90, max_restarts: 2 },
};
const greeting: DeploymentVariable = {
	id: "greeting",
	name: "Greeting",
	data_type: "String",
	value_type: "Normal",
	secret: false,
};
function readerFor(
	result: Record<string, unknown>,
	state = "completed",
): ManagementCall {
	return async (command) => {
		expect(command).toEqual({
			type: "placement_configuration",
			placement_id: "api-on-device",
		});
		return { operation_id: "read", state, result };
	};
}
async function existingPlacement(): Promise<PlacementConfiguration> {
	return readExistingDeployment(
		readerFor({
			placement_id: "api-on-device",
			project_id: "project",
			deployment_id: "deployment",
			config_revision: 4,
			config: storedConfig,
		}),
		"api-on-device",
		"project",
	);
}
async function updateInput() {
	return {
		...input(),
		existing: {
			...(await existingPlacement()),
			desired_state: "running" as const,
		},
		removeOverrides: ["retired"],
		variables: [secret, greeting],
		overrides: { greeting: "welcome" },
		serviceToken: "",
	};
}

test("device certificates are selected per placement and preserved or explicitly removed on updates", async () => {
	const certificateId = "00000000-0000-4000-8000-000000000001";
	const replacement = "00000000-0000-4000-8000-000000000002";
	expect(
		createDeploymentPlan({ ...input(), tlsCertificateId: certificateId }).config
			.tls_certificate_id,
	).toBe(certificateId);
	const update = await updateInput();
	update.existing.config.tls_certificate_id = certificateId;
	expect(createDeploymentPlan(update).config.tls_certificate_id).toBe(
		certificateId,
	);
	expect(
		createDeploymentPlan({ ...update, tlsCertificateId: replacement }).config
			.tls_certificate_id,
	).toBe(replacement);
	expect(
		createDeploymentPlan({ ...update, tlsCertificateId: null }).config
			.tls_certificate_id,
	).toBeUndefined();
	expect(() =>
		createDeploymentPlan({ ...input(), tlsCertificateId: "../private-key" }),
	).toThrow();
	const workflow = {
		...event,
		hosted: false,
		event_type: "mcp",
		readiness_kind: "listener" as const,
	};
	expect(
		createDeploymentPlan({
			...input(),
			events: [workflow],
			replicas: 1,
			tlsCertificateId: certificateId,
		}).config.tls_certificate_id,
	).toBe(certificateId);
});

test("isolation limits are complete and retained across project updates", async () => {
	const limits = {
		profile: "linux_sandbox" as const,
		cpu_millis: 1000,
		memory_bytes: 1024 ** 3,
		max_processes: 128,
		disk_bytes: 4 * 1024 ** 3,
	};
	expect(
		placementResourcesSchema.safeParse({ ...limits, disk_bytes: null }).success,
	).toBe(false);
	expect(
		placementResourcesSchema.safeParse({
			profile: "trusted_process",
			memory_bytes: 1024 ** 3,
		}).success,
	).toBe(false);
	const update = await updateInput();
	update.existing.config.resources = limits;
	expect(createDeploymentPlan(update).config.resources).toEqual(limits);
	expect(
		createDeploymentPlan({ ...update, resourceLimits: null }).config.resources,
	).toBeUndefined();
	expect(
		createDeploymentPlan({ ...input(), resourceLimits: limits }).config
			.resources,
	).toEqual(limits);
});

test("existing placement reads bind the placement, project and revision", async () => {
	const existing = await existingPlacement();
	expect(existing.config_revision).toBe(4);
	expect(existing.deployment_id).toBe("deployment");
	expect(existing.config.secret_overrides).toEqual({
		credential: "variable-stored",
	});
	await expect(
		readExistingDeployment(
			readerFor({
				placement_id: "api-on-device",
				project_id: "project",
				deployment_id: "deployment",
				config_revision: 4,
				config: { ...storedConfig, project_id: "other" },
			}),
			"api-on-device",
			"project",
		),
	).rejects.toThrow("does not match");
	await expect(
		readExistingDeployment(
			readerFor({ error: "rejected" }, "rejected"),
			"api-on-device",
			"project",
		),
	).rejects.toThrow("deploy access");
});
test("an update stops on CAS apply then publishes replacement secrets at the new revision", async () => {
	const plan = createDeploymentPlan({
		...(await updateInput()),
		overrides: { greeting: "welcome", credential: "rotated-secret" },
		serviceToken: "rotated-token-".repeat(4),
	});
	expect(plan.expected_revision).toBe(4);
	expect(plan.steps.map((step) => step.command.type)).toEqual([
		"apply",
		"set_secret",
		"set_secret",
	]);
	expect(plan.steps.map((step) => step.command.expected_revision)).toEqual([
		4, 5, 5,
	]);
	expect(plan.config.variables).toEqual({ greeting: "welcome" });
	expect(plan.config.restart).toEqual(storedConfig.restart);
	expect(plan.config.artifact_pins).toEqual(storedConfig.artifact_pins);
	expect(plan.config.resource_grant).toEqual(storedConfig.resource_grant);
	expect(plan.config.project_path).toBe(installed.project_path);
	const hosting = plan.config.hosting as Record<string, unknown>;
	expect(hosting.max_in_flight).toBe(8);
	expect(hosting.request_timeout_secs).toBe(30);
	expect(hosting.port).toBe(8080);
	expect(hosting.auth_secret).not.toBe("service-access");
	expect(plan.steps[2].command.name).toBe(hosting.auth_secret);
	const references = plan.config.secret_overrides as Record<string, string>;
	expect(references.credential).not.toBe("variable-stored");
	expect(JSON.stringify(plan.config)).not.toContain("rotated-");
});
test("a blank replacement keeps stored secrets and tokens; stray overrides must be resolved", async () => {
	const base = await updateInput();
	const plan = createDeploymentPlan(base);
	expect(plan.steps.map((step) => step.command.type)).toEqual(["apply"]);
	expect(plan.config.secret_overrides).toEqual({
		credential: "variable-stored",
	});
	expect((plan.config.hosting as Record<string, unknown>).auth_secret).toBe(
		"service-access",
	);
	expect(() => createDeploymentPlan({ ...base, removeOverrides: [] })).toThrow(
		"Stored override retired",
	);
	expect(() =>
		createDeploymentPlan({
			...base,
			variables: [{ ...secret, secret: false }, greeting],
		}),
	).toThrow("changed between public and secret");
	expect(() =>
		createDeploymentPlan({ ...base, deployment: "another-deployment" }),
	).toThrow("keeps its identity");
});
test("a rejected update reports a stale revision when the placement moved on", async () => {
	const plan = createDeploymentPlan(await updateInput());
	const types: unknown[] = [];
	const stale: ManagementCall = async (command, id) => {
		types.push(command.type);
		if (command.type === "placement_configuration")
			return {
				operation_id: "read",
				state: "completed",
				result: {
					placement_id: "api-on-device",
					project_id: "project",
					deployment_id: "deployment",
					config_revision: 5,
					config: storedConfig,
				},
			};
		return {
			operation_id: requiredOperationId(id),
			state: "rejected",
			result: { error: "rejected" },
		};
	};
	await expect(executeDeploymentPlan(stale, plan)).rejects.toBeInstanceOf(
		StaleDeploymentRevisionError,
	);
	expect(types).toEqual(["apply", "placement_configuration"]);
	for (const revision of [5]) {
		const confirmed = createDeploymentPlan(await updateInput());
		await executeDeploymentPlan(
			async (_command, id) => ({
				operation_id: requiredOperationId(id),
				state: "accepted",
				result: { placement_id: "api-on-device", config_revision: revision },
			}),
			confirmed,
		);
	}
	await expect(
		executeDeploymentPlan(
			async (_command, id) => ({
				operation_id: requiredOperationId(id),
				state: "accepted",
				result: { placement_id: "api-on-device", config_revision: 4 },
			}),
			createDeploymentPlan(await updateInput()),
		),
	).rejects.toThrow("was not confirmed");
});
test("stored values render back to the text the parser accepts", () => {
	const path = { ...greeting, data_type: "PathBuf" };
	expect(variableText(path, "/data/in")).toBe("/data/in");
	expect(variableValue(path, variableText(path, "/data/in"))).toBe("/data/in");
	const count = { ...greeting, data_type: "Integer" };
	expect(variableValue(count, variableText(count, 3))).toBe(3);
});

test("configuration reads reject forged scope, unbounded payloads and malformed known settings", async () => {
	const result = {
		placement_id: "api-on-device",
		project_id: "project",
		deployment_id: "deployment",
		config_revision: 4,
		config: storedConfig,
	};
	for (const change of [
		{ placement_id: "other" },
		{ project_id: "other" },
		{ deployment_id: "other" },
		{ config_revision: Number.MAX_SAFE_INTEGER },
		{
			config: {
				...storedConfig,
				hosting: { ...storedConfig.hosting, port: 0 },
			},
		},
		{ config: { ...storedConfig, max_replicas: 33 } },
		{
			config: {
				...storedConfig,
				variables: { credential: "unsafe plaintext" },
			},
		},
		{
			config: {
				...storedConfig,
				resource_grant: {
					grant_id: "grant",
					authz_version: 1,
					billing_grant_id: "allowance",
				},
			},
		},
		{ config: { ...storedConfig, future: "x".repeat(16 * 1024) } },
	]) {
		await expect(
			readExistingDeployment(
				readerFor({ ...result, ...change }),
				"api-on-device",
				"project",
			),
		).rejects.toThrow();
	}
});

test("updates retain the replica ceiling, resource attribution, artifact pins and unknown settings", async () => {
	const base = await updateInput();
	base.existing.config.future_setting = { enabled: true, limit: 7 };
	const assets = {
		bit_pins: [{ bit_id: "model", metadata_sha256: "1".repeat(64) }],
		package_pins: [
			{
				package_id: "wasm",
				version: "1.2.3",
				wasm_sha256: "2".repeat(64),
				manifest_sha256: "3".repeat(64),
			},
		],
	};
	const plan = createDeploymentPlan({
		...base,
		installed: { ...installed, assets },
	});
	expect(plan.config.future_setting).toEqual(
		base.existing.config.future_setting,
	);
	expect(plan.config.artifact_pins).toEqual(storedConfig.artifact_pins);
	expect(plan.config.bit_pins).toEqual(assets.bit_pins);
	expect(plan.config.package_pins).toEqual(assets.package_pins);
	for (const change of [
		{ replicas: 1 },
		{ resourceGrant: { grant_id: "another-payer", authz_version: 1 } },
		{ installed: { ...installed, source: "online" as const } },
		{ installed: { ...installed, project_id: "other" } },
	])
		expect(() => createDeploymentPlan({ ...base, ...change })).toThrow();
});

test("updates reject semantic no-ops before stopping a running placement", async () => {
	const existing = await existingPlacement();
	existing.config.variables = { greeting: "hello" };
	const hosting = existing.config.hosting;
	if (!hosting) throw new Error("Expected a hosted placement");
	const unchanged = {
		...(await updateInput()),
		existing,
		installed: {
			...installed,
			revision: existing.config.revision,
			project_path: existing.config.project_path,
		},
		events: [
			{
				...event,
				event_version: [1, 0, 0] as [number, number, number],
				board_version: [1, 0, 0] as [number, number, number],
			},
		],
		host: hosting.host,
		port: hosting.port,
		overrides: { greeting: "hello", credential: "" },
	};
	expect(() => createDeploymentPlan(unchanged)).toThrow(
		"no configuration changes",
	);
	expect(
		createDeploymentPlan({ ...unchanged, overrides: { greeting: "changed" } })
			.expected_revision,
	).toBe(4);
});

test("retained values must fit the selected variable contract without coercion", async () => {
	const base = { ...(await updateInput()), overrides: {} };
	base.existing.config.variables = { greeting: 123 };
	expect(() => createDeploymentPlan(base)).toThrow("incompatible");
	expect(
		createDeploymentPlan({ ...base, overrides: { greeting: "123" } }).config
			.variables,
	).toEqual({ greeting: "123" });
	expect(
		createDeploymentPlan({ ...base, removeOverrides: ["greeting"] }).config
			.variables,
	).toEqual({});
	const integers = { ...greeting, data_type: "Integer", value_type: "Array" };
	for (const value of [[1, "2"], [[1]], { first: 1 }])
		expect(() => validateVariableValue(integers, value)).toThrow();
	expect(() => validateVariableValue(integers, [1, 2])).not.toThrow();
	expect(() =>
		validateVariableValue(
			{ ...integers, value_type: "HashMap" },
			{ first: 1, second: "2" },
		),
	).toThrow();
});

test("changing a public override to secret requires replacement or explicit removal", async () => {
	const base = await updateInput();
	const variables = [secret, { ...greeting, secret: true }];
	const replacements: Record<string, string>[] = [{}, { greeting: "" }];
	for (const overrides of replacements)
		expect(() =>
			createDeploymentPlan({ ...base, variables, overrides }),
		).toThrow("between public and secret");
	const replaced = createDeploymentPlan({
		...base,
		variables,
		overrides: { greeting: "new-secret" },
	});
	expect(replaced.config.variables).toEqual({});
	expect(JSON.stringify(replaced.config)).not.toContain("new-secret");
	expect(JSON.stringify(replaced.config)).not.toContain("hello");
	expect(replaced.steps[1].command.expected_revision).toBe(5);
	const removed = createDeploymentPlan({
		...base,
		variables,
		overrides: {},
		removeOverrides: ["retired", "greeting"],
	});
	expect(removed.config.variables).toEqual({});
	expect(removed.config.secret_overrides).toEqual({
		credential: "variable-stored",
	});
});

test("update retry reads each accepted operation and never repeats an uncertain apply or secret", async () => {
	const plan = createDeploymentPlan({
		...(await updateInput()),
		overrides: { credential: "new-secret" },
	});
	const journal = new Map<string, Awaited<ReturnType<ManagementCall>>>();
	const mutations: string[] = [];
	const reads: string[] = [];
	const call: ManagementCall = async (command, id) => {
		if (command.type === "operation") {
			reads.push(String(command.operation_id));
			const receipt = journal.get(String(command.operation_id));
			if (!receipt) throw new Error("Missing journal receipt");
			return receipt;
		}
		const operationId = requiredOperationId(id);
		mutations.push(operationId);
		journal.set(operationId, {
			operation_id: operationId,
			state: "completed",
			result:
				command.type === "apply"
					? { placement_id: plan.config.id, config_revision: 5 }
					: {
							placement_id: plan.config.id,
							name: command.name,
							secret: "completed",
						},
		});
		throw new Error("response lost");
	};
	await expect(executeDeploymentPlan(call, plan)).rejects.toThrow(
		"response lost",
	);
	await expect(executeDeploymentPlan(call, plan)).rejects.toThrow(
		"response lost",
	);
	await executeDeploymentPlan(call, plan);
	expect(mutations).toEqual(plan.steps.map((step) => step.id));
	expect(reads).toEqual([plan.steps[0].id, plan.steps[0].id, plan.steps[1].id]);
});

test("secret rejection compares against the newly applied revision and uncertain failures are not stale", async () => {
	for (const current of [5, 6]) {
		const plan = createDeploymentPlan({
			...(await updateInput()),
			overrides: { credential: "new-secret" },
		});
		const call: ManagementCall = async (command, id) => {
			if (command.type === "placement_configuration")
				return {
					operation_id: "read",
					state: "completed",
					result: {
						placement_id: "api-on-device",
						project_id: "project",
						deployment_id: "deployment",
						config_revision: current,
						config: plan.config,
					},
				};
			return {
				operation_id: requiredOperationId(id),
				state: command.type === "apply" ? "accepted" : "rejected",
				result:
					command.type === "apply"
						? { placement_id: plan.config.id, config_revision: 5 }
						: { error: "rejected" },
			};
		};
		try {
			await executeDeploymentPlan(call, plan);
			throw new Error("Expected a rejected secret publication");
		} catch (error) {
			expect(error instanceof StaleDeploymentRevisionError).toBe(current === 6);
			if (current === 5) expect(String(error)).toContain("rejected secret");
		}
	}
	const plan = createDeploymentPlan(await updateInput());
	const uncertain = new Error("connection lost");
	await expect(
		executeDeploymentPlan(async () => {
			throw uncertain;
		}, plan),
	).rejects.toBe(uncertain);
});

test("aborting a retry after journal lookup prevents another mutation", async () => {
	const plan = createDeploymentPlan(await updateInput());
	plan.steps[0].attempted = true;
	const abort = new AbortController();
	const commands: unknown[] = [];
	await expect(
		executeDeploymentPlan(
			async (command) => {
				commands.push(command.type);
				abort.abort();
				return { operation_id: "read", state: "rejected", result: {} };
			},
			plan,
			abort.signal,
		),
	).rejects.toThrow();
	expect(commands).toEqual(["operation"]);
});

test("secret byte limits are checked before a placement can be applied", async () => {
	const base = await updateInput();
	for (const credential of [
		"x".repeat(4095),
		"é".repeat(2048),
		'"'.repeat(2048),
	])
		expect(() =>
			createDeploymentPlan({ ...base, overrides: { credential } }),
		).toThrow("4096 UTF-8 bytes");
	expect(
		createDeploymentPlan({
			...base,
			overrides: { credential: "x".repeat(4094) },
		}).steps,
	).toHaveLength(2);
});

test("failed secret receipts require reload and detect publication-time revision conflicts", async () => {
	for (const current of [5, 6]) {
		const plan = createDeploymentPlan({
			...(await updateInput()),
			overrides: { credential: "new-secret" },
		});
		const call: ManagementCall = async (command, id) => {
			if (command.type === "placement_configuration")
				return {
					operation_id: "read",
					state: "completed",
					result: {
						placement_id: "api-on-device",
						project_id: "project",
						deployment_id: "deployment",
						config_revision: current,
						config: plan.config,
					},
				};
			if (command.type === "operation")
				return {
					operation_id: String(command.operation_id),
					state: "failed",
					result: {
						placement_id: plan.config.id,
						name: plan.steps[1].command.name,
						secret: "failed",
					},
				};
			return {
				operation_id: requiredOperationId(id),
				state: "accepted",
				result:
					command.type === "apply"
						? { placement_id: plan.config.id, config_revision: 5 }
						: {
								placement_id: plan.config.id,
								name: command.name,
								secret: "pending",
							},
			};
		};
		await expect(executeDeploymentPlan(call, plan)).rejects.toBeInstanceOf(
			current === 5
				? DeploymentPublicationFailedError
				: StaleDeploymentRevisionError,
		);
	}
});

function rejected(
	id: string | undefined,
	code: string,
	error = `${code} cause`,
	retryable = ["busy", "failed"].includes(code),
) {
	return {
		operation_id: id ?? "read",
		state: "rejected",
		result: { error, code, retryable },
	};
}
const currentPlacement = {
	operation_id: "read",
	state: "completed",
	result: {
		placement_id: "api-on-device",
		project_id: "project",
		deployment_id: "deployment",
		config_revision: 4,
		config: storedConfig,
		desired_state: "running",
	},
};

test("a definitive apply rejection ends retries with its reason while busy and older agents stay retryable", async () => {
	const plan = createDeploymentPlan(await updateInput());
	const types: unknown[] = [];
	const refused = await executeDeploymentPlan(async (command, id) => {
		types.push(command.type);
		return command.type === "placement_configuration"
			? currentPlacement
			: rejected(id, "host_policy", "This host requires Linux isolation");
	}, plan).catch((error: unknown) => error);
	expect(refused).toBeInstanceOf(DeploymentRejectedError);
	expect(String(refused)).toContain("This host requires Linux isolation");
	expect(String(refused)).toContain("isolation settings");
	expect(types).toEqual(["apply", "placement_configuration"]);
	for (const response of [
		(id?: string) => rejected(id, "busy"),
		(id?: string) => ({
			operation_id: id ?? "read",
			state: "rejected",
			result: { error: "Command rejected" },
		}),
	]) {
		const retried = await executeDeploymentPlan(
			async (command, id) =>
				command.type === "placement_configuration"
					? currentPlacement
					: response(id),
			createDeploymentPlan(await updateInput()),
		).catch((error: unknown) => error);
		expect(retried).not.toBeInstanceOf(DeploymentRejectedError);
		expect(String(retried)).toContain("was not confirmed");
	}
	const failed = await executeDeploymentPlan(
		async (command, id) =>
			command.type === "placement_configuration"
				? currentPlacement
				: rejected(id, "failed", "Placement store is locked"),
		createDeploymentPlan(await updateInput()),
	).catch((error: unknown) => error);
	expect(failed).not.toBeInstanceOf(DeploymentRejectedError);
	expect(String(failed)).toContain(
		"Device response: Placement store is locked",
	);
	expect(String(failed)).toContain("Retry to confirm the same operations");
	const created = await executeDeploymentPlan(
		async (_command, id) =>
			rejected(id, "revision_conflict", "Placement exists"),
		createDeploymentPlan(input()),
	).catch((error: unknown) => error);
	expect(created).toBeInstanceOf(DeploymentRejectedError);
	expect(String(created)).toContain("creation of placement api-on-device");
	expect(String(created)).toContain("choose another placement ID");
});

test("a definitive secret rejection after Apply requires reloading the applied placement", async () => {
	const plan = createDeploymentPlan({
		...(await updateInput()),
		overrides: { credential: "new-secret" },
	});
	const error = await executeDeploymentPlan(async (command, id) => {
		if (command.type === "placement_configuration")
			return {
				...currentPlacement,
				result: { ...currentPlacement.result, config_revision: 5 },
			};
		if (command.type === "apply")
			return {
				operation_id: requiredOperationId(id),
				state: "accepted",
				result: { placement_id: plan.config.id, config_revision: 5 },
			};
		return rejected(id, "limit", "The secret store is full");
	}, plan).catch((value: unknown) => value);
	expect(error).toBeInstanceOf(DeploymentPublicationFailedError);
	expect(String(error)).toContain("The secret store is full");
});

test("definitive rollout rejections stop retries and leave a staged update discardable", async () => {
	const staged = createDeploymentPlan({
		...(await updateInput()),
		healthChecked: true,
	});
	await expect(
		executeDeploymentPlan(
			async (command, id) =>
				command.type === "placement_configuration"
					? currentPlacement
					: rejected(id, "invalid", "Offline writes need one replica"),
			staged,
		),
	).rejects.toBeInstanceOf(DeploymentRejectedError);
	const plan = createDeploymentPlan({
		...(await updateInput()),
		healthChecked: true,
		overrides: { credential: "replacement" },
	});
	const statuses: DeploymentRolloutStatus[] = [];
	const scope = {
		rollout_id: plan.rollout_id,
		placement_id: plan.config.id,
		project_id: plan.config.project_id,
	};
	const error = await executeDeploymentPlan(
		async (command, id) => {
			if (command.type === "stage_rollout")
				return {
					operation_id: requiredOperationId(id),
					state: "accepted",
					result: { ...scope, state: "staged" },
				};
			if (command.type === "rollout")
				return {
					operation_id: "read",
					state: "completed",
					result: { ...scope, state: "staged" },
				};
			return rejected(id, "limit", "Rollout secret budget exhausted");
		},
		plan,
		undefined,
		(status) => statuses.push(status),
	).catch((value: unknown) => value);
	expect(error).toBeInstanceOf(DeploymentRejectedError);
	expect(String(error)).toContain("Rollout secret budget exhausted");
	expect(String(error)).toContain("Discard the staged update");
	expect(statuses.map((status) => status.state)).toEqual(["staged"]);
});

test("removing buffered resources requires an empty offline queue", async () => {
	const buffering = offlineWritesSchema.parse({
		tables: [
			{
				purpose: "storage",
				database: "db",
				table: "orders",
				primary_key: "id",
			},
		],
		files: [{ purpose: "files", prefix: "exports" }],
	});
	expect(removesOfflineBuffering(undefined, null)).toBe(false);
	expect(removesOfflineBuffering(buffering, buffering)).toBe(false);
	expect(
		removesOfflineBuffering(buffering, { ...buffering, max_operations: 5 }),
	).toBe(false);
	expect(removesOfflineBuffering(buffering, null)).toBe(true);
	expect(removesOfflineBuffering(buffering, { ...buffering, files: [] })).toBe(
		true,
	);
	expect(
		removesOfflineBuffering(buffering, {
			...buffering,
			tables: [{ ...buffering.tables[0], table: "invoices" }],
		}),
	).toBe(true);
	const queues =
		(pending: number[]): ManagementCall =>
		async (command) => ({
			operation_id: "queue",
			state: "completed",
			result: {
				placement_id: command.placement_id,
				queues: pending.map((count, index) => ({
					scope: String(index).repeat(64),
					quarantined: false,
					pending_count: count,
					pending_bytes: count * 10,
					oldest_at: count ? 1 : null,
					head: null,
				})),
				next: null,
			},
		});
	await assertOfflineQueuesDrained(queues([0, 0]), "api-on-device");
	await expect(
		assertOfflineQueuesDrained(queues([0, 40]), "api-on-device"),
	).rejects.toThrow("still has 40 buffered writes waiting to replay");
	await expect(
		assertOfflineQueuesDrained(
			async () => rejected(undefined, "unauthorized", "Deploy access only"),
			"api-on-device",
		),
	).rejects.toThrow("requires reading its offline queue first");
});

test("oversized configurations name the pins or values that exceed one management message", () => {
	const bits = Array.from({ length: 120 }, (_, index) => ({
		bit_id: `model-${index}`,
		metadata_sha256: "e".repeat(64),
	}));
	expect(() =>
		createDeploymentPlan({
			...input(),
			installed: { ...installed, assets: { bit_pins: bits, package_pins: [] } },
		}),
	).toThrow(
		`one remote management message carries at most ${DEPLOYMENT_CONFIG_BYTES}`,
	);
	expect(() =>
		createDeploymentPlan({
			...input(),
			installed: { ...installed, assets: { bit_pins: bits, package_pins: [] } },
		}),
	).toThrow("Remove Bits or node packages");
	expect(() =>
		createDeploymentPlan({
			...input(),
			variables: [secret, greeting],
			overrides: { ...input().overrides, greeting: "x".repeat(12_000) },
		}),
	).toThrow("Shorten the public variable overrides");
});

test("rollout reads keep the timeline fields and pass unknown agent fields through", async () => {
	const scope = {
		rollout_id: crypto.randomUUID(),
		placement_id: "placement",
		project_id: "project",
	};
	const timeline = {
		base_revision: 11,
		active_revision: 12,
		active_intent: 4,
		previous_replicas: 1,
		candidate_replicas: 1,
		stabilization_seconds: 10,
		deadline_seconds: 120,
		created_at: 1790769560,
		updated_at: 1790769600,
		deadline_at: 1790769690,
		stable_since: 1790769594,
	};
	const status = await readDeploymentRollout(
		async () => ({
			operation_id: "read",
			state: "completed",
			result: {
				...scope,
				state: "activating",
				...timeline,
				cohort: "boot-1",
			},
		}),
		scope,
	);
	expect(status).toMatchObject({ ...scope, state: "activating", ...timeline });
	expect((status as Record<string, unknown>).cohort).toBe("boot-1");
	const older = await readDeploymentRollout(
		async () => ({
			operation_id: "read",
			state: "completed",
			result: { ...scope, state: "healthy" },
		}),
		scope,
	);
	expect(older.deadline_at).toBeUndefined();
	expect(older.stable_since).toBeUndefined();
	await expect(
		readDeploymentRollout(
			async () => ({
				operation_id: "read",
				state: "completed",
				result: { ...scope, state: "activating", deadline_at: "soon" },
			}),
			scope,
		),
	).rejects.toThrow();
});

const shadowVariant = {
	name: "shadow",
	board_id: "shadow",
	node_id: "entry",
	mode: { Shadow: { sample_rate: 0.5 } },
	created_at: time,
	updated_at: time,
	variables: {},
};
const canaryTarget = {
	board_id: "canary",
	node_id: "entry",
	weight: 1,
	created_at: time,
	updated_at: time,
	variables: {},
};
const configBytes = (config: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(config)));
const nightly = configBytes({
	expression: "0 0 2 * * *",
	timezone: "Europe/Berlin",
});
const cron = (config: Record<string, unknown>): Partial<IEvent> => ({
	event_type: "cron",
	config: configBytes(config),
});
/* The configs the event editor saves (`packages/ui/lib/event-definitions.ts`). */
const API_DEFAULT = {
	sink_type: "http",
	method: "GET",
	path: "/cm1abc",
	public_endpoint: false,
};
const TELEGRAM_DEFAULT = {
	sink_type: "telegram",
	bot_token: "",
	bot_name: "Flow-Like Bot",
	bot_description: "",
	chat_whitelist: [],
	chat_blacklist: [],
	respond_to_mentions: true,
	respond_to_private: true,
	command_prefix: "/",
};
const DISCORD_DEFAULT = {
	sink_type: "discord",
	token: "",
	bot_name: "Flow-Like Bot",
	bot_description: "",
	intents: ["Guilds", "GuildMessages", "MessageContent"],
	channel_whitelist: [],
	channel_blacklist: [],
	respond_to_mentions: true,
	respond_to_dms: true,
	command_prefix: "!",
};
const ELIGIBILITY_CASES: [
	Partial<IEvent>,
	EventIneligibleCode | null,
	boolean,
	EventReadiness,
	EventKind | null,
][] = [
	[{}, null, true, "listener", "served"],
	[{ event_type: "simple_chat" }, null, true, "listener", "served"],
	[
		{ event_type: "generic_form", default_page_id: "page" },
		null,
		true,
		"listener",
		"served",
	],
	[{ event_type: "rest" }, null, false, "listener", "own_server"],
	[{ event_type: "mcp" }, null, false, "listener", "own_server"],
	[{ event_type: "daemon" }, null, false, "explicit", "background"],
	[
		{ event_type: "cron", config: nightly },
		null,
		false,
		"explicit",
		"scheduled",
	],
	[cron({ cron: "0 9 * * 1-5" }), null, false, "explicit", "scheduled"],
	[
		{ event_type: "cron", config: nightly, default_page_id: "page" },
		null,
		true,
		"listener",
		"served",
	],
	[
		{ event_type: "cron", default_page_id: "page" },
		null,
		true,
		"listener",
		"served",
	],
	[{ event_type: "cron" }, "schedule_missing", false, "explicit", "scheduled"],
	[
		cron({ scheduled_for: { date: "2026-12-24", time: "18:00" } }),
		null,
		false,
		"explicit",
		"scheduled",
	],
	[
		cron({ scheduled_for: { date: "2020-01-01", time: "00:00" } }),
		null,
		false,
		"explicit",
		"scheduled",
	],
	[
		cron({
			scheduled_for: { date: "2027-03-28", time: "02:30" },
			timezone: "Europe/Berlin",
		}),
		"schedule_invalid",
		false,
		"explicit",
		"scheduled",
	],
	[
		cron({ expression: "*/30 * * * * *" }),
		"schedule_too_often",
		false,
		"explicit",
		"scheduled",
	],
	[
		cron({ expression: "0 0 9 ? * 1-5" }),
		"schedule_invalid",
		false,
		"explicit",
		"scheduled",
	],
	[
		cron({ expression: "0 0 2 * * *", timezone: "Mars/Olympus" }),
		"schedule_invalid",
		false,
		"explicit",
		"scheduled",
	],
	[{ event_type: "api" }, null, true, "listener", "served"],
	[
		{ event_type: "api", config: configBytes(API_DEFAULT) },
		null,
		true,
		"listener",
		"served",
	],
	[
		{ event_type: "http", config: [] },
		"route_missing",
		true,
		"listener",
		"served",
	],
	[
		{ event_type: "api", config: configBytes({ path: 7 }) },
		"route_missing",
		true,
		"listener",
		"served",
	],
	[
		{ event_type: "http", config: configBytes({ path: "/a?x=1" }) },
		"route_invalid",
		true,
		"listener",
		"served",
	],
	[
		{ event_type: "api", config: configBytes({ path: "/ui/x" }) },
		"route_reserved",
		true,
		"listener",
		"served",
	],
	[
		{
			event_type: "api",
			config: configBytes({ path: "/ui/x" }),
			default_page_id: "page",
		},
		null,
		true,
		"listener",
		"served",
	],
	[{ event_type: "quick_action" }, null, false, "explicit", "on_demand"],
	[{ event_type: "generic_form" }, null, false, "explicit", "on_demand"],
	[
		{ event_type: "telegram", config: configBytes(TELEGRAM_DEFAULT) },
		null,
		false,
		"explicit",
		"bot",
	],
	[
		{ event_type: "discord", config: configBytes(DISCORD_DEFAULT) },
		null,
		false,
		"explicit",
		"bot",
	],
	[
		{
			event_type: "discord",
			config: configBytes({ ...DISCORD_DEFAULT, intents: ["Guilds", "Nope"] }),
		},
		"bot_invalid",
		false,
		"explicit",
		"bot",
	],
	[
		{ event_type: "telegram", config: [] },
		"bot_invalid",
		false,
		"explicit",
		"bot",
	],
	[{ event_type: "email" }, "type", false, "unsupported", null],
	[{ event_type: "teams" }, "type", false, "unsupported", null],
	[{ event_type: "deeplink" }, "type", false, "unsupported", null],
	[
		{ active: false, event_type: "telegram", config: [] },
		"paused",
		false,
		"explicit",
		"bot",
	],
	[
		{ active: false, event_type: "cron", config: nightly },
		"paused",
		false,
		"explicit",
		"scheduled",
	],
	[{ board_version: null }, null, true, "listener", "served"],
	[
		{ board_version: null, event_type: "cron", config: nightly },
		null,
		false,
		"explicit",
		"scheduled",
	],
	[
		{ event_version: [1, 2, 4294967295] },
		"latest_flow",
		true,
		"listener",
		"served",
	],
	[
		{ board_version: [4294967295, 0, 0] },
		"latest_flow",
		true,
		"listener",
		"served",
	],
	[
		{ canary: canaryTarget, event_type: "cron", config: nightly },
		"canary",
		false,
		"explicit",
		"scheduled",
	],
	[
		{ variants: [shadowVariant], event_type: "cron" },
		"variants",
		false,
		"explicit",
		"scheduled",
	],
	[{ variants: [shadowVariant] }, "variants", true, "listener", "served"],
];

test("event eligibility names the first failing device rule and agrees with online discovery", () => {
	for (const [change, code, hosted, readiness, kind] of ELIGIBILITY_CASES) {
		const record = approvedEvent(change);
		const rule = eventEligibility(record);
		expect([rule.code, rule.hosted, rule.readiness, rule.kind]).toEqual([
			code,
			hosted,
			readiness,
			kind,
		]);
		expect(rule.eligible).toBe(code === null);
		expect(rule.followsLatest).toBe(record.board_version == null);
		if (!rule.eventVersion || !rule.boardVersion) continue;
		const [discovered] = approvedOnlineCatalog(
			approvedDocuments([record]),
		).events;
		expect([
			discovered.eligible,
			discovered.hosted,
			discovered.readiness_kind,
			discovered.rollout_supported,
			discovered.kind ?? null,
		]).toEqual([
			rule.eligible,
			rule.hosted,
			rule.readiness,
			rule.rolloutSupported,
			rule.kind,
		]);
		expect(discovered.schedule ?? null).toEqual(
			rule.schedule
				? {
						expression: rule.schedule.expression,
						timezone: rule.schedule.timezone,
					}
				: null,
		);
		expect(discovered.once ?? null).toEqual(
			rule.once
				? {
						date: rule.once.date,
						time: rule.once.time,
						at: rule.once.at,
						timezone: rule.once.timezone,
					}
				: null,
		);
		expect<EventRoute | null>(discovered.route ?? null).toEqual(
			rule.route ?? null,
		);
		expect<string | null>(discovered.ineligible_code ?? null).toBe(
			(DEVICE_INELIGIBLE_CODES as readonly string[]).includes(rule.code ?? "")
				? rule.code
				: null,
		);
	}
});

test("a schedule a device can run carries its expression and effective zone; safe updates need a valid one", () => {
	const valid = eventEligibility(
		approvedEvent({ event_type: "cron", config: nightly }),
	);
	expect(valid.schedule).toEqual({
		expression: "0 0 2 * * *",
		timezone: "Europe/Berlin",
		zoneSet: true,
	});
	expect(valid.rolloutSupported).toBe(true);
	expect(
		eventEligibility(approvedEvent(cron({ expression: "0 9 * * 1-5" })))
			.schedule,
	).toEqual({ expression: "0 9 * * 1-5", timezone: "UTC", zoneSet: false });
	const broken = eventEligibility(
		approvedEvent(cron({ expression: "0 0 9 ? * 1-5" })),
	);
	expect(broken.schedule).toBeUndefined();
	expect(broken.scheduleDetail).toEqual({ code: "syntax", field: "?" });
	expect(broken.rolloutSupported).toBe(false);
	// A served `cron` Page is not a schedule: its config is never read.
	const page = eventEligibility(
		approvedEvent({
			event_type: "cron",
			default_page_id: "page",
			config: configBytes({ expression: "not a schedule" }),
		}),
	);
	expect([page.eligible, page.kind, page.schedule]).toEqual([
		true,
		"served",
		undefined,
	]);
});

test("the schedule half of a config wins over the record's bytes, and null means no schedule", () => {
	const record = approvedEvent({ event_type: "cron", config: nightly });
	expect(
		eventEligibility({ ...record, schedule: { expression: "0 0 6 * * *" } })
			.schedule?.expression,
	).toBe("0 0 6 * * *");
	expect(eventEligibility({ ...record, schedule: null }).code).toBe(
		"schedule_missing",
	);
	expect(
		eventEligibility(
			approvedEvent({ event_type: "cron", config: configBytes(["no"]) }),
		).code,
	).toBe("schedule_missing");
});

test("an event that follows Latest is eligible; only its narrow cases are refused", () => {
	const latest = approvedEvent({ board_version: null });
	const rule = eventEligibility(latest);
	expect([rule.eligible, rule.followsLatest, rule.boardVersion]).toEqual([
		true,
		true,
		null,
	]);
	expect(rule.latestFlow).toBeUndefined();
	expect(eventEligibility({ ...latest, board_version: undefined })).toEqual(
		rule,
	);
	for (const reason of ["hub", "role", "target", "copy"] as const) {
		const refused = eventEligibility(latest, { latestFlow: reason });
		expect([refused.code, refused.latestFlow]).toEqual(["latest_flow", reason]);
	}
	// The cases describe Latest events only; a pinned event is never refused by them.
	expect(eventEligibility(approvedEvent(), { latestFlow: "hub" }).code).toBe(
		null,
	);
	for (const change of [
		{ event_version: [1, 2, 4294967295] },
		{ board_version: [3, 2, 4294967295] },
		{ board_version: [3, 2] },
		{ event_version: undefined },
	] as Partial<IEvent>[]) {
		const unreadable = eventEligibility(approvedEvent(change), {
			latestFlow: "hub",
		});
		expect([
			unreadable.code,
			unreadable.latestFlow,
			unreadable.followsLatest,
		]).toEqual(["latest_flow", "other", false]);
	}
	expect(
		eventEligibility(approvedEvent({ active: false, board_version: null }), {
			latestFlow: "hub",
		}).code,
	).toBe("paused");
});

test("an older hub refuses every schedule of an online app, after the schedule's own rule", () => {
	const schedule = approvedEvent({ event_type: "cron", config: nightly });
	expect(eventEligibility(schedule, { hubSchedules: false }).code).toBe(
		"hub_schedules",
	);
	expect(eventEligibility(schedule, { hubSchedules: true }).code).toBeNull();
	expect(eventEligibility(schedule, {}).code).toBeNull();
	expect(
		eventEligibility(approvedEvent({ event_type: "cron" }), {
			hubSchedules: false,
		}).code,
	).toBe("schedule_missing");
	expect(
		eventEligibility(approvedEvent(), { hubSchedules: false }).code,
	).toBeNull();
	expect(
		eventEligibility(
			approvedEvent({
				event_type: "cron",
				config: nightly,
				default_page_id: "page",
			}),
			{ hubSchedules: false },
		).code,
	).toBeNull();
	expect(
		eventEligibility(schedule, {
			hubSchedules: false,
			ineligibleReason: "Board board 3.2.1 cannot be deployed",
		}).code,
	).toBe("hub_schedules");
});

test("bundle and device refusals come after the event's own rules", () => {
	const flow = eventEligibility(approvedEvent(), {
		ineligibleReason: "Board board 3.2.1 cannot be deployed",
		deviceRefusal: "edge requires sandboxed services",
	});
	expect([flow.code, flow.detail]).toEqual([
		"flow_error",
		"Board board 3.2.1 cannot be deployed",
	]);
	const refused = eventEligibility(approvedEvent({ event_type: "daemon" }), {
		deviceRefusal: "edge requires sandboxed services",
	});
	expect([refused.eligible, refused.code, refused.detail]).toEqual([
		false,
		"refuse",
		"edge requires sandboxed services",
	]);
	expect(
		eventEligibility(approvedEvent({ active: false }), {
			deviceRefusal: "edge requires sandboxed services",
		}).code,
	).toBe("paused");
});
