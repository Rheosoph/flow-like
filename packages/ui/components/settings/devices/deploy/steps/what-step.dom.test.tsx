import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	clickByText,
	installDom,
	typeInto,
} from "../../testing/dom-harness";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { serveShopOnEdge } = await import("../../testing/schedule-scenarios");
const { APPS, BOTS, configBytes } = await import(
	"../../../../../lib/device-management/model/__fixtures__/apps"
);
const kit = await import("../deploy-test-kit");
const { EDGE, VISITOR, CRM, INVOICE, text } = kit;
await preloadDevices();

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const checkbox = (root: ParentNode, eventId: string) =>
	root.querySelector<HTMLButtonElement>(`#deploy-event-${eventId}`);
const summary = () => text(byRole("region", "Your choices"));

describe("What · events (APP §3.5)", () => {
	test("Whole app ticks every event that can run and locks the ticks", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const page = checkbox(view.container, "evt_visitor_page");
		expect(page?.getAttribute("data-state")).toBe("checked");
		expect(page?.disabled).toBe(true);
		expect(text(view.container)).toContain(
			"Served by the device · checks its web server",
		);
		expect(text(view.container)).toContain(
			"Runs on its own · waits for the flow to report ready",
		);
		expect(text(view.container)).toContain("event 0.4.0 · flow 0.4.0");
	});

	test("Choose events unlocks them; with none ticked Continue says why", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		await clickByText("Choose events", view.container);
		const page = checkbox(view.container, "evt_visitor_page");
		expect(page?.disabled).toBe(false);
		await click(checkbox(view.container, "evt_badge_printer") as Element);
		expect(summary()).toContain("1 event · 1 service");
		// One event left: the service takes one ID field, no split choice.
		expect(view.container.querySelector("#deploy-split")).toBeNull();

		await click(page as Element);
		expect(kit.footBlocking(view.container)).toBe("Pick at least one event.");
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		expect(text(view.container)).toContain("Step 1 of 8: What to run");
	});

	test("Can't run on devices is collapsed, with the reason and its fix", async () => {
		const view = await kit.mountApp(mountDevices, CRM);
		expect(text(view.container)).not.toContain("Not offered when you deploy");
		await clickByText(/Can't run on devices · 1/, view.container);
		const page = text(view.container);
		expect(page).toContain("Not offered when you deploy");
		expect(page).toContain(
			"Splits traffic with a canary. A device can't split traffic; end the canary first.",
		);
		const row = view.container.querySelector('[data-event="evt_crm_rest"]');
		expect(row?.getAttribute("data-dim")).toBe("true");
		expect(
			row?.querySelector<HTMLButtonElement>("button[role=checkbox]")?.disabled,
		).toBe(true);
		// A canary can be ended in Events.
		expect(
			byRole("link", "Open Events", view.container).getAttribute("href"),
		).toBe(`/library/config/events?id=${CRM}&event=evt_crm_rest`);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});
});

