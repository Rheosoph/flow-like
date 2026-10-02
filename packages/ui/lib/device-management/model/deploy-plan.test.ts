import { describe, expect, test } from "bun:test";
import {
	type DeploymentPlan,
	type OfflineWritesConfig,
	type PlacementConfiguration,
	createDeploymentPlan,
	mergeVariables,
} from "../deployment";
import {
	APPS,
	CRM_CATALOG,
	CRM_INSTALLED,
	CRM_PLAN_APP,
	HASH,
	NOW0,
	PLAN_DEVICES,
	VARIABLES,
	VISITOR_CATALOG,
	VISITOR_INSTALLED,
	VISITOR_PLAN_APP,
} from "./__fixtures__/apps";
import {
	type ApprovalDraft,
	type ApprovalIssueCode,
	DEFAULT_ISOLATION,
	type DeployDraft,
	type DeploymentPlanInput,
	type PlanApp,
	type PlanEntry,
	type PlanFacts,
	type SpendingDraft,
	approvalRequest,
	checkApprovalDraft,
	checkPlan,
	diffPlacementConfig,
	draftWithoutSecrets,
	makePlan,
	planPhases,
	planServices,
	resolvePlan,
	serviceSlug,
	wirePlan,
} from "./deploy-plan";
import type { GateFailure } from "./types";

const TOKEN = "visitor-checkin-access-token-0123456789";
const INVOICE_PLAN_APP: PlanApp = APPS.app_invoice_ai;

function entry(
	app: PlanApp,
	route: Partial<PlanEntry["route"]> = {},
	extra: Partial<PlanEntry> = {},
): PlanEntry {
	return {
		scope: { kind: "app", appId: app.id },
		route: { deviceIds: [], appId: app.id, ...route },
		app,
		deploymentId: "dep-plan",
		now: NOW0,
		...extra,
	};
}

function facts(
	app: PlanApp | null,
	change: Partial<PlanFacts> = {},
): PlanFacts {
	return {
		app,
		devices: PLAN_DEVICES,
		platform: "desktop",
		now: NOW0,
		isAppOwner: true,
		...change,
	};
}

function draftFor(
	app: PlanApp,
	devices: string[],
	change: Partial<DeployDraft> = {},
	route: Partial<PlanEntry["route"]> = {},
): DeployDraft {
	return {
		...makePlan(entry(app, { deviceIds: devices, ...route })),
		...change,
	};
}

function planFor(
	draft: DeployDraft,
	app: PlanApp | null,
	change: Partial<PlanFacts> = {},
) {
	const plan = resolvePlan(draft, facts(app, change));
	return { plan, check: checkPlan(plan, facts(app, change)) };
}

function codes(
	draft: DeployDraft,
	app: PlanApp | null,
	change: Partial<PlanFacts> = {},
) {
	return planFor(draft, app, change).check.issues.map((issue) =>
		issue.deviceId ? `${issue.code}@${issue.deviceId}` : issue.code,
	);
}

function normalised(plan: DeploymentPlan): unknown {
	return JSON.parse(
		JSON.stringify(plan).replace(
			/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g,
			"<uuid>",
		),
	);
}

describe("entries (APP §3.1)", () => {
	test("app-first new deploy ticks every event that can run and fixes the mode", () => {
		const draft = makePlan(
			entry(VISITOR_PLAN_APP, { deviceIds: ["studio-mac-mini"] }),
		);
		expect(draft).toMatchObject({
			entry: "app",
			ctx: "app",
			appId: "app_visitor_checkin",
			scope: "app",
			events: ["evt_visitor_page", "evt_badge_printer"],
			targets: [
				{ deviceId: "studio-mac-mini", choices: {}, serveBoth: [], over: {} },
			],
			endpoint: { host: "127.0.0.1", port: 8080, token: "per_device" },
			isolation: { profile: "auto" },
			writes: null,
			approval: {
				files: "read_only",
				models: [],
				maxInstances: 1,
				expiresAt: NOW0 + 30 * 86400,
			},
			order: "one",
			stopOnFail: true,
			strategy: "auto",
			deploymentId: "dep-plan",
		});
	});

	test("single-event, device-first and update entries", () => {
		expect(
			makePlan(
				entry(CRM_PLAN_APP, { eventId: "evt_crm_webhook", from: "events" }),
			),
		).toMatchObject({
			entry: "event",
			ctx: "events",
			scope: "event",
			events: ["evt_crm_webhook"],
			approval: { files: "none" },
		});
		expect(
			makePlan(
				entry(
					CRM_PLAN_APP,
					{ deviceIds: ["edge-berlin-01", "edge-berlin-01"] },
					{ scope: { kind: "account" } },
				),
			),
		).toMatchObject({
			entry: "device",
			ctx: null,
			targets: [{ deviceId: "edge-berlin-01" }],
		});
		const update = makePlan(
			entry(
				CRM_PLAN_APP,
				{
					deviceIds: ["edge-berlin-01"],
					serviceId: "nightly-sync",
					mode: "update",
				},
				{ updateEvents: ["evt_crm_nightly"] },
			),
		);
		expect(update).toMatchObject({
			entry: "update",
			events: ["evt_crm_nightly"],
			targets: [
				{
					choices: { main: { kind: "update", serviceId: "nightly-sync" } },
				},
			],
			endpoint: { host: null, port: null, token: "keep" },
		});
		expect(update.isolation).toBeUndefined();
		expect(update.writes).toBeUndefined();
	});

	test("saved progress keeps every choice and no secret", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["studio-mac-mini"], {
			vars: { var_site_name: "Berlin office" },
			secrets: { var_host_token: "host-directory-token" },
			edited: ["var_host_token"],
			endpoint: {
				host: "0.0.0.0",
				port: 8080,
				token: "own",
				tokenValue: TOKEN,
			},
		});
		draft.targets[0].over = {
			vars: { var_printer_url: "ipp://192.168.1.40/print" },
			secrets: { var_host_token: "per-device-secret" },
			token: `${TOKEN}-studio`,
			port: 8082,
			allowUnencrypted: true,
		};
		const saved = draftWithoutSecrets(draft);
		expect(saved).toEqual({
			...draft,
			secrets: {},
			endpoint: { ...draft.endpoint, tokenValue: "" },
			targets: [
				{
					...draft.targets[0],
					over: {
						vars: { var_printer_url: "ipp://192.168.1.40/print" },
						port: 8082,
						allowUnencrypted: true,
					},
				},
			],
		});
		const text = JSON.stringify(saved);
		for (const secret of ["host-directory-token", "per-device-secret", TOKEN])
			expect(text).not.toContain(secret);
		expect(draft.secrets).toEqual({ var_host_token: "host-directory-token" });
	});

	test("default service ids are slugs that a device accepts", () => {
		expect(
			[
				"Visitor Check-in",
				"Invoice tools (MCP)",
				"Ünïcode Ëvent",
				"",
				"device",
				"---",
			].map(serviceSlug),
		).toEqual([
			"visitor-check-in",
			"invoice-tools-mcp",
			"unicode-event",
			"service",
			"service",
			"service",
		]);
	});
});

