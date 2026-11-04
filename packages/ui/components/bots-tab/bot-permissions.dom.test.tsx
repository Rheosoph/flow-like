import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IBot,
	IBotPermissions,
} from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useBackendStore } = await import("../../state/backend-state");
const { BotPermissions } = await import("./bot-permissions");

const bot = (id: string, name: string) =>
	({ id, name, system: null, appearance: {} }) as unknown as IBot;
const pip = bot("bot_pip", "Pip");
const flux = bot("bot_flux", "Flux");
const scout = bot("bot_scout", "Scout");

const grants: IBotPermissions = { granted: [], received: [] };
const calls: { from: string; to: string }[] = [];

const backend = {
	botState: {
		listBots: async () => [pip, flux, scout],
		listPermissions: async () => ({
			granted: [...grants.granted],
			received: [],
		}),
		grantPermission: async (from: string, to: string) => {
			calls.push({ from, to });
			grants.granted.push({
				id: `${from}>${to}`,
				from_bot_id: from,
				to_bot_id: to,
				granted_at: "",
				granted_by: "me",
			});
			return grants.granted[grants.granted.length - 1];
		},
		revokePermission: async () => {},
	},
};

const previousBackend = useBackendStore.getState().backend;
const client = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
let host: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(async () => {
	installDomGlobals();
	useBackendStore.getState().setBackend(backend as never);
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(BotPermissions, { bot: pip, name: "Pip" }),
			),
		),
	);
	await settle();
});

afterAll(async () => {
	await act(async () => root.unmount());
	client.clear();
	useBackendStore.setState({ backend: previousBackend });
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

describe("BotPermissions grant action", () => {
	test("offers the owner's other bots and grants the chosen one", async () => {
		const trigger = [...host.querySelectorAll("button")].find((item) =>
			item.textContent?.includes("Allow Pip to ask"),
		);
		expect(trigger).toBeDefined();
		await act(async () => {
			trigger?.dispatchEvent(
				new window.PointerEvent("pointerdown", {
					bubbles: true,
					button: 0,
				}) as never,
			);
		});
		await settle();
		const items = [...window.document.querySelectorAll("[role='menuitem']")];
		expect(items.map((item) => item.textContent)).toEqual(["Flux", "Scout"]);
		await act(async () => (items[0] as unknown as HTMLElement).click());
		await settle();
		expect(calls).toEqual([{ from: "bot_pip", to: "bot_flux" }]);
		await settle(100);
		expect(host.textContent).toContain("Flux");
	});
});
