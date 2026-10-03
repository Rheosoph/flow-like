import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { installDom } from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, eventRecord, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { APPS, ROUTES } = await import(
	"../../../../lib/device-management/model/__fixtures__/apps"
);
const { SHOP, openShop, text } = await import("./status-test-kit");
const { EDGE, openTab, patchConfig, until } = await import("./config-test-kit");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

type Record = ReturnType<typeof eventRecord>;
const bytes = (value: unknown) => [
	...new TextEncoder().encode(JSON.stringify(value)),
];
const shopEvents = () => APPS.app_shop_assistant.events.map(eventRecord);

/**
 * The app's store as the hub has it: the events as Events lists them now, and
 * the version a service pins. `live` replaces the Orders route Events has now.
 */
function store(live?: unknown) {
	const current = (): Record[] =>
		shopEvents().map((event) =>
			event.id === SHOP.orders && live
				? { ...event, config: bytes(live) }
				: event,
		);
	return {
		eventState: {
			getEvents: async () => current(),
			getEventAuthoritative: async (_app: string, eventId: string) =>
				shopEvents().find((event) => event.id === eventId),
			isEventSinkActive: async () => false,
		},
	} as never;
}

const block = (root: ParentNode) =>
	root.querySelector<HTMLElement>("#svc-requests");
const request = (root: ParentNode, eventId: string) =>
	root.querySelector<HTMLElement>(`[data-request="${eventId}"]`);
const part = (root: ParentNode | null, name: string) =>
	text(root?.querySelector(`[data-request-${name}]`) ?? null);

describe("Service › Endpoint · Requests", () => {
	test("each Endpoint the service answers: its route, full address, a curl line, and who may call it", async () => {
		const { container } = await openShop({
			tab: "endpoint",
			events: [SHOP.orders, SHOP.form],
			mount: { backend: store() },
		});
		await until(() => request(container, SHOP.orders) !== null);
		const row = request(container, SHOP.orders) as HTMLElement;
		expect(text(row)).toContain("Orders");
		expect(part(row, "route")).toBe("GET /orders");
		expect(part(row, "address")).toBe("http://127.0.0.1:8480/orders");
		expect(part(row, "curl")).toBe(
			'curl -X GET -H "Authorization: Bearer <access token>" "http://127.0.0.1:8480/orders"',
		);
		expect(row.querySelector("[data-request-changed]")).toBeNull();
		expect(text(block(container))).toContain(
			"Every request needs this service's access token. The token set in Events is not used on a device.",
		);
		// The token Events has for the Endpoint never shows.
		expect(text(container)).not.toContain(ROUTES.orders.auth_token);
		// Only Endpoints answer requests here: the list comes before the page link.
		const blocks = [...container.querySelectorAll("[data-block]")].map(
			(node) => node.id,
		);
		expect(blocks.indexOf("svc-requests")).toBeLessThan(
			blocks.indexOf("svc-endpoint"),
		);
		expect(text(container.querySelector("[data-also-on-page]") ?? null)).toBe(
			"Also on the service page: 1 action or form",
		);
	});

	test("a route changed in Events since the deploy: the deployed one, with the new one beside it", async () => {
		const { container } = await openShop({
			tab: "endpoint",
			events: [SHOP.orders],
			mount: {
				backend: store({
					...ROUTES.orders,
					path: "/orders/v2",
					method: "POST",
				}),
			},
		});
		await until(() => request(container, SHOP.orders) !== null);
		const row = request(container, SHOP.orders) as HTMLElement;
		expect(part(row, "route")).toBe("GET /orders");
		expect(part(row, "changed")).toBe(
			"Events has POST /orders/v2 now. Update shop-assistant to answer that instead.",
		);
	});

	test("pinned versions that can't be read: the reason, never the live record", async () => {
		const unreadable = {
			eventState: {
				getEvents: async () => shopEvents(),
				getEventAuthoritative: async () => {
					throw new Error("The hub can't read that version.");
				},
				isEventSinkActive: async () => false,
			},
		} as never;
		const { container } = await openShop({
			tab: "endpoint",
			events: [SHOP.orders],
			mount: { backend: unreadable },
		});
		await until(() => block(container) !== null);
		await until(
			() => container.querySelector("[data-requests-unknown]") !== null,
		);
		expect(text(block(container))).toContain(
			"The paths of the deployed version can't be read here: reading them failed. Open this tab again to retry",
		);
		expect(request(container, SHOP.orders)).toBeNull();
		expect(text(block(container))).not.toContain("/orders");
	});

	test("a local-only service: the routes of the copy the device holds, after the page link", async () => {
		const view = await openTab({
			tab: "endpoint",
			device: EDGE,
			service: "support-bot",
		});
		const { container } = view;
		await until(() => request(container, "evt_support_http") !== null);
		const row = request(container, "evt_support_http") as HTMLElement;
		expect(text(row)).toContain("Support API");
		expect(part(row, "route")).toBe("POST /support");
		// All networks, and no address saved on this computer: the path is known, the host isn't.
		expect(part(row, "address")).toBe("https://…:8443/support");
		expect(part(row, "curl")).toContain(
			`-H "Content-Type: application/json" -d '{}' "https://…:8443/support"`,
		);
		expect(text(block(container))).toContain(
			"Enter the address people use under Service page to copy full addresses.",
		);
		const blocks = [...container.querySelectorAll("[data-block]")].map(
			(node) => node.id,
		);
		expect(blocks.indexOf("svc-endpoint")).toBeLessThan(
			blocks.indexOf("svc-requests"),
		);
	});

	test("a service without Endpoints has no Requests", async () => {
		const { container } = await openShop({
			tab: "endpoint",
			events: [SHOP.form],
			mount: { backend: store() },
		});
		await until(() => container.querySelector("#svc-endpoint") !== null);
		expect(block(container)).toBeNull();
	});
});

describe("Service › Endpoint · a service without a web endpoint", () => {
	test("its events by their type names, and where its actions and forms run", async () => {
		const { container } = await openShop({
			tab: "endpoint",
			events: [SHOP.form, SHOP.telegram],
			// Only person-started events and a bot: no web endpoint (design R2 §4.1).
			arrange: (fake) =>
				patchConfig(fake, SHOP.device, SHOP.service, (config) => {
					config.hosting = null;
				}),
		});
		await until(() => container.querySelector("#svc-endpoint") !== null);
		const page = text(container.querySelector("#svc-endpoint"));
		expect(page).toContain("This service has no web endpoint.");
		expect(page).toContain(
			"Its events run as Form and Telegram bot, so it has no service page.",
		);
		expect(page).toContain(
			"Run its actions and forms from Devices, under Status.",
		);
	});
});