describe("services (A5: split only where the code forces it)", () => {
	test("one service for the whole app; a background event limits it to 1 instance", () => {
		const draft = draftFor(VISITOR_PLAN_APP, [], { maxInstances: 3 });
		expect(planServices(draft, VISITOR_PLAN_APP)).toEqual([
			{
				key: "main",
				id: "visitor-check-in",
				events: ["evt_visitor_page", "evt_badge_printer"],
				maxInstances: 1,
				hosted: true,
				why: [{ code: "background", eventId: "evt_badge_printer" }],
			},
		]);
	});

	test("one service per event keeps the requested instances where every event is hosted", () => {
		const draft = draftFor(VISITOR_PLAN_APP, [], {
			split: "per_event",
			maxInstances: 3,
		});
		expect(
			planServices(draft, VISITOR_PLAN_APP).map((service) => [
				service.key,
				service.id,
				service.maxInstances,
			]),
		).toEqual([
			["evt_visitor_page", "check-in-page", 3],
			["evt_badge_printer", "badge-printer", 1],
		]);
		const buffered = draftFor(VISITOR_PLAN_APP, [], {
			split: "per_event",
			maxInstances: 3,
			writes: { tables: [], files: [] } as never,
		});
		expect(planServices(buffered, VISITOR_PLAN_APP)[0]).toMatchObject({
			maxInstances: 1,
			why: [{ code: "writes" }],
		});
	});

	test("events that disagree about a shared variable become two services", () => {
		const draft = draftFor(CRM_PLAN_APP, [], {
			serviceIds: { main: "crm-sync" },
		});
		expect(planServices(draft, CRM_PLAN_APP)).toEqual([
			{
				key: "main",
				id: "crm-sync",
				events: ["evt_crm_nightly", "evt_crm_webhook"],
				maxInstances: 1,
				hosted: true,
				why: [{ code: "background", eventId: "evt_crm_nightly" }],
			},
			{
				key: "evt_crm_watch",
				id: "watch-import-folder",
				events: ["evt_crm_watch"],
				maxInstances: 1,
				hosted: false,
				why: [
					{
						code: "split_variables",
						variable: "Export folder",
						events: ["evt_crm_nightly", "evt_crm_watch"],
					},
					{ code: "background", eventId: "evt_crm_watch" },
				],
			},
		]);
	});

	test("events that can't run on devices never reach a service", () => {
		const draft = draftFor(CRM_PLAN_APP, [], {
			events: ["evt_crm_hourly", "evt_crm_rest", "evt_crm_webhook"],
		});
		expect(planServices(draft, CRM_PLAN_APP)).toMatchObject([
			{ events: ["evt_crm_webhook"] },
		]);
		expect(planServices(draft, null)).toEqual([]);
	});
});

