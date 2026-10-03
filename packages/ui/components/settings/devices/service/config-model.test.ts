import { describe, expect, test } from "bun:test";
import type { DeploymentVariable } from "../../../../lib/device-management/deployment";
import {
	CONFIG_MAX_BYTES,
	ConfigTooLargeError,
	DEFAULT_APPLY,
	EMPTY_DRAFT,
	type FieldContext,
	type PlacementConfig,
	applyDraft,
	boardDefinitions,
	checkApplyHow,
	exposureOf,
	extraDiffRows,
	hostingOf,
	jsonBytes,
	parseJsonSettings,
	queueTarget,
	restartOf,
	serviceDefinitions,
	servicePageAddress,
	settingFields,
	settingsPlan,
} from "./config-model";

const GIB = 1024 ** 3;

function config(patch: Record<string, unknown> = {}): PlacementConfig {
	return {
		id: "support-bot",
		project_id: "app_support_portal",
		deployment_id: "5d2ee3ca-1b6c-4e42-a152-bafa2370440f",
		revision: "71c6216b",
		source: "offline",
		project_path: "projects/app_support_portal",
		events: [
			{
				event_id: "evt_support_chat",
				event_version: [2, 3, 0],
				board_version: [5, 1, 2],
			},
		],
		hosting: {
			host: "0.0.0.0",
			port: 8443,
			max_in_flight: 64,
			request_timeout_secs: 300,
			auth_secret: "service-access",
		},
		max_replicas: 2,
		variables: { greeting: "Hello", retries: 3 },
		secret_overrides: { credential: "variable-1" },
		future_setting: { retain_me: true },
		...patch,
	} as unknown as PlacementConfig;
}

const context: FieldContext = {
	canAssignCertificate: true,
	multiInstance: true,
};

const definition = (
	id: string,
	dataType: string,
	rest: Partial<DeploymentVariable> = {},
): DeploymentVariable => ({
	id,
	name: id,
	data_type: dataType,
	value_type: "Normal",
	secret: false,
	...rest,
});

const draft = (values: Record<string, string>, removed: string[] = []) => ({
	values,
	removed,
});

describe("views", () => {
	test("hosting, exposure and the service page address", () => {
		const hosting = hostingOf(config());
		expect(hosting?.exposure).toBe("all");
		expect(hosting?.origins).toEqual([]);
		expect(exposureOf("127.0.0.1")).toBe("loopback");
		expect(exposureOf("192.168.1.9")).toBe("one");
		if (!hosting) throw new Error("no hosting");
		expect(servicePageAddress(hosting, true)).toBeUndefined();
		expect(servicePageAddress(hosting, true, "edge.example.com")).toBe(
			"https://edge.example.com:8443/ui/",
		);
		expect(
			servicePageAddress(
				{ host: "::1", port: 8080, exposure: "loopback" },
				false,
			),
		).toBe("http://[::1]:8080/ui/");
		expect(hostingOf(config({ hosting: null }))).toBeNull();
	});

	test("the restart policy is absent, not zero, when the device sent none", () => {
		expect(restartOf(config())).toBeNull();
		expect(
			restartOf(
				config({
					restart: {
						initial_backoff_secs: 2,
						max_backoff_secs: 60,
						max_restarts: 5,
					},
				}),
			),
		).toEqual({ initialS: 2, maxS: 60, maxRestarts: 5 });
	});

	test("queue targets come from the device's resource descriptor", () => {
		expect(
			queueTarget('{"kind":"table","purpose":"storage","table":"notes"}'),
		).toEqual({ kind: "table", name: "notes", purpose: "storage" });
		expect(
			queueTarget('{"kind":"file","purpose":"user","path":"a/b.pdf"}'),
		).toEqual({ kind: "file", name: "a/b.pdf", purpose: "user" });
		expect(queueTarget("not json")).toBeNull();
	});

	test("flow variables a device can set, from the board and its layers", () => {
		expect(
			boardDefinitions({
				variables: {
					a: {
						id: "greeting",
						name: "Greeting",
						data_type: "String",
						value_type: "Normal",
						exposed: true,
					},
					b: { id: "hidden", name: "Hidden", exposed: false },
				},
				layers: {
					one: {
						variables: {
							c: {
								id: "token",
								name: "Token",
								data_type: "String",
								value_type: "Normal",
								secret: true,
								runtime_configured: true,
							},
						},
					},
				},
			}).map((entry) => [entry.id, entry.secret]),
		).toEqual([
			["greeting", false],
			["token", true],
		]);
	});
});

