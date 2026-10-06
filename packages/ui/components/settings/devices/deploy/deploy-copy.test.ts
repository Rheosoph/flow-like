import { describe, expect, test } from "bun:test";
import type {
	DeployPlan,
	PlanExceptionCode,
	PlanIssue,
	PlanIssueCode,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	APPROVAL_ISSUE_CODES,
	PLAN_ACKNOWLEDGEMENTS,
	PLAN_ISSUE_CODES,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import { deployExitHref, devicesHref } from "../routing/devices-href";
import {
	type IssueNames,
	currentStep,
	deploySteps,
	exceptionText,
	exitRoute,
	issueText,
	prefilledSteps,
	prepareFailureText,
	runRefusalText,
	stepLabel,
	stepTitle,
} from "./deploy-copy";

/** English defaults with their `{{param}}` slots filled, as i18next renders them before extraction. */
const t = ((
	_key: string,
	fallback: string,
	params: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)(?:, \w+)?\}\}/g, (_, name) =>
		String(params[name] ?? ""),
	)) as unknown as DevicesT;

const APP: DevicesScope = { kind: "app", appId: "app_visitor_checkin" };
const ACCOUNT: DevicesScope = { kind: "account" };

describe("steps", () => {
	test("step 6 follows the app's mode", () => {
		expect(deploySteps("online")[5]).toBe("access_cost");
		expect(deploySteps("offline")[5]).toBe("copy_upload");
		expect(deploySteps(null)).toHaveLength(8);
	});

	test("labels: step 5 reads Limits when nothing is served by the device", () => {
		expect(stepLabel(t, "endpoint")).toBe("Endpoint & limits");
		expect(stepLabel(t, "endpoint", true)).toBe("Limits");
		expect(stepTitle(t, "what")).toBe("What to run");
		expect(stepTitle(t, "where")).toBe("Where it runs");
		expect(stepTitle(t, "copy_upload")).toBe("Copy & upload");
	});

	test("a route opens its own step; the other mode's step 6 is corrected", () => {
		const fresh = { resumed: false, reached: 0 };
		const online = deploySteps("online");
		expect(currentStep({ step: "settings" }, online, fresh)).toBe("settings");
		expect(currentStep({ step: "copy_upload" }, online, fresh)).toBe(
			"access_cost",
		);
		expect(
			currentStep({ step: "access_cost" }, deploySteps("offline"), fresh),
		).toBe("copy_upload");
		expect(currentStep({}, online, fresh)).toBe("what");
	});

	test("a resumed draft lands on its furthest step, Review at most", () => {
		const online = deploySteps("online");
		expect(currentStep({}, online, { resumed: true, reached: 4 })).toBe(
			"endpoint",
		);
		expect(currentStep({}, online, { resumed: true, reached: 7 })).toBe(
			"review",
		);
	});

	test("prefilled steps come from the entry", () => {
		expect([...prefilledSteps({ deviceIds: [] }, APP)]).toEqual(["what"]);
		expect([...prefilledSteps({ deviceIds: ["d1"] }, ACCOUNT)]).toEqual([
			"where",
		]);
		expect([
			...prefilledSteps({ deviceIds: ["d1"], appId: "app_x" }, ACCOUNT),
		]).toEqual(["what", "where"]);
	});
});

describe("exit", () => {
	const routes: [Omit<DeployRoute, "screen">, DevicesScope][] = [
		[{ deviceIds: [], mode: "new" }, APP],
		[{ deviceIds: ["d1", "d2"], mode: "update" }, APP],
		[{ deviceIds: ["d1"] }, ACCOUNT],
		[{ deviceIds: ["d1"], appId: "app_x", serviceId: "svc" }, ACCOUNT],
		[{ deviceIds: [] }, ACCOUNT],
		[{ deviceIds: ["d1", "d2"] }, ACCOUNT],
	];
	for (const [route, scope] of routes)
		test(`${scope.kind} · ${JSON.stringify(route)} matches deployExitHref`, () => {
			const full: DeployRoute = { screen: "deploy", ...route };
			expect(devicesHref(exitRoute(full, scope), scope)).toBe(
				deployExitHref(full, scope),
			);
		});
});

const names: IssueNames = {
	app: "CRM Sync",
	device: () => "edge-berlin-01",
	event: () => "Hourly sync",
	at: (atS) => `@${atS}`,
};