describe("targets (APP §3.7)", () => {
	test("a taken id gets -2; refusals and duplicates are left out unless served in both", () => {
		const draft = draftFor(CRM_PLAN_APP, ["edge-berlin-01"], {
			serviceIds: { main: "nightly-sync" },
		});
		const { plan, check } = planFor(draft, CRM_PLAN_APP);
		expect(plan.targets[0].services).toEqual([
			{
				key: "main",
				serviceId: "nightly-sync-2",
				kind: "new",
				events: ["evt_crm_webhook"],
				leftOut: [
					{
						eventId: "evt_crm_nightly",
						why: "duplicate",
						serviceId: "nightly-sync",
					},
				],
				removedEvents: [],
				renamedFrom: "nightly-sync",
			},
			{
				key: "evt_crm_watch",
				serviceId: "watch-import-folder",
				kind: "new",
				events: [],
				leftOut: [
					{
						eventId: "evt_crm_watch",
						why: "refuse",
						detail: expect.stringContaining("requires sandboxed services"),
					},
				],
				removedEvents: [],
			},
		]);
		expect(check.exceptions.map((row) => [row.code, row.tone])).toEqual([
			["left_out_duplicate", "paused"],
			["renamed", "info"],
			["left_out_refuse", "paused"],
		]);
		draft.targets[0].serveBoth = ["evt_crm_nightly"];
		expect(
			resolvePlan(draft, facts(CRM_PLAN_APP)).targets[0].services[0].events,
		).toEqual(["evt_crm_nightly", "evt_crm_webhook"]);
	});

	test("two services of one plan never share an id on a device", () => {
		const draft = draftFor(CRM_PLAN_APP, ["edge-berlin-01"], {
			serviceIds: { main: "nightly-sync", evt_crm_watch: "nightly-sync-2" },
		});
		const { plan, check } = planFor(draft, CRM_PLAN_APP);
		expect(
			plan.targets[0].services.map((service) => [
				service.serviceId,
				service.renamedFrom,
			]),
		).toEqual([
			["nightly-sync-2", "nightly-sync"],
			["nightly-sync-2-2", "nightly-sync-2"],
		]);
		expect(check.issues).toEqual([]);
		draft.serviceIds = {};
		draft.targets[0].choices = {
			main: { kind: "update", serviceId: "nightly-sync" },
			evt_crm_watch: { kind: "add", serviceId: "nightly-sync" },
		};
		expect(
			codes(draft, CRM_PLAN_APP).filter((code) =>
				code.startsWith("service_id"),
			),
		).toEqual([
			"service_id_twice@edge-berlin-01",
			"service_id_twice@edge-berlin-01",
		]);
	});

	test("Add to an existing service keeps its events; an update can drop one", () => {
		const draft = draftFor(
			CRM_PLAN_APP,
			["edge-berlin-01"],
			{},
			{ eventId: "evt_crm_webhook" },
		);
		draft.targets[0].choices = {
			main: { kind: "add", serviceId: "nightly-sync" },
		};
		expect(
			resolvePlan(draft, facts(CRM_PLAN_APP)).targets[0].services[0],
		).toMatchObject({
			kind: "add",
			serviceId: "nightly-sync",
			events: ["evt_crm_nightly", "evt_crm_webhook"],
			removedEvents: [],
		});
		const update = draftFor(
			CRM_PLAN_APP,
			["edge-berlin-01"],
			{ events: ["evt_crm_webhook"] },
			{ serviceId: "nightly-sync", mode: "update" },
		);
		const resolved = planFor(update, CRM_PLAN_APP);
		expect(resolved.plan.targets[0].services[0]).toMatchObject({
			kind: "update",
			events: ["evt_crm_webhook"],
			removedEvents: ["evt_crm_nightly"],
		});
		expect(resolved.check.firstBlocking).toMatchObject({
			code: "removed_events",
			step: "what",
			params: { count: 1 },
		});
		expect(
			planFor(
				{ ...update, acceptRemovedEvents: true },
				CRM_PLAN_APP,
			).check.issues.map((issue) => issue.code),
		).not.toContain("removed_events");
	});

	test("a version-only update of several services keeps each service's own events", () => {
		const devices = {
			...PLAN_DEVICES,
			"lab-gpu-02": {
				...PLAN_DEVICES["lab-gpu-02"],
				locked: false,
				services: [
					{
						serviceId: "invoice-extractor-gpu",
						projectId: "app_invoice_ai",
						events: ["evt_gpu_extract", "evt_invoice_reconcile"],
					},
				],
			},
		};
		const draft = draftFor(
			INVOICE_PLAN_APP,
			["edge-berlin-01", "lab-gpu-02"],
			{},
			{ mode: "update" },
		);
		expect(draft).toMatchObject({ entry: "update", keepEvents: true });
		draft.targets[0].choices = {
			main: { kind: "update", serviceId: "invoice-extractor" },
		};
		draft.targets[1].choices = {
			main: { kind: "update", serviceId: "invoice-extractor-gpu" },
		};
		const { plan, check } = planFor(draft, INVOICE_PLAN_APP, { devices });
		expect(plan.services.map((service) => service.key)).toEqual(["main"]);
		expect(
			plan.targets.map((target) => [
				target.deviceId,
				target.services[0].serviceId,
				target.services[0].events,
				target.services[0].removedEvents,
			]),
		).toEqual([
			["edge-berlin-01", "invoice-extractor", ["evt_extract_http"], []],
			[
				"lab-gpu-02",
				"invoice-extractor-gpu",
				["evt_gpu_extract"],
				["evt_invoice_reconcile"],
			],
		]);
		// The schedule can't run on devices any more: dropping it needs the user's yes.
		expect(
			check.issues.map((issue) => `${issue.code}@${issue.deviceId}`),
		).toEqual(["removed_events@lab-gpu-02"]);
		expect(
			planFor({ ...draft, acceptRemovedEvents: true }, INVOICE_PLAN_APP, {
				devices,
			}).check.ok,
		).toBe(true);
		expect(
			makePlan(
				entry(INVOICE_PLAN_APP, {
					deviceIds: ["edge-berlin-01"],
					mode: "update",
					serviceId: "invoice-extractor",
				}),
			).keepEvents,
		).toBeUndefined();
	});

	test("a port in use moves to the next free one; the device's own override stays", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
			endpoint: {
				host: "127.0.0.1",
				port: 8081,
				token: "per_device",
				tokenValue: "",
			},
		});
		const { plan, check } = planFor(draft, VISITOR_PLAN_APP);
		expect(plan.targets[0].endpoint).toEqual({
			host: "127.0.0.1",
			port: 8082,
			portMovedFrom: 8081,
			certificateId: null,
		});
		expect(check.exceptions).toContainEqual({
			code: "port_moved",
			step: "endpoint",
			deviceId: "edge-berlin-01",
			tone: "info",
			params: { from: 8081, to: 8082 },
		});
		draft.targets[0].over.port = 8081;
		expect(
			resolvePlan(draft, facts(VISITOR_PLAN_APP)).targets[0].endpoint.port,
		).toBe(8081);
	});

	test("two served services of one plan never share a port on a device (A5)", () => {
		const draft = draftFor(INVOICE_PLAN_APP, ["studio-mac-mini"], {
			split: "per_event",
			endpoint: {
				host: "127.0.0.1",
				port: 8089,
				token: "per_device",
				tokenValue: "",
			},
		});
		draft.approval.ownerConsent = true;
		draft.targets[0].over.trustAgent = true;
		const { plan, check } = planFor(draft, INVOICE_PLAN_APP);
		expect(
			plan.targets[0].services.map((service) => [
				service.serviceId,
				service.port,
			]),
		).toEqual([
			["extract-invoice", undefined],
			["extract-invoice-gpu", 8091],
			["invoice-tools-mcp", undefined],
		]);
		expect(plan.targets[0].endpoint.port).toBe(8089);
		expect(check.issues).toEqual([]);
		expect(check.exceptions).toContainEqual({
			code: "port_moved",
			step: "endpoint",
			deviceId: "studio-mac-mini",
			tone: "info",
			params: { from: 8089, to: 8091, service: "extract-invoice-gpu" },
		});
		const wired = (serviceKey: string) =>
			wirePlan(
				plan,
				{ deviceId: "studio-mac-mini", serviceKey },
				{
					installed: VISITOR_INSTALLED,
					events: [],
					variables: {},
					serviceToken: TOKEN,
					canManageCertificates: false,
				},
			).port;
		expect([wired("evt_extract_http"), wired("evt_gpu_extract")]).toEqual([
			8089, 8091,
		]);
	});

	test("a port typed for a device that another service uses there is refused", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"]);
		draft.approval.ownerConsent = true;
		draft.targets[0].over.port = 8081;
		expect(planFor(draft, VISITOR_PLAN_APP).check.issues).toEqual([
			{
				code: "port_in_use",
				step: "endpoint",
				severity: "error",
				deviceId: "edge-berlin-01",
				params: { port: 8081, service: "invoice-extractor" },
			},
		]);
		draft.targets[0].over.port = 80;
		expect(planFor(draft, VISITOR_PLAN_APP).check.issues[0].params).toEqual({
			port: 80,
		});
	});

	test("isolation follows each device's policy and is clamped to its memory", () => {
		const draft = draftFor(VISITOR_PLAN_APP, [
			"edge-berlin-01",
			"studio-mac-mini",
			"lab-gpu-02",
		]);
		const devices = {
			...PLAN_DEVICES,
			"lab-gpu-02": {
				...PLAN_DEVICES["lab-gpu-02"],
				isolation: "optional" as const,
				memoryBytes: 512 * 1024 ** 2,
			},
		};
		const plan = resolvePlan(draft, facts(VISITOR_PLAN_APP, { devices }));
		expect(
			plan.targets.map((target) => [
				target.deviceId,
				target.runsAsAgent,
				target.resources?.memory_bytes ?? null,
			]),
		).toEqual([
			["edge-berlin-01", false, 1024 ** 3],
			["studio-mac-mini", true, null],
			["lab-gpu-02", false, 512 * 1024 ** 2],
		]);
	});
});

