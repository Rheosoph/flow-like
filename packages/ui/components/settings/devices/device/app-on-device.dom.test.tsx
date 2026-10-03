import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const kit = await import("./device-test-kit");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { SHOP, serveShopOnEdge } = await import("../testing/schedule-scenarios");
const { formField } = await import("../testing/fake-runs");
const { useOverlayStore } = await import("../workspace/overlay-store");
type FakeEventForm = import("../testing/fake-runs").FakeEventForm;

const { IDS, MACHINE, lastNavigation, openDevice, text } = kit;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

/** Shop Assistant on edge-berlin-01; `form` replaces the fields its return form has (one of them a file). */
async function openShop(form?: FakeEventForm) {
	const fake = await createFakeWorkspace();
	if (form) fake.hub.eventForms[SHOP.form] = form;
	await serveShopOnEdge(fake);
	const view = await openDevice(IDS.edge, {
		tab: "services",
		app: SHOP.app,
		fake,
	});
	const app = view.container.querySelector<HTMLElement>("#dv-app");
	if (!app) throw new Error("no app block");
	const event = (eventId: string) => {
		const found = app.querySelector<HTMLElement>(`tr[data-event="${eventId}"]`);
		if (!found) throw new Error(`no row for ${eventId}`);
		return found;
	};
	return { ...view, app, event };
}

describe("an app's events on one device (design R2 §6.4 App › Devices)", () => {
	test("each new kind is a row with its type and its own line", async () => {
		const { app, event } = await openShop();
		expect(text(event(SHOP.orders))).toContain("OrdersEndpoint");
		expect(text(event(SHOP.orders))).toContain("GET /orders");
		expect(text(event(SHOP.form))).toContain("Return requestForm");
		expect(text(event(SHOP.form))).toContain(
			"Started by a person · from Devices or the service page",
		);
		expect(text(event(SHOP.telegram))).toContain(
			"Runs on its own · stays connected to Telegram",
		);
		expect(text(event(SHOP.once))).toContain(
			"Once on 2026-10-15 at 09:00 · Europe/Berlin",
		);
		expect(text(app)).toContain("Can't run on devices · 1 event");
		expect(text(app)).not.toMatch(MACHINE);
	});

	test("the service's cell says the bot's state and when the one-time schedule runs", async () => {
		const { event } = await openShop();
		const cell = (eventId: string) =>
			event(eventId).querySelector('[data-matrix-cell="served"]');
		expect(text(cell(SHOP.telegram) ?? undefined)).toContain(
			"Connected as Shop helper",
		);
		expect(text(cell(SHOP.discord) ?? undefined)).toContain(
			"Connected as Shop support",
		);
		expect(
			cell(SHOP.once)?.querySelector("[data-schedule-run]")?.textContent,
		).toMatch(/^Runs once Oct 15 at 09:00 GMT\+2 · in 2 wk\./);
	});

	test("Run now… under the form, never under another kind", async () => {
		const view = await openShop({
			fields: [formField("order", { label: "Order number" })],
		});
		const before = lastNavigation(view);
		await click(byRole("button", "Run now…", view.event(SHOP.form)));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "run_now",
			deviceId: IDS.edge,
			serviceId: SHOP.service,
			eventId: SHOP.form,
		});
		expect(lastNavigation(view)).toEqual(before);
		for (const eventId of [SHOP.orders, SHOP.telegram, SHOP.once])
			expect(
				[
					eventId,
					queryByRole("button", "Run now…", view.event(eventId)),
				].join(),
			).toBe(`${eventId},`);
	});

	test("a form that takes a file: Run now… is off and points to the service page", async () => {
		const view = await openShop();
		const run = byRole("button", "Run now…", view.event(SHOP.form));
		expect(run.getAttribute("aria-disabled")).toBe("true");
		expect(text(view.event(SHOP.form))).toContain(
			"This form takes a file. Open it on the service page.",
		);
		await click(run);
		expect(useOverlayStore.getState().overlay.kind).not.toBe("run_now");
	});
});