describe("issue sentences (R3)", () => {
	const codes: PlanIssueCode[] = [
		...PLAN_ISSUE_CODES,
		...APPROVAL_ISSUE_CODES.map((code) => `approval.${code}` as const),
	];

	test("every check code has a sentence without codes in it", () => {
		for (const code of codes) {
			const issue: PlanIssue = {
				code,
				step: "what",
				severity: "error",
				deviceId: "d1",
				params: {
					service: "crm-webhook",
					variable: "Batch size",
					port: 8081,
					count: 2,
					event: "evt_crm_hourly",
					other: "evt_crm_webhook",
					device: "d2",
					time: 1790769900,
					method: "GET",
					path: "/orders",
					provider: "telegram",
					exception: "bot_open",
				},
			};
			const sentence = issueText(t, issue, names);
			expect(sentence.length).toBeGreaterThan(10);
			expect(sentence).not.toMatch(/\{\{|undefined|\b[a-z]+[_.][a-z_.]+\b/);
		}
	});

	test("each acknowledgement a device still owes has its own sentence", () => {
		const owed = (exception: string) =>
			issueText(
				t,
				{
					code: "not_acknowledged",
					step: "where",
					severity: "error",
					deviceId: "d1",
					params: { exception },
				},
				names,
			);
		const sentences = PLAN_ACKNOWLEDGEMENTS.map(owed);
		expect(new Set(sentences).size).toBe(PLAN_ACKNOWLEDGEMENTS.length);
		for (const sentence of sentences)
			expect(sentence).toStartWith("Confirm on edge-berlin-01 that");
		expect(owed("endpoint_shared_token")).toBe(
			"Confirm on edge-berlin-01 that the Endpoint uses the service's access settings. Its token from Events is not used.",
		);
	});

	test("a bot's issues speak of a bot, and its provider by name", () => {
		const bot: IssueNames = {
			...names,
			event: () => "Shop helper",
			kind: () => "bot",
		};
		const say = (code: PlanIssueCode, params: PlanIssue["params"]) =>
			issueText(
				t,
				{ code, step: "where", severity: "error", deviceId: "d1", params },
				bot,
			);
		expect(
			say("needs_single_instance", {
				service: "shop",
				count: 2,
				event: "evt_shop_telegram",
			}),
		).toBe(
			"shop runs 2 instances. A bot needs a service with 1 instance: set it to 1, or deploy the bot as its own service.",
		);
		expect(
			say("bot_token_shape", {
				event: "evt_shop_discord",
				provider: "discord",
			}),
		).toBe("That doesn't look like a Discord bot token.");
		expect(say("bot_elsewhere", { event: "evt_shop_telegram" })).toBe(
			"Shop helper is already assigned to a device you can't see. A bot runs in one place: take it back in Events first.",
		);
		expect(
			say("route_conflict", {
				event: "evt_a",
				other: "evt_b",
				method: "POST",
				path: "/run/evt_form",
			}),
		).toBe(
			"Shop helper and Shop helper both answer POST /run/evt_form. A service answers each path once: change one in Events, or deploy them as two services.",
		);
	});
});

describe("exception sentences", () => {
	const plan = {
		draft: {} as DeployPlan["draft"],
		app: {
			id: "app_shop_assistant",
			name: "Shop Assistant",
			visibility: "Private",
			events: [
				{
					id: "evt_shop_telegram",
					name: "Shop helper",
					active: true,
					event_type: "telegram",
				},
			],
		},
		mode: "online",
		services: [],
		targets: [],
	} as DeployPlan;
	const codes: PlanExceptionCode[] = [
		"left_out_refuse",
		"left_out_agent",
		"left_out_duplicate",
		"renamed",
		"port_moved",
		"no_certificate",
		"runs_as_agent",
		"schedule_two_devices",
		"schedule_local_trigger",
		...PLAN_ACKNOWLEDGEMENTS,
	];

	test("every exception has both cells, without codes in them", () => {
		for (const code of codes) {
			const cells = exceptionText(
				t,
				{
					code,
					step: "where",
					deviceId: "d1",
					tone: "warning",
					params: {
						event: "evt_shop_telegram",
						service: "shop",
						reason: "it can't",
						feature: "telegram_bots",
						from: 8080,
						to: 8081,
						count: 2,
						time: 1790769900,
					},
				},
				plan,
				undefined,
				(atS) => `@${atS}`,
			);
			for (const cell of [cells.differs, cells.why]) {
				expect(cell.length).toBeGreaterThan(5);
				expect(cell).not.toMatch(/\{\{|undefined|\b[a-z]+[_.][a-z_.]+\b/);
			}
		}
	});

	test("an agent left out names the flag it lacks", () => {
		const why = (feature: string) =>
			exceptionText(
				t,
				{
					code: "left_out_agent",
					step: "where",
					deviceId: "d1",
					tone: "paused",
					params: { event: "evt_shop_telegram", feature },
				},
				plan,
			).why;
		expect(why("telegram_bots")).toBe(
			"a device's agent is too old to run Telegram bots.",
		);
		expect(why("api_events")).toBe(
			"a device's agent is too old to serve Endpoints.",
		);
	});

	test("a one-time schedule due soon says when", () => {
		const cells = exceptionText(
			t,
			{
				code: "once_soon",
				step: "where",
				deviceId: "d1",
				tone: "warning",
				params: { event: "evt_shop_telegram", time: 42 },
			},
			plan,
			undefined,
			(atS) => `@${atS}`,
		);
		expect(cells.why).toBe(
			"Shop helper runs at @42, in less than 5 minutes. If the deploy isn't finished by then, it doesn't run.",
		);
	});
});

describe("refusals of a run and of preparing", () => {
	test("every flag an agent can lack has its own end of the sentence", () => {
		const said = new Set<string>();
		for (const feature of [
			"api_events",
			"scheduled_events",
			"scheduled_once",
			"on_demand_events",
			"telegram_bots",
			"discord_bots",
		]) {
			const text = runRefusalText(t, {
				code: "agent_feature",
				detail: feature,
			});
			expect(text).toStartWith("the device's agent is too old");
			said.add(text ?? "");
		}
		expect(said.size).toBe(6);
		expect(runRefusalText(t, { code: "agent_feature" })).toBe(
			"the device's agent is too old for one of its events",
		);
	});

	test("a bot's release refusals speak of a bot", () => {
		expect(
			runRefusalText(t, { code: "bot_elsewhere", detail: "Shop helper" }),
		).toBe(
			"Shop helper is already assigned to another service, and a bot runs in one place",
		);
		expect(runRefusalText(t, { code: "bot_hub" })).toBe(
			"this hub can't hand bots to devices yet",
		);
	});

	test("an event missing from the bundle of a hub without its type: Update the hub, not Check it in Events", () => {
		const plan = {
			app: {
				events: [
					{ id: "evt_orders", name: "Orders", event_type: "api", active: true },
					{
						id: "evt_page",
						name: "Page",
						event_type: "simple_chat",
						active: true,
					},
				],
			},
		} as unknown as DeployPlan;
		const missing = (eventId: string, hubTypes?: string[]) =>
			prepareFailureText(
				t,
				{ check: "check_events", kind: "event", eventId, detail: "" },
				plan,
				hubTypes,
			);
		expect(missing("evt_orders", [])).toBe(
			"This hub can't hand Endpoints to devices yet. Update the hub.",
		);
		expect(missing("evt_orders", ["api"])).toBe(
			"Orders isn't in what the hub publishes for devices. Check it in Events, then prepare again.",
		);
		// While the hub's list is unknown nothing is blamed on the hub.
		expect(missing("evt_orders")).toContain("Check it in Events");
		expect(missing("evt_page", [])).toContain("Check it in Events");
	});
});

describe("issue sentences with their facts", () => {
	test("a port in use names the service that holds it, when the device says", () => {
		const issue: PlanIssue = {
			code: "port_in_use",
			step: "endpoint",
			severity: "error",
			deviceId: "d1",
			params: { port: 8081, service: "invoice-extractor" },
		};
		expect(issueText(t, issue, names)).toBe(
			"Port 8081 is used by invoice-extractor on edge-berlin-01.",
		);
		expect(issueText(t, { ...issue, params: { port: 80 } }, names)).toBe(
			"Port 80 is already in use on edge-berlin-01.",
		);
	});

	test("the same id twice reads differently for the plan and for one device", () => {
		const base: PlanIssue = {
			code: "service_id_twice",
			step: "what",
			severity: "error",
		};
		expect(issueText(t, base, names)).toBe("Each service needs its own ID.");
		expect(
			issueText(
				t,
				{
					...base,
					step: "where",
					deviceId: "d1",
					params: { service: "nightly-sync" },
				},
				names,
			),
		).toContain("would both become nightly-sync on edge-berlin-01");
	});

	test("a schedule that can't move says who holds it, until when, or who may move it", () => {
		const schedule = (
			code: PlanIssueCode,
			params: PlanIssue["params"],
		): string =>
			issueText(
				t,
				{ code, step: "where", severity: "error", deviceId: "d1", params },
				names,
			);
		expect(
			schedule("schedule_elsewhere", {
				event: "evt_crm_hourly",
				device: "d2",
				service: "crm-sync",
			}),
		).toBe(
			"Hourly sync is already assigned to edge-berlin-01 › crm-sync. A schedule runs in one place: remove it there first, or run it on the hub again in Events.",
		);
		// A device the viewer can't see has no name to give.
		expect(schedule("schedule_elsewhere", { event: "evt_crm_hourly" })).toBe(
			"Hourly sync is already assigned to a device you can't see. A schedule runs in one place: run it on the hub again in Events first.",
		);
		expect(
			schedule("schedule_returning", { event: "evt_crm_hourly", time: 42 }),
		).toBe(
			"Hourly sync is still returning to the hub from where it ran. Deploy it after @42.",
		);
		expect(schedule("schedule_role", { event: "evt_crm_hourly" })).toBe(
			"Moving Hourly sync off the hub needs the right to edit this app's events. Ask someone who has it, or leave the schedule out.",
		);
		expect(
			schedule("needs_single_instance", { service: "support-bot", count: 4 }),
		).toBe(
			"support-bot runs 4 instances. A schedule needs a service with 1 instance: set it to 1, or deploy the schedule as its own service.",
		);
		expect(schedule("too_many_latest", { count: 70 })).toBe(
			"This deploy has 70 events that follow Latest; one deploy can take 64. Deploy fewer at a time, or pin some in Events.",
		);
	});
});