function adjust(draft: DeployDraft, change: (draft: DeployDraft) => void) {
	change(draft);
	return draft;
}

const OFFLINE_GATE: GateFailure = {
	ok: false,
	gate: "G6",
	kind: "live",
	hide: false,
	copy: { code: "device_offline" as never },
};

const GATED_DEVICES = {
	...PLAN_DEVICES,
	"studio-mac-mini": { ...PLAN_DEVICES["studio-mac-mini"], gate: OFFLINE_GATE },
};

const CHECK_CASES: [
	string,
	() => DeployDraft,
	PlanApp | null,
	Partial<PlanFacts>,
	string[],
][] = [
	[
		"no targets",
		() => draftFor(CRM_PLAN_APP, []),
		CRM_PLAN_APP,
		{},
		["no_targets"],
	],
	[
		"a local-only app on the web",
		() => draftFor(CRM_PLAN_APP, ["edge-berlin-01"]),
		CRM_PLAN_APP,
		{ platform: "web" },
		["local_only_web"],
	],
	[
		"an id the device used before",
		() =>
			draftFor(CRM_PLAN_APP, ["edge-berlin-01"], {
				serviceIds: { main: "crm-sync-old" },
			}),
		CRM_PLAN_APP,
		{},
		["service_id_reserved@edge-berlin-01"],
	],
	[
		"gated, locked and empty targets",
		() =>
			draftFor(CRM_PLAN_APP, ["studio-mac-mini", "lab-gpu-02"], { events: [] }),
		CRM_PLAN_APP,
		{ devices: GATED_DEVICES },
		[
			"no_events",
			"target_gated@studio-mac-mini",
			"target_no_events@studio-mac-mini",
			"target_locked@lab-gpu-02",
			"target_no_events@lab-gpu-02",
			"agent_trust@studio-mac-mini",
		],
	],
	[
		"an update entry that names no service to update never creates one",
		() =>
			draftFor(
				CRM_PLAN_APP,
				["edge-berlin-01"],
				{ events: ["evt_crm_webhook"] },
				{ mode: "update" },
			),
		CRM_PLAN_APP,
		{},
		["update_needs_service@edge-berlin-01", "token_invalid@edge-berlin-01"],
	],
	[
		"ids a device refuses",
		() =>
			draftFor(VISITOR_PLAN_APP, [], {
				split: "per_event",
				serviceIds: { evt_visitor_page: "device", evt_badge_printer: "device" },
			}),
		VISITOR_PLAN_APP,
		{},
		["service_id_invalid", "service_id_invalid", "no_targets"],
	],
	[
		"one id for two services",
		() =>
			draftFor(VISITOR_PLAN_APP, [], {
				split: "per_event",
				serviceIds: { evt_visitor_page: "same", evt_badge_printer: "same" },
			}),
		VISITOR_PLAN_APP,
		{},
		["service_id_twice", "service_id_twice", "no_targets"],
	],
	[
		"no app",
		() => draftFor(VISITOR_PLAN_APP, []),
		null,
		{},
		["app_missing", "no_events", "no_targets"],
	],
	[
		"address, port, token and instances",
		() =>
			draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
				endpoint: {
					host: "edge.local",
					port: 0,
					token: "own",
					tokenValue: "short",
				},
				maxInstances: 40,
			}),
		VISITOR_PLAN_APP,
		{},
		[
			"host_invalid@edge-berlin-01",
			"port_invalid@edge-berlin-01",
			"token_invalid@edge-berlin-01",
			"service_instances_range",
			"approval.files_need_consent",
		],
	],
	[
		"a new service can't keep a token; a sandbox-only device refuses the agent",
		() =>
			adjust(
				draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
					endpoint: {
						host: "127.0.0.1",
						port: 8080,
						token: "keep",
						tokenValue: "",
					},
					isolation: { ...DEFAULT_ISOLATION, profile: "trusted_process" },
				}),
				(draft) => {
					draft.approval.ownerConsent = true;
					draft.targets[0].over.trustAgent = true;
				},
			),
		VISITOR_PLAN_APP,
		{},
		["token_invalid@edge-berlin-01", "isolation_required@edge-berlin-01"],
	],
];

