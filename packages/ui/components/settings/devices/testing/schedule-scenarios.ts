import { botTokenKey } from "../../../../lib/device-management/bot-config";
import {
	eventKind,
	readExistingDeployment,
} from "../../../../lib/device-management/deployment";
import {
	APPS,
	HASH,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type {
	PlacementEvent,
	PlacementStatusPlus,
	ScheduleHold,
} from "../../../../lib/device-management/model/types";
import type { FakeWorkspace } from "./fake-workspace";

/*
 * Test scenarios over the fake hub and agents: the sample's nightly schedule,
 * its event that follows Latest, the quick action of Support Portal, and
 * Shop Assistant with one event of each kind of round two, on a device.
 */

export const NIGHTLY = {
	app: SAMPLE_APPS.invoiceAi,
	event: "evt_invoice_reconcile",
	device: SAMPLE_IDS.edge,
	service: "invoice-extractor",
} as const;

/** Invoice AI's "Review queue": it follows Latest, and the flow's current version is 0.9.2. */
export const REVIEW = {
	...NIGHTLY,
	event: "evt_invoice_review",
	flow: "flow_review",
} as const;

const EXTRACT: PlacementEvent = {
	event_id: "evt_extract_http",
	event_version: [1, 5, 0],
	board_version: [2, 2, 0],
};

/**
 * Updates a service as a deploy would: the settings it has with these events
 * added, started, and the status the device publishes afterwards.
 */
async function addEvents(
	fake: FakeWorkspace,
	target: { device: string; service: string; app: string },
	events: PlacementEvent[],
) {
	const { api, workspace } = fake;
	const call = workspace.live.call(target.device);
	const existing = await readExistingDeployment(
		call,
		target.service,
		target.app,
	);
	await call({
		type: "apply",
		config: { ...existing.config, events },
		expected_revision: existing.config_revision,
		start: true,
	});
	api.hub.publishStatus(target.device, api.agent(target.device));
}

export interface NightlyOptions {
	/** False: nobody released the schedule, so the service holds it and the hub keeps running it. */
	release?: boolean;
}

/**
 * invoice-extractor also serves the nightly schedule: a person releases the
 * schedule, the service starts, claims it on the hub and arms it. Returns the
 * hold the service reports (null = it runs).
 */
export async function serveNightlyOnEdge(
	fake: FakeWorkspace,
	options: NightlyOptions = {},
): Promise<ScheduleHold | null> {
	const { api } = fake;
	if (options.release !== false)
		api.hub.schedules.release(
			NIGHTLY.app,
			NIGHTLY.event,
			NIGHTLY.device,
			NIGHTLY.service,
		);
	await addEvents(fake, NIGHTLY, [
		EXTRACT,
		{
			event_id: NIGHTLY.event,
			event_version: [1, 0, 0],
			board_version: [1, 3, 0],
		},
	]);
	return (
		api.agent(NIGHTLY.device).placement(NIGHTLY.service)?.schedules?.[0]
			?.hold ?? null
	);
}

/**
 * invoice-extractor also serves the event that follows Latest, pinned to the
 * flow version a deploy resolved (the flow's current one unless given).
 */
export async function serveReviewOnEdge(
	fake: FakeWorkspace,
	flowVersion: [number, number, number] = [0, 9, 2],
): Promise<void> {
	await addEvents(fake, NIGHTLY, [
		EXTRACT,
		{
			event_id: REVIEW.event,
			event_version: [0, 9, 0],
			board_version: flowVersion,
		},
	]);
}

/** Support Portal's quick action "Quick reply" (local-only) beside the chat of support-bot on edge-berlin-01. */
export const QUICK_REPLY = {
	app: SAMPLE_APPS.supportPortal,
	event: "evt_support_reply",
	device: SAMPLE_IDS.edge,
	service: "support-bot",
} as const;

/** support-bot also holds the quick action: started, so it can be run from Devices. */
export async function serveQuickReplyOnEdge(
	fake: FakeWorkspace,
): Promise<PlacementStatusPlus | undefined> {
	const call = fake.workspace.live.call(QUICK_REPLY.device);
	const existing = await readExistingDeployment(
		call,
		QUICK_REPLY.service,
		QUICK_REPLY.app,
	);
	await addEvents(fake, QUICK_REPLY, [
		...(existing.config.events as PlacementEvent[]),
		{
			event_id: QUICK_REPLY.event,
			event_version: [1, 0, 0],
			board_version: [5, 2, 0],
		},
	]);
	return fake.api.agent(QUICK_REPLY.device).placement(QUICK_REPLY.service);
}

/** Shop Assistant (online, design R2 §6.6): an Endpoint, a form, two bots, a one-time schedule. */
export const SHOP = {
	app: APPS.app_shop_assistant.id,
	device: SAMPLE_IDS.edge,
	service: "shop-assistant",
	orders: "evt_shop_orders",
	form: "evt_shop_return",
	telegram: "evt_shop_telegram",
	discord: "evt_shop_discord",
	once: "evt_shop_prices",
} as const;

export interface ShopOptions {
	/** The events the service holds; every one a device can run by default. */
	events?: readonly string[];
	/** False: nobody released its bots and its one-time schedule, so the service holds them. */
	release?: boolean;
}

/** The secret name a deploy gives a bot's token (any name; the value never reaches the fake). */
export const shopTokenSecret = (eventId: string) => `secret-${eventId}`;

/**
 * Shop Assistant on edge-berlin-01 as a deploy leaves it: a cloud approval,
 * a person released its bots and its one-time schedule (unless `release` is
 * false), the service holds the events with their bot token keys and, when it
 * holds the Endpoint, a web endpoint (its form is then on the service page),
 * started, and the status the device publishes afterwards. Returns the
 * service's row.
 */
export async function serveShopOnEdge(
	fake: FakeWorkspace,
	options: ShopOptions = {},
): Promise<PlacementStatusPlus | undefined> {
	const { api, workspace } = fake;
	const ids = options.events ?? [
		SHOP.orders,
		SHOP.form,
		SHOP.telegram,
		SHOP.discord,
		SHOP.once,
	];
	const events = APPS.app_shop_assistant.events.filter((event) =>
		ids.includes(event.id),
	);
	const kinds = events.map((event) => eventKind(event));
	const grant = await api.post<{ grant_id: string }>(
		api.profile,
		`devices/${SHOP.device}/resource-grants`,
		{
			placement_id: SHOP.service,
			deployment_id: `dep-${SHOP.service}`,
			project_id: SHOP.app,
			app_id: SHOP.app,
		},
	);
	if (options.release !== false)
		for (const [index, event] of events.entries())
			if (kinds[index] === "bot" || kinds[index] === "scheduled")
				api.hub.schedules.release(
					SHOP.app,
					event.id,
					SHOP.device,
					SHOP.service,
				);
	const agent = api.agent(SHOP.device);
	await workspace.live.call(SHOP.device)({
		type: "apply",
		config: {
			id: SHOP.service,
			project_id: SHOP.app,
			deployment_id: `dep-${SHOP.service}`,
			revision: HASH.shop10,
			source: "online",
			project_path: `projects/${SHOP.app}`,
			online_metadata_sha256: HASH.shop10,
			events: events.map((event) => ({
				event_id: event.id,
				event_version: event.event_version,
				board_version: event.board_version,
			})),
			// Only a served event gives a service its web endpoint (design R2 §4.1).
			hosting: kinds.some((kind) => kind === "served")
				? {
						host: "127.0.0.1",
						port: 8_480,
						max_in_flight: 32,
						request_timeout_secs: 60,
						auth_secret: `${SHOP.service}-auth`,
					}
				: null,
			max_replicas: 1,
			variables: {},
			secret_overrides: Object.fromEntries(
				events
					.filter((_, index) => kinds[index] === "bot")
					.map((event) => [botTokenKey(event.id), shopTokenSecret(event.id)]),
			),
			resource_grant: { grant_id: grant.grant_id, authz_version: 1 },
			offline_writes: null,
			resources: null,
		},
		expected_revision: agent.placement(SHOP.service)?.config_revision ?? 0,
		start: true,
	});
	api.hub.publishStatus(SHOP.device, agent);
	return agent.placement(SHOP.service);
}
