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
		await clickByText(/Can't run on devices · 2/, view.container);
		const page = text(view.container);
		expect(page).toContain("Not offered when you deploy");
		expect(page).toContain("Schedules can't run on a device yet.");
		const row = view.container.querySelector('[data-event="evt_crm_hourly"]');
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

	test("an event that follows the latest flow links to Events to pin it", async () => {
		const view = await kit.mountApp(mountDevices, INVOICE);
		await clickByText(/Can't run on devices/, view.container);
		const fix = byRole("link", "Pin a flow version in Events", view.container);
		expect(fix.getAttribute("href")).toBe(
			`/library/config/events?id=${INVOICE}&event=evt_invoice_review`,
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
			"Badge printer runs on its own, so this service runs 1 instance. Only Web request, Chat and Page events can run several.",
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