describe("checks (APP §3.5–§3.12)", () => {
	test("the defaults on a Mac need the trust check and the owner's file consent", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["studio-mac-mini"]);
		const { check } = planFor(draft, VISITOR_PLAN_APP);
		expect(check.issues.map((issue) => [issue.code, issue.step])).toEqual([
			["agent_trust", "endpoint"],
			["approval.files_need_consent", "access_cost"],
		]);
		expect(check.firstBlocking?.code).toBe("agent_trust");
		expect(check.ok).toBe(false);
		draft.targets[0].over.trustAgent = true;
		draft.approval.ownerConsent = true;
		expect(planFor(draft, VISITOR_PLAN_APP).check).toMatchObject({
			ok: true,
			issues: [],
			firstBlocking: null,
		});
	});

	for (const [name, draft, app, change, expected] of CHECK_CASES)
		test(name, () => {
			expect(codes(draft(), app, change)).toEqual(expected);
		});

	test("settings are checked per device with the device's own values", () => {
		const draft = draftFor(
			CRM_PLAN_APP,
			["edge-berlin-01", "studio-mac-mini"],
			{
				vars: { var_batch_size: "500", var_export_folder: "/data/export" },
				isolation: {
					...DEFAULT_ISOLATION,
					profile: "linux_sandbox",
				},
			},
		);
		draft.targets[1].over.vars = { var_batch_size: "many" };
		expect(codes(draft, CRM_PLAN_APP)).toEqual([
			"variable_invalid@studio-mac-mini",
			"isolation_unavailable@studio-mac-mini",
		]);
		const secret = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
			secrets: { var_host_token: "x".repeat(5000) },
		});
		expect(planFor(secret, VISITOR_PLAN_APP).check.issues[0]).toMatchObject({
			code: "secret_size",
			step: "settings",
			deviceId: "edge-berlin-01",
			params: { variable: "Host directory token" },
		});
		const empty = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
			secrets: { var_host_token: "" },
		});
		expect(codes(empty, VISITOR_PLAN_APP)).not.toContain(
			"variable_invalid@edge-berlin-01",
		);
	});

	test("an exposed address needs a certificate or an acknowledgement", () => {
		const exposed = adjust(
			draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
				endpoint: {
					host: "0.0.0.0",
					port: 8080,
					token: "same",
					tokenValue: TOKEN,
				},
			}),
			(draft) => {
				draft.approval.ownerConsent = true;
			},
		);
		const first = planFor(exposed, VISITOR_PLAN_APP).check;
		expect(first.issues.map((issue) => issue.code)).toEqual(["unencrypted"]);
		expect(first.exceptions.map((row) => row.code)).toEqual(["no_certificate"]);
		exposed.targets[0].over.allowUnencrypted = true;
		expect(planFor(exposed, VISITOR_PLAN_APP).check.ok).toBe(true);
		exposed.targets[0].over.certificateId =
			"00000000-0000-4000-8000-000000000001";
		expect(planFor(exposed, VISITOR_PLAN_APP).check.exceptions).toEqual([]);
	});

	test("write buffering is online-only and validated", () => {
		const writes: OfflineWritesConfig = {
			tables: [
				{
					purpose: "storage",
					database: "db",
					table: "visits",
					primary_key: "id",
				},
			],
			files: [],
			max_queue_bytes: 256 * 1048576,
			max_operations: 10000,
			max_age_seconds: 7 * 86400,
			max_mirror_bytes: 2 * 1024 ** 3,
		};
		const offline = draftFor(CRM_PLAN_APP, ["edge-berlin-01"], { writes });
		expect(codes(offline, CRM_PLAN_APP)).toEqual(["writes_offline"]);
		const online = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"], {
			writes: { ...writes, tables: [] },
		});
		online.approval.ownerConsent = true;
		expect(codes(online, VISITOR_PLAN_APP)).toEqual(["writes_invalid"]);
	});

	test("model access for a local-only app needs no app; updates keep their approval", () => {
		const draft = draftFor(CRM_PLAN_APP, ["edge-berlin-01"]);
		draft.approval = {
			...draft.approval,
			files: "read_only",
			models: ["mistral-small-3"],
		};
		expect(planFor(draft, CRM_PLAN_APP).check.issues).toEqual([
			{
				code: "approval.files_need_app",
				step: "copy_upload",
				severity: "error",
				params: undefined,
			},
			{
				code: "approval.files_need_consent",
				step: "copy_upload",
				severity: "error",
				params: undefined,
			},
			{
				code: "approval.spending_required",
				step: "copy_upload",
				severity: "error",
				params: undefined,
			},
		]);
		draft.approval.files = "none";
		draft.spending = {
			limitMicros: 10_000_000,
			expiresAt: draft.approval.expiresAt,
			consent: true,
		};
		expect(codes(draft, CRM_PLAN_APP)).toEqual([]);
		const update = draftFor(
			VISITOR_PLAN_APP,
			["edge-berlin-01"],
			{},
			{ serviceId: "visitor-check-in", mode: "update" },
		);
		expect(codes(update, VISITOR_PLAN_APP)).toEqual([]);
	});
});