describe("What · schedules", () => {
	const SCHEDULE = "evt_visitor_report";
	const rowOf = (root: ParentNode, eventId: string) =>
		root.querySelector<HTMLElement>(`[data-event="${eventId}"]`);

	test("a schedule is offered unticked, also under Whole app, and says when and where it runs", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const box = checkbox(view.container, SCHEDULE);
		expect(box?.getAttribute("data-state")).toBe("unchecked");
		// The other ticks are fixed by Whole app; a schedule's is the person's own.
		expect(box?.disabled).toBe(false);
		expect(summary()).toContain("2 events · 1 service");
		const row = text(rowOf(view.container, SCHEDULE) ?? undefined);
		expect(row).toContain("Schedule");
		expect(row).toContain("At 18:00 every day · Europe/Berlin");
		expect(row).toContain("Runs on a schedule · the device starts it");
		expect(row).toContain(
			"Next run by its schedule: 18:00 GMT+2 · in 4 hr. · 16:00 your time",
		);
		expect(row).toContain(
			"Runs on the hub today. Tick it to run it on the device instead.",
		);
		expect(text(view.container)).toContain(
			"Whole app ticks every event that can run on a device, except schedules and bots: tick one yourself to move it to the device.",
		);
	});

	test("ticking a schedule adds it and limits its service to one instance, with the reason", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		await click(checkbox(view.container, SCHEDULE) as Element);
		expect(summary()).toContain("3 events · 1 service");
		expect(text(view.container)).toContain(
			"Daily visitor report runs on a schedule, so this service runs 1 instance. Two instances would start every run twice.",
		);
		// Whole app again leaves it out: it never ticks a schedule.
		await clickByText("Choose events", view.container);
		await clickByText("Whole app", view.container);
		expect(checkbox(view.container, SCHEDULE)?.getAttribute("data-state")).toBe(
			"unchecked",
		);
		expect(summary()).toContain("2 events · 1 service");
	});

	test("a local-only app has no hub: its schedule runs in the desktop app today", async () => {
		const view = await kit.mountApp(mountDevices, CRM);
		const row = text(rowOf(view.container, "evt_crm_hourly") ?? undefined);
		expect(row).toContain(
			"At :00 past every hour · UTC (the event sets no time zone)",
		);
		expect(row).toContain(
			"Runs in the desktop app while it is open. On a device it runs without a computer.",
		);
		expect(row).not.toContain("Runs on the hub today");
		expect(
			checkbox(view.container, "evt_crm_hourly")?.getAttribute("data-state"),
		).toBe("unchecked");
	});

	test("a hub that can't hand schedules to devices: the schedule is listed under Can't run, with that reason", async () => {
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{},
			{
				hubVersion: "old",
			},
		);
		await clickByText(/Can't run on devices/, view.container);
		expect(text(rowOf(view.container, SCHEDULE) ?? undefined)).toContain(
			"This hub can't run schedules on devices yet. Update the hub.",
		);
		expect(checkbox(view.container, SCHEDULE)?.disabled).toBe(true);
	});
});

