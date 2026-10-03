import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { DeploymentVariable } from "../../../../../lib/device-management/deployment";
import {
	byRole,
	click,
	clickByText,
	installDom,
	queryByRole,
	typeInto,
} from "../../testing/dom-harness";
import type { FakeWorkspace } from "../../testing/fake-workspace";

const dom = installDom();
const { mountDevices, cleanupDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
const { createFakeWorkspace } = await import("../../testing/fake-workspace");
const { serveShopOnEdge } = await import("../../testing/schedule-scenarios");
const kit = await import("../deploy-test-kit");
const { EDGE, STUDIO, VISITOR, CRM, text } = kit;
await preloadDevices();

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	globalThis.sessionStorage.clear();
});
afterAll(dom.restore);

const row = (root: ParentNode, variableId: string) =>
	root.querySelector<HTMLElement>(
		`[data-variable="${variableId}"]`,
	) as HTMLElement;
const toggle = (root: ParentNode, id: string) =>
	click(root.querySelector(`#${id}`) as Element);
const summary = () => text(byRole("region", "Your choices"));
const settings = (appId: string, params = {}, options = {}) =>
	kit.mountApp(mountDevices, appId, { step: "settings", ...params }, options);

describe("Settings · new services (APP §3.8)", () => {
	test("each variable says which events use it and starts on the app's default", async () => {
		const view = await settings(VISITOR, { device: [EDGE, STUDIO] });
		const site = text(row(view.container, "var_site_name"));
		expect(site).toContain("Site name");
		expect(site).toContain("Used by Check-in page, Badge printer");
		expect(site).toContain("Uses the app's default.");
		const token = text(row(view.container, "var_host_token"));
		expect(token).toContain("Text · secret");
		expect(token).toContain("Not set. The flow gets no value.");
		expect(summary()).toContain("3 app defaults");
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.primaries(view.container)).toBe(1);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("app-settings: one value for the service, another one on one device", async () => {
		const view = await settings(VISITOR, { device: [EDGE, STUDIO] });
		await toggle(view.container, "deploy-var-var_printer_url-on");
		await typeInto(
			view.container.querySelector("#deploy-var-var_printer_url") as Element,
			"ipp://10.0.4.20/print",
		);
		await toggle(view.container, "deploy-var-var_printer_url-differs");
		const printer = row(view.container, "var_printer_url");
		expect(text(printer)).toContain("edge-berlin-01Same as shared");
		const [, studio] = Array.from(
			printer.querySelectorAll<HTMLButtonElement>("ul button"),
		);
		expect(studio.textContent).toBe("Set for this device");
		await click(studio);
		await typeInto(
			byRole("textbox", "Badge printer address on studio-mac-mini", printer),
			"ipp://192.168.1.40/print",
		);
		expect(summary()).toContain(
			"1 value · 1 differs by device · 2 app defaults",
		);

		const saved = kit.savedText();
		expect(saved).toContain("ipp://10.0.4.20/print");
		expect(saved).toContain("ipp://192.168.1.40/print");

		await clickByText("Use shared", printer);
		expect(summary()).toContain("1 value · 2 app defaults");
	});

	test("a secret is typed here and never kept in this window's saved progress", async () => {
		const view = await settings(VISITOR, { device: [EDGE, STUDIO] });
		await toggle(view.container, "deploy-var-var_host_token-on");
		const secret = "host-directory-token-0123456789";
		await typeInto(
			view.container.querySelector("#deploy-secret-var_host_token") as Element,
			secret,
		);
		expect(summary()).toContain("1 secret · 2 app defaults");
		expect(kit.savedText()).not.toContain(secret);
		expect(kit.savedText()).toContain("var_host_token");

		// Each device can get its own secret.
		await clickByText("Different per device", view.container);
		const token = row(view.container, "var_host_token");
		const own = "studio-only-token-abcdefghijklmnop";
		await typeInto(
			token.querySelector(`#deploy-secret-var_host_token-${STUDIO}`) as Element,
			own,
		);
		expect(
			token.querySelector(`#deploy-secret-var_host_token-${EDGE}`),
		).toBeTruthy();
		expect(kit.savedText()).not.toContain(own);
	});

	test("a value its type refuses is said at the variable and stops Continue", async () => {
		const fake = await createFakeWorkspace();
		const retries: DeploymentVariable = {
			id: "var_retries",
			name: "Retries",
			data_type: "Integer",
			value_type: "Normal",
			secret: false,
		};
		fake.hub.appVariables = {
			...fake.hub.appVariables,
			evt_visitor_page: [
				...(fake.hub.appVariables.evt_visitor_page ?? []),
				retries,
			],
		};
		const view = await settings(VISITOR, { device: EDGE }, { fake });
		expect(text(row(view.container, "var_retries"))).toContain("Number");
		await toggle(view.container, "deploy-var-var_retries-on");
		await typeInto(
			view.container.querySelector("#deploy-var-var_retries") as Element,
			"three",
		);
		expect(text(row(view.container, "var_retries"))).toContain(
			"edge-berlin-01: Retries isn't a valid value.",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"edge-berlin-01: Retries isn't a valid value.",
		);
		await typeInto(
			view.container.querySelector("#deploy-var-var_retries") as Element,
			"3",
		);
		expect(kit.footBlocking(view.container)).toBeNull();
	});

	test("a value typed for an event that is no longer picked isn't part of the plan", async () => {
		const view = await settings(VISITOR, { device: EDGE });
		await toggle(view.container, "deploy-var-var_printer_url-on");
		await typeInto(
			view.container.querySelector("#deploy-var-var_printer_url") as Element,
			"ipp://10.0.4.20/print",
		);
		expect(summary()).toContain("1 value");
		await click(
			byRole("button", /What to run/, byRole("region", "Your choices")),
		);
		await view.settle();
		await clickByText("Choose events", view.container);
		await click(
			view.container.querySelector(
				"#deploy-event-evt_badge_printer",
			) as Element,
		);
		await click(byRole("button", /Settings/, byRole("region", "Your choices")));
		await view.settle();
		// Badge printer was the only event that uses the address.
		expect(
			view.container.querySelector('[data-variable="var_printer_url"]'),
		).toBeNull();
		expect(summary()).toContain("2 app defaults");
		expect(summary()).not.toContain("1 value");
	});

	test("a local-only app's settings wait for the copy: said as not loaded, never as empty", async () => {
		const view = await settings(
			CRM,
			{ device: STUDIO },
			{ platform: "desktop" },
		);
		const state = view.container.querySelector('[data-kind="notloaded"]');
		expect(text(state as HTMLElement)).toContain(
			"Settings show once the copy is on a device",
		);
		expect(view.container.querySelector('[data-kind="empty"]')).toBeNull();
		await clickByText("Go to Copy & upload", view.container);
		expect(view.navigations.at(-1)?.href).toContain("step=copy_upload");
	});
});

const NOTES = "app_field_notes";
const GREETING: DeploymentVariable = {
	id: "var_greeting",
	name: "Greeting",
	data_type: "String",
	value_type: "Normal",
	secret: false,
};
const API_KEY: DeploymentVariable = {
	id: "var_api_key",
	name: "API key",
	data_type: "String",
	value_type: "Normal",
	secret: true,
};
const COMPACT: DeploymentVariable = {
	id: "var_compact",
	name: "Compact",
	data_type: "Boolean",
	value_type: "Normal",
	secret: false,
};

/** field-notes on studio-mac-mini with stored values: one fine, one of the wrong type, one nobody uses, one secret. */
async function storedFieldNotes(): Promise<FakeWorkspace> {
	const fake = await createFakeWorkspace();
	fake.hub.appVariables = {
		...fake.hub.appVariables,
		evt_notes_http: [GREETING, API_KEY, COMPACT],
	};
	const agent = fake.api.agent(STUDIO);
	const placement = agent.placement("field-notes");
	if (!placement)
		throw new Error("The sample fleet has no field-notes service");
	agent.configs.set("field-notes", {
		id: "field-notes",
		project_id: NOTES,
		deployment_id: placement.deployment_id,
		revision: placement.revision,
		source: "online",
		project_path: `projects/${NOTES}`,
		online_metadata_sha256: "a".repeat(64),
		events: [
			{
				event_id: "evt_notes_http",
				event_version: [2, 0, 0],
				board_version: [3, 0, 0],
			},
		],
		hosting: {
			host: "127.0.0.1",
			port: 8090,
			max_in_flight: 64,
			request_timeout_secs: 300,
			auth_secret: "service-token",
		},
		tls_certificate_id: null,
		max_replicas: 1,
		variables: {
			var_greeting: "Hello",
			var_compact: "yes",
			var_obsolete: "old value",
		},
		secret_overrides: { var_api_key: "variable-1" },
		resource_grant: {
			grant_id: "6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73",
			authz_version: 1,
		},
		offline_writes: null,
		resources: null,
	});
	return fake;
}

describe("Settings · updates", () => {
	const update = (fake: FakeWorkspace) =>
		settings(
			NOTES,
			{ mode: undefined, device: STUDIO, service: "field-notes" },
			{ fake },
		);

	test("current values are prefilled and a stored secret is kept unless replaced", async () => {
		const view = await update(await storedFieldNotes());
		expect(text(view.container)).toContain("Settings for field-notes");
		expect(
			view.container.querySelector<HTMLInputElement>("#deploy-var-var_greeting")
				?.value,
		).toBe("Hello");
		const key = row(view.container, "var_api_key");
		expect(
			byRole("button", "Keep stored", key).getAttribute("aria-pressed"),
		).toBe("true");
		expect(text(key)).toContain(
			"Stored secret · can't be read back, not even by you.",
		);
		expect(key.querySelector("input")).toBeNull();

		await clickByText("Set new", key);
		const secret = "a-new-api-key-0123456789";
		await typeInto(
			key.querySelector("#deploy-secret-var_api_key") as Element,
			secret,
		);
		expect(kit.savedText()).not.toContain(secret);
	});

	test("stored values that no longer fit must be replaced or removed before Continue", async () => {
		const view = await update(await storedFieldNotes());
		const list = byRole(
			"list",
			"Stored values that need a decision",
			view.container,
		);
		expect(
			Array.from(list.querySelectorAll("[data-unresolved]")).map((item) =>
				item.getAttribute("data-unresolved"),
			),
		).toEqual(["invalid", "outside"]);
		expect(text(list)).toContain(
			"Compact · studio-mac-miniThe stored value doesn't fit its type any more.",
		);
		expect(text(list)).toContain(
			"Set for an event this update no longer serves.",
		);
		expect(kit.footBlocking(view.container)).toBe(
			"2 stored values no longer fit this update. Replace or remove them.",
		);
		const before = view.navigations.length;
		await clickByText("Continue", view.container);
		expect(view.navigations.length).toBe(before);

		// Removing is explicit, and it can be taken back.
		const outside = list.querySelector('[data-unresolved="outside"]');
		await clickByText("Remove value", outside as HTMLElement);
		expect(kit.footBlocking(view.container)).toBe(
			"A stored value no longer fits this update. Replace or remove it.",
		);
		expect(text(view.container)).toContain(
			"After the update it uses the app's default.",
		);
		expect(byRole("button", "Keep it", view.container)).toBeTruthy();

		// A new value of the right type replaces the stored one.
		await clickByText("Yes", row(view.container, "var_compact"));
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("switching a stored value off removes it with the update, and Keep it restores it", async () => {
		const view = await update(await storedFieldNotes());
		await toggle(view.container, "deploy-var-var_greeting-on");
		expect(text(row(view.container, "var_greeting"))).toContain(
			"Uses the app's default.",
		);
		const removed = byRole("region", /Removed overrides/, view.container);
		expect(text(removed)).toContain("Greeting · studio-mac-mini");
		const keep = Array.from(removed.querySelectorAll("button")).find(
			(button) => button.textContent === "Keep it",
		);
		await click(keep as Element);
		expect(
			view.container.querySelector<HTMLInputElement>("#deploy-var-var_greeting")
				?.value,
		).toBe("Hello");
	});

	test("an offline service's settings come from the device that runs it", async () => {
		const fake = await createFakeWorkspace(undefined, { platform: "desktop" });
		const batch: DeploymentVariable = {
			id: "var_batch_size",
			name: "Batch size",
			data_type: "Integer",
			value_type: "Normal",
			secret: false,
		};
		const agent = fake.api.agent(EDGE);
		const revision = agent.placement("nightly-sync")?.revision;
		agent.handle("artifact", (command) => {
			const request = command.request as Record<string, unknown>;
			if (request.kind !== "describe")
				return {
					state: "completed",
					result: { device: null, project: null, revisions: [] },
				};
			return {
				state: "completed",
				result: {
					project_id: CRM,
					revision,
					event_id: request.event_id,
					items: [batch],
					next: null,
				},
			};
		});
		const view = await settings(
			CRM,
			{ mode: undefined, device: EDGE, service: "nightly-sync" },
			{ fake },
		);
		expect(text(row(view.container, "var_batch_size"))).toContain(
			"Used by Nightly CRM sync",
		);
		expect(text(view.container)).toContain("Settings for nightly-sync");
		expect(
			fake.api.commands.some(
				([deviceId, type, command]) =>
					deviceId === EDGE &&
					type === "artifact" &&
					(command.request as { kind?: string }).kind === "describe",
			),
		).toBe(true);
	});
});

describe("Settings · bot tokens (R2 §1.10)", () => {
	const SHOP = "app_shop_assistant";
	const TELEGRAM = "evt_shop_telegram";
	const DISCORD = "evt_shop_discord";
	/** The token saved on the Telegram event's record. */
	const SAVED = "123456789:AAHfixture-token-0123456789abcdef";
	const DISCORD_TOKEN =
		"MTE4MDAwMDAwMDAwMDAwMDAwMQ.test-only.not-a-real-discord-token";
	const tokenRow = (root: ParentNode, eventId: string) =>
		root.querySelector<HTMLElement>(
			`[data-bot-token="${eventId}"]`,
		) as HTMLElement;

	test("a bot without a saved token needs one: entered here, shape-checked, never kept in saved progress", async () => {
		const view = await settings(SHOP, { device: EDGE, event: DISCORD });
		await view.settle();
		const bot = tokenRow(view.container, DISCORD);
		expect(text(bot)).toContain("Bot token of Shop support");
		expect(text(bot)).toContain("Discord bot · secret");
		expect(text(bot)).toContain(
			"Stored on edge-berlin-01 as a secret. It can't be read back.",
		);
		// Nothing was saved on the event, so there is nothing to choose: only the field.
		expect(
			queryByRole("button", "Use the token saved in Events", bot),
		).toBeNull();
		expect(kit.footBlocking(view.container)).toBe(
			"Shop support needs its bot token. Enter it under Settings.",
		);
		const field = bot.querySelector(`#deploy-bot-token-${DISCORD}`) as Element;
		await typeInto(field, "my bot token");
		expect(text(bot)).toContain("That doesn't look like a Discord bot token.");
		expect(kit.footBlocking(view.container)).toBe(
			"That doesn't look like a Discord bot token.",
		);
		await typeInto(field, DISCORD_TOKEN);
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(summary()).toContain("1 secret");
		expect(kit.savedText()).not.toContain(DISCORD_TOKEN);
		expect(kit.copyOf(view.container)).not.toMatch(kit.MACHINE_WORDS);
	});

	test("a bot whose record has a token takes it by default; it never reaches saved progress", async () => {
		const view = await settings(SHOP, { device: EDGE, event: TELEGRAM });
		await view.settle();
		const bot = tokenRow(view.container, TELEGRAM);
		const saved = byRole("button", "Use the token saved in Events", bot);
		expect(saved.getAttribute("aria-pressed")).toBe("true");
		expect(bot.querySelector("input")).toBeNull();
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(summary()).toContain("1 secret");
		expect(view.container.textContent).not.toContain(SAVED);

		await click(byRole("button", "Enter a token", bot));
		expect(bot.querySelector(`#deploy-bot-token-${TELEGRAM}`)).not.toBeNull();
		expect(kit.footBlocking(view.container)).toBe(
			"Shop helper needs its bot token. Enter it under Settings.",
		);
		await click(byRole("button", "Use the token saved in Events", bot));
		expect(kit.footBlocking(view.container)).toBeNull();
		expect(kit.savedText()).not.toContain(SAVED);
		expect(kit.savedText()).not.toContain("AAHfixture");
	});

	test("an update keeps the token stored on the device unless it is set anew", async () => {
		const fake = await createFakeWorkspace();
		await serveShopOnEdge(fake);
		const view = await settings(
			SHOP,
			{ mode: undefined, device: EDGE, service: "shop-assistant" },
			{ fake },
		);
		await view.settle();
		const bot = tokenRow(view.container, TELEGRAM);
		expect(
			byRole("button", "Keep stored", bot).getAttribute("aria-pressed"),
		).toBe("true");
		expect(text(bot)).toContain(
			"Stored secret · can't be read back, not even by you.",
		);
		await clickByText("Set new", bot);
		expect(
			byRole("button", "Use the token saved in Events", bot).getAttribute(
				"aria-pressed",
			),
		).toBe("true");
		expect(kit.savedText()).not.toContain(SAVED);
		expect(kit.footBlocking(view.container)).toBeNull();
	});
});
