import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { StrictMode, act } from "react";
import type { InstalledProject } from "../../../lib/device-management/deployment";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import type {
	ManagementResponse,
	PlacementStatus,
} from "../../../lib/device-management/types";
import type { IProfile } from "../../../types";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	HTMLInputElement: window.HTMLInputElement,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
let httpCalls = 0;
let onlineReads: string[] = [];
mock.module("../../../state/backend-state", () => ({
	useBackendReady: () => true,
	useBackend: () => ({
		apiState: {
			get: async () => {
				httpCalls++;
				throw new Error("Unexpected cloud request during offline deployment");
			},
		},
		eventState: {
			getEventsAuthoritative: async (projectId: string) => {
				expect(projectId).toBe("project");
				onlineReads.push("events");
				return [{ ...event, board_id: "published-board", active: true }];
			},
			getEventAuthoritative: async (projectId: string, eventId: string) => {
				expect([projectId, eventId]).toEqual(["project", event.id]);
				onlineReads.push("event");
				return { ...event, board_id: "published-board", active: true };
			},
		},
		boardState: {
			getBoardAuthoritative: async (
				projectId: string,
				boardId: string,
				version: number[],
			) => {
				expect([projectId, boardId, version]).toEqual([
					"project",
					"published-board",
					event.board_version,
				]);
				onlineReads.push("board");
				return {
					id: boardId,
					version,
					layers: {},
					variables: Object.fromEntries(
						discoveredVariables.map((variable) => [
							variable.id,
							{ ...variable, exposed: true },
						]),
					),
				};
			},
		},
	}),
}));
const { createRoot } = await import("react-dom/client");
const { DeviceDeploymentForm } = await import("./device-deployment-form");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const installed = {
	project_id: "project",
	project_path: "/device/projects/project/revisions/pinned",
	revision: "a".repeat(64),
	source: "offline" as const,
};
let project: InstalledProject = installed;
const event = {
	id: "http-event",
	name: "Published API",
	event_type: "http",
	event_version: [1, 0, 0],
	board_version: [2, 0, 0],
	hosted: true,
	eligible: true,
};
const variables = [
	{
		id: "credential",
		name: "API credential",
		data_type: "String",
		value_type: "Normal",
		secret: true,
	},
];
const serviceToken = "a-private-service-token-from-password-manager";
const variableSecret = "private-variable-value";
let calls: { command: Record<string, unknown>; id?: string }[] = [];
let journals = new Map<string, Record<string, unknown>>();
let applied = 0;
let callOverride: ManagementCall | undefined;
let discoveredVariables = variables;
let placementRows: PlacementStatus[] = [];
const initialConfiguration = {
	placement_id: "existing-service",
	project_id: "project",
	deployment_id: "existing-deployment",
	config_revision: 7,
	desired_state: "running",
	config: {
		id: "existing-service",
		project_id: "project",
		deployment_id: "existing-deployment",
		revision: "b".repeat(64),
		source: "offline",
		project_path: "/device/projects/project/revisions/previous",
		events: [
			{
				event_id: event.id,
				event_version: [0, 1, 0],
				board_version: [1, 0, 0],
			},
		],
		hosting: {
			host: "0.0.0.0",
			port: 9345,
			max_in_flight: 17,
			request_timeout_secs: 73,
			auth_secret: "existing-access",
		},
		variables: { greeting: "Welcome from this device" },
		secret_overrides: { credential: "stored-credential" },
		max_replicas: 3,
		restart: "on_failure",
		resource_grant: {
			grant_id: "original-grant",
			authz_version: 9,
			billing_grant_id: "original-billing",
			billing_authz_version: 4,
		},
		bit_pins: [{ id: "local-model", sha256: "c".repeat(64) }],
		package_pins: [
			{ id: "wasm-package", version: "1.2.3", sha256: "d".repeat(64) },
		],
		future_setting: { retain_me: true },
	},
};
let configuration = structuredClone(initialConfiguration);
const call: ManagementCall = async (command, id) => {
	calls.push({ command, id });
	if (callOverride) return callOverride(command, id);
	return respond(command, id);
};
function respond(
	command: Record<string, unknown>,
	id = crypto.randomUUID(),
): ManagementResponse {
	if (command.type === "placement_configuration") {
		expect(command.placement_id).toBe(configuration.placement_id);
		return {
			operation_id: id,
			state: "completed",
			result: structuredClone(configuration),
		};
	}
	if (command.type === "artifact") {
		const request = command.request as Record<string, unknown>;
		expect(request.kind).toBe("describe");
		expect(request.project_id).toBe(installed.project_id);
		expect(request.revision).toBe(installed.revision);
		return {
			operation_id: id,
			state: "completed",
			result: {
				project_id: installed.project_id,
				revision: installed.revision,
				event_id: request.event_id,
				items: request.event_id ? discoveredVariables : [event],
				next: null,
			},
		};
	}
	if (command.type === "apply") {
		const config = command.config as Record<string, unknown>;
		return {
			operation_id: id,
			state: "accepted",
			result: {
				placement_id: config.id,
				config_revision: Number(command.expected_revision) + 1,
			},
		};
	}
	if (command.type === "set_secret") {
		journals.set(id, command);
		return {
			operation_id: id,
			state: "accepted",
			result: {
				placement_id: command.placement_id,
				name: command.name,
				secret: "pending",
			},
		};
	}
	if (command.type === "operation") {
		const original = journals.get(String(command.operation_id));
		expect(original).toBeDefined();
		return {
			operation_id: String(command.operation_id),
			state: "completed",
			result: {
				placement_id: original?.placement_id,
				name: original?.name,
				secret: "completed",
			},
		};
	}
	throw new Error(`Unexpected management command: ${command.type}`);
}
async function render(connected = true) {
	await act(async () =>
		root.render(
			<StrictMode>
				<DeviceDeploymentForm
					installed={project}
					connected={connected}
					placements={placementRows}
					deviceId="device"
					profile={{ id: "profile" } as IProfile}
					run={(operation) => operation(call)}
					onApplied={async () => {
						applied++;
					}}
				/>
			</StrictMode>,
		),
	);
}
async function click(text: string) {
	const button = [...container.querySelectorAll("button")].find(
		(value) => value.textContent === text,
	);
	expect(button).toBeDefined();
	await act(async () => button?.click());
}
async function toggle(label: string) {
	const row = [...container.querySelectorAll("label")].find((value) =>
		value.textContent?.includes(label),
	);
	const checkbox = row?.querySelector<HTMLInputElement>(
		'input[type="checkbox"]',
	);
	expect(checkbox).toBeDefined();
	await act(async () => checkbox?.click());
}
async function fill(input: HTMLInputElement | null, value: string) {
	expect(input).not.toBeNull();
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, value);
		input?.dispatchEvent(new Event("input", { bubbles: true }));
	});
}
function input(label: string) {
	return (
		[...container.querySelectorAll("label")]
			.find((value) => value.textContent?.trim().startsWith(label))
			?.querySelector<HTMLInputElement>("input") ?? null
	);
}
async function chooseUpdate() {
	const select = container.querySelector("select");
	expect(select).not.toBeNull();
	await act(async () => {
		if (!select) return;
		select.value = configuration.placement_id;
		select.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await click("Read current placement and published events");
}
async function prepareUpdate(automatic = false) {
	discoveredVariables = [
		...variables,
		{
			id: "greeting",
			name: "Greeting",
			data_type: "String",
			value_type: "Normal",
			secret: false,
		},
	];
	placementRows = [
		{
			id: configuration.placement_id,
			project_id: "project",
			deployment_id: configuration.deployment_id,
			revision: configuration.config.revision,
			config_revision: configuration.config_revision,
			intent_revision: 12,
			applied_revision: 7,
			desired_state: "running",
			observed_state: "running",
		},
	];
	await render();
	await chooseUpdate();
	if (!automatic) await toggle("Check startup and roll back automatically");
}
async function prepare() {
	await render();
	await click("Read published events");
	await toggle("Published API");
	await toggle("API credential");
	await fill(
		container.querySelector('[aria-label="API credential value"]'),
		variableSecret,
	);
	const label = [...container.querySelectorAll("label")].find((value) =>
		value.textContent?.includes("Service access token"),
	);
	await fill(label?.querySelector("input") ?? null, serviceToken);
}
async function until(predicate: () => boolean) {
	for (let attempt = 0; attempt < 200; attempt++) {
		if (predicate()) return;
		await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
	}
	throw new Error("Timed out waiting for deployment UI");
}
afterEach(async () => {
	await act(async () => root.render(null));
	calls = [];
	journals = new Map();
	applied = 0;
	httpCalls = 0;
	onlineReads = [];
	project = installed;
	callOverride = undefined;
	discoveredVariables = variables;
	placementRows = [];
	configuration = structuredClone(initialConfiguration);
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.restore();
	await window.happyDOM.close();
});

test("pinned offline discovery provisions stopped placement with secrets only in encrypted management commands", async () => {
	await prepare();
	await click("Create stopped placement");
	await until(() => applied === 1);
	const mutation = calls.filter(
		({ command }) =>
			command.type !== "artifact" && command.type !== "operation",
	);
	expect(mutation.map(({ command }) => command.type)).toEqual([
		"apply",
		"set_secret",
		"set_secret",
	]);
	const apply = mutation[0].command;
	expect(apply.start).toBe(false);
	expect(apply.expected_revision).toBe(0);
	const config = apply.config as Record<string, unknown>;
	expect(config.project_path).toBe(installed.project_path);
	expect(config.revision).toBe(installed.revision);
	expect(config.events).toEqual([
		{
			event_id: "http-event",
			event_version: [1, 0, 0],
			board_version: [2, 0, 0],
		},
	]);
	expect(config.variables).toEqual({});
	expect(JSON.stringify(config)).not.toContain(variableSecret);
	expect(JSON.stringify(config)).not.toContain(serviceToken);
	const refs = config.secret_overrides as Record<string, unknown>;
	expect(mutation[1].command).toMatchObject({
		placement_id: config.id,
		expected_revision: 1,
		name: refs.credential,
		value: JSON.stringify(variableSecret),
	});
	expect(mutation[2].command).toMatchObject({
		placement_id: config.id,
		expected_revision: 1,
		name: "service-access",
		value: serviceToken,
	});
	expect(new Set(mutation.map(({ id }) => id)).size).toBe(3);
	expect(container.querySelector("output")?.textContent).toContain(
		"stopped and ready for review",
	);
	for (const input of container.querySelectorAll<HTMLInputElement>(
		'input[type="password"]',
	))
		expect(input.value).toBe("");
	expect(httpCalls).toBe(0);
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
});

test("unmount after accepted apply prevents sending the remaining secret commands", async () => {
	await prepare();
	let finish!: (response: ManagementResponse) => void;
	let pendingApply: Record<string, unknown> | undefined;
	let pendingId: string | undefined;
	callOverride = async (command, id) => {
		if (command.type !== "apply") return respond(command, id);
		pendingApply = command;
		pendingId = id;
		return new Promise((resolve) => {
			finish = resolve;
		});
	};
	await click("Create stopped placement");
	expect(pendingApply).toBeDefined();
	await act(async () => root.render(null));
	const accepted = pendingApply;
	if (!accepted) throw new Error("Apply was not submitted");
	await act(async () => finish(respond(accepted, pendingId)));
	expect(calls.some(({ command }) => command.type === "set_secret")).toBe(
		false,
	);
	expect(applied).toBe(0);
	expect(httpCalls).toBe(0);
});

test("unmount while the last secret receipt is in flight does not refresh an obsolete dialog", async () => {
	await prepare();
	let finish: ((response: ManagementResponse) => void) | undefined;
	let finalReceipt: ManagementResponse | undefined;
	callOverride = async (command, id) => {
		if (
			command.type === "operation" &&
			journals.get(String(command.operation_id))?.name === "service-access"
		) {
			finalReceipt = respond(command, id);
			return new Promise((resolve) => {
				finish = resolve;
			});
		}
		return respond(command, id);
	};
	await click("Create stopped placement");
	await until(() => finish !== undefined);
	await act(async () => root.render(null));
	if (!finish || !finalReceipt)
		throw new Error("Final secret receipt was not requested");
	const resolve = finish;
	const receipt = finalReceipt;
	await act(async () => resolve(receipt));
	expect(applied).toBe(0);
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	expect(
		calls.filter(({ command }) => command.type === "set_secret"),
	).toHaveLength(2);
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
});

test("reconnecting an unlocked form preserves its plan and queries a lost apply receipt before continuing", async () => {
	await prepare();
	let accepted: ManagementResponse | undefined;
	let applyId: string | undefined;
	callOverride = async (command, id) => {
		if (command.type === "apply") {
			applyId = id;
			accepted = respond(command, id);
			throw new Error(
				"The device accepted this operation but its response was lost.",
			);
		}
		if (command.type === "operation" && command.operation_id === applyId) {
			if (!accepted) throw new Error("Missing recorded apply result");
			return accepted;
		}
		return respond(command, id);
	};
	await click("Create stopped placement");
	expect(accepted?.state).toBe("accepted");
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	expect(calls.some(({ command }) => command.type === "set_secret")).toBe(
		false,
	);
	const beforeReconnect = calls.length;
	await render(false);
	const retry = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retry the same provisioning operations",
	);
	expect(retry?.disabled).toBe(true);
	expect(container.querySelector("fieldset")?.disabled).toBe(true);
	await act(async () => {
		retry?.click();
		container
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
	});
	expect(calls).toHaveLength(beforeReconnect);
	await render(true);
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(false);
	await click("Retry the same provisioning operations");
	await until(() => applied === 1);
	expect(calls[beforeReconnect].command).toEqual({
		type: "operation",
		operation_id: applyId,
	});
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	const secrets = calls.filter(({ command }) => command.type === "set_secret");
	expect(secrets).toHaveLength(2);
	expect(secrets[0].command.placement_id).toBe(accepted?.result.placement_id);
	expect(secrets[0].command.value).toBe(JSON.stringify(variableSecret));
	expect(secrets[1].command.value).toBe(serviceToken);
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
	expect(httpCalls).toBe(0);
});

test("updating reads current configuration and preserves secret references and advanced settings", async () => {
	await prepareUpdate();
	expect(input("Placement ID")?.disabled).toBe(true);
	expect(input("Deployment ID")?.disabled).toBe(true);
	expect(input("Maximum replicas")?.disabled).toBe(true);
	expect(input("Listener IP")?.value).toBe("0.0.0.0");
	expect(input("Port")?.value).toBe("9345");
	expect(
		container.querySelector<HTMLInputElement>('[aria-label="Greeting value"]')
			?.value,
	).toBe("Welcome from this device");
	expect(container.textContent).toContain("stored-credential");
	expect(container.textContent).toContain("existing-access");
	expect(container.textContent).toContain(
		"Applying the update stops its running services",
	);
	await fill(
		container.querySelector('[aria-label="Greeting value"]'),
		"Updated greeting",
	);
	await click("Apply placement update");
	await until(() => applied === 1);
	const mutations = calls.filter(({ command }) =>
		["apply", "set_secret", "start"].includes(String(command.type)),
	);
	expect(mutations).toHaveLength(1);
	const update = mutations[0].command;
	expect(update).toMatchObject({
		type: "apply",
		expected_revision: 7,
		start: false,
	});
	const config = update.config as typeof initialConfiguration.config;
	expect(config.id).toBe(initialConfiguration.placement_id);
	expect(config.deployment_id).toBe(initialConfiguration.deployment_id);
	expect(config.project_path).toBe(installed.project_path);
	expect(config.revision).toBe(installed.revision);
	expect(config.hosting).toEqual(initialConfiguration.config.hosting);
	expect(config.variables).toEqual({ greeting: "Updated greeting" });
	expect(config.secret_overrides).toEqual({ credential: "stored-credential" });
	expect(config.resource_grant).toEqual(
		initialConfiguration.config.resource_grant,
	);
	expect(config.bit_pins).toEqual(initialConfiguration.config.bit_pins);
	expect(config.package_pins).toEqual(initialConfiguration.config.package_pins);
	expect(config.max_replicas).toBe(3);
	expect(config.restart).toBe("on_failure");
	expect(config.future_setting).toEqual({ retain_me: true });
	expect(container.querySelector("output")?.textContent).toContain(
		"stopped and ready for review",
	);
	expect(httpCalls).toBe(0);
});

test("older configuration responses offer manual updates even when inspection says running", async () => {
	callOverride = async (command, id) => {
		const response = respond(command, id);
		if (command.type === "placement_configuration") {
			const { desired_state: _desiredState, ...legacy } = response.result;
			response.result = legacy;
		}
		return response;
	};
	await prepareUpdate(true);
	expect(placementRows[0].desired_state).toBe("running");
	expect(container.textContent).not.toContain(
		"Check startup and roll back automatically",
	);
	expect(container.textContent).toContain(
		"This update uses manual review and Start",
	);
	await click("Apply placement update");
	await until(() => applied === 1);
	expect(calls.some(({ command }) => command.type === "stage_rollout")).toBe(
		false,
	);
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
});

for (const source of ["offline", "online"] as const) {
	test(`a running native ${source} update stages secrets and reports confirmed automatic startup`, async () => {
		if (source === "online") {
			project = {
				project_id: "project",
				project_path: "/device/online/projects/project",
				revision: "published-2",
				source,
			};
			Object.assign(configuration, { rollout_sources: ["offline", "online"] });
			configuration.config.source = source;
			configuration.config.project_path = project.project_path;
		}
		let rolloutId = "";
		callOverride = async (command, id) => {
			if (command.type === "stage_rollout") rolloutId = String(id);
			if (
				[
					"stage_rollout",
					"rollout_secret",
					"activate_rollout",
					"rollout",
				].includes(String(command.type))
			)
				return {
					operation_id: id ?? crypto.randomUUID(),
					state: command.type === "rollout" ? "completed" : "accepted",
					result: {
						rollout_id: rolloutId,
						placement_id: configuration.placement_id,
						project_id: "project",
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
			return respond(command, id);
		};
		await prepareUpdate(true);
		expect(container.textContent).toContain(
			"restores the previous configuration if startup fails",
		);
		await fill(input("Service access token"), serviceToken);
		await click("Update with startup checks");
		await until(() => applied === 1);
		expect(
			calls
				.filter(({ command }) =>
					[
						"stage_rollout",
						"rollout_secret",
						"activate_rollout",
						"rollout",
					].includes(String(command.type)),
				)
				.map(({ command }) => command.type),
		).toEqual([
			"stage_rollout",
			"rollout_secret",
			"activate_rollout",
			"rollout",
		]);
		expect(
			calls.some(({ command }) =>
				["apply", "start", "set_secret"].includes(String(command.type)),
			),
		).toBe(false);
		expect(container.textContent).toContain(
			"Updated services started and passed the listener startup check",
		);
		expect(input("Service access token")?.value).toBe("");
		const staged = calls.find(({ command }) => command.type === "stage_rollout")
			?.command.config;
		expect(staged).toMatchObject({
			source,
			project_path: project.project_path,
			revision: project.revision,
			resource_grant: initialConfiguration.config.resource_grant,
			variables: initialConfiguration.config.variables,
			secret_overrides: initialConfiguration.config.secret_overrides,
		});
		expect(httpCalls).toBe(0);
		if (source === "online") {
			expect(onlineReads).toEqual(["events", "event", "board"]);
			expect(calls.some(({ command }) => command.type === "artifact")).toBe(
				false,
			);
		} else expect(onlineReads).toEqual([]);
	});
}

test("an online project on an older agent offers manual updates without claiming startup checks", async () => {
	project = {
		project_id: "project",
		project_path: "/device/online/projects/project",
		revision: "published-2",
		source: "online",
	};
	configuration.config.source = "online";
	configuration.config.project_path = project.project_path;
	await prepareUpdate(true);
	expect(container.textContent).not.toContain(
		"Check startup and roll back automatically",
	);
	expect(container.textContent).toContain(
		"This update uses manual review and Start",
	);
	await click("Apply placement update");
	await until(() => applied === 1);
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	expect(calls.some(({ command }) => command.type === "stage_rollout")).toBe(
		false,
	);
	expect(httpCalls).toBe(0);
});

test("an online placement explicitly selects buffered resources and budgets", async () => {
	project = { ...installed, source: "online", revision: "published-2" };
	configuration.config.source = "online";
	await prepareUpdate(true);
	expect(container.textContent).toContain(
		"Buffer selected writes on this device",
	);
	expect(input("Queue budget (MiB)")).toBeNull();
	await toggle("Buffer selected writes on this device");
	await click("Add buffered table");
	await fill(input("Table name"), "measurements");
	await click("Add buffered directory");
	await fill(input("Directory prefix"), "exports");
	await fill(input("Queue budget (MiB)"), "32");
	await click("Apply placement update");
	await until(() => applied === 1);
	const appliedConfig = calls.find(({ command }) => command.type === "apply")
		?.command.config as Record<string, unknown>;
	expect(appliedConfig.offline_writes).toMatchObject({
		tables: [
			{
				purpose: "storage",
				database: "db",
				table: "measurements",
				primary_key: "id",
			},
		],
		files: [{ purpose: "storage", prefix: "exports" }],
		max_queue_bytes: 32 * 1048576,
	});
	expect(httpCalls).toBe(0);
});

test("a restored configuration is reported as rollback and requires review before another update", async () => {
	let rolloutId = "";
	callOverride = async (command, id) => {
		if (command.type === "stage_rollout") rolloutId = String(id);
		if (
			["stage_rollout", "activate_rollout", "rollout"].includes(
				String(command.type),
			)
		)
			return {
				operation_id: id ?? crypto.randomUUID(),
				state: command.type === "rollout" ? "completed" : "accepted",
				result: {
					rollout_id: rolloutId,
					placement_id: configuration.placement_id,
					project_id: "project",
					state:
						command.type === "stage_rollout"
							? "staged"
							: command.type === "activate_rollout"
								? "validating"
								: "rolled_back",
				},
			};
		return respond(command, id);
	};
	await prepareUpdate(true);
	await click("Update with startup checks");
	await until(
		() =>
			container.textContent?.includes("did not pass startup checks") ?? false,
	);
	expect(container.textContent).toContain(
		"restored the previous configuration and secret references",
	);
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(true);
	expect(container.textContent).not.toContain("Updated services started");
});

test("reopened deployment reads durable rollout status without resubmitting an update", async () => {
	const rolloutId = crypto.randomUUID();
	Object.assign(configuration, {
		rollout: {
			rollout_id: rolloutId,
			placement_id: configuration.placement_id,
			project_id: "project",
			state: "activating",
		},
	});
	callOverride = async (command, id) =>
		command.type === "rollout"
			? {
					operation_id: id ?? crypto.randomUUID(),
					state: "completed",
					result: {
						rollout_id: rolloutId,
						placement_id: configuration.placement_id,
						project_id: "project",
						state: "healthy",
					},
				}
			: respond(command, id);
	await prepareUpdate(true);
	expect(container.textContent).toContain(
		"Starting the new revision and observing listener readiness",
	);
	await click("Follow rollout status");
	await until(() => applied === 1);
	expect(
		calls.some(({ command }) =>
			["apply", "stage_rollout", "activate_rollout"].includes(
				String(command.type),
			),
		),
	).toBe(false);
	expect(container.textContent).toContain(
		"Updated services started and passed the listener startup check",
	);
});

test("a reopened staged update can be discarded while its current services keep running", async () => {
	const status = {
		rollout_id: "previous-session-update",
		placement_id: configuration.placement_id,
		project_id: "project",
		state: "staged",
	};
	Object.assign(configuration, { desired_state: "running", rollout: status });
	callOverride = async (command, id) => {
		if (command.type === "cancel_rollout") status.state = "cancelled";
		if (["rollout", "cancel_rollout"].includes(String(command.type)))
			return {
				operation_id: id ?? crypto.randomUUID(),
				state: command.type === "rollout" ? "completed" : "accepted",
				result: { ...status },
			};
		return respond(command, id);
	};
	await prepareUpdate(true);
	expect(container.textContent).toContain("current services keep running");
	await click("Discard staged update");
	await until(() => applied === 1);
	expect(container.textContent).toContain("rollout was cancelled");
	expect(
		calls.some(({ command }) =>
			["stop", "start", "stage_rollout", "apply"].includes(
				String(command.type),
			),
		),
	).toBe(false);
	const reads = calls.filter(
		({ command }) => command.type === "placement_configuration",
	).length;
	await click("Reload current placement and published events");
	expect(
		calls.filter(({ command }) => command.type === "placement_configuration")
			.length,
	).toBe(reads + 1);
});

for (const terminal of ["rolled_back", "healthy"] as const) {
	test(`following a disconnected rollout clears pending secrets when it becomes ${terminal}`, async () => {
		let rolloutId = "";
		let reads = 0;
		callOverride = async (command, id) => {
			if (command.type === "stage_rollout") rolloutId = String(id);
			if (command.type === "rollout" && ++reads === 2)
				throw new Error("Disconnected while observing startup");
			if (
				[
					"stage_rollout",
					"rollout_secret",
					"activate_rollout",
					"rollout",
				].includes(String(command.type))
			)
				return {
					operation_id: id ?? crypto.randomUUID(),
					state: command.type === "rollout" ? "completed" : "accepted",
					result: {
						rollout_id: rolloutId,
						placement_id: configuration.placement_id,
						project_id: "project",
						...(command.type === "rollout_secret"
							? { name: command.name, secret: "completed" }
							: {
									state:
										command.type === "stage_rollout"
											? "staged"
											: command.type === "activate_rollout" || reads === 1
												? "validating"
												: terminal,
								}),
					},
				};
			return respond(command, id);
		};
		await prepareUpdate(true);
		await fill(input("Service access token"), serviceToken);
		await click("Update with startup checks");
		await until(
			() =>
				container.textContent?.includes(
					"Disconnected while observing startup",
				) ?? false,
		);
		expect(input("Service access token")?.value).toBe(serviceToken);
		await click("Follow rollout status");
		await until(() => applied === 1);
		expect(input("Service access token")?.value).toBe("");
		expect(container.textContent).not.toContain(
			"Retry the same provisioning operations",
		);
		if (terminal === "rolled_back") {
			const reload = [
				...container.querySelectorAll<HTMLButtonElement>("button"),
			].find(
				(value) =>
					value.textContent === "Reload current placement and published events",
			);
			expect(reload?.disabled).toBe(false);
			const previousReads = calls.filter(
				({ command }) => command.type === "placement_configuration",
			).length;
			await click("Reload current placement and published events");
			expect(
				calls.filter(
					({ command }) => command.type === "placement_configuration",
				).length,
			).toBe(previousReads + 1);
		} else
			expect(container.textContent).toContain(
				"Updated services started and passed the listener startup check",
			);
		expect(
			calls.filter(({ command }) => command.type === "stage_rollout"),
		).toHaveLength(1);
	});
}

test("removed and incompatible stored overrides require explicit replacement or removal", async () => {
	await prepareUpdate();
	(configuration.config.variables as Record<string, unknown>).obsolete =
		"old value";
	discoveredVariables = [
		variables[0],
		{
			id: "greeting",
			name: "Greeting",
			data_type: "Boolean",
			value_type: "Normal",
			secret: false,
		},
	];
	await click("Reload current placement and published events");
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(true);
	expect(container.querySelector('[aria-label="Greeting value"]')).toBeNull();
	expect(container.textContent).toContain("Remove override greeting");
	expect(container.textContent).toContain("Remove override obsolete");
	await click("Remove override obsolete");
	await toggle("Greeting");
	await fill(container.querySelector('[aria-label="Greeting value"]'), "true");
	await click("Apply placement update");
	await until(() => applied === 1);
	const update = calls.find(({ command }) => command.type === "apply");
	expect((update?.command.config as Record<string, unknown>).variables).toEqual(
		{ greeting: true },
	);
	expect(
		(update?.command.config as Record<string, unknown>).secret_overrides,
	).toEqual({ credential: "stored-credential" });
	expect(
		calls.filter(({ command }) => command.type === "set_secret"),
	).toHaveLength(0);
});

test("stale configuration blocks the update until the user reloads and reviews it", async () => {
	await prepareUpdate();
	let rejected = false;
	callOverride = async (command, id) => {
		if (command.type === "apply" && !rejected) {
			rejected = true;
			configuration.config_revision = 8;
			configuration.config.variables.greeting = "Changed by another controller";
			return { operation_id: id ?? "", state: "rejected", result: {} };
		}
		return respond(command, id);
	};
	await click("Apply placement update");
	expect(container.textContent).toContain("Reload the current placement");
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(true);
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	await act(async () =>
		container
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
	);
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	await click("Reload current placement and published events");
	expect(
		container.querySelector<HTMLInputElement>('[aria-label="Greeting value"]')
			?.value,
	).toBe("Changed by another controller");
	await click("Apply placement update");
	await until(() => applied === 1);
	const mutations = calls.filter(({ command }) => command.type === "apply");
	expect(mutations.map(({ command }) => command.expected_revision)).toEqual([
		7, 8,
	]);
	expect(mutations[1].id).not.toBe(mutations[0].id);
});

test("an update with a lost apply response keeps its CAS and operation ID across reconnect", async () => {
	await prepareUpdate();
	await toggle("API credential");
	await fill(
		container.querySelector('[aria-label="API credential value"]'),
		"replacement-private-value",
	);
	let accepted: ManagementResponse | undefined;
	let applyId: string | undefined;
	callOverride = async (command, id) => {
		if (command.type === "apply") {
			applyId = id;
			accepted = respond(command, id);
			throw new Error("Apply response was lost.");
		}
		if (command.type === "operation" && command.operation_id === applyId) {
			if (!accepted) throw new Error("Missing apply receipt");
			return accepted;
		}
		return respond(command, id);
	};
	await click("Apply placement update");
	expect(
		calls.filter(({ command }) => command.type === "set_secret"),
	).toHaveLength(0);
	const before = calls.length;
	await render(false);
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(true);
	await render(true);
	await click("Retry the same provisioning operations");
	await until(() => applied === 1);
	expect(calls[before].command).toEqual({
		type: "operation",
		operation_id: applyId,
	});
	const applies = calls.filter(({ command }) => command.type === "apply");
	expect(applies).toHaveLength(1);
	expect(applies[0].command.expected_revision).toBe(7);
	const secrets = calls.filter(({ command }) => command.type === "set_secret");
	expect(secrets).toHaveLength(1);
	expect(secrets[0].command).toMatchObject({
		placement_id: "existing-service",
		expected_revision: 8,
		value: JSON.stringify("replacement-private-value"),
	});
	expect(secrets[0].command.name).not.toBe("stored-credential");
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
});

test("locking an update form after apply prevents its remaining secret publications", async () => {
	await prepareUpdate();
	await toggle("API credential");
	await fill(
		container.querySelector('[aria-label="API credential value"]'),
		"replacement-private-value",
	);
	let finish: ((response: ManagementResponse) => void) | undefined;
	let receipt: ManagementResponse | undefined;
	callOverride = async (command, id) => {
		if (command.type !== "apply") return respond(command, id);
		receipt = respond(command, id);
		return new Promise((resolve) => {
			finish = resolve;
		});
	};
	await click("Apply placement update");
	expect(finish).toBeDefined();
	await act(async () => root.render(null));
	if (!finish || !receipt) throw new Error("Update was not submitted");
	const resolve = finish;
	const response = receipt;
	await act(async () => resolve(response));
	expect(
		calls.filter(({ command }) => command.type === "set_secret"),
	).toHaveLength(0);
	expect(applied).toBe(0);
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
});

test("a failed secret publication requires reloading the created placement and clears private inputs", async () => {
	await prepare();
	callOverride = async (command, id) => {
		if (command.type === "apply") {
			const config = command.config as typeof initialConfiguration.config;
			configuration = {
				placement_id: config.id,
				project_id: config.project_id,
				deployment_id: config.deployment_id,
				config_revision: 1,
				desired_state: "stopped",
				config: structuredClone(config),
			};
		}
		if (command.type === "operation") {
			const original = journals.get(String(command.operation_id));
			if (original)
				return {
					operation_id: String(command.operation_id),
					state: "failed",
					result: {
						placement_id: original.placement_id,
						name: original.name,
						secret: "failed",
					},
				};
		}
		return respond(command, id);
	};
	await click("Create stopped placement");
	await until(() => applied === 1);
	expect(container.querySelector("select")?.value).toBe(
		configuration.placement_id,
	);
	expect(
		container.querySelector<HTMLButtonElement>('button[type="submit"]')
			?.disabled,
	).toBe(true);
	expect(container.textContent).not.toContain(
		"Retry the same provisioning operations",
	);
	for (const password of container.querySelectorAll<HTMLInputElement>(
		'input[type="password"]',
	))
		expect(password.value).toBe("");
	expect(calls.filter(({ command }) => command.type === "apply")).toHaveLength(
		1,
	);
	expect(
		calls.filter(({ command }) => command.type === "set_secret"),
	).toHaveLength(1);
	await click("Read current placement and published events");
	expect(container.textContent).toContain("Updating configuration revision 1");
	expect(input("Placement ID")?.value).toBe(configuration.placement_id);
	expect(input("Placement ID")?.disabled).toBe(true);
	expect(calls.some(({ command }) => command.type === "start")).toBe(false);
});
