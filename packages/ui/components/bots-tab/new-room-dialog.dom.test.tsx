import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
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

const PERSON = { id: "user_mia", kind: "user", label: "Mia", mode: "invite" };
const SERVER_BOT = { id: "bot_srv", kind: "bot", label: "Server bot" };
const PEER_BOT = {
	id: "bot_peer",
	kind: "bot",
	label: "Peer bot",
	mode: "invite",
};

const actualPicker = { ...(await import("./principal-picker")) };
mock.module("./principal-picker", () => ({
	...actualPicker,
	PrincipalPicker: ({
		onPick,
		unavailable,
	}: {
		onPick: (principal: unknown) => void;
		unavailable?: (principal: unknown) => string | undefined;
	}) =>
		createElement(
			"div",
			null,
			[PERSON, SERVER_BOT, PEER_BOT].map((principal) =>
				createElement(
					"button",
					{
						key: principal.id,
						type: "button",
						"data-pick": principal.id,
						onClick: () => onPick(principal),
					},
					`${principal.label} ${unavailable?.(principal) ?? ""}`,
				),
			),
		),
}));

const created: Record<string, unknown>[] = [];
const lookups: string[][] = [];
let keyed = ["user_mia"];
const inboxState = {
	createConversation: async (request: Record<string, unknown>) => {
		created.push(request);
		return { id: "room-1" };
	},
	getKeys: async (ids: string[]) => {
		lookups.push(ids);
		return ids
			.filter((id) => keyed.includes(id))
			.map((id) => ({
				principal_id: id,
				algorithm: "hpke-x25519-sha256-chacha20poly1305",
			}));
	},
};
const actualBackend = { ...(await import("../../state/backend-state")) };
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({ inboxState }),
}));

const { createRoot } = await import("react-dom/client");
const { NewRoomDialog } = await import("./new-room-dialog");
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;

beforeAll(installDomGlobals);
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	client.clear();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("./principal-picker", () => actualPicker);
	mock.module("../../state/backend-state", () => actualBackend);
	restoreDomGlobals();
});

async function show(people: boolean, withKeys = ["user_mia"]) {
	created.length = 0;
	lookups.length = 0;
	keyed = withKeys;
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	const source = {
		me: "me",
		bots: [{ id: "bot_srv", location: "server", capabilities: [] }],
		recentPeopleIds: [],
		directory: (id: string) => ({ id, name: id }),
		people,
	};
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(NewRoomDialog, {
					open: true,
					onOpenChange: () => {},
					source: source as never,
					onCreated: () => {},
				}),
			),
		),
	);
	await settle();
}

function pick(id: string) {
	const found = window.document.querySelector(`[data-pick="${id}"]`);
	if (!found) throw new Error(`Missing picker row: ${id}`);
	return found as unknown as HTMLElement;
}

function sealSwitch() {
	return window.document.querySelector(
		'[role="switch"]',
	) as unknown as HTMLElement | null;
}

function submit() {
	const found = [...window.document.querySelectorAll("button")].find(
		(item) => item.getAttribute("type") === "submit",
	);
	if (!found) throw new Error("Missing submit button");
	return found as unknown as HTMLElement & { disabled: boolean };
}

describe("creating an end-to-end encrypted room", () => {
	test("offers no switch when this device cannot seal", async () => {
		await show(false);
		expect(sealSwitch()).toBeNull();
	});

	test("creates a plain room by default", async () => {
		await show(true);
		await act(async () => pick("user_mia").click());
		await act(async () => submit().click());
		await settle();
		expect(created).toHaveLength(1);
		expect(created[0]?.kind).toBe("group");
		expect(created[0]).not.toHaveProperty("encryption");
	});

	test("sends the encryption choice when the switch is on", async () => {
		await show(true);
		await act(async () => (sealSwitch() as HTMLElement).click());
		await act(async () => pick("user_mia").click());
		await settle();
		await act(async () => submit().click());
		await settle();
		expect(created[0]?.encryption).toBe("e2e");
	});

	test("shows a server bot as unavailable instead of failing on submit", async () => {
		await show(true);
		await act(async () => (sealSwitch() as HTMLElement).click());
		await act(async () => pick("bot_srv").click());
		await settle();
		expect(window.document.body.textContent).toContain(
			"Runs on the server, which can't read an encrypted room",
		);
		expect(submit().disabled).toBe(true);
		expect(created).toHaveLength(0);
	});

	test("is offered only when every invited person and bot has a key, and names who is missing", async () => {
		await show(true, []);
		await act(async () => pick("user_mia").click());
		await act(async () => pick("bot_peer").click());
		await settle();
		expect(lookups.at(-1)).toEqual(["user_mia", "bot_peer"]);
		const sealSwitchNow = sealSwitch() as HTMLElement & { disabled: boolean };
		expect(sealSwitchNow.disabled).toBe(true);
		const text = window.document.body.textContent ?? "";
		expect(text).toContain(
			"Mia: Has no encryption key yet; opening the app signed in publishes one",
		);
		expect(text).toContain(
			"Peer bot: Has no encryption key yet; its host publishes one while it runs the bot with end-to-end encryption",
		);
		expect(submit().disabled).toBe(false);
		await act(async () => submit().click());
		await settle();
		expect(created[0]).not.toHaveProperty("encryption");
	});

	test("turns on once everyone picked has published a key", async () => {
		await show(true, ["user_mia", "bot_peer"]);
		await act(async () => pick("user_mia").click());
		await act(async () => pick("bot_peer").click());
		await settle();
		const sealSwitchNow = sealSwitch() as HTMLElement & { disabled: boolean };
		expect(sealSwitchNow.disabled).toBe(false);
		expect(window.document.body.textContent).not.toContain(
			"Has no encryption key yet",
		);
		await act(async () => sealSwitchNow.click());
		await act(async () => submit().click());
		await settle();
		expect(created[0]).toMatchObject({
			encryption: "e2e",
			members: ["user_mia", "bot_peer"],
		});
	});

	test("says what an encrypted room means", async () => {
		await show(true);
		expect(window.document.body.textContent).toContain(
			"each person reads only what is sent after they join on a device that has published its key",
		);
	});
});
