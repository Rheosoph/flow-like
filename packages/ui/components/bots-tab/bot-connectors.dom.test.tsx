import { afterAll, beforeAll, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBotConnector } from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	hooks: { ...(await import("../../hooks/use-bots")) },
};
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const mutate = mock((_: unknown, __?: unknown) => {});
const reset = mock(() => {});
let connectorList: IBotConnector[] = [];
const docs: IBotConnector = {
	id: "bc_docs",
	bot_id: "bot_1",
	kind: "mcp",
	label: "Docs",
	url: "https://mcp.example.com/mcp",
	auth: { type: "none" },
	has_grant: false,
	scopes: [],
	default_policy: "ask",
	tool_policies: { create: "read" },
	tools_snapshot: [
		{
			name: "search",
			description: "Find pages",
			read_only_hint: true,
			schema_hash: "a",
		},
		{
			name: "create",
			description: "Make a page",
			schema_hash: "b",
		},
		{
			name: "drop",
			description: "Delete a page",
			destructive_hint: true,
			schema_hash: "c",
		},
	],
	updated_at: "2026-11-05T10:00:00Z",
};
mock.module("../../hooks/use-bots", () => ({
	...actual.hooks,
	useBotConnectors: () => ({ data: connectorList }),
	useBotSecrets: () => ({ data: [] }),
	useConnectedAccounts: () => ({ data: [] }),
	useBotConnectorAction: () => ({ mutate, reset, isPending: false }),
}));

const { createRoot } = await import("react-dom/client");
const { BotConnectorsCard } = await import("./bot-connectors");

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("../../hooks/use-bots", () => actual.hooks);
	restoreDomGlobals();
});

async function render(botId: string | null) {
	connectorList = [docs];
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(createElement(BotConnectorsCard, { botId })),
	);
	return {
		container,
		rerender: () => root.render(createElement(BotConnectorsCard, { botId })),
		cleanup: async () => {
			await act(async () => root.unmount());
			container.remove();
		},
	};
}

test("an unsaved bot asks to be saved before services are connected", async () => {
	const view = await render(null);
	try {
		expect(view.container.textContent).toContain("Save the bot first");
		expect(
			view.container.querySelector("[data-slot=bot-connectors]"),
		).toBeNull();
	} finally {
		await view.cleanup();
	}
});

test("every tool shows its hints and a write tool allowed to run unasked is flagged", async () => {
	const view = await render("bot_1");
	try {
		const text = view.container.textContent ?? "";
		expect(text).toContain("Docs");
		expect(text).toContain("search");
		expect(text).toContain("Read only");
		expect(text).toContain("Can delete");
		expect(text).toContain("will run without asking");
		expect(text.match(/will run without asking/g)).toHaveLength(1);
	} finally {
		await view.cleanup();
	}
});

test("refresh and remove act on the connector", async () => {
	mutate.mockClear();
	const view = await render("bot_1");
	try {
		const buttons = Array.from(view.container.querySelectorAll("button"));
		await act(async () =>
			buttons.find((b) => b.textContent === "Refresh tools")?.click(),
		);
		expect(mutate.mock.calls[0]?.[0]).toEqual({
			botId: "bot_1",
			action: { type: "refresh", connectorId: "bc_docs" },
		});
		await act(async () =>
			buttons
				.find((b) => b.getAttribute("aria-label") === "Remove Docs")
				?.click(),
		);
		expect(mutate.mock.calls[1]?.[0]).toEqual({
			botId: "bot_1",
			action: { type: "delete", connectorId: "bc_docs" },
		});
	} finally {
		await view.cleanup();
	}
});

test("adding an MCP server opens the form with a sign-in choice", async () => {
	const view = await render("bot_1");
	try {
		await act(async () =>
			Array.from(view.container.querySelectorAll("button"))
				.find((b) => b.textContent?.includes("Add MCP server"))
				?.click(),
		);
		expect(
			view.container.querySelector('input[aria-label="Server URL"]'),
		).not.toBeNull();
		const add = Array.from(view.container.querySelectorAll("button")).find(
			(b) => b.textContent === "Add server",
		);
		expect((add as unknown as HTMLButtonElement).disabled).toBe(true);
	} finally {
		await view.cleanup();
	}
});