describe("fields", () => {
	test("write buffering and background events lock the instance count", () => {
		const max = (fields: ReturnType<typeof settingFields>) =>
			fields.find((field) => field.id === "max");
		expect(max(settingFields(config(), context))?.lock).toBeUndefined();
		expect(
			max(
				settingFields(
					config({
						source: "online",
						max_replicas: 1,
						offline_writes: {
							tables: [],
							files: [],
							max_queue_bytes: 1,
							max_operations: 1,
							max_age_seconds: 60,
							max_mirror_bytes: 1,
						},
					}),
					context,
				),
			)?.lock,
		).toBe("buffering_single");
		expect(
			max(
				settingFields(config({ max_replicas: 1 }), {
					...context,
					multiInstance: false,
				}),
			)?.lock,
		).toBe("background_single");
	});

	test("stored values are flagged against the definitions; unknown definitions flag nothing", () => {
		const unknown = settingFields(config(), context);
		expect(unknown.some((field) => field.variable?.issue)).toBe(false);
		const fields = settingFields(config(), {
			...context,
			definitions: [
				definition("greeting", "Boolean"),
				definition("extra", "String"),
			],
		});
		const issue = (id: string) =>
			fields.find((field) => field.variable?.id === id)?.variable?.issue;
		expect(issue("greeting")).toBe("incompatible");
		expect(issue("retries")).toBe("unused");
		expect(issue("credential")).toBe("unused");
		const extra = fields.find((field) => field.variable?.id === "extra");
		expect(extra?.variable?.stored).toBe(false);
		expect(extra?.value).toBe("");
	});

	test("a stored secret the app no longer treats as a secret can only be removed", () => {
		const fields = settingFields(config(), {
			...context,
			definitions: [
				definition("greeting", "String"),
				definition("retries", "Integer"),
				definition("credential", "String"),
			],
		});
		const secret = fields.find((field) => field.variable?.id === "credential");
		expect(secret?.kind).toBe("secret");
		expect(secret?.variable?.issue).toBe("incompatible");
		expect(
			applyDraft(config(), fields, draft({ port: "9000" })).errors.map(
				(error) => [error.code, error.field],
			),
		).toEqual([["must_resolve", "var.credential"]]);
		const removed = applyDraft(config(), fields, draft({}, ["credential"]));
		expect(removed.errors).toEqual([]);
		expect(removed.config.secret_overrides).toEqual({});
		expect(removed.config.variables).toEqual({ greeting: "Hello", retries: 3 });
	});
});

describe("draft", () => {
	const fields = settingFields(config(), context);

	test("only what the draft touches changes; everything else stays as the device sent it", () => {
		const result = applyDraft(
			config(),
			fields,
			draft({ port: "9443", timeout: "300" }),
		);
		expect(result.errors).toEqual([]);
		expect(result.changed).toEqual(["port"]);
		expect(result.config.hosting?.port).toBe(9443);
		expect(result.config.hosting?.auth_secret).toBe("service-access");
		expect(result.config.secret_overrides).toEqual({
			credential: "variable-1",
		});
		expect((result.config as Record<string, unknown>).future_setting).toEqual({
			retain_me: true,
		});
		expect(config().hosting?.port).toBe(8443);
	});

	test("nothing changed, wrong numbers and ranges are errors, not commands", () => {
		expect(applyDraft(config(), fields, EMPTY_DRAFT).errors).toEqual([
			{ code: "nothing_changed" },
		]);
		const codes = (values: Record<string, string>) =>
			applyDraft(config(), fields, draft(values)).errors.map(
				(error) => error.code,
			);
		expect(codes({ port: "abc" })).toEqual(["not_a_number"]);
		expect(codes({ port: "80.5" })).toEqual(["not_whole"]);
		expect(codes({ max: "40" })).toEqual(["out_of_range"]);
		expect(codes({ "var.retries": "many" })).toEqual(["invalid_value"]);
	});

	test("ports of other services, Let's Encrypt and the cloud approval limit are checked", () => {
		const errors = (values: Record<string, string>) =>
			applyDraft(config(), fields, draft(values), {
				otherPorts: [{ port: 8081, service: "invoice-extractor" }],
				port80Reserved: true,
				approvalMaxInstances: 2,
			}).errors.map((error) => error.code);
		expect(errors({ port: "8081" })).toEqual(["port_in_use"]);
		expect(errors({ port: "80" })).toEqual(["port_reserved"]);
		expect(errors({ max: "3" })).toEqual(["instances_over_approval"]);
		expect(errors({ max: "1" })).toEqual([]);
	});

	test("a removed stored value and a replaced one resolve their issues", () => {
		const flagged = settingFields(config(), {
			...context,
			definitions: [
				definition("greeting", "Boolean"),
				definition("credential", "String", { secret: true }),
			],
		});
		expect(
			applyDraft(config(), flagged, draft({ port: "9000" })).errors.map(
				(error) => [error.code, error.field],
			),
		).toEqual([
			["must_resolve", "var.greeting"],
			["must_resolve", "var.retries"],
		]);
		const fixed = applyDraft(
			config(),
			flagged,
			draft({ "var.greeting": "true" }, ["retries"]),
		);
		expect(fixed.errors).toEqual([]);
		expect(fixed.config.variables).toEqual({ greeting: true });
		expect(fixed.config.secret_overrides).toEqual({
			credential: "variable-1",
		});
		expect(fixed.changed).toEqual(["var.greeting", "rm.retries"]);
	});

	test("sandbox limits and buffering budgets convert to the device's units", () => {
		const sandboxed = config({
			source: "online",
			max_replicas: 1,
			resources: {
				profile: "linux_sandbox",
				cpu_millis: 2000,
				memory_bytes: 2 * GIB,
				max_processes: 256,
				disk_bytes: 4 * GIB,
			},
			offline_writes: {
				tables: [],
				files: [],
				max_queue_bytes: 256 * 1024 ** 2,
				max_operations: 10_000,
				max_age_seconds: 7 * 86_400,
				max_mirror_bytes: 2 * GIB,
			},
		});
		const result = applyDraft(
			sandboxed,
			settingFields(sandboxed, context),
			draft({ cpu: "1.5", mem: "3", qmib: "512", qage: "14" }),
		);
		expect(result.errors).toEqual([]);
		expect(result.config.resources?.cpu_millis).toBe(1500);
		expect(result.config.resources?.memory_bytes).toBe(3 * GIB);
		expect(result.config.offline_writes?.max_queue_bytes).toBe(512 * 1024 ** 2);
		expect(result.config.offline_writes?.max_age_seconds).toBe(14 * 86_400);
		expect(result.config.offline_writes?.max_operations).toBe(10_000);
	});

	test("settings over the message limit are refused", () => {
		const big = config({ variables: { greeting: "x".repeat(12_000) } });
		const result = applyDraft(
			big,
			settingFields(big, context),
			draft({ port: "9000" }),
		);
		expect(result.errors.map((error) => error.code)).toEqual(["too_large"]);
	});
});

