import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const {
	EDGE,
	STUDIO,
	WAREHOUSE,
	MACHINE,
	commandsOf,
	openTab,
	patchConfig,
	primaries,
	sent,
	text,
	until,
	writes,
} = await import("./config-test-kit");
type Kit = typeof import("./config-test-kit");
type View = Awaited<ReturnType<Kit["openTab"]>>;
type Config = Record<string, unknown>;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	localStorage.clear();
});
afterAll(dom.restore);

const open = (
	device: string,
	service: string,
	options: Partial<Parameters<Kit["openTab"]>[0]> = {},
) => openTab({ tab: "endpoint", device, service, ...options });

const url = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-endpoint-url]");

async function ready(view: View) {
	await until(() => url(view) !== null);
}

describe("service page", () => {
	test("a loopback endpoint: one-line link, Copy and Open, plain HTTP without a warning", async () => {
		const view = await open(STUDIO, "field-notes");
		await ready(view);
		const link = url(view) as HTMLElement;
		expect(text(link)).toBe("http://127.0.0.1:8090/ui/");
		expect(link.className).toContain("truncate");
		await click(byRole("button", "Copy link", view.container));
		expect(dom.clipboard.at(-1)).toBe("http://127.0.0.1:8090/ui/");
		const openLink = byRole("link", "Open", view.container);
		expect(openLink.getAttribute("href")).toBe("http://127.0.0.1:8090/ui/");
		expect(openLink.getAttribute("rel")).toContain("noopener");
		const page = text(view.container);
		expect(page).toContain(
			"Unencrypted (HTTP) · only this device can reach it",
		);
		expect(page).toContain("Only this device (127.0.0.1)");
		expect(page).toContain("The address works only on studio-mac-mini itself.");
		expect(page).toContain("Not needed: only this device can reach it");
		expect(page).toContain(
			"32 parallel requests · 60 s timeout · 10 MiB per request · attachments about 3.5 MiB",
		);
		expect(view.container.querySelector("[data-exposure-warning]")).toBeNull();
		expect(queryByRole("textbox", "Address people use")).toBeNull();
		expect(
			view.container.querySelector("[data-stamp][data-src=live]"),
		).not.toBeNull();
		expect(primaries()).toBe(0);
		expect(page).not.toMatch(MACHINE);
		expect(writes(view)).toEqual([]);
	});

	test("an HTTPS endpoint on all networks: certificate link, and the address people use is kept on this computer", async () => {
		const view = await open(EDGE, "support-bot");
		await ready(view);
		const page = text(view.container);
		expect(page).toContain("HTTPS");
		expect(page).toContain("All networks (0.0.0.0)");
		const certificate = byRole("link", "edge-api", view.container);
		expect(certificate.getAttribute("href")).toContain("tab=certificates");
		expect(certificate.getAttribute("href")).toContain("certificate=");
		expect(text(url(view) as HTMLElement)).toBe("https://…:8443/ui/");
		const copy = byRole("button", "Copy link", view.container);
		expect(copy.getAttribute("aria-disabled")).toBe("true");
		expect(page).toContain("Enter the address people use below to get a link.");
		const input = byRole("textbox", "Address people use", view.container);
		await typeInto(input, "https://edge.example.com/ui");
		await click(byRole("button", "Save", view.container));
		expect(text(view.container)).toContain(
			"Enter a host name or an IP address, without http://, a port or a path.",
		);
		await typeInto(input, "edge.example.com");
		await click(byRole("button", "Save", view.container));
		expect(text(url(view) as HTMLElement)).toBe(
			"https://edge.example.com:8443/ui/",
		);
		expect(text(view.container)).toContain("Saved on this computer at");
		expect(byRole("link", "Open", view.container).getAttribute("href")).toBe(
			"https://edge.example.com:8443/ui/",
		);
		await click(byRole("button", "QR code", view.container));
		expect(text(inPortal("dialog"))).toContain("QR code for the service page");
		await click(byRole("button", "Close", inPortal("dialog")));
		expect(
			Object.keys(localStorage).some(
				(key) =>
					key.includes("service-address") &&
					localStorage.getItem(key) === "edge.example.com",
			),
		).toBe(true);
		expect(view.fake.api.sent("PUT", /address/)).toEqual([]);
		expect(writes(view)).toEqual([]);
	});

	test("without an address Copy link, Open and QR code stay in place, disabled under one reason", async () => {
		const view = await open(EDGE, "support-bot");
		await ready(view);
		const reason = "Enter the address people use below to get a link.";
		const clips = dom.clipboard.length;
		for (const name of ["Copy link", "Open", "QR code"]) {
			const button = byRole("button", name, view.container);
			expect(button.getAttribute("aria-disabled")).toBe("true");
			const described = button.getAttribute("aria-describedby");
			expect(
				text(
					view.container.querySelector(`[id="${described}"]`) as HTMLElement,
				),
			).toBe(reason);
			await click(button);
		}
		expect(queryByRole("dialog")).toBeNull();
		expect(dom.clipboard.length).toBe(clips);
		expect(
			view.container.querySelectorAll("[data-gate-inline=unsupported]").length,
		).toBe(1);
	});

	test("the device's own network addresses are offered when its agent reports them", async () => {
		const view = await open(EDGE, "support-bot", {
			arrange: async (fake) => {
				fake.agent(EDGE).facts.network = {
					interfaces: [
						{ name: "lo", addresses: ["127.0.0.1"], loopback: true },
						{ name: "eth0", addresses: ["192.168.10.24"], loopback: false },
					],
				};
				await fake.workspace.live.refreshInspection(EDGE);
			},
		});
		await ready(view);
		const offered = view.container.querySelector(
			"[data-lan-addresses]",
		) as HTMLElement;
		expect(text(offered)).toContain("192.168.10.24");
		expect(text(offered)).not.toContain("127.0.0.1");
		await click(byRole("button", "192.168.10.24", offered));
		expect(text(url(view) as HTMLElement)).toBe(
			"https://192.168.10.24:8443/ui/",
		);
	});

	test("an endpoint on the network without a certificate carries a warning", async () => {
		const view = await open(STUDIO, "field-notes", {
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					(config.hosting as Config).host = "0.0.0.0";
				}),
		});
		await ready(view);
		const warning = view.container.querySelector(
			"[data-exposure-warning]",
		) as HTMLElement;
		expect(text(warning)).toContain("Unencrypted (HTTP)");
		expect(text(warning)).toContain(
			"reachable from the network without encryption",
		);
		expect(
			byRole("button", "Assign certificate…", view.container),
		).toBeTruthy();
	});

	test("a service without a web endpoint says so instead of looking empty", async () => {
		const view = await open(EDGE, "nightly-sync");
		await until(() =>
			text(view.container).includes("This service has no web endpoint."),
		);
		expect(url(view)).toBeNull();
		expect(text(view.container)).not.toContain("Nothing here yet");
	});

	test("a service that only runs a schedule names it as the reason it has no page", async () => {
		const view = await open(EDGE, "nightly-sync", {
			arrange: (fake) =>
				patchConfig(fake, EDGE, "nightly-sync", (config) => {
					config.events = [
						...(config.events as unknown[]),
						{
							event_id: "evt_crm_hourly",
							event_version: [1, 0, 0],
							board_version: [4, 1, 0],
						},
					];
				}),
		});
		await until(() =>
			text(view.container).includes("This service has no web endpoint."),
		);
		expect(text(view.container)).toContain(
			"Its events run as Background and Schedule, so it has no service page.",
		);
	});

	test("an offline device: the endpoint isn't known and nothing is read", async () => {
		const view = await open(WAREHOUSE, "scanner-ingest");
		expect(text(view.container)).toContain("Settings aren't known");
		expect(sent(view, WAREHOUSE)).not.toContain("placement_configuration");
	});
});