const APPROVAL: ApprovalDraft = {
	files: "read_write",
	ownerConsent: true,
	models: ["mistral-small-3"],
	maxInstances: 2,
	expiresAt: NOW0 + 30 * 86400,
};
const SPENDING: SpendingDraft = {
	limitMicros: 10_000_000,
	expiresAt: NOW0 + 30 * 86400,
	consent: true,
};
const APPROVAL_CONTEXT: Parameters<typeof checkApprovalDraft>[1] = {
	appId: "app_visitor_checkin",
	now: NOW0,
	serviceMaxInstances: 2,
	isAppOwner: true,
	spending: SPENDING,
};
const APPROVAL_CASES: [
	string,
	Partial<ApprovalDraft>,
	Partial<typeof APPROVAL_CONTEXT>,
	ApprovalIssueCode[],
][] = [
	["valid", {}, {}, []],
	["duplicate models", { models: ["a", "a"] }, {}, ["models_invalid"]],
	["wildcard model", { models: ["gpt-*"] }, {}, ["models_invalid"]],
	[
		"too many models",
		{ models: Array.from({ length: 65 }, (_, i) => `m${i}`) },
		{},
		["models_invalid"],
	],
	[
		"nothing",
		{ models: [], files: "none" },
		{ spending: undefined },
		["nothing_approved"],
	],
	["files without app", {}, { appId: null }, ["files_need_app"]],
	["files without owner", {}, { isAppOwner: false }, ["files_need_owner"]],
	[
		"files without consent",
		{ ownerConsent: false },
		{},
		["files_need_consent"],
	],
	["instances 0", { maxInstances: 0 }, {}, ["instances_range"]],
	["instances 101", { maxInstances: 101 }, {}, ["instances_range"]],
	[
		"instances below service",
		{ maxInstances: 1 },
		{},
		["instances_below_service"],
	],
	["expired", { expiresAt: NOW0 }, { spending: undefined }, ["expiry_past"]],
	["beyond a year", { expiresAt: NOW0 + 366 * 86400 }, {}, ["expiry_too_far"]],
	["spending without models", { models: [] }, {}, ["spending_without_models"]],
	[
		"models without a spending limit",
		{},
		{ spending: null },
		["spending_required"],
	],
	["files only need no spending limit", { models: [] }, { spending: null }, []],
	[
		"spending 0",
		{},
		{ spending: { ...SPENDING, limitMicros: 0 } },
		["spending_range"],
	],
	[
		"spending above the cap",
		{},
		{ spending: { ...SPENDING, limitMicros: 1e12 + 1 } },
		["spending_range"],
	],
	[
		"spending after the approval",
		{},
		{ spending: { ...SPENDING, expiresAt: NOW0 + 31 * 86400 } },
		["spending_expiry"],
	],
	[
		"spending without consent",
		{},
		{ spending: { ...SPENDING, consent: false } },
		["spending_consent"],
	],
];
describe("approval drafts (APP §3.10)", () => {
	for (const [name, change, extra, expected] of APPROVAL_CASES)
		test(name, () => {
			expect(
				checkApprovalDraft(
					{ ...APPROVAL, ...change },
					{ ...APPROVAL_CONTEXT, ...extra },
				).map((issue) => issue.code),
			).toEqual(expected);
		});

	test("the hub request for one service", () => {
		const identity = {
			placementId: "visitor-check-in",
			deploymentId: "dep-plan",
			projectId: "app_visitor_checkin",
			appId: "app_visitor_checkin",
		};
		expect(approvalRequest(APPROVAL, identity)).toEqual({
			placement_id: "visitor-check-in",
			deployment_id: "dep-plan",
			project_id: "app_visitor_checkin",
			app_id: "app_visitor_checkin",
			online_access: "read_write",
			model_ids: ["mistral-small-3"],
			max_instances: 2,
			expires_at: NOW0 + 30 * 86400,
		});
		expect(
			approvalRequest(
				{ ...APPROVAL, files: "none" },
				{ ...identity, appId: null },
			),
		).not.toHaveProperty("online_access");
	});
});

const GRANT = { grant_id: "grant-visitor", authz_version: 1 };

const VISITOR_LEGACY: DeploymentPlanInput = {
	installed: VISITOR_INSTALLED,
	existing: undefined,
	healthChecked: false,
	removeOverrides: [],
	placement: "visitor-check-in",
	deployment: "dep-plan",
	events: VISITOR_CATALOG.events,
	variables: mergeVariables([
		VISITOR_CATALOG.variables.evt_badge_printer,
		VISITOR_CATALOG.variables.evt_visitor_page,
	]),
	previousVariables: [],
	overrides: {
		var_site_name: "Berlin office",
		var_printer_url: "ipp://10.0.4.20/print",
		var_host_token: "host-directory-token",
	},
	host: "0.0.0.0",
	port: 8080,
	replicas: 1,
	serviceToken: TOKEN,
	tlsCertificateId: null,
	offlineWrites: null,
	resourceLimits: null,
	resourceGrant: GRANT,
};

