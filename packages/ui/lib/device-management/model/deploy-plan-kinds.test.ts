import { describe, expect, test } from "bun:test";
import { botTokenKey } from "../bot-config";
import type { PlacementConfiguration } from "../deployment";
import {
	APPS,
	NOW0,
	PLAN_DEVICES,
	ROUTES,
	SHOP_ONCE_AT,
	configBytes,
	v,
} from "./__fixtures__/apps";
import type { AppEventInput } from "./app-plan";
import {
	type DeployDraft,
	MAX_CLAIMED_PER_SERVICE,
	ONCE_SOON_S,
	type PlanApp,
	type PlanEntry,
	type PlanFacts,
	checkPlan,
	claimedEventIds,
	claimedEvents,
	makePlan,
	planServices,
	releasedSchedules,
	resolvePlan,
	wholeAppEvents,
	wirePlan,
	withBotTokens,
} from "./deploy-plan";
import type { ScheduleWhere } from "./schedule-where";

/* Round two's kinds in the plan (design R2 §6.2): Endpoints, one-time schedules, forms and quick actions, bots. */

const SHOP: PlanApp = APPS.app_shop_assistant;
const SHOP_LOCAL: PlanApp = { ...SHOP, visibility: "Offline" };
const ORDERS = "evt_shop_orders";
const RETURN = "evt_shop_return";
const TELEGRAM = "evt_shop_telegram";
const DISCORD = "evt_shop_discord";
const PRICES = "evt_shop_prices";
const TELEGRAM_TOKEN = "123456789:AAHfixture-token-0123456789abcdef";
const STUDIO = "studio-mac-mini";
const EDGE = "edge-berlin-01";

function event(
	id: string,
	type: string,
	extra: Partial<AppEventInput> = {},
): AppEventInput {
	return {
		id,
		name: id,
		active: true,
		event_type: type,
		event_version: v("1.0.0"),
		board_version: v("1.0.0"),
		...extra,
	};
}

function withEvents(app: PlanApp, events: AppEventInput[]): PlanApp {
	return { ...app, events: [...app.events, ...events] };
}

function entry(app: PlanApp, deviceIds: string[]): PlanEntry {
	return {
		scope: { kind: "app", appId: app.id },
		route: { deviceIds, appId: app.id },
		app,
		deploymentId: "dep-r2",
		now: NOW0,
	};
}

function draftFor(
	app: PlanApp,
	deviceIds: string[],
	change: Partial<DeployDraft> = {},
): DeployDraft {
	const draft = { ...makePlan(entry(app, deviceIds)), ...change };
	draft.approval.ownerConsent = true;
	for (const target of draft.targets) target.over.trustAgent = true;
	return draft;
}

function factsFor(app: PlanApp, change: Partial<PlanFacts> = {}): PlanFacts {
	return {
		app,
		devices: PLAN_DEVICES,
		platform: "desktop",
		now: NOW0,
		isAppOwner: true,
		schedules: {},
		...change,
	};
}

function planFor(
	draft: DeployDraft,
	app: PlanApp,
	change: Partial<PlanFacts> = {},
) {
	const facts = factsFor(app, change);
	const plan = resolvePlan(draft, facts);
	return { plan, check: checkPlan(plan, facts) };
}

const ROUND_TWO = new Set([
	"route_conflict",
	"once_passed",
	"bot_token_missing",
	"bot_token_shape",
	"bot_local_trigger",
	"bot_elsewhere",
	"bot_returning",
	"bot_role",
	"too_many_claimed",
	"form_needs_page",
	"not_acknowledged",
	"needs_single_instance",
]);

/** The round-two issues of a plan, as `code@device` with their params. */
function issues(
	draft: DeployDraft,
	app: PlanApp,
	change: Partial<PlanFacts> = {},
) {
	return planFor(draft, app, change)
		.check.issues.filter((issue) => ROUND_TWO.has(issue.code))
		.map((issue) => [issue.code, issue.deviceId ?? null, issue.params ?? {}]);
}

const ROUND_TWO_EXCEPTIONS = new Set([
	"once_soon",
	"bot_open",
	"bot_other_computers",
	"endpoint_shared_token",
]);

