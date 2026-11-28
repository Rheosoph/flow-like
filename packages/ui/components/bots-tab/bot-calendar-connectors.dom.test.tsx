import { afterAll, beforeAll, beforeEach, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IBotConnector,
	IConnectedAccount,
} from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	hooks: { ...(await import("../../hooks/use-bots")) },
	hub: { ...(await import("../../hooks/use-hub")) },
	workspace: { ...(await import("./use-bot-workspace")) },
};
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const mutate = mock((_: unknown, __?: unknown) => {});
const reset = mock(() => {});
let connectorList: IBotConnector[] = [];
let accountList: IConnectedAccount[] = [];
let hubProviders: Record<string, unknown> = {};
mock.module("../../hooks/use-bots", () => ({
	...actual.hooks,
	useBotConnectors: () => ({ data: connectorList }),
	useBotSecrets: () => ({ data: [] }),
	useConnectedAccounts: () => ({
		data: accountList,
		refetch: async () => ({ data: accountList }),
	}),
	useBotConnectorAction: () => ({ mutate, reset, isPending: false }),
}));
mock.module("../../hooks/use-hub", () => ({
	...actual.hub,
	useHub: () => ({ hub: { oauth_providers: hubProviders } }),
}));
mock.module("./use-bot-workspace", () => ({
	...actual.workspace,
	useBotWorkspace: () => ({ query: { data: { app_id: "app_ws" } } }),
}));

const checkOAuthRequirements = mock(
	async (
		_appId: string,
		required: { provider_id: string; scopes?: string[] }[],
	) => ({
		missingProviders: required.map((item) => ({ id: item.provider_id })),
	}),
);

const { createRoot } = await import("react-dom/client");
const { useBackendStore } = await import("../../state/backend-state");
const { BotCalendarConnectorsCard } = await import("./bot-calendar-connectors");
const previousBackend = useBackendStore.getState().backend;

