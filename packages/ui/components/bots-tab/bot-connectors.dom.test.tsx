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
	useBotConnectors: () => ({ data: [docs] }),
	useBotSecrets: () => ({ data: [] }),
	useConnectedAccounts: () => ({ data: [] }),
	useBotConnectorAction: () => ({ mutate, isPending: false }),
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
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as Element);
	await act(async () =>
		root.render(createElement(BotConnectorsCard, { botId })),
	);
	return {
		container,
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