const apple: IBotConnector = {
	id: "bc_cal",
	bot_id: "bot_1",
	kind: "caldav",
	label: "iCloud",
	url: "https://caldav.icloud.com/",
	auth: { type: "basic", username: "ann@example.com" },
	has_grant: true,
	scopes: [],
	default_policy: "ask",
	tool_policies: {},
	tools_snapshot: [],
	calendar_url: "https://p01/work/",
	calendars: [
		{ url: "https://p01/work/", name: "Work", writable: true },
		{ url: "https://p01/holidays/", name: "Holidays", writable: false },
	],
	updated_at: "2026-11-20T10:00:00Z",
};

function setValue(input: Element, value: string) {
	const el = input as unknown as HTMLInputElement;
	const setter = Object.getOwnPropertyDescriptor(
		Object.getPrototypeOf(el),
		"value",
	)?.set;
	setter?.call(el, value);
	el.dispatchEvent(
		new window.Event("input", { bubbles: true }) as unknown as Event,
	);
}

async function openCalDav(view: Awaited<ReturnType<typeof render>>) {
	await act(async () =>
		Array.from(view.container.querySelectorAll("button"))
			.find((b) => b.textContent?.includes("Add calendar (CalDAV)"))
			?.click(),
	);
}

async function fillAndConnect(view: Awaited<ReturnType<typeof render>>) {
	await act(async () => {
		setValue(
			view.container.querySelector("#caldav-user") as Element,
			"ann@example.com",
		);
		setValue(
			view.container.querySelector("#caldav-password") as Element,
			"abcd-efgh",
		);
	});
	await act(async () =>
		(
			view.container.querySelector("form") as unknown as HTMLFormElement
		).dispatchEvent(
			new window.Event("submit", {
				bubbles: true,
				cancelable: true,
			}) as unknown as Event,
		),
	);
}

test("a CalDAV connector shows its address, the chosen calendar and read-only marks", async () => {
	const view = await render("bot_1");
	try {
		connectorList = [apple];
		await act(async () => view.rerender());
		const text = view.container.textContent ?? "";
		expect(text).toContain("Calendar (CalDAV)");
		expect(text).toContain("https://caldav.icloud.com/");
		expect(text).toContain("Refresh calendars");
		expect(text).toContain("Reading (GET, HEAD)");
	} finally {
		await view.cleanup();
	}
});

test("connecting a CalDAV calendar sends the password as the grant, then offers the calendars", async () => {
	mutate.mockClear();
	reset.mockClear();
	const view = await render("bot_1");
	try {
		await openCalDav(view);
		const password = view.container.querySelector("#caldav-password");
		expect(password?.getAttribute("type")).toBe("password");
		expect(password?.getAttribute("autocomplete")).toBe("new-password");
		await fillAndConnect(view);
		const call = mutate.mock.calls[0];
		expect(call?.[0]).toEqual({
			botId: "bot_1",
			action: {
				type: "create",
				request: {
					kind: "caldav",
					label: "iCloud",
					url: "https://caldav.icloud.com/",
					auth: { type: "basic", username: "ann@example.com" },
					grant: "abcd-efgh",
				},
			},
		});
		connectorList = [apple];
		await act(async () =>
			(call?.[1] as { onSuccess: (c: IBotConnector) => void }).onSuccess(apple),
		);
		expect(reset).toHaveBeenCalled();
		expect(view.container.querySelector("#caldav-password")).toBeNull();
		const text = view.container.textContent ?? "";
		expect(text).toContain("Calendar the bot uses");
		expect(text).toContain("Reading (GET, HEAD)");
		expect(view.container.textContent).not.toContain("abcd-efgh");
	} finally {
		await view.cleanup();
	}
});

test("a failed discovery shows the server's message with the sign-in hint", async () => {
	mutate.mockClear();
	const view = await render("bot_1");
	try {
		await openCalDav(view);
		await fillAndConnect(view);
		const call = mutate.mock.calls[0];
		await act(async () =>
			(call?.[1] as { onError: (e: Error) => void }).onError(
				new Error("PROPFIND caldav.icloud.com answered 401"),
			),
		);
		const alert = view.container.querySelector("[role=alert]");
		expect(alert?.textContent).toContain("answered 401");
		expect(alert?.textContent).toContain("app password");
		expect(view.container.querySelector("#caldav-user")).not.toBeNull();
	} finally {
		await view.cleanup();
	}
});