const NIGHTLY_EXISTING: PlacementConfiguration = {
	placement_id: "nightly-sync",
	project_id: "app_crm_sync",
	deployment_id: "dep-nightly",
	config_revision: 3,
	desired_state: "stopped",
	config: {
		id: "nightly-sync",
		project_id: "app_crm_sync",
		deployment_id: "dep-nightly",
		revision: HASH.crm30,
		source: "offline",
		project_path: `/private/projects/app_crm_sync/revisions/${HASH.crm30}`,
		events: [
			{
				event_id: "evt_crm_nightly",
				event_version: [1, 1, 0],
				board_version: [4, 0, 0],
			},
		],
		max_replicas: 1,
		variables: { var_batch_size: 500, var_export_folder: "/data/export" },
		secret_overrides: {},
		resources: {
			profile: "linux_sandbox",
			cpu_millis: 1000,
			memory_bytes: 1024 ** 3,
			max_processes: 256,
			disk_bytes: 4 * 1024 ** 3,
		},
	},
};

const NIGHTLY_LEGACY: DeploymentPlanInput = {
	installed: CRM_INSTALLED,
	existing: NIGHTLY_EXISTING,
	healthChecked: false,
	removeOverrides: [],
	placement: "nightly-sync",
	deployment: "dep-nightly",
	events: [CRM_CATALOG.events[0]],
	variables: [VARIABLES.batch, VARIABLES.exportPath],
	previousVariables: [],
	overrides: { var_batch_size: "800" },
	host: "127.0.0.1",
	port: 8080,
	replicas: 1,
	serviceToken: "",
	tlsCertificateId: null,
	offlineWrites: null,
	resourceLimits: NIGHTLY_EXISTING.config.resources,
	resourceGrant: undefined,
};

describe("wirePlan oracle: one target equals today's createDeploymentPlan", () => {
	test("a new online service with a secret, an exposed address and a typed token", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["studio-mac-mini"], {
			vars: {
				var_site_name: "Berlin office",
				var_printer_url: "ipp://10.0.4.20/print",
				var_unrelated: "ignored",
			},
			secrets: { var_host_token: "host-directory-token" },
			endpoint: {
				host: "0.0.0.0",
				port: 8080,
				token: "own",
				tokenValue: TOKEN,
			},
		});
		const plan = resolvePlan(draft, facts(VISITOR_PLAN_APP));
		const wired = wirePlan(
			plan,
			{ deviceId: "studio-mac-mini", serviceKey: "main" },
			{
				installed: VISITOR_INSTALLED,
				events: VISITOR_CATALOG.events,
				variables: VISITOR_CATALOG.variables,
				serviceToken: TOKEN,
				resourceGrant: GRANT,
				canManageCertificates: true,
			},
		);
		expect(wired).toEqual(VISITOR_LEGACY);
		expect(normalised(createDeploymentPlan(wired))).toEqual(
			normalised(createDeploymentPlan(VISITOR_LEGACY)),
		);
	});

	test("an update of a local-only service sends only edited values and keeps the rest", () => {
		const draft = draftFor(
			CRM_PLAN_APP,
			["edge-berlin-01"],
			{
				vars: { var_batch_size: "800", var_export_folder: "/data/export" },
				edited: ["var_batch_size"],
			},
			{ serviceId: "nightly-sync", mode: "update" },
		);
		draft.events = ["evt_crm_nightly"];
		const plan = resolvePlan(draft, facts(CRM_PLAN_APP));
		const wired = wirePlan(
			plan,
			{ deviceId: "edge-berlin-01", serviceKey: "main" },
			{
				installed: CRM_INSTALLED,
				existing: NIGHTLY_EXISTING,
				events: CRM_CATALOG.events,
				variables: CRM_CATALOG.variables,
				serviceToken: "",
				canManageCertificates: true,
			},
		);
		expect(wired).toMatchObject({
			placement: "nightly-sync",
			deployment: "dep-nightly",
			overrides: { var_batch_size: "800" },
			replicas: 1,
			healthChecked: false,
		});
		const fromPlan = createDeploymentPlan(wired);
		expect(normalised(fromPlan)).toEqual(
			normalised(createDeploymentPlan(NIGHTLY_LEGACY)),
		);
		expect(fromPlan.config).toMatchObject({
			variables: { var_batch_size: 800, var_export_folder: "/data/export" },
			events: [
				{
					event_id: "evt_crm_nightly",
					event_version: [1, 2, 0],
					board_version: [4, 1, 0],
				},
			],
		});
		expect(
			diffPlacementConfig(NIGHTLY_EXISTING.config, fromPlan.config),
		).toEqual([
			{
				kind: "changed",
				field: "app_version",
				before: HASH.crm30,
				after: HASH.crm31,
			},
			{
				kind: "changed",
				field: "event",
				key: "evt_crm_nightly",
				before: { event_version: [1, 1, 0], board_version: [4, 0, 0] },
				after: { event_version: [1, 2, 0], board_version: [4, 1, 0] },
			},
			{
				kind: "changed",
				field: "variable",
				key: "var_batch_size",
				before: 500,
				after: 800,
			},
		]);
	});

	test("a quick strategy never stages; an unknown service is a programming error", () => {
		const draft = draftFor(VISITOR_PLAN_APP, ["studio-mac-mini"], {
			strategy: "quick",
		});
		const plan = resolvePlan(draft, facts(VISITOR_PLAN_APP));
		expect(() =>
			wirePlan(
				plan,
				{ deviceId: "edge-berlin-01", serviceKey: "main" },
				{
					installed: VISITOR_INSTALLED,
					events: VISITOR_CATALOG.events,
					variables: VISITOR_CATALOG.variables,
					serviceToken: TOKEN,
					canManageCertificates: false,
				},
			),
		).toThrow("no service main on device edge-berlin-01");
		expect(
			wirePlan(
				plan,
				{ deviceId: "studio-mac-mini", serviceKey: "main" },
				{
					installed: VISITOR_INSTALLED,
					events: VISITOR_CATALOG.events,
					variables: VISITOR_CATALOG.variables,
					serviceToken: TOKEN,
					canManageCertificates: false,
				},
			),
		).toMatchObject({ healthChecked: false, tlsCertificateId: undefined });
	});
});