describe("What · Endpoints, forms, one-time schedules and bots (R2 §6.4)", () => {
	const SHOP = "app_shop_assistant";
	const ORDERS = "evt_shop_orders";
	const FORM = "evt_shop_return";
	const TELEGRAM = "evt_shop_telegram";
	const DISCORD = "evt_shop_discord";
	const ONCE = "evt_shop_prices";
	const rowOf = (root: ParentNode, eventId: string) =>
		text(
			root.querySelector<HTMLElement>(`[data-event="${eventId}"]`) ?? undefined,
		);
	/**
	 * Shop Assistant as its records come from the hub: the form's record lists
	 * its fields (`inputs`), and the one-time schedule's time is `once` when given.
	 */
	const shopApps = (once?: { date: string; time: string }) => ({
		...APPS,
		[SHOP]: {
			...APPS.app_shop_assistant,
			events: APPS.app_shop_assistant.events.map((event) => {
				if (event.id === FORM)
					return {
						...event,
						inputs: ["String", "Integer", "PathBuf"].map((data_type) => ({
							data_type,
						})),
					} as typeof event;
				return event.id === ONCE && once
					? {
							...event,
							schedule: { scheduled_for: once, timezone: "Europe/Berlin" },
						}
					: event;
			}),
		},
	});
	const pastOnce = () => shopApps({ date: "2026-09-01", time: "09:00" });

	test("Whole app ticks the Endpoint and the form; bots and the one-time schedule start unticked, each in its own words", async () => {
		const view = await kit.mountApp(
			mountDevices,
			SHOP,
			{},
			{ apps: shopApps() },
		);
		for (const eventId of [ORDERS, FORM]) {
			const box = checkbox(view.container, eventId);
			expect(box?.getAttribute("data-state")).toBe("checked");
			expect(box?.disabled).toBe(true);
		}
		for (const eventId of [TELEGRAM, DISCORD, ONCE]) {
			const box = checkbox(view.container, eventId);
			expect(box?.getAttribute("data-state")).toBe("unchecked");
			expect(box?.disabled).toBe(false);
		}
		expect(summary()).toContain("2 events · 1 service");
		const orders = rowOf(view.container, ORDERS);
		expect(orders).toContain("Endpoint");
		expect(orders).toContain("GET /orders");
		expect(orders).toContain(
			"Its own token from Events is not used on a device.",
		);
		const form = rowOf(view.container, FORM);
		expect(form).toContain("Started by a person · 3 fields");
		expect(form).toContain(
			"It takes a file: only a service page can send one.",
		);
		const telegram = rowOf(view.container, TELEGRAM);
		expect(telegram).toContain("Telegram bot");
		expect(telegram).toContain("Runs on its own · stays connected to Telegram");
		expect(telegram).toContain(
			"No device runs it. Tick it to keep it connected from the device.",
		);
		const once = rowOf(view.container, ONCE);
		expect(once).toContain("Runs once on 2026-10-15 at 09:00 (Europe/Berlin)");
		expect(once).toContain("Runs once · the device starts it");
		// Only some hubs fire a one-time schedule: the row never says "runs on the hub".
		expect(once).not.toContain("hub");
		expect(text(view.container)).toContain(
			"Whole app ticks every event that can run on a device, except schedules and bots: tick one yourself to move it to the device.",
		);
		await clickByText(/Can't run on devices · 1/, view.container);
		expect(rowOf(view.container, "evt_shop_mail")).toContain(
			"Inbound email is handled by the hub.",
		);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	/** Shop Assistant with the settings of some of its bots replaced. */
	const shopWithBots = (bots: Record<string, Record<string, unknown>>) => ({
		...APPS,
		[SHOP]: {
			...APPS.app_shop_assistant,
			events: APPS.app_shop_assistant.events.map((event) =>
				Object.hasOwn(bots, event.id)
					? { ...event, config: configBytes(bots[event.id]) }
					: event,
			),
		},
	});
	const { command_prefix: _slash, ...telegramNoPrefix } = BOTS.telegram;
	const { command_prefix: _bang, ...discordNoPrefix } = BOTS.discord;
	const EVERY =
		"In groups and servers it answers every message: no command prefix is set.";

	test("a bot with a prefix and mentions on says it answers mentions, replies and the prefix, on Telegram and on Discord", async () => {
		const view = await kit.mountApp(mountDevices, SHOP);
		expect(rowOf(view.container, TELEGRAM)).toContain(
			"In groups and servers it answers mentions, replies and every message that starts with /.",
		);
		expect(rowOf(view.container, DISCORD)).toContain(
			"In groups and servers it answers mentions, replies and every message that starts with !.",
		);
	});

	test("a bot without a prefix says it answers every message: a Telegram bot always, a Discord bot with mentions off", async () => {
		const apps = shopWithBots({
			[TELEGRAM]: telegramNoPrefix,
			[DISCORD]: {
				...BOTS.discord,
				command_prefix: "",
				respond_to_mentions: false,
			},
		});
		const view = await kit.mountApp(mountDevices, SHOP, {}, { apps });
		for (const eventId of [TELEGRAM, DISCORD]) {
			const row = rowOf(view.container, eventId);
			expect([eventId, row.includes(EVERY)]).toEqual([eventId, true]);
			expect(row).not.toContain("starts with");
		}
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("a Discord bot without a prefix keeps to mentions and replies while mentions are on; a prefix without mentions answers only the prefix", async () => {
		const apps = shopWithBots({
			[DISCORD]: discordNoPrefix,
			[TELEGRAM]: { ...BOTS.telegram, respond_to_mentions: false },
		});
		const view = await kit.mountApp(mountDevices, SHOP, {}, { apps });
		const discord = rowOf(view.container, DISCORD);
		expect(discord).toContain(
			"In groups and servers it answers mentions and replies.",
		);
		expect(discord).not.toContain("every message");
		const telegram = rowOf(view.container, TELEGRAM);
		expect(telegram).toContain(
			"In groups and servers it answers every message that starts with /.",
		);
		expect(telegram).not.toContain("mentions");
	});

	test("a Discord bot with a prefix and mentions off answers only the prefix", async () => {
		const apps = shopWithBots({
			[DISCORD]: { ...BOTS.discord, respond_to_mentions: false },
		});
		const view = await kit.mountApp(mountDevices, SHOP, {}, { apps });
		const discord = rowOf(view.container, DISCORD);
		expect(discord).toContain(
			"In groups and servers it answers every message that starts with !.",
		);
		expect(discord).not.toContain("mentions");
	});

	test("a bot ticked by hand gives its service one instance, with the reason", async () => {
		const view = await kit.mountApp(mountDevices, SHOP);
		await click(checkbox(view.container, TELEGRAM) as Element);
		expect(summary()).toContain("3 events · 1 service");
		expect(text(view.container)).toContain(
			"Shop helper is a bot, so this service runs 1 instance. Two instances would answer every message twice.",
		);
	});

	test("a bot this computer runs says so in its row", async () => {
		const view = await kit.mountApp(
			mountDevices,
			SHOP,
			{},
			{ platform: "desktop", localTriggers: [TELEGRAM] },
		);
		await view.settle();
		expect(rowOf(view.container, TELEGRAM)).toContain(
			"Runs in the desktop app on this computer today. Tick it to keep it connected from the device instead.",
		);
		expect(rowOf(view.container, DISCORD)).toContain("No device runs it.");
	});

	test("a one-time schedule whose time has passed can't be added: the row and Continue say why", async () => {
		const view = await kit.mountApp(
			mountDevices,
			SHOP,
			{ device: EDGE },
			{ apps: pastOnce() },
		);
		await view.settle();
		expect(rowOf(view.container, ONCE)).toContain(
			"Its time has passed. Set a new time in Events.",
		);
		await click(checkbox(view.container, ONCE) as Element);
		expect(kit.footBlocking(view.container)).toMatch(
			/^Price update's time \(.+\) has passed\. Set a new time in Events, or leave it out\.$/,
		);
		await click(checkbox(view.container, ONCE) as Element);
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("an update of a service that has a passed one-time schedule is not blocked by it", async () => {
		const fake = await createFakeWorkspace();
		await serveShopOnEdge(fake);
		const view = await kit.mountApp(
			mountDevices,
			SHOP,
			{ mode: undefined, device: EDGE, service: "shop-assistant" },
			{ fake, apps: pastOnce() },
		);
		await view.settle();
		expect(checkbox(view.container, ONCE)?.getAttribute("data-state")).toBe(
			"checked",
		);
		expect(rowOf(view.container, ONCE)).toContain("Served now");
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("a hub that can't hand the new types to devices: they are listed under Can't run, with Update the hub", async () => {
		const fake = await createFakeWorkspace();
		fake.hub.capabilities.eventTypes = false;
		const view = await kit.mountApp(mountDevices, SHOP, {}, { fake });
		await view.settle();
		await clickByText(/Can't run on devices/, view.container);
		expect(rowOf(view.container, ORDERS)).toContain(
			"This hub can't hand Endpoints to devices yet. Update the hub.",
		);
		expect(rowOf(view.container, FORM)).toContain(
			"This hub can't hand forms and quick actions to devices yet. Update the hub.",
		);
		expect(rowOf(view.container, TELEGRAM)).toContain(
			"This hub can't hand bots to devices yet. Update the hub.",
		);
		expect(checkbox(view.container, ORDERS)?.disabled).toBe(true);
		// One-time schedules need nothing new from the hub.
		expect(checkbox(view.container, ONCE)?.disabled).toBe(false);
	});
});

describe("What · events that follow Latest", () => {
	const REVIEW = "evt_invoice_review";
	const rowText = (root: ParentNode) =>
		text(
			root.querySelector<HTMLElement>(`[data-event="${REVIEW}"]`) ?? undefined,
		);

	test("it is offered like any other event, and says which flow version the deploy takes", async () => {
		const view = await kit.mountApp(mountDevices, INVOICE);
		expect(checkbox(view.container, REVIEW)?.getAttribute("data-state")).toBe(
			"checked",
		);
		const row = rowText(view.container);
		expect(row).toContain("event 0.9.0 · flow as it is now");
		expect(row).toContain("Follows Latest");
		expect(row).toContain("Deploys flow 0.9.2.");
		expect(row).not.toContain("Preparing creates");
	});

	test("flow edits no version holds: the row says that Preparing creates the version", async () => {
		const fake = await createFakeWorkspace();
		fake.api.hub.flows.edit(INVOICE, "flow_review");
		const view = await kit.mountApp(mountDevices, INVOICE, {}, { fake });
		const row = rowText(view.container);
		expect(row).toContain(
			"Preparing creates a flow version from the current edits. It stays in the flow's history.",
		);
		expect(row).not.toContain("Deploys flow");
		expect(checkbox(view.container, REVIEW)?.disabled).toBe(true);
		expect(checkbox(view.container, REVIEW)?.getAttribute("data-state")).toBe(
			"checked",
		);
	});

	test("a role that can't create a flow version can't deploy flow edits, and is told who can", async () => {
		const fake = await createFakeWorkspace();
		fake.api.hub.flows.edit(INVOICE, "flow_review");
		const view = await kit.mountApp(
			mountDevices,
			INVOICE,
			{},
			{
				fake,
				backend: {
					roleState: {
						// Read boards and Write events, no Write boards.
						getOwnRole: async () => ({
							role_id: "role-operator",
							role_name: "Operator",
							permissions: 256 + 16_384,
							is_owner: false,
							can_leave: true,
						}),
					},
				} as never,
			},
		);
		await clickByText(/Can't run on devices · 2/, view.container);
		expect(rowText(view.container)).toContain(
			"This flow has edits that aren't published as a version, and your role can't create one. Ask someone who can edit the app to create a version.",
		);
		expect(checkbox(view.container, REVIEW)?.disabled).toBe(true);
	});

	test("a hub that can't deploy Latest yet says so and points at Events", async () => {
		const view = await kit.mountApp(
			mountDevices,
			INVOICE,
			{},
			{
				hubVersion: "old",
			},
		);
		await clickByText(/Can't run on devices/, view.container);
		expect(rowText(view.container)).toContain(
			"This hub can't deploy events that follow Latest yet. Update the hub, or pin a flow version in Events.",
		);
	});
});

describe("What · services (A5)", () => {
	test("one service by default; a background event limits it to one instance", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const id = view.container.querySelector<HTMLInputElement>(
			"#deploy-service-id-main",
		);
		expect(id?.value).toBe("visitor-check-in");
		expect(text(view.container)).toContain(
			"Badge printer runs on its own, so this service runs 1 instance. Only services with a Page, chat or Endpoint can run several.",
		);
	});

	test("one service per event gives each its own ID", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		await click(byRole("radio", /One service per event/, view.container));
		const ids = Array.from(
			view.container.querySelectorAll<HTMLInputElement>(
				"input[id^=deploy-service-id-]",
			),
		).map((input) => input.value);
		expect(ids).toEqual(["check-in-page", "badge-printer"]);
		expect(summary()).toContain("2 events · 2 services");
	});

	test("an ID the device would refuse is said at the field and stops Continue", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		const id = view.container.querySelector(
			"#deploy-service-id-main",
		) as HTMLInputElement;
		await typeInto(id, "not a valid id");
		expect(id.getAttribute("aria-invalid")).toBe("true");
		expect(text(view.container)).toContain(
			"A service ID has 1 to 128 letters, digits, dots, dashes or underscores.",
		);
		expect(kit.footBlocking(view.container)).toContain("A service ID has");
		await typeInto(id, "visitors");
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.savedText()).toContain('"main":"visitors"');
	});

	test("two services can't share an ID", async () => {
		const view = await kit.mountApp(mountDevices, VISITOR);
		await click(byRole("radio", /One service per event/, view.container));
		const [, second] = Array.from(
			view.container.querySelectorAll<HTMLInputElement>(
				"input[id^=deploy-service-id-]",
			),
		);
		await typeInto(second, "check-in-page");
		expect(kit.footBlocking(view.container)).toBe(
			"Each service needs its own ID.",
		);
	});
});

describe("What · the viewer's role on the app (APP §3.5 item 1)", () => {
	const READ_TEAM = 4;
	const READ_BOARDS = 256;

	test("only the Owner permission names the app's owner: the hub says `is_owner` for an Admin too", async () => {
		const { roleFacts } = await import("../use-deploy-reads");
		const role = (permissions: number, is_owner: boolean) => ({
			role_id: "role",
			role_name: "Role",
			permissions,
			is_owner,
			can_leave: true,
		});
		const all = { canEditEvents: true, canEditFlows: true };
		expect(roleFacts(role(1, true))).toEqual({
			canReadFlows: true,
			isOwner: true,
			...all,
		});
		expect(roleFacts(role(2, true))).toEqual({
			canReadFlows: true,
			isOwner: false,
			...all,
		});
		// Reading the flows gives neither the right to move a schedule nor to create a flow version.
		expect(roleFacts(role(READ_TEAM + READ_BOARDS, false))).toEqual({
			canReadFlows: true,
			isOwner: false,
			canEditEvents: false,
			canEditFlows: false,
		});
		expect(roleFacts(role(READ_BOARDS + 1024 + 16_384, false))).toMatchObject({
			canEditEvents: true,
			canEditFlows: true,
		});
		// No role answer (local-only app, signed out): nothing is claimed either way.
		expect(roleFacts(null)).toEqual({ canReadFlows: true });
	});
	/** The hub's role answer per app; every other app can be read. */
	const roles = (unreadable: string) =>
		({
			roleState: {
				getOwnRole: async (appId: string) => ({
					role_id: "role-member",
					role_name: "Member",
					permissions:
						appId === unreadable ? READ_TEAM : READ_TEAM + READ_BOARDS,
					is_owner: false,
					can_leave: true,
				}),
			},
		}) as never;

	test("a role without Read boards can't deploy the app: said in place, Continue off, nothing sent", async () => {
		const view = await kit.mountApp(
			mountDevices,
			VISITOR,
			{},
			{ backend: roles(VISITOR) },
		);
		const page = text(view.container);
		expect(page).toContain(
			"Your role can't read the flows of Visitor Check-in",
		);
		expect(page).toContain(
			"Deploying needs Read boards on the app. Ask its owner for it.",
		);
		expect(checkbox(view.container, "evt_visitor_page")).toBeNull();
		expect(kit.footBlocking(view.container)).toBe(
			"Your role can't read the flows of Visitor Check-in.",
		);
		const calls = view.fake.api.calls.length;
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);
		expect(view.fake.api.calls.length).toBe(calls);
		expect(kit.primaries(view.container)).toBe(1);
	});

	test("device-first: an app the role can't read is listed, not offered", async () => {
		const view = await kit.mountAccount(
			mountDevices,
			{ device: EDGE },
			{ backend: roles("app_partner_reports") },
		);
		expect(
			byRole("radio", /Partner Reports/, view.container).hasAttribute(
				"disabled",
			),
		).toBe(true);
		expect(text(view.container)).toContain(
			"Your role can't read the flows of Partner Reports.",
		);
		expect(
			byRole("radio", /Visitor Check-in/, view.container).hasAttribute(
				"disabled",
			),
		).toBe(false);
	});

	test("a hub that answers no role leaves every app open", async () => {
		const view = await kit.mountAccount(mountDevices, { device: EDGE });
		expect(
			byRole("radio", /Partner Reports/, view.container).hasAttribute(
				"disabled",
			),
		).toBe(false);
	});
});

describe("What · update (APP §3.5 item 6, §3.14)", () => {
	const update = () =>
		kit.mountApp(mountDevices, CRM, {
			mode: undefined,
			device: EDGE,
			service: "nightly-sync",
		});

	test("keeping the version says that nothing is uploaded", async () => {
		const view = await update();
		await click(byRole("radio", /Keep each service's version/, view.container));
		expect(summary()).toContain("CRM Sync · keeps each version · 1 service");
		expect(text(view.container)).toContain(
			"Only settings or events change; nothing is uploaded.",
		);
	});

	test("an event the update drops needs its acknowledgement", async () => {
		const view = await update();
		await click(checkbox(view.container, "evt_crm_nightly") as Element);
		await click(checkbox(view.container, "evt_crm_webhook") as Element);
		expect(text(view.container)).toContain(
			"Nightly CRM sync stops being served by nightly-sync on edge-berlin-01 after the update.",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"Confirm the events that stop being served after the update.",
		);
		await clickByText(
			"Stop serving these events after the update",
			view.container,
		);
		expect(kit.footBlocking(view.container)).toBeNull();
	});
});