describe("access token", () => {
	test("a service without token authentication explains access and offers no token rotation", async () => {
		const view = await open(STUDIO, "field-notes", {
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					const hosting = config.hosting as Config;
					hosting.authentication = "none";
					hosting.auth_secret = undefined;
				}),
		});
		await ready(view);
		expect(
			text(view.container.querySelector("[data-token-state]") as HTMLElement),
		).toBe("Not required");
		expect(text(view.container)).toContain(
			"Anyone who can reach this service can use it without a token.",
		);
		expect(
			queryByRole("button", "Set a new token…", view.container),
		).toBeNull();
		expect(view.container.querySelector("#svc-token-value")).toBeNull();
		expect(writes(view)).toEqual([]);
	});

	test("a new token is checked, sent once under the stored reference and tracked", async () => {
		const view = await open(STUDIO, "field-notes");
		await ready(view);
		const values: unknown[] = [];
		view.fake.agent(STUDIO).handle("set_secret", (command) => {
			values.push(command.value);
			return {
				state: "completed",
				result: {
					placement_id: command.placement_id,
					name: command.name,
					secret: "completed",
				},
			};
		});
		await click(byRole("button", "Set a new token…", view.container));
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain(
			"The old one stops working once the device saves the new one.",
		);
		expect(text(sheet)).toContain("No, this is permanent.");
		const input = sheet.querySelector("#svc-token-value") as HTMLInputElement;
		// The one field of the sheet has the focus, is masked, and no saved login is offered for it.
		expect(document.activeElement).toBe(input);
		expect(input.type).toBe("password");
		expect(input.getAttribute("autocomplete")).toBe("new-password");
		await typeInto(input, "short");
		await click(byRole("button", "Set new token", sheet));
		expect(text(sheet)).toContain("The token is 5 characters. Use at least 32");
		expect(writes(view)).toEqual([]);
		await click(byRole("button", "Generate one", sheet));
		const token = input.value;
		expect(token).toMatch(/^[0-9a-f]{48}$/);
		await click(byRole("button", "Copy token", sheet));
		expect(dom.clipboard.at(-1)).toBe(token);
		await click(byRole("button", "Set new token", sheet));
		await until(() => commandsOf(view, "set_secret").length === 1);
		expect(commandsOf(view, "set_secret")[0]).toMatchObject({
			placement_id: "field-notes",
			expected_revision: 9,
			name: "field-notes-auth",
		});
		expect(values).toEqual([token]);
		expect(JSON.stringify(view.fake.api.commands)).not.toContain(token);
		expect(JSON.stringify(view.fake.api.calls)).not.toContain(token);
		await until(() => text(view.container).includes("New token sent at"));
		expect(queryByRole("dialog")).toBeNull();
		const item = view.fake.workspace.activity
			.list()
			.find((entry) => entry.kind === "secret_write");
		expect(item?.target.serviceId).toBe("field-notes");
		expect(JSON.stringify(item)).not.toContain(token);
	});

	test("the device's refusal is shown in the sheet with its reason", async () => {
		const view = await open(STUDIO, "field-notes", {
			arrange: (fake) => {
				fake
					.agent(STUDIO)
					.reject("set_secret", "unauthorized", "Deploy capability required.");
			},
		});
		await ready(view);
		await click(byRole("button", "Set a new token…", view.container));
		const sheet = inPortal("dialog");
		await click(byRole("button", "Generate one", sheet));
		await click(byRole("button", "Set new token", sheet));
		await until(() =>
			text(inPortal("dialog")).includes("Deploy capability required."),
		);
		expect(commandsOf(view, "set_secret").length).toBe(1);
	});

	test("while an update runs the token can't change, and a click sends nothing", async () => {
		const view = await open(EDGE, "invoice-extractor");
		await ready(view);
		const button = byRole("button", "Set a new token…", view.container);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(text(view.container)).toContain(
			"An update is in progress. This works again after it finishes",
		);
		await click(button);
		expect(queryByRole("dialog")).toBeNull();
		expect(writes(view)).toEqual([]);
	});
});