beforeAll(() => {
	installDomGlobals();
	useBackendStore
		.getState()
		.setBackend({ eventState: { checkOAuthRequirements } } as never);
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("../../hooks/use-bots", () => actual.hooks);
	mock.module("../../hooks/use-hub", () => actual.hub);
	mock.module("./use-bot-workspace", () => actual.workspace);
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});
beforeEach(() => {
	mutate.mockClear();
	reset.mockClear();
	checkOAuthRequirements.mockClear();
	connectorList = [];
	accountList = [];
	hubProviders = { google: {}, microsoft: {} };
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

const googleAccount: IBotConnector = {
	id: "bc_google",
	bot_id: "bot_1",
	kind: "oauth",
	label: "google",
	provider_id: "google",
	auth: { type: "oauth", provider_id: "google" },
	has_grant: true,
	scopes: ["email"],
	default_policy: "ask",
	tool_policies: {},
	tools_snapshot: [],
	updated_at: "2026-11-20T10:00:00Z",
};

async function render(botId: string | null, localOnly = false) {
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as Element);
	const element = () =>
		createElement(BotCalendarConnectorsCard, { botId, localOnly });
	await act(async () => root.render(element()));
	return {
		container,
		rerender: () => root.render(element()),
		buttons: () => Array.from(container.querySelectorAll("button")),
		cleanup: async () => {
			await act(async () => root.unmount());
			container.remove();
		},
	};
}

type View = Awaited<ReturnType<typeof render>>;

async function click(view: View, text: string) {
	await act(async () =>
		view
			.buttons()
			.find((b) => b.textContent?.includes(text))
			?.click(),
	);
	await settle();
}

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

async function fillAndConnect(view: View) {
	await act(async () => {
		setValue(
			view.container.querySelector("#caldav-user") as unknown as Element,
			"ann@example.com",
		);
		setValue(
			view.container.querySelector("#caldav-password") as unknown as Element,
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

test("an unsaved bot asks to be saved before calendars are connected", async () => {
	const view = await render(null);
	try {
		expect(view.container.textContent).toContain("Save the bot first");
		expect(
			view.container.querySelector("[data-slot=bot-calendars]"),
		).toBeNull();
	} finally {
		await view.cleanup();
	}
});

test("Google, Outlook and CalDAV are offered where the hub signs in to them", async () => {
	hubProviders = { google: {} };
	const view = await render("bot_1");
	try {
		const labels = view.buttons().map((b) => b.textContent);
		expect(labels).toContain("Google Calendar");
		expect(labels).not.toContain("Outlook calendar");
		expect(labels).toContain("iCloud & CalDAV");
	} finally {
		await view.cleanup();
	}
});

test("a bot on this computer offers CalDAV only and says why", async () => {
	const view = await render("bot_1", true);
	try {
		const labels = view.buttons().map((b) => b.textContent);
		expect(labels).not.toContain("Google Calendar");
		expect(labels).not.toContain("Outlook calendar");
		expect(labels).toContain("iCloud & CalDAV");
		expect(view.container.textContent).toContain(
			"Google and Outlook calendars need a bot on your account",
		);
	} finally {
		await view.cleanup();
	}
});

test("choosing Google Calendar attaches the connected account with calendar access", async () => {
	accountList = [
		{
			handle: "fl_oauth_g",
			provider_id: "google",
			scopes: ["email"],
			state: "ready",
		},
	];
	const view = await render("bot_1");
	try {
		await click(view, "Google Calendar");
		const call = mutate.mock.calls[0];
		const scopes = ["email", "https://www.googleapis.com/auth/calendar.events"];
		expect(call?.[0]).toEqual({
			botId: "bot_1",
			action: {
				type: "create",
				request: {
					kind: "oauth",
					label: "Google Calendar",
					auth: { type: "oauth", provider_id: "google" },
					grant: "fl_oauth_g",
					scopes,
				},
			},
		});
		await act(async () => (call?.[1] as { onSuccess: () => void }).onSuccess());
		expect(checkOAuthRequirements.mock.calls[0]?.slice(0, 2)).toEqual([
			"app_ws",
			[{ provider_id: "google", scopes }],
		]);
	} finally {
		await view.cleanup();
	}
});

test("without a connected account, Outlook starts the sign-in with calendar access", async () => {
	const requested: Event[] = [];
	const listener = (event: Event) => requested.push(event);
	window.addEventListener("flow:oauth-required", listener as never);
	const view = await render("bot_1");
	try {
		await click(view, "Outlook calendar");
		expect(mutate).not.toHaveBeenCalled();
		expect(checkOAuthRequirements.mock.calls[0]?.slice(0, 2)).toEqual([
			"app_ws",
			[{ provider_id: "microsoft", scopes: ["Calendars.ReadWrite"] }],
		]);
		expect(requested).toHaveLength(1);
	} finally {
		window.removeEventListener("flow:oauth-required", listener as never);
		await view.cleanup();
	}
});

test("an account attached without calendar access can be granted it", async () => {
	connectorList = [googleAccount, apple];
	const view = await render("bot_1");
	try {
		const text = view.container.textContent ?? "";
		expect(text).toContain("Connected without calendar access");
		expect(text).toContain("iCloud");
		expect(view.buttons().map((b) => b.textContent)).not.toContain(
			"Google Calendar",
		);
		await click(view, "Allow calendar access");
		expect(mutate.mock.calls[0]?.[0]).toEqual({
			botId: "bot_1",
			action: {
				type: "update",
				connectorId: "bc_google",
				request: {
					scopes: ["email", "https://www.googleapis.com/auth/calendar.events"],
				},
			},
		});
	} finally {
		await view.cleanup();
	}
});

test("a CalDAV connector shows its address, the chosen calendar and read-only marks", async () => {
	connectorList = [apple];
	const view = await render("bot_1");
	try {
		const text = view.container.textContent ?? "";
		expect(text).toContain("Calendar (CalDAV)");
		expect(text).toContain("https://caldav.icloud.com/");
		expect(text).toContain("Refresh calendars");
		expect(text).toContain("Reading (GET, HEAD)");
		expect(text).not.toContain("Connected without calendar access");
	} finally {
		await view.cleanup();
	}
});

test("connecting a CalDAV calendar sends the password as the grant, then offers the calendars", async () => {
	const view = await render("bot_1");
	try {
		await click(view, "iCloud & CalDAV");
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
	const view = await render("bot_1");
	try {
		await click(view, "iCloud & CalDAV");
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