describe("phases (APP §3.13)", () => {
	function phases(
		draft: DeployDraft,
		app: PlanApp,
		options = { secrets: true, safe: true },
	) {
		const plan = resolvePlan(draft, facts(app));
		return planPhases(plan, plan.targets[0].services[0], options);
	}

	test("new services, online and offline", () => {
		const online = draftFor(VISITOR_PLAN_APP, ["edge-berlin-01"]);
		online.spending = { limitMicros: 1, expiresAt: NOW0 + 1, consent: true };
		expect(phases(online, VISITOR_PLAN_APP)).toEqual([
			"approve",
			"spending",
			"upload",
			"install",
			"create",
			"secrets",
			"start",
		]);
		const offline = draftFor(CRM_PLAN_APP, ["studio-mac-mini"], {
			start: false,
		});
		expect(
			phases(offline, CRM_PLAN_APP, { secrets: false, safe: false }),
		).toEqual(["upload", "check_events", "install", "create"]);
	});

	test("an offline copy with hosted models gets its model access and spending limit first", () => {
		const offline = draftFor(CRM_PLAN_APP, ["studio-mac-mini"]);
		offline.approval = { ...offline.approval, models: ["mistral-small-3"] };
		offline.spending = {
			limitMicros: 10_000_000,
			expiresAt: offline.approval.expiresAt,
			consent: true,
		};
		expect(
			phases(offline, CRM_PLAN_APP, { secrets: false, safe: false }),
		).toEqual([
			"approve",
			"spending",
			"upload",
			"check_events",
			"install",
			"create",
			"start",
		]);
	});

	test("updates: safe, quick and settings only", () => {
		const update = draftFor(
			CRM_PLAN_APP,
			["edge-berlin-01"],
			{ events: ["evt_crm_nightly"] },
			{ serviceId: "nightly-sync", mode: "update" },
		);
		// The device's order: stage the update, write its new secrets, then check and switch.
		expect(phases(update, CRM_PLAN_APP)).toEqual([
			"upload",
			"install",
			"prepare_update",
			"secrets",
			"check_new",
			"switch",
		]);
		expect(
			phases(update, CRM_PLAN_APP, { secrets: false, safe: false }),
		).toEqual(["upload", "install", "stop", "start"]);
		expect(
			phases({ ...update, version: "keep" }, CRM_PLAN_APP, {
				secrets: false,
				safe: true,
			}),
		).toEqual(["prepare_update", "check_new", "switch"]);
	});
});

const DIFF_BEFORE = {
	revision: "r1",
	variables: { greeting: "hello", moved: "plain" },
	secret_overrides: { credential: "variable-old-reference" },
	hosting: { host: "127.0.0.1", port: 8080, auth_secret: "service-access" },
	max_replicas: 1,
};
const DIFF_AFTER = {
	revision: "r1",
	variables: { greeting: "welcome" },
	secret_overrides: {
		credential: "variable-new-reference",
		moved: "variable-moved",
	},
	hosting: {
		host: "0.0.0.0",
		port: 8080,
		auth_secret: "service-access-rotated",
	},
	tls_certificate_id: "00000000-0000-4000-8000-000000000001",
	max_replicas: 1,
	resource_grant: {
		grant_id: "g2",
		authz_version: 1,
		billing_grant_id: "b1",
	},
	bit_pins: [],
	package_pins: [],
};

describe("config diff", () => {
	test("secret values and references never leave the diff", () => {
		const rows = diffPlacementConfig(DIFF_BEFORE, DIFF_AFTER);
		expect(rows).toEqual([
			{
				kind: "changed",
				field: "variable",
				key: "greeting",
				before: "hello",
				after: "welcome",
			},
			{ kind: "removed", field: "variable", key: "moved", before: "plain" },
			{ kind: "changed", field: "secret", key: "credential" },
			{ kind: "added", field: "secret", key: "moved" },
			{
				kind: "changed",
				field: "endpoint_host",
				before: "127.0.0.1",
				after: "0.0.0.0",
			},
			{ kind: "changed", field: "token" },
			{
				kind: "added",
				field: "certificate",
				after: "00000000-0000-4000-8000-000000000001",
			},
			{ kind: "added", field: "cloud_access", after: "g2" },
			{ kind: "added", field: "spending", after: "b1" },
		]);
		const text = JSON.stringify(rows);
		for (const secret of [
			"variable-old-reference",
			"variable-new-reference",
			"variable-moved",
			"service-access",
		])
			expect(text).not.toContain(secret);
		expect(diffPlacementConfig(DIFF_BEFORE, DIFF_BEFORE)).toEqual([]);
	});

	test("a new service lists everything as added", () => {
		expect(
			diffPlacementConfig(null, {
				revision: "r1",
				events: [
					{ event_id: "e", event_version: [1, 0, 0], board_version: [1, 0, 0] },
				],
				max_replicas: 2,
				package_pins: [{ name: "csv-parse" }],
			}).map((row) => [row.kind, row.field, row.key]),
		).toEqual([
			["added", "app_version", undefined],
			["added", "event", "e"],
			["added", "instances", undefined],
			["added", "packages", undefined],
		]);
	});
});