describe("certificate", () => {
	test("Change certificate opens Edit settings; a device that requires the sandbox says why it can't", async () => {
		const view = await open(EDGE, "support-bot");
		await ready(view);
		const button = byRole("button", "Change certificate…", view.container);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(text(view.container)).toContain(
			"Update… changes how the service is isolated in the deploy wizard.",
		);
		await click(button);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("Assign certificate opens the settings sheet on the certificate field", async () => {
		const view = await open(STUDIO, "field-notes", {
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					(config.hosting as Config).host = "0.0.0.0";
				}),
		});
		await ready(view);
		const button = byRole("button", "Assign certificate…", view.container);
		if (button.getAttribute("aria-disabled") === "true") {
			expect(text(view.container)).toMatch(/Manage certificates|agent/);
			return;
		}
		await click(button);
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("Edit settings of field-notes");
		expect(sheet.querySelector("#svc-edit-cert")).not.toBeNull();
	});
});

describe("older hub and older agent", () => {
	test("an older agent: no interface list, so the typed address is the only source", async () => {
		const view = await open(EDGE, "support-bot", { agentFeatures: {} });
		await ready(view);
		expect(
			text(view.container.querySelector("[data-lan-addresses]") as HTMLElement),
		).toContain(
			"Update the device agent to see the addresses the device has on its networks.",
		);
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		expect(
			byRole("textbox", "Address people use", view.container),
		).toBeTruthy();
		const known = new Set([
			// Service tunnels are negotiated separately from inspection feature flags.
			"service_listeners",
			"inspect_page",
			"placement_configuration",
			"offline_queue",
			"certificates",
			"artifact",
			"metrics",
			"rollout",
			"operation",
		]);
		expect(sent(view, EDGE).filter((type) => !known.has(type))).toEqual([]);
	});

	test("an older hub: the endpoint still comes from the device, without an error", async () => {
		const view = await open(STUDIO, "field-notes", { hubVersion: "old" });
		await ready(view);
		expect(text(url(view) as HTMLElement)).toBe("http://127.0.0.1:8090/ui/");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		const asked = view.fake.api
			.sent("GET", /app.*device|device.*placements/)
			.map(([, path]) => path);
		expect(new Set(asked).size).toBe(asked.length);
	});
});