/** The round-two exceptions of a plan, with device and params. */
function exceptions(
	draft: DeployDraft,
	app: PlanApp,
	change: Partial<PlanFacts> = {},
) {
	return planFor(draft, app, change)
		.check.exceptions.filter((row) => ROUND_TWO_EXCEPTIONS.has(row.code))
		.map((row) => [row.code, row.deviceId, row.params ?? {}]);
}

const tokens = (draft: DeployDraft, values: Record<string, string>) => {
	draft.secrets = { ...draft.secrets, ...values };
	return draft;
};

const acknowledge = (draft: DeployDraft, deviceId: string) => {
	const target = draft.targets.find((row) => row.deviceId === deviceId);
	if (target)
		target.over.acknowledged = [
			"once_soon",
			"bot_open",
			"bot_other_computers",
			"endpoint_shared_token",
		];
	return draft;
};

describe("what a deploy ticks and how many instances a service runs", () => {
	test("Whole app ticks Endpoints, forms and quick actions; schedules of both kinds and bots stay unticked", () => {
		expect(wholeAppEvents(SHOP)).toEqual([ORDERS, RETURN]);
		expect(makePlan(entry(SHOP, [STUDIO])).events).toEqual([ORDERS, RETURN]);
		expect([...claimedEventIds(SHOP)].sort()).toEqual(
			[DISCORD, PRICES, TELEGRAM].sort(),
		);
		// Field Notes now runs its form too; Support Portal its quick action.
		expect(wholeAppEvents(APPS.app_field_notes)).toEqual([
			"evt_notes_form",
			"evt_notes_http",
		]);
		expect(wholeAppEvents(APPS.app_support_portal)).toEqual([
			"evt_support_chat",
			"evt_support_reply",
			"evt_support_http",
		]);
	});

	test("a form beside a chat keeps the chat's instances; a bot gives one, for its own reason", () => {
		const support = draftFor(APPS.app_support_portal, [], {
			events: ["evt_support_chat", "evt_support_reply"],
			maxInstances: 3,
		});
		expect(planServices(support, APPS.app_support_portal)).toMatchObject([
			{ maxInstances: 3, why: [], hosted: true },
		]);
		const bot = draftFor(SHOP, [], {
			events: [ORDERS, TELEGRAM],
			maxInstances: 3,
		});
		expect(planServices(bot, SHOP)).toMatchObject([
			{ maxInstances: 1, why: [{ code: "bot", eventId: TELEGRAM }] },
		]);
		const once = draftFor(SHOP, [], {
			events: [ORDERS, PRICES],
			maxInstances: 3,
		});
		expect(planServices(once, SHOP)[0]).toMatchObject({
			maxInstances: 1,
			why: [{ code: "scheduled", eventId: PRICES }],
		});
	});

	test("a service with only forms and quick actions has no web endpoint, so it runs 1 instance", () => {
		const draft = draftFor(SHOP, [], { events: [RETURN], maxInstances: 4 });
		expect(planServices(draft, SHOP)).toMatchObject([
			{
				hosted: false,
				maxInstances: 1,
				why: [{ code: "on_demand", eventId: RETURN }],
			},
		]);
	});

	test("Deploy it as its own service splits one event out of the shared service", () => {
		const page = event("evt_shop_page", "page", { default_page_id: "pg" });
		const app = withEvents(SHOP, [page]);
		const shared = draftFor(app, [], { events: [ORDERS, page.id] });
		expect(planServices(shared, app).map((service) => service.events)).toEqual([
			[ORDERS, page.id],
		]);
		const split = draftFor(app, [], {
			events: [ORDERS, page.id],
			ownService: [ORDERS],
		});
		expect(
			planServices(split, app).map((service) => [service.key, service.events]),
		).toEqual([
			["main", [page.id]],
			[ORDERS, [ORDERS]],
		]);
	});
});

