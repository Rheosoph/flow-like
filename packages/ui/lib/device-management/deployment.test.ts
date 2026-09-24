import { expect, test } from "bun:test";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IEventState } from "../../state/backend-state/event-state";
import type { IEvent } from "../schema/flow/event";
import {
	type DeploymentEvent,
	DeploymentPublicationFailedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentVariable,
	type InstalledProject,
	type PlacementConfiguration,
	StaleDeploymentRevisionError,
	cancelDeploymentRollout,
	createDeploymentPlan,
	discoverOfflineEvents,
	discoverOnlineEvents,
	discoverOnlineVariables,
	executeDeploymentPlan,
	mergeVariables,
	offlineWritesSchema,
	placementResourcesSchema,
	readExistingDeployment,
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

function onlineFixture() {
	const time = { secs_since_epoch: 100, nanos_since_epoch: 0 };
	const fixture = {
		current: {
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
		} as IEvent,
		eventReads: [] as unknown[][],
		boardReads: [] as unknown[][],
	};
	const events = {
		getEventsAuthoritative: async () => [fixture.current],
		getEventAuthoritative: async (
			...args: [string, string, [number, number, number]?]
		) => {
			fixture.eventReads.push(args);
			if (args[2] !== undefined)
				throw new Error("The current event has no archive yet");
			return fixture.current;
		},
	} as unknown as IEventState;
	const boards = {
		getBoardAuthoritative: async (
			...args: [string, string, [number, number, number]?]
		) => {
			fixture.boardReads.push(args);
			return {
				id: "board",
				version: [3, 2, 1],
				layers: {},
				variables: {
					credential: {
						...secret,
						exposed: true,
						editable: true,
						default_value: [65, 66],
					},
				},
			} as unknown as Awaited<ReturnType<IBoardState["getBoardAuthoritative"]>>;
		},
	} as IBoardState;
	return { fixture, events, boards, time };
}

test("online discovery reads an unarchived current event and still pins its board exactly", async () => {
	const { fixture, events, boards } = onlineFixture();
	const [selected] = await discoverOnlineEvents(events, "project");
	expect(selected).toEqual({
		...event,
		readiness_kind: "listener",
		rollout_supported: true,
	});
	expect(
		await discoverOnlineVariables(events, boards, "project", selected),
	).toEqual([secret]);
	expect(fixture.eventReads).toEqual([["project", event.id]]);
	expect(fixture.boardReads).toEqual([["project", "board", [3, 2, 1]]]);
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

test("online discovery rejects changed event pins, type and eligibility before reading a board", async () => {
	const { fixture, events, boards, time } = onlineFixture();
	const initial = fixture.current;
	const [selected] = await discoverOnlineEvents(events, "project");
	const changes: Partial<IEvent>[] = [
		{ id: "another-event" },
		{ event_version: [1, 2, 4] },
		{ board_version: [3, 2, 2] },
		{ board_version: null },
		{ event_version: [4294967295, 0, 0] },
		{ active: false },
		{ event_type: "mcp" },
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
		fixture.current = { ...initial, ...change };
		await expect(
			discoverOnlineVariables(events, boards, "project", selected),
		).rejects.toThrow("Reload the project");
	}
	expect(fixture.boardReads).toHaveLength(0);
	fixture.current = { ...initial, event_type: "daemon" };
	const [daemon] = await discoverOnlineEvents(events, "project");
	fixture.current = { ...fixture.current, default_page_id: "new-page" };
	await expect(
		discoverOnlineVariables(events, boards, "project", daemon),
	).rejects.toThrow("Reload the project");
	expect(fixture.boardReads).toHaveLength(0);
});

test("online discovery propagates authority failure and refuses a different board snapshot", async () => {
	const { fixture, events, boards } = onlineFixture();
	const [selected] = await discoverOnlineEvents(events, "project");
	const failure = new Error("Event read denied");
	const refused = {
		...events,
		getEventAuthoritative: async () => {
			throw failure;
		},
	};
	await expect(
		discoverOnlineVariables(refused, boards, "project", selected),
	).rejects.toBe(failure);
	expect(fixture.boardReads).toHaveLength(0);
	for (const change of [{ id: "another-board" }, { version: [3, 2, 2] }]) {
		const replaced = {
			...boards,
			getBoardAuthoritative: async (
				...args: Parameters<IBoardState["getBoardAuthoritative"]>
			) => ({ ...(await boards.getBoardAuthoritative(...args)), ...change }),
		};
		await expect(
			discoverOnlineVariables(events, replaced, "project", selected),
		).rejects.toThrow("board pins differ");
	}
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
		installed: { ...installed, source: "online" as const },
		resourceGrant: { grant_id: "grant", authz_version: 1 },
	};
	expect(
		createDeploymentPlan({ ...online, offlineWrites: buffering }).config
			.offline_writes,
	).toEqual(buffering);
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
		installed: { ...base.installed, source: "online" as const },
		existing: {
			...base.existing,
			config: {
				...base.existing.config,
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