describe("JSON", () => {
	test("identity, validity, size and no-change are checked before anything is sent", () => {
		const base = config();
		const code = (text: string) => {
			const parsed = parseJsonSettings(base, text, new Set());
			return "error" in parsed ? parsed.error.code : "ok";
		};
		expect(code("{")).toBe("invalid_json");
		expect(code("[]")).toBe("not_an_object");
		expect(code(JSON.stringify(base))).toBe("nothing_changed");
		expect(code(JSON.stringify({ ...base, id: "other" }))).toBe(
			"identity_changed",
		);
		expect(
			code(JSON.stringify({ ...base, padding: "x".repeat(CONFIG_MAX_BYTES) })),
		).toBe("too_large");
		expect(code(JSON.stringify({ ...base, max_replicas: 3 }))).toBe("ok");
	});

	test("an event that needs the wizard can't be typed in: it would skip the agent check, the hub and the token", () => {
		const base = config();
		const event = (id: string) => ({
			event_id: id,
			event_version: [1, 0, 0],
			board_version: [1, 0, 0],
		});
		const withEvents = (...ids: string[]) =>
			JSON.stringify({ ...base, events: [...base.events, ...ids.map(event)] });
		const result = (text: string, gated: ReadonlySet<string> | null) => {
			const parsed = parseJsonSettings(base, text, gated);
			return "error" in parsed ? parsed.error : "ok";
		};
		const gated = new Set(["evt_nightly", "evt_orders", "evt_helper"]);

		for (const id of gated)
			expect(result(withEvents(id), gated)).toEqual({
				code: "event_added",
				params: { event: id },
			});
		expect(result(withEvents("evt_faq"), gated)).toBe("ok");
		// The app's events are not loaded: what an added event is, isn't known.
		expect(result(withEvents("evt_faq"), null)).toEqual({
			code: "event_added",
			params: { event: "evt_faq" },
		});
		// Nothing is added: removing by hand, and other edits, stay possible.
		expect(result(JSON.stringify({ ...base, events: [] }), null)).toBe("ok");
		expect(result(JSON.stringify({ ...base, max_replicas: 3 }), null)).toBe(
			"ok",
		);
		// A schedule the service already runs is not "added".
		const running = config({
			events: [...base.events, event("evt_nightly")],
		});
		const kept = parseJsonSettings(
			running,
			JSON.stringify({ ...running, max_replicas: 3 }),
			gated,
		);
		expect("config" in kept).toBe(true);
		// Something that is not an event list is left to the device.
		expect(result(JSON.stringify({ ...base, events: "none" }), null)).toBe(
			"ok",
		);
	});

	test("a bot token key typed in by hand is refused; one the settings hold already stays", () => {
		const base = config();
		const typed = (overrides: Record<string, string>) =>
			parseJsonSettings(
				base,
				JSON.stringify({ ...base, secret_overrides: overrides }),
				new Set(),
			);
		expect(typed({ "event.evt_helper.bot_token": "secret-1" })).toEqual({
			error: { code: "event_added", params: { event: "evt_helper" } },
		});
		expect("config" in typed({ erp_password: "variable-7f3a" })).toBe(true);
		const holding = config({
			secret_overrides: { "event.evt_helper.bot_token": "secret-1" },
		});
		const kept = parseJsonSettings(
			holding,
			JSON.stringify({ ...holding, max_replicas: 3 }),
			new Set(),
		);
		expect("config" in kept).toBe(true);
	});

	test("definitions: a bot token is a secret of its bot, and a flow can't define one", () => {
		const base = config({
			events: [
				{
					event_id: "evt_helper",
					event_version: [1, 0, 0],
					board_version: [1, 0, 0],
				},
			],
			secret_overrides: {
				"event.evt_helper.bot_token": "secret-1",
				"event.evt_gone.bot_token": "secret-2",
			},
		});
		const flow = (id: string) => ({
			id,
			name: id,
			data_type: "String",
			value_type: "Normal",
			secret: true,
		});
		expect(
			serviceDefinitions(base, [
				flow("erp_password"),
				flow("event.evt_helper.bot_token"),
			]),
		).toEqual([
			flow("erp_password"),
			{
				id: "event.evt_helper.bot_token",
				name: "evt_helper",
				data_type: "String",
				value_type: "Normal",
				secret: true,
			},
		]);
	});

	test("what the shared diff doesn't cover still shows as a row", () => {
		const base = config();
		const next = config({
			hosting: { ...base.hosting, max_in_flight: 32, request_timeout_secs: 60 },
			future_setting: { retain_me: false },
		});
		expect(extraDiffRows(base, next).map((row) => row.field)).toEqual([
			"limit_parallel",
			"limit_timeout",
			"other",
		]);
		expect(extraDiffRows(base, config())).toEqual([]);
	});
});