describe("bots: a token, one place, and what a person confirms", () => {
	test("the token setting comes with the bot and leaves with it; a flow variable can't use its key", () => {
		const reserved = {
			id: botTokenKey(TELEGRAM),
			name: "Sneaky",
			data_type: "String",
			value_type: "Normal",
			secret: true,
		};
		const plain = {
			id: "var_greeting",
			name: "Greeting",
			data_type: "String",
			value_type: "Normal",
			secret: false,
		};
		const variables = withBotTokens(SHOP.events, {
			[TELEGRAM]: [reserved, plain],
			[ORDERS]: [{ ...reserved, id: botTokenKey(ORDERS) }],
		});
		expect(variables?.[TELEGRAM]).toEqual([
			plain,
			{
				id: botTokenKey(TELEGRAM),
				name: "Shop helper",
				data_type: "String",
				value_type: "Normal",
				secret: true,
			},
		]);
		expect(variables?.[ORDERS]).toEqual([]);
		expect(variables?.[DISCORD]?.map((row) => row.id)).toEqual([
			botTokenKey(DISCORD),
		]);
		expect(withBotTokens(APPS.app_crm_sync.events, undefined)).toBeUndefined();
	});

	test("a bot can't be planned without its token, and a token must look like one", () => {
		const draft = draftFor(SHOP, [STUDIO], { events: [TELEGRAM] });
		acknowledge(draft, STUDIO);
		expect(issues(draft, SHOP)).toEqual([
			["bot_token_missing", STUDIO, { event: TELEGRAM }],
		]);
		tokens(draft, { [botTokenKey(TELEGRAM)]: "Paste the token here" });
		expect(issues(draft, SHOP)).toEqual([
			["bot_token_shape", STUDIO, { event: TELEGRAM, provider: "telegram" }],
		]);
		tokens(draft, { [botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN });
		expect(issues(draft, SHOP)).toEqual([]);
		// Per device: another device without its own value has none.
		draft.secretsMode = "per_device";
		expect(issues(draft, SHOP)).toEqual([
			["bot_token_missing", STUDIO, { event: TELEGRAM }],
		]);
	});

	test("an update of a service that has the bot keeps its stored token", () => {
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "shop",
						projectId: SHOP.id,
						events: [TELEGRAM],
						maxInstances: 1,
					},
				],
			},
		};
		const draft = draftFor(SHOP, [STUDIO], { events: [TELEGRAM] });
		draft.targets[0].choices = { main: { kind: "update", serviceId: "shop" } };
		const where: Record<string, ScheduleWhere> = {
			[TELEGRAM]: {
				fact: "device",
				deviceId: STUDIO,
				serviceId: "shop",
				since: NOW0 - 3600,
				seenAt: NOW0 - 60,
				live: true,
				bot: {
					event_id: TELEGRAM,
					provider: "telegram",
					state: "connected",
					hold: null,
				},
			},
		};
		const { plan, check } = planFor(draft, SHOP, {
			devices,
			schedules: where,
		});
		expect(plan.targets[0].services[0]).toMatchObject({
			events: [TELEGRAM],
			addedBots: [],
		});
		expect(check.issues.filter((row) => ROUND_TWO.has(row.code))).toEqual([]);
		expect(claimedEvents(plan, plan.targets[0].services[0])).toEqual([]);
	});

	test("this computer's own trigger blocks a bot until it is stopped here", () => {
		const draft = tokens(draftFor(SHOP, [STUDIO], { events: [TELEGRAM] }), {
			[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
		});
		acknowledge(draft, STUDIO);
		expect(issues(draft, SHOP, { localTriggers: [TELEGRAM] })).toEqual([
			["bot_local_trigger", STUDIO, { event: TELEGRAM }],
		]);
		expect(issues(draft, SHOP, { localTriggers: [] })).toEqual([]);
	});

	test("online: another place holds it, it is on its way back, or the person may not move it", () => {
		const draft = tokens(draftFor(SHOP, [STUDIO], { events: [TELEGRAM] }), {
			[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
		});
		acknowledge(draft, STUDIO);
		const held: ScheduleWhere = {
			fact: "device_idle",
			why: "not_connected",
			botState: "token_refused",
			deviceId: EDGE,
			serviceId: "shop-edge",
			since: NOW0 - 3600,
			seenAt: NOW0 - 60,
		};
		expect(issues(draft, SHOP, { schedules: { [TELEGRAM]: held } })).toEqual([
			[
				"bot_elsewhere",
				STUDIO,
				{ event: TELEGRAM, device: EDGE, service: "shop-edge" },
			],
		]);
		expect(
			issues(draft, SHOP, {
				schedules: {
					[TELEGRAM]: { fact: "returning", hubResumesAt: NOW0 + 300 },
				},
			}),
		).toEqual([
			["bot_returning", STUDIO, { event: TELEGRAM, time: NOW0 + 300 }],
		]);
		expect(issues(draft, SHOP, { canEditEvents: false })).toEqual([
			["bot_role", STUDIO, { event: TELEGRAM }],
		]);
		// Two devices in one plan: the second is refused.
		const both = tokens(
			draftFor(SHOP, [STUDIO, EDGE], { events: [TELEGRAM] }),
			{ [botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN },
		);
		acknowledge(both, STUDIO);
		acknowledge(both, EDGE);
		expect(issues(both, SHOP)).toEqual([
			[
				"bot_elsewhere",
				EDGE,
				{ event: TELEGRAM, device: STUDIO, service: "shop-assistant" },
			],
		]);
	});

	test("local-only: a device the viewer sees that serves the bot refuses a second one", () => {
		const draft = tokens(
			draftFor(SHOP_LOCAL, [STUDIO], { events: [TELEGRAM] }),
			{
				[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
			},
		);
		acknowledge(draft, STUDIO);
		expect(issues(draft, SHOP_LOCAL)).toEqual([]);
		const devices = {
			...PLAN_DEVICES,
			[EDGE]: {
				...PLAN_DEVICES[EDGE],
				services: [
					{ serviceId: "shop-edge", projectId: SHOP.id, events: [TELEGRAM] },
				],
			},
		};
		expect(issues(draft, SHOP_LOCAL, { devices })).toEqual([
			[
				"bot_elsewhere",
				STUDIO,
				{ event: TELEGRAM, device: EDGE, service: "shop-edge" },
			],
		]);
		// No hub to release it on.
		const { plan } = planFor(draft, SHOP_LOCAL);
		expect(claimedEvents(plan, plan.targets[0].services[0])).toEqual([]);
	});

	test("a bot can't be added to a service that runs more than one instance", () => {
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "shop",
						projectId: SHOP.id,
						events: [ORDERS],
						maxInstances: 3,
					},
				],
			},
		};
		const draft = tokens(draftFor(SHOP, [STUDIO], { events: [TELEGRAM] }), {
			[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
		});
		draft.targets[0].choices = { main: { kind: "add", serviceId: "shop" } };
		acknowledge(draft, STUDIO);
		expect(issues(draft, SHOP, { devices })).toEqual([
			[
				"needs_single_instance",
				STUDIO,
				{ service: "shop", count: 3, event: TELEGRAM },
			],
		]);
	});

	test("an open bot and other computers each need a yes on every device", () => {
		const draft = tokens(
			draftFor(SHOP, [STUDIO], { events: [TELEGRAM, DISCORD] }),
			{
				[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
				[botTokenKey(DISCORD)]:
					"MTE4MDAwMDAwMDAwMDAwMDAwMQ.test-only.not-a-real-discord-token",
			},
		);
		expect(exceptions(draft, SHOP)).toEqual([
			["bot_open", STUDIO, { event: TELEGRAM }],
			["bot_other_computers", STUDIO, { event: TELEGRAM }],
			["bot_other_computers", STUDIO, { event: DISCORD }],
		]);
		expect(issues(draft, SHOP)).toEqual([
			["not_acknowledged", STUDIO, { exception: "bot_open" }],
			["not_acknowledged", STUDIO, { exception: "bot_other_computers" }],
		]);
		draft.targets[0].over.acknowledged = ["bot_open"];
		expect(issues(draft, SHOP)).toEqual([
			["not_acknowledged", STUDIO, { exception: "bot_other_computers" }],
		]);
		acknowledge(draft, STUDIO);
		expect(issues(draft, SHOP)).toEqual([]);
	});

	test("the run releases the bots and schedules a deploy adds, online only", () => {
		const draft = tokens(
			draftFor(SHOP, [STUDIO], { events: [TELEGRAM, PRICES] }),
			{ [botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN },
		);
		const { plan } = planFor(draft, SHOP);
		const service = plan.targets[0].services[0];
		expect(service).toMatchObject({
			addedSchedules: [PRICES],
			addedBots: [TELEGRAM],
		});
		expect(claimedEvents(plan, service)).toEqual([PRICES, TELEGRAM]);
		expect(releasedSchedules(plan, service)).toEqual([PRICES, TELEGRAM]);
		expect(releasedSchedules(plan, { addedSchedules: [PRICES] })).toEqual([
			PRICES,
		]);
	});
});

describe("one-time schedules (§3.1)", () => {
	const draft = () =>
		acknowledge(draftFor(SHOP, [STUDIO], { events: [PRICES] }), STUDIO);

	test("one whose time has passed can't be added; it never blocks an update of a service that has it", () => {
		const late = { now: SHOP_ONCE_AT + 60 };
		expect(issues(draft(), SHOP, late)).toEqual([
			["once_passed", null, { event: PRICES, time: SHOP_ONCE_AT }],
		]);
		expect(
			planFor(draft(), SHOP, late).check.issues.find(
				(row) => row.code === "once_passed",
			)?.step,
		).toBe("what");
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "shop",
						projectId: SHOP.id,
						events: [PRICES],
						maxInstances: 1,
					},
				],
			},
		};
		const update = draft();
		update.targets[0].choices = { main: { kind: "update", serviceId: "shop" } };
		expect(issues(update, SHOP, { ...late, devices })).toEqual([]);
	});

	test("one that runs in less than 5 minutes needs a yes, when added and when its service is updated", () => {
		const soon = { now: SHOP_ONCE_AT - ONCE_SOON_S + 1 };
		const fresh = draftFor(SHOP, [STUDIO], { events: [PRICES] });
		expect(exceptions(fresh, SHOP, soon)).toEqual([
			["once_soon", STUDIO, { event: PRICES, time: SHOP_ONCE_AT }],
		]);
		expect(issues(fresh, SHOP, soon)).toEqual([
			["not_acknowledged", STUDIO, { exception: "once_soon" }],
		]);
		expect(issues(draft(), SHOP, soon)).toEqual([]);
		// Five minutes or more ahead: nothing to confirm.
		expect(
			exceptions(fresh, SHOP, { now: SHOP_ONCE_AT - ONCE_SOON_S }),
		).toEqual([]);
	});
});

