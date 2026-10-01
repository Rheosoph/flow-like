import { expect, test } from "bun:test";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IEventState } from "../../state/backend-state/event-state";
import type { IEvent } from "../schema/flow/event";
import {
	DEPLOYMENT_CONFIG_BYTES,
	type DeploymentEvent,
	DeploymentPublicationFailedError,
	DeploymentRejectedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentRolloutStatus,
	type DeploymentVariable,
	type EventIneligibleCode,
	type EventReadiness,
	type InstalledProject,
	type PlacementConfiguration,
	StaleDeploymentRevisionError,
	approvedOnlineCatalog,
	assertOfflineQueuesDrained,
	cancelDeploymentRollout,
	createDeploymentPlan,
	discoverOfflineEvents,
	discoverOnlineEvents,
	discoverOnlineVariables,
	discoverPreviousOfflineVariables,
	discoverPreviousOnlineVariables,
	eventEligibility,
	executeDeploymentPlan,
	mergeVariables,
	offlineWritesSchema,
	placementResourcesSchema,
	readDeploymentRollout,
	readExistingDeployment,
	removesOfflineBuffering,
	validateVariableValue,
	variableText,
	variableValue,
	waitForDeploymentRollout,
} from "./deployment";
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
function approvedEvent(change: Partial<IEvent> = {}): IEvent {
	return {
		id: event.id,
		name: event.name,
		event_type: event.event_type,
		event_version: [1, 2, 3],
		board_id: "board",
		board_version: [3, 2, 1],
		active: true,
		config: [],
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
const ELIGIBILITY_CASES: [
	Partial<IEvent>,
	EventIneligibleCode | null,
	boolean,
	EventReadiness,
][] = [
	[{}, null, true, "listener"],
	[{ event_type: "simple_chat" }, null, true, "listener"],
	[
		{ event_type: "generic_form", default_page_id: "page" },
		null,
		true,
		"listener",
	],
	[{ event_type: "rest" }, null, false, "listener"],
	[{ event_type: "mcp" }, null, false, "listener"],
	[{ event_type: "daemon" }, null, false, "explicit"],
	[{ event_type: "cron" }, "type", false, "unsupported"],
	[{ event_type: "api" }, "api_type", false, "unsupported"],
	[{ active: false, event_type: "cron" }, "paused", false, "unsupported"],
	[{ board_version: null }, "latest_flow", true, "listener"],
	[{ event_version: [1, 2, 4294967295] }, "latest_flow", true, "listener"],
	[
		{ canary: canaryTarget, event_type: "cron" },
		"canary",
		false,
		"unsupported",
	],
	[{ variants: [shadowVariant] }, "variants", true, "listener"],
];

test("event eligibility names the first failing device rule and agrees with online discovery", () => {
	for (const [change, code, hosted, readiness] of ELIGIBILITY_CASES) {
		const record = approvedEvent(change);
		const rule = eventEligibility(record);
		expect([rule.code, rule.hosted, rule.readiness]).toEqual([
			code,
			hosted,
			readiness,
		]);
		expect(rule.eligible).toBe(code === null);
		if (!rule.eventVersion || !rule.boardVersion) continue;
		const [discovered] = approvedOnlineCatalog(
			approvedDocuments([record]),
		).events;
		expect([
			discovered.eligible,
			discovered.hosted,
			discovered.readiness_kind,
			discovered.rollout_supported,
		]).toEqual([
			rule.eligible,
			rule.hosted,
			rule.readiness,
			rule.rolloutSupported,
		]);
	}
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