describe("plans", () => {
	const existing = { config_revision: 7 };

	test("a quick update is one command that keeps the requested state", () => {
		const plan = settingsPlan(
			existing,
			config(),
			{ ...DEFAULT_APPLY, mode: "quick" },
			true,
		);
		expect(plan.rollout_id).toBeUndefined();
		expect(plan.expected_revision).toBe(7);
		expect(plan.steps.map((step) => step.command.type)).toEqual(["apply"]);
		expect(plan.steps[0]?.command).toMatchObject({
			expected_revision: 7,
			start: true,
		});
		expect(
			settingsPlan(
				existing,
				config(),
				{ ...DEFAULT_APPLY, mode: "quick" },
				false,
			).steps[0]?.command.start,
		).toBe(false);
	});

	test("a safe update stages with the chosen checks and activates; no secret is part of it", () => {
		const plan = settingsPlan(
			existing,
			config(),
			{ mode: "safe", stabilizeS: 20, deadlineS: 90 },
			true,
		);
		expect(plan.steps.map((step) => step.command.type)).toEqual([
			"stage_rollout",
			"activate_rollout",
		]);
		expect(plan.rollout_id).toBe(plan.steps[0]?.id);
		expect(plan.steps[0]?.command).toMatchObject({
			expected_revision: 7,
			stabilization_seconds: 20,
			deadline_seconds: 90,
		});
		expect(plan.steps[1]?.command.rollout_id).toBe(plan.rollout_id);
	});

	test("a command over the message limit is refused before it is built", () => {
		const big = config({ variables: { greeting: "x".repeat(11_990) } });
		expect(jsonBytes(big)).toBeGreaterThan(CONFIG_MAX_BYTES);
		expect(() => settingsPlan(existing, big, DEFAULT_APPLY, true)).toThrow(
			ConfigTooLargeError,
		);
	});

	test("the timing of a safe update has to fit together", () => {
		expect(checkApplyHow(DEFAULT_APPLY)).toBeNull();
		expect(checkApplyHow({ mode: "safe", stabilizeS: 1, deadlineS: 120 })).toBe(
			"stabilize_range",
		);
		expect(
			checkApplyHow({ mode: "safe", stabilizeS: 10, deadlineS: 700 }),
		).toBe("deadline_range");
		expect(checkApplyHow({ mode: "safe", stabilizeS: 10, deadlineS: 15 })).toBe(
			"deadline_short",
		);
		expect(
			checkApplyHow({ mode: "quick", stabilizeS: 0, deadlineS: 0 }),
		).toBeNull();
	});
});