describe("what one service holds together", () => {
	const page = event("evt_shop_page", "page", { default_page_id: "pg" });

	test("two events of one service can't answer the same method and path", () => {
		const twin = event("evt_shop_orders_v2", "api", {
			config: configBytes({ ...ROUTES.orders, auth_token: "" }),
		});
		const app = withEvents(SHOP, [twin]);
		const draft = acknowledge(
			draftFor(app, [STUDIO], { events: [ORDERS, twin.id] }),
			STUDIO,
		);
		expect(issues(draft, app)).toEqual([
			[
				"route_conflict",
				STUDIO,
				{ event: ORDERS, other: twin.id, method: "GET", path: "/orders" },
			],
		]);
		// Two services: each answers it on its own port.
		draft.split = "per_event";
		expect(issues(draft, app)).toEqual([]);
	});

	test("an Endpoint at POST /run/{id} of a form collides only in a service with a web endpoint", () => {
		const run = event("evt_shop_run", "api", {
			config: configBytes({ method: "POST", path: `/run/${RETURN}` }),
		});
		const app = withEvents(SHOP, [run, page]);
		const withPage = acknowledge(
			draftFor(app, [STUDIO], { events: [run.id, RETURN, page.id] }),
			STUDIO,
		);
		expect(
			issues(withPage, app).filter(([code]) => code === "route_conflict"),
		).toEqual([
			[
				"route_conflict",
				STUDIO,
				{
					event: RETURN,
					other: run.id,
					method: "POST",
					path: `/run/${RETURN}`,
				},
			],
		]);
		const get = event("evt_shop_run_get", "api", {
			config: configBytes({ method: "GET", path: `/run/${RETURN}` }),
		});
		const other = withEvents(SHOP, [get, page]);
		expect(
			issues(
				acknowledge(
					draftFor(other, [STUDIO], { events: [get.id, RETURN, page.id] }),
					STUDIO,
				),
				other,
			).filter(([code]) => code === "route_conflict"),
		).toEqual([]);
	});

	test("an Endpoint with its own token in a shared service needs a yes; its own service needs none", () => {
		const app = withEvents(SHOP, [page]);
		const draft = draftFor(app, [STUDIO], { events: [ORDERS, page.id] });
		expect(exceptions(draft, app)).toContainEqual([
			"endpoint_shared_token",
			STUDIO,
			{ event: ORDERS, service: "shop-assistant" },
		]);
		expect(issues(draft, app)).toEqual([
			["not_acknowledged", STUDIO, { exception: "endpoint_shared_token" }],
		]);
		draft.targets[0].over.acknowledged = ["endpoint_shared_token"];
		expect(issues(draft, app)).toEqual([]);
		const split = draftFor(app, [STUDIO], {
			events: [ORDERS, page.id],
			ownService: [ORDERS],
		});
		expect(
			exceptions(split, app).filter(
				([code]) => code === "endpoint_shared_token",
			),
		).toEqual([]);
		// Alone, or with no token of its own, or a config that can't be read: nothing to ask.
		const alone = draftFor(app, [STUDIO], { events: [ORDERS] });
		expect(exceptions(alone, app)).toEqual([]);
		for (const ownToken of [false, undefined]) {
			const quiet = {
				...app,
				events: app.events.map((row) =>
					row.id === ORDERS ? { ...row, ownToken } : row,
				),
			};
			expect(
				exceptions(
					draftFor(quiet, [STUDIO], { events: [ORDERS, page.id] }),
					quiet,
				).filter(([code]) => code === "endpoint_shared_token"),
			).toEqual([]);
		}
	});

	test("an update that only keeps a shared Endpoint asks nothing again", () => {
		const app = withEvents(SHOP, [page]);
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "shop",
						projectId: SHOP.id,
						events: [ORDERS, page.id],
						maxInstances: 1,
					},
				],
			},
		};
		const draft = draftFor(app, [STUDIO], { events: [ORDERS, page.id] });
		draft.targets[0].choices = { main: { kind: "update", serviceId: "shop" } };
		expect(
			exceptions(draft, app, { devices }).filter(
				([code]) => code === "endpoint_shared_token",
			),
		).toEqual([]);
	});

	test("a form with a file field in a service without a web endpoint is a warning", () => {
		const alone = draftFor(SHOP, [STUDIO], { events: [RETURN] });
		const found = planFor(alone, SHOP).check.issues.filter(
			(row) => row.code === "form_needs_page",
		);
		expect(found).toMatchObject([
			{
				severity: "warning",
				deviceId: STUDIO,
				params: { event: RETURN, service: "shop-assistant" },
			},
		]);
		// A warning never blocks the deploy.
		expect(planFor(alone, SHOP).check.firstBlocking?.code).not.toBe(
			"form_needs_page",
		);
		const app = withEvents(SHOP, [page]);
		expect(
			planFor(
				draftFor(app, [STUDIO], { events: [RETURN, page.id] }),
				app,
			).check.issues.filter((row) => row.code === "form_needs_page"),
		).toEqual([]);
		// A form without a file field is run from Devices.
		expect(
			planFor(
				draftFor(APPS.app_field_notes, [STUDIO], {
					events: ["evt_notes_form"],
				}),
				APPS.app_field_notes,
			).check.issues.filter((row) => row.code === "form_needs_page"),
		).toEqual([]);
	});

	test("a service runs at most 64 schedules and bots", () => {
		const many = Array.from({ length: MAX_CLAIMED_PER_SERVICE + 1 }, (_, n) =>
			event(`evt_cron_${n}`, "cron", {
				schedule: { expression: "0 0 3 * * *", timezone: "UTC" },
			}),
		);
		const app: PlanApp = { ...SHOP_LOCAL, events: many };
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "crons",
						projectId: SHOP.id,
						events: many.slice(0, 40).map((row) => row.id),
						maxInstances: 1,
					},
				],
			},
		};
		const draft = draftFor(app, [STUDIO], {
			events: many.slice(40).map((row) => row.id),
		});
		draft.targets[0].choices = { main: { kind: "add", serviceId: "crons" } };
		expect(
			issues(draft, app, { devices }).filter(
				([code]) => code === "too_many_claimed",
			),
		).toEqual([
			[
				"too_many_claimed",
				STUDIO,
				{ service: "crons", count: MAX_CLAIMED_PER_SERVICE + 1 },
			],
		]);
	});
});

describe("agents that lack a flag", () => {
	const OLD = {
		placement_events: 1,
		scheduled_events: 1,
	} as const;
	const withFeatures = (
		features: PlanFacts["devices"][string]["features"],
	) => ({
		...PLAN_DEVICES,
		[STUDIO]: { ...PLAN_DEVICES[STUDIO], features },
	});

	test("each new kind is left out of an older agent with the flag it lacks", () => {
		const draft = draftFor(SHOP, [STUDIO], {
			events: [ORDERS, RETURN, TELEGRAM, DISCORD, PRICES],
		});
		const { plan, check } = planFor(draft, SHOP, {
			devices: withFeatures(OLD),
		});
		expect(plan.targets[0].services[0].leftOut).toEqual([
			{ eventId: ORDERS, why: "agent", feature: "api_events" },
			{ eventId: RETURN, why: "agent", feature: "on_demand_events" },
			{ eventId: TELEGRAM, why: "agent", feature: "telegram_bots" },
			{ eventId: DISCORD, why: "agent", feature: "discord_bots" },
			{ eventId: PRICES, why: "agent", feature: "scheduled_once" },
		]);
		expect(check.exceptions).toContainEqual({
			code: "left_out_agent",
			step: "where",
			deviceId: STUDIO,
			tone: "paused",
			params: { event: ORDERS, feature: "api_events" },
		});
		// Unknown flags refuse nothing: the run asks the live agent.
		expect(
			planFor(draft, SHOP, { devices: withFeatures(undefined) }).plan.targets[0]
				.services[0].leftOut,
		).toEqual([]);
	});

	test("an http event outside the strict route form needs api_events; one in it deploys as before", () => {
		const loose = event("evt_loose", "http", {
			config: configBytes({ path: "loose" }),
		});
		const strict = event("evt_strict", "http", {
			config: configBytes(ROUTES.support),
		});
		const app = withEvents(SHOP_LOCAL, [loose, strict]);
		const draft = draftFor(app, [STUDIO], { events: [loose.id, strict.id] });
		expect(
			planFor(draft, app, { devices: withFeatures(OLD) }).plan.targets[0]
				.services[0],
		).toMatchObject({
			events: [strict.id],
			leftOut: [{ eventId: loose.id, why: "agent", feature: "api_events" }],
		});
	});
});

describe("wiring a bot", () => {
	const installed = {
		project_id: SHOP.id,
		project_path: "/private/projects/app_shop_assistant",
		revision: "rev-shop",
		source: "offline" as const,
	};
	const catalogEvent = (
		id: string,
		eventType: string,
		kind: "bot" | "served",
	) => ({
		id,
		name: id,
		event_type: eventType,
		event_version: v("1.0.0"),
		board_version: v("1.2.0"),
		hosted: kind === "served",
		readiness_kind:
			kind === "served" ? ("listener" as const) : ("explicit" as const),
		rollout_supported: true,
		eligible: true,
		kind,
	});

	test("an update sends the token of a bot it adds and drops the stored token of one it removes", () => {
		const existing: PlacementConfiguration = {
			placement_id: "shop",
			project_id: SHOP.id,
			deployment_id: "dep-shop",
			config_revision: 4,
			config: {
				id: "shop",
				project_id: SHOP.id,
				deployment_id: "dep-shop",
				revision: "rev-shop",
				source: "offline",
				events: [],
				max_replicas: 1,
				variables: {},
				secret_overrides: { [botTokenKey(DISCORD)]: "variable-old" },
			},
		} as unknown as PlacementConfiguration;
		const devices = {
			...PLAN_DEVICES,
			[STUDIO]: {
				...PLAN_DEVICES[STUDIO],
				services: [
					{
						serviceId: "shop",
						projectId: SHOP.id,
						events: [ORDERS, DISCORD],
						maxInstances: 1,
					},
				],
			},
		};
		const draft = tokens(
			draftFor(SHOP_LOCAL, [STUDIO], { events: [ORDERS, TELEGRAM] }),
			{ [botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN },
		);
		draft.targets[0].choices = { main: { kind: "update", serviceId: "shop" } };
		draft.acceptRemovedEvents = true;
		const { plan } = planFor(draft, SHOP_LOCAL, { devices });
		const input = wirePlan(
			plan,
			{ deviceId: STUDIO, serviceKey: "main" },
			{
				installed,
				existing,
				events: [
					catalogEvent(ORDERS, "api", "served"),
					catalogEvent(TELEGRAM, "telegram", "bot"),
				],
				variables: {},
				serviceToken: "",
				canManageCertificates: false,
			},
		);
		expect(input.overrides).toEqual({
			[botTokenKey(TELEGRAM)]: TELEGRAM_TOKEN,
		});
		expect(input.removeOverrides).toEqual([botTokenKey(DISCORD)]);
		expect(input.variables.map((row) => row.id)).toEqual([
			botTokenKey(TELEGRAM),
		]);
	});
});
