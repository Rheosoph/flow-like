import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { Window } from "happy-dom";
import { act } from "react";
import { ApiResponseError } from "../../../lib/api-error";
import type { IBackendState } from "../../../state/backend-state";
import type {
	IBot,
	IBotApp,
	IBotSecretInfo,
	IBotState,
	ICreateBotRequest,
	IUpdateBotRequest,
	IUpsertBotSecretRequest,
} from "../../../state/backend-state/bot-state";
import { deviceRow } from "../testing/mount-with-backend";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

const STAMP = "2026-11-01T00:00:00Z";

function bot(overrides: Partial<IBot> = {}): IBot {
	return {
		id: "bot_1",
		owner_id: "user_1",
		system: null,
		preset_id: null,
		name: "Scout",
		description: null,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: null, fallbacks: [] },
		classification: { pii: false },
		capabilities: [],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: STAMP,
		updated_at: STAMP,
		...overrides,
	};
}

function secret(
	overrides: Partial<IBotSecretInfo> & Pick<IBotSecretInfo, "name">,
): IBotSecretInfo {
	return {
		encryption: "server",
		description: null,
		created_at: STAMP,
		updated_at: STAMP,
		last_used_at: null,
		...overrides,
	};
}

/** An attach or detach the test settles by hand. */
interface AppChange {
	appId: string;
	attached: boolean;
	resolve: () => void;
	reject: (error: Error) => void;
}

interface Calls {
	created: ICreateBotRequest[];
	updated: { botId: string; request: IUpdateBotRequest }[];
	deleted: string[];
	cloned: string[];
	saved: IBot[];
	duplicated: IBot[];
	openChanges: boolean[];
	appChanges: AppChange[];
	secrets: { name: string; request: IUpsertBotSecretRequest }[];
	workspaces: string[];
}

interface Fixture {
	existing?: IBot;
	apps?: { id: string; name: string }[];
	secrets?: IBotSecretInfo[];
	/** The host keeps bots on this device (the desktop). */
	localHost?: boolean;
	signedOut?: boolean;
	/** The editor was opened from this managed device. */
	initialDeviceId?: string;
	/** The host can copy bots; a message makes the copy fail with it. */
	clone?: true | string;
	/** Every create, save and copy is refused with this. */
	refuseWith?: Error;
}

/** What the host's clone answers: a custom bot named as a copy. */
function copyOf(original: IBot): IBot {
	return bot({
		...original,
		id: "bot_copy",
		system: null,
		name: `${original.name ?? "Flux"} copy`,
	});
}

function fakeBackend(
	calls: Calls,
	{
		existing,
		apps = [],
		secrets = [],
		localHost = false,
		clone,
		refuseWith,
	}: Fixture,
): IBackendState {
	const attached = new Set<string>();
	const changeApp = (appId: string, attach: boolean) =>
		new Promise<void>((resolve, reject) => {
			calls.appChanges.push({
				appId,
				attached: attach,
				resolve: () => {
					if (attach) attached.add(appId);
					else attached.delete(appId);
					resolve();
				},
				reject,
			});
		});
	const botState: Partial<IBotState> = {
		listBots: async () => (existing ? [existing] : []),
		getBot: async () => {
			if (!existing) throw new Error("No bot");
			return existing;
		},
		getWorkspace: async (botId) => {
			calls.workspaces.push(botId);
			return { app_id: `workspace_${botId}` };
		},
		createBot: async (request) => {
			if (refuseWith) throw refuseWith;
			calls.created.push(request);
			return bot({ ...request, id: "bot_new", name: request.name } as never);
		},
		updateBot: async (botId, request) => {
			if (refuseWith) throw refuseWith;
			calls.updated.push({ botId, request });
			return { ...(existing ?? bot()), ...request } as IBot;
		},
		deleteBot: async (botId) => {
			calls.deleted.push(botId);
		},
		...(clone && {
			cloneBot: async (botId: string) => {
				calls.cloned.push(botId);
				if (refuseWith) throw refuseWith;
				if (typeof clone === "string") throw new Error(clone);
				return copyOf(existing ?? bot());
			},
		}),
		attachBotApp: (_, appId) => changeApp(appId, true),
		detachBotApp: (_, appId) => changeApp(appId, false),
		listBotApps: async () =>
			[...attached].map(
				(app_id): IBotApp => ({
					app_id,
					added_at: STAMP,
					owner_role: "owner",
				}),
			),
		listBotSecrets: async () => secrets,
		upsertBotSecret: async (_, name, request) => {
			calls.secrets.push({ name, request });
			return secret({ name });
		},
		...(localHost && {
			local: {
				list: async () => [],
				checkMove: async () => {
					throw new Error("Not in this test");
				},
				move: async () => {
					throw new Error("Not in this test");
				},
			},
		}),
	};
	return {
		botState,
		appState: {
			getApps: async () =>
				apps.map((app) => [
					{ id: app.id, visibility: "Private" },
					{ name: app.name },
				]),
		},
		apiState: {
			get: async () => [
				deviceRow("dev_a", {
					display_name: "Berlin box",
					bot_host_seen_at: Date.now(),
				}),
			],
		},
		dbState: { listTables: async () => [] },
		eventState: { getEvents: async () => [] },
		boardState: { getBoards: async () => [] },
		userState: {
			getProfiles: async () => [],
			getSettingsProfile: async () => ({
				hub_profile: { id: "profile_1", bits: [] },
			}),
		},
		bitState: {
			searchBits: async () => [],
			listCustomBits: async () => [],
			getProfileBits: async () => [
				{
					id: "embedding_1",
					type: "Embedding",
					meta: { en: { name: "Nomic Embed" } },
				},
				{
					id: "embedding_2",
					type: "Embedding",
					meta: { en: { name: "BGE Small" } },
				},
				{ id: "llm_1", type: "Llm", meta: { en: { name: "Chat model" } } },
				{ id: "stt_1", type: "Stt", meta: { en: { name: "Whisper Small" } } },
				{ id: "tts_1", type: "Tts", meta: { en: { name: "Kokoro" } } },
				{
					id: "image_1",
					type: "ImageGeneration",
					meta: { en: { name: "Flux Schnell" } },
				},
				{ id: "vlm_1", type: "Vlm", meta: { en: { name: "Sight model" } } },
			],
		},
		capabilities: () => ({}),
	} as unknown as IBackendState;
}

/** Types the way a person does, so React's change handler sees it. */
function typeInto(
	dom: Window,
	input: HTMLInputElement | HTMLTextAreaElement,
	value: string,
) {
	return act(async () => {
		const prototype =
			input.tagName === "TEXTAREA"
				? dom.HTMLTextAreaElement.prototype
				: dom.HTMLInputElement.prototype;
		Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
			input,
			value,
		);
		input.dispatchEvent(
			new dom.InputEvent("input", { bubbles: true }) as unknown as Event,
		);
	});
}

/** The step buttons, in order, without their numbers. */
function stepLabels(body: HTMLElement) {
	return [
		...body.querySelectorAll("nav[aria-label='Bot setup steps'] button"),
	].map((step) => step.textContent?.replace(/^\d+/, "") ?? "");
}

function noCalls() {
	const calls: Calls = {
		created: [],
		updated: [],
		deleted: [],
		cloned: [],
		saved: [],
		duplicated: [],
		openChanges: [],
		appChanges: [],
		secrets: [],
		workspaces: [],
	};
	return calls;
}

/** Puts a fresh happy-dom window on `globalThis`, before react-dom loads; `restore` takes it off. */
function installWindow() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		HTMLIFrameElement: window.HTMLIFrameElement,
		SVGElement: window.SVGElement,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		InputEvent: window.InputEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		MutationObserver: window.MutationObserver,
		ResizeObserver: window.ResizeObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	const restore = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	return { window, restore };
}

async function renderDialog(fixture: Fixture = {}) {
	const { window, restore } = installWindow();
	createI18n({ language: "en" });
	const { createRoot } = await import("react-dom/client");
	const { AuthContext } = await import("react-oidc-context");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { toast } = await import("sonner");
	const { BackendContext, useAuthStatusStore } = await import(
		"../../../state/backend-state"
	);
	const { BotEditorDialog } = await import("./bot-editor-dialog");
	const toastError = spyOn(toast, "error").mockImplementation(() => "");
	const toastSuccess = spyOn(toast, "success").mockImplementation(() => "");
	const signedIn = useAuthStatusStore.getState().signedIn;
	if (fixture.signedOut) useAuthStatusStore.getState().setSignedIn(false);
	const calls = noCalls();
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(
			<AuthContext.Provider
				value={{ isAuthenticated: false, isLoading: false } as never}
			>
				<QueryClientProvider client={client}>
					<BackendContext.Provider value={fakeBackend(calls, fixture)}>
						<BotEditorDialog
							open
							botId={fixture.existing?.id}
							initialDeviceId={fixture.initialDeviceId}
							onOpenChange={(open) => calls.openChanges.push(open)}
							onSaved={(saved) => calls.saved.push(saved)}
							onDuplicated={(copy) => calls.duplicated.push(copy)}
						/>
					</BackendContext.Provider>
				</QueryClientProvider>
			</AuthContext.Provider>,
		),
	);
	cleanup = async () => {
		await act(async () => root.unmount());
		client.clear();
		toastError.mockRestore();
		toastSuccess.mockRestore();
		useAuthStatusStore.setState({ signedIn });
		await window.happyDOM.close();
		restore();
	};
	const body = window.document.body as unknown as HTMLElement;
	const button = (text: string, scope: ParentNode = body) => {
		const found = [...scope.querySelectorAll("button")].find((element) =>
			element.textContent?.includes(text),
		);
		if (!found) throw new Error(`No button with "${text}"`);
		return found as HTMLButtonElement;
	};
	const click = (text: string) => act(async () => button(text).click());
	const press = (element: HTMLElement) => act(async () => element.click());
	const settle = async () => {
		for (let attempt = 0; attempt < 5; attempt++)
			await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
	};
	/** The list row (an app or a secret) that mentions `text`. */
	const row = (text: string) => {
		const found = [...body.querySelectorAll("li")].find((item) =>
			item.textContent?.includes(text),
		);
		if (!found) throw new Error(`No row with "${text}"`);
		return found as HTMLLIElement;
	};
	const appBox = (name: string) => {
		const found = row(name).querySelector("[role='checkbox']");
		if (!found) throw new Error(`No checkbox for "${name}"`);
		return found as HTMLButtonElement;
	};
	const ticked = (name: string) =>
		appBox(name).getAttribute("aria-checked") === "true";
	const field = (selector: string) => {
		const found = body.querySelector(selector);
		if (!found) throw new Error(`No field "${selector}"`);
		return found as HTMLInputElement;
	};
	const type = (input: HTMLInputElement | HTMLTextAreaElement, value: string) =>
		typeInto(window, input, value);
	const toastErrors = () => toastError.mock.calls.map(([message]) => message);
	const toastSuccesses = () =>
		toastSuccess.mock.calls.map(([message]) => message);
	const steps = () => stepLabels(body);
	return {
		body,
		button,
		click,
		press,
		settle,
		row,
		appBox,
		ticked,
		field,
		type,
		toastErrors,
		toastError,
		toastSuccesses,
		steps,
		client,
		calls,
	};
}

/** Opens Add secret and returns the form's three fields. */
async function openSecretForm(view: Awaited<ReturnType<typeof renderDialog>>) {
	await view.click("Add secret");
	return {
		name: view.field("form input[id$='-name']"),
		value: view.field("form input[id$='-value']"),
		description: view.field("form input[id$='-description']"),
	};
}

describe("BotEditorDialog", () => {
	test("creates a bot from a picked preset", async () => {
		const { body, button, click, calls } = await renderDialog();
		expect(body.textContent).toContain("New bot");
		expect(body.textContent).toContain("Describe the bot you want");
		expect(body.querySelectorAll("button[aria-pressed]").length).toBe(8);

		await click("Secretary");
		expect(button("Secretary").getAttribute("aria-pressed")).toBe("true");

		await click("Next");
		const name = body.querySelector<HTMLInputElement>(
			"input[value='Secretary']",
		);
		expect(name).not.toBeNull();
		expect(body.querySelector(".fl-bot-customizer")).not.toBeNull();

		await click("Data & safety");
		const create = button("Create bot");
		expect(create.disabled).toBe(false);
		await click("Create bot");

		expect(calls.created).toHaveLength(1);
		expect(calls.created[0]).toMatchObject({
			name: "Secretary",
			preset_id: "secretary",
			location: "server",
			capabilities: [
				"selected_apps",
				"email",
				"push_notifications",
				"scheduling",
				"memory",
			],
			classification: { classification: "confidential", pii: true },
		});
		expect(calls.saved.map((saved) => saved.id)).toEqual(["bot_new"]);
		expect(calls.openChanges).toEqual([false]);
	});

	test("the Abilities step shows a card for each new capability", async () => {
		const view = await renderDialog({});
		await view.click("Secretary");
		await view.click("Abilities");
		await view.settle();
		const text = window.document.body.textContent ?? "";
		for (const label of [
			"Documents",
			"Data connections",
			"Knowledge",
			"Connectors",
			"Channels",
			"Skills",
			"Image generation",
			"Delegation",
		]) {
			expect(text).toContain(label);
		}
	});

	test("creates the bot once while Create is clicked again during app attachment", async () => {
		const view = await renderDialog({
			apps: [{ id: "app_atlas", name: "Atlas" }],
		});
		const { button, click, press, settle, calls } = view;
		await click("Secretary");
		await click("Abilities");
		await settle();
		await press(view.appBox("Atlas"));
		await click("Data & safety");

		const create = button("Create bot");
		await act(async () => {
			create.click();
			create.click();
		});
		await settle();
		expect(calls.created).toHaveLength(1);
		expect(calls.appChanges.map((change) => change.appId)).toEqual([
			"app_atlas",
		]);
		expect(button("Creating").disabled).toBe(true);
		await press(button("Creating"));
		expect(calls.created).toHaveLength(1);

		await act(async () => calls.appChanges[0].resolve());
		await settle();
		expect(calls.created).toHaveLength(1);
		expect(calls.saved.map((saved) => saved.id)).toEqual(["bot_new"]);
		expect(calls.openChanges).toEqual([false]);
	});

	test("deletes a custom bot after confirming", async () => {
		const existing = bot();
		const { button, click, settle, calls } = await renderDialog({ existing });
		await settle();
		expect(button("Save changes").disabled).toBe(true);

		await click("Delete bot");
		await click("Delete");
		await settle();
		expect(calls.deleted).toEqual([existing.id]);
		expect(calls.openChanges).toEqual([false]);
	});

	test("deleting the bot whose chat is open leaves that chat for Flux's", async () => {
		const existing = bot();
		const { useGlobalChatStore } = await import(
			"../../../state/global-chat/global-chat-store"
		);
		const { click, settle, calls } = await renderDialog({ existing });
		await settle();
		useGlobalChatStore.setState({
			activeConversationId: `bot:${existing.id}`,
			messages: [],
		});

		await click("Delete bot");
		await click("Delete");
		await settle();
		expect(calls.deleted).toEqual([existing.id]);
		expect(useGlobalChatStore.getState().activeConversationId).not.toStartWith(
			"bot:",
		);
	});
});

const FULL_SERVER =
	"Your FREE plan allows 3 bots on the server and you have 3; move one to a desktop or device, delete one, or upgrade your plan";
const FULL_SERVER_SAID =
	"Your FREE plan allows 3 bots on the server and you have 3. Run it on this desktop or a device, delete one, or upgrade your plan.";

function serverLimit(message = FULL_SERVER) {
	return new ApiResponseError({
		status: 409,
		code: "BOT_SERVER_LIMIT_REACHED",
		message,
	});
}

describe("BotEditorDialog when the plan's bots on the server are used up", () => {
	test("creating says so with the plan's numbers and the ways out", async () => {
		const view = await renderDialog({ refuseWith: serverLimit() });
		await view.click("Secretary");
		await view.click("Data & safety");
		await view.click("Create bot");
		await view.settle();
		expect(view.body.querySelector("[role='alert']")?.textContent).toBe(
			FULL_SERVER_SAID,
		);
		expect(view.calls.openChanges).toEqual([]);
		expect(view.button("Create bot").disabled).toBe(false);
	});

	test("saving a bot's move to the server says the same", async () => {
		const view = await renderDialog({
			existing: bot({ location: "desktop" }),
			refuseWith: serverLimit(),
		});
		await view.settle();
		await view.type(view.field("input[id$='-name']"), "Scout II");
		await view.click("Save changes");
		await view.settle();
		expect(view.body.querySelector("[role='alert']")?.textContent).toBe(
			FULL_SERVER_SAID,
		);
		expect(view.calls.openChanges).toEqual([]);
	});

	test("a copy that would exceed it is a toast with the same words", async () => {
		const view = await renderDialog({
			existing: bot(),
			clone: true,
			refuseWith: serverLimit(),
		});
		await view.settle();
		await view.click("Duplicate");
		await view.settle();
		expect(view.toastErrors()).toEqual([FULL_SERVER_SAID]);
		expect(view.calls.duplicated).toEqual([]);
	});

	test("a refusal without the plan's wording still names the ways out", async () => {
		const view = await renderDialog({ refuseWith: serverLimit("") });
		await view.click("Secretary");
		await view.click("Data & safety");
		await view.click("Create bot");
		await view.settle();
		expect(view.body.querySelector("[role='alert']")?.textContent).toBe(
			"Your plan's bots on the server are used up. Run it on this desktop or a device, delete one, or upgrade your plan.",
		);
	});

	test("the all-bots cap keeps its own words", async () => {
		const view = await renderDialog({
			refuseWith: new ApiResponseError({
				status: 409,
				code: "BOT_LIMIT_REACHED",
				message: "You already have 50 bots",
			}),
		});
		await view.click("Secretary");
		await view.click("Data & safety");
		await view.click("Create bot");
		await view.settle();
		expect(view.body.querySelector("[role='alert']")?.textContent).toBe(
			"You have reached the maximum number of bots. Delete one to create another.",
		);
	});
});

describe("BotEditorDialog apps of an existing bot", () => {
	const apps = [
		{ id: "app_atlas", name: "Atlas" },
		{ id: "app_beacon", name: "Beacon" },
	];

	test("quick ticks settle one by one, and a refused tick is taken back", async () => {
		const existing = bot({ capabilities: ["selected_apps"] });
		const view = await renderDialog({ existing, apps });
		const { click, press, settle, appBox, ticked, calls } = view;
		await settle();
		await click("Abilities");
		await settle();
		await press(appBox("Atlas"));
		await press(appBox("Beacon"));
		expect(
			calls.appChanges.map(({ appId, attached }) => [appId, attached]),
		).toEqual([
			["app_atlas", true],
			["app_beacon", true],
		]);
		expect(ticked("Atlas")).toBe(true);
		expect(ticked("Beacon")).toBe(true);

		const [atlas, beacon] = calls.appChanges;
		await act(async () => atlas.reject(new Error("Not a member")));
		await act(async () => beacon.resolve());
		await settle();
		expect(ticked("Atlas")).toBe(false);
		expect(ticked("Beacon")).toBe(true);
		expect(view.toastErrors()).toEqual(["The app couldn't be updated."]);
	});

	test("an older change of the same app doesn't undo a newer tick", async () => {
		const existing = bot({ capabilities: ["selected_apps"] });
		const view = await renderDialog({ existing, apps });
		const { click, press, settle, appBox, ticked, calls } = view;
		await settle();
		await click("Abilities");
		await settle();
		await press(appBox("Atlas"));
		await press(appBox("Atlas"));
		const [attach, detach] = calls.appChanges;
		expect([attach.attached, detach.attached]).toEqual([true, false]);

		await act(async () => attach.resolve());
		await settle();
		expect(ticked("Atlas")).toBe(false);

		await act(async () => detach.resolve());
		await settle();
		expect(ticked("Atlas")).toBe(false);
		expect(view.toastErrors()).toEqual([]);
	});
});

describe("BotEditorDialog host", () => {
	test("says which abilities a move switched off and which it switched back on", async () => {
		const existing = bot({
			location: "desktop",
			capabilities: ["computer_use"],
		});
		const { body, button, click, press, settle } = await renderDialog({
			existing,
		});
		await settle();
		await click("Runs on");
		const radio = (location: string) => {
			const found = body.querySelector(`[role='radio'][value='${location}']`);
			if (!found) throw new Error(`No ${location} option`);
			return found as HTMLButtonElement;
		};

		await press(radio("server"));
		expect(body.textContent).toContain(
			"Switched off because the cloud has no screen or shell: Computer use.",
		);
		expect(button("Save changes").disabled).toBe(false);

		await press(radio("desktop"));
		expect(body.querySelector("[role='status']")?.textContent).toBe(
			"Switched back on because this host has a screen and shell: Computer use.",
		);
		expect(body.textContent).not.toContain("Switched off because");
		expect(button("Save changes").disabled).toBe(true);
	});
});

describe("BotEditorDialog opened from a device", () => {
	test("the device is selected at the location step and the bot is created there", async () => {
		const { body, click, settle, calls } = await renderDialog({
			initialDeviceId: "dev_a",
		});
		await click("Secretary");
		await click("Runs on");
		await settle();
		expect(
			body
				.querySelector("[role='radio'][value='device']")
				?.getAttribute("aria-checked"),
		).toBe("true");
		expect(body.textContent).toContain("Berlin box");
		await click("Data & safety");
		await click("Create bot");
		expect(calls.created[0]).toMatchObject({
			location: "device",
			device_id: "dev_a",
		});
	});
});

describe("BotEditorDialog for a bot on this computer", () => {
	const radios = (body: HTMLElement) =>
		[...body.querySelectorAll("[role='radio']")].map((radio) =>
			radio.getAttribute("value"),
		);
	const runsOn = (body: HTMLElement) =>
		body.querySelector("[role='radiogroup']")?.textContent ?? "";

	test("signed out, a new bot runs on this computer only, even from a cloud preset", async () => {
		const { body, click, calls } = await renderDialog({
			localHost: true,
			signedOut: true,
		});
		await click("Secretary");
		await click("Runs on");
		expect(radios(body)).toEqual(["desktop"]);
		expect(runsOn(body)).toContain("This computer");
		expect(runsOn(body)).not.toContain("Cloud");
		expect(body.textContent).toContain(
			"Bots on the server or a device need an account.",
		);

		await click("Data & safety");
		await click("Create bot");
		expect(calls.created).toHaveLength(1);
		expect(calls.created[0]).toMatchObject({
			name: "Secretary",
			location: "desktop",
			device_id: null,
		});
	});

	test("signed in, a bot that lives here keeps its one host and saves nothing new", async () => {
		const existing = bot({ location: "desktop", local: true });
		const { body, button, click, settle } = await renderDialog({
			existing,
			localHost: true,
		});
		await settle();
		await click("Runs on");
		expect(radios(body)).toEqual(["desktop"]);
		expect(body.textContent).toContain(
			"Bots on the server or a device need an account.",
		);
		expect(button("Save changes").disabled).toBe(true);
	});

	test("an account bot keeps every host", async () => {
		const { body, click, settle } = await renderDialog({
			existing: bot({ location: "desktop" }),
			localHost: true,
		});
		await settle();
		await click("Runs on");
		expect(radios(body)).toEqual(["server", "desktop", "device"]);
		expect(body.textContent).not.toContain("need an account");
	});
});

describe("BotEditorDialog secrets", () => {
	const known = [
		secret({ name: "OPENAI_KEY", description: "Production key" }),
		secret({ name: "HOST_TOKEN", encryption: "sealed" }),
	];

	async function openSecrets() {
		const view = await renderDialog({ existing: bot(), secrets: known });
		await view.settle();
		await view.click("Secrets");
		await view.settle();
		return view;
	}

	test("naming an existing secret shows its description and saves it unchanged", async () => {
		const view = await openSecrets();
		const { name, value, description } = await openSecretForm(view);
		await view.type(name, "openai key");
		expect(name.value).toBe("OPENAI_KEY");
		expect(description.value).toBe("Production key");

		await view.type(name, "OPENAI_KEY_2");
		expect(description.value).toBe("");
		await view.type(name, "OPENAI_KEY");
		expect(description.value).toBe("Production key");

		await view.type(value, "sk-new");
		await view.click("Save secret");
		await view.settle();
		expect(view.calls.secrets).toEqual([
			{
				name: "OPENAI_KEY",
				request: { value: "sk-new", description: "Production key" },
			},
		]);
	});

	test("a description the user empties is sent as null", async () => {
		const view = await openSecrets();
		const { name, value, description } = await openSecretForm(view);
		await view.type(name, "OPENAI_KEY");
		await view.type(description, "");
		expect(description.value).toBe("");

		await view.type(value, "sk-new");
		await view.click("Save secret");
		await view.settle();
		expect(view.calls.secrets).toEqual([
			{ name: "OPENAI_KEY", request: { value: "sk-new", description: null } },
		]);
	});

	test("a sealed secret can't be replaced from here", async () => {
		const view = await openSecrets();
		const sealedRow = view.row("HOST_TOKEN");
		expect(view.button("Replace", sealedRow).disabled).toBe(true);
		expect(sealedRow.textContent).toContain(
			"Sealed for the bot's host. Delete it and add it again to store a new value through Flow-Like.",
		);
		expect(view.button("Replace", view.row("OPENAI_KEY")).disabled).toBe(false);

		const { name, value } = await openSecretForm(view);
		await view.type(name, "HOST_TOKEN");
		await view.type(value, "plain");
		expect(view.body.querySelector("form")?.textContent).toContain(
			"HOST_TOKEN is sealed for the bot's host.",
		);
		expect(view.button("Save secret").disabled).toBe(true);
	});
});

describe("BotEditorDialog chat settings", () => {
	test("a decision model can be disabled without changing the chat model", async () => {
		const view = await renderDialog({
			existing: bot({
				model: {
					primary: "chat_model",
					fallbacks: [],
					decision: "decision_model",
				},
			}),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(view.body.textContent).toContain(
			"Server and managed-device bots use hosted decision models.",
		);
		await view.click("Unknown model (decision_model)");
		await view.click("Disabled");
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{
				botId: "bot_1",
				request: { model: { primary: "chat_model", fallbacks: [] } },
			},
		]);
	});
	test("keeps the bot model and memory management together in Brain", async () => {
		const view = await renderDialog({
			existing: bot({ model: { primary: "chat_model", fallbacks: [] } }),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(view.body.textContent).toContain("Unknown model (chat_model)");
		expect(
			view.body.querySelector(
				'[role="combobox"][aria-label="Memory embedding model"]',
			),
		).toBeNull();
		expect(view.button("Review & manage saved memories")).toBeDefined();
		expect(view.body.textContent).not.toContain("Default chat model");
	});
	test("Flux picks its own model in Brain, like every bot", async () => {
		const view = await renderDialog({
			existing: bot({
				system: "flux",
				name: null,
				model: { primary: "flux_model", fallbacks: [] },
			}),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(view.body.textContent).toContain("Unknown model (flux_model)");
		expect(view.body.textContent).not.toContain("Default chat model");
		expect(view.button("Review & manage saved memories")).toBeDefined();
	});

	test("Flux without its own model shows the chat's model settings", async () => {
		const view = await renderDialog({
			existing: bot({ system: "flux", name: null }),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(
			view.body.querySelector("button[aria-label='Model: Automatic']"),
		).not.toBeNull();
		expect(view.body.textContent).toContain("Default chat model");
	});
});

/** What the preview beside a step shows: name, host, data labels and abilities. */
function previewText(body: HTMLElement) {
	return body.querySelector("aside[aria-label='Bot preview']")?.textContent;
}

/** The switch an ability row's label names. */
function switchFor(body: HTMLElement, label: string) {
	const found = [...body.querySelectorAll("label")].find(
		(item) => item.textContent === label,
	);
	const id = found?.getAttribute("for");
	const control = id ? body.ownerDocument.getElementById(id) : null;
	if (!control) throw new Error(`No switch for "${label}"`);
	return control as HTMLButtonElement;
}

describe("BotEditorDialog built-in bots", () => {
	const flux = () =>
		bot({ id: "bot_flux", system: "flux", name: null, capabilities: [] });

	test("Flux saves its name and instructions and keeps its abilities and host", async () => {
		const view = await renderDialog({ existing: flux() });
		await view.settle();
		expect(view.steps()).toEqual([
			"Look",
			"Abilities",
			"Brain",
			"Data & safety",
			"Secrets",
		]);
		await view.type(view.field("input[id$='-name']"), "Builder");

		await view.click("Abilities");
		expect(view.body.textContent).toContain("What it can do");
		expect(view.body.textContent).toContain(
			"Builds, edits and debugs your apps and flows, FlowScript included",
		);
		expect(view.body.querySelector("[role='switch']")).toBeNull();
		expect(previewText(view.body)).toContain("FlowPilot");
		expect(previewText(view.body)).not.toContain("Cloud");

		await view.click("Brain");
		await view.settle();
		await view.type(
			view.field("textarea[id$='-instructions']"),
			"Explain each change.",
		);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{
				botId: "bot_flux",
				request: { name: "Builder", instructions: "Explain each change." },
			},
		]);
		expect(view.calls.openChanges).toEqual([false]);
	});

	test("Flux's encryption switch can't be used", async () => {
		const view = await renderDialog({ existing: flux() });
		await view.settle();
		await view.click("Data & safety");
		expect(switchFor(view.body, "End-to-end encryption").disabled).toBe(true);
	});

	test("the Assistant edits like a custom bot, abilities and host included", async () => {
		const existing = bot({
			id: "bot_general",
			system: "general",
			name: null,
			capabilities: ["internet"],
		});
		const view = await renderDialog({ existing });
		await view.settle();
		expect(view.steps()).toContain("Runs on");
		await view.type(view.field("input[id$='-name']"), "Helper");

		await view.click("Abilities");
		expect(previewText(view.body)).toContain("Cloud");
		const internet = switchFor(view.body, "Internet");
		expect(internet.getAttribute("aria-checked")).toBe("true");
		await view.press(internet);
		expect(() => view.button("Delete bot")).toThrow();

		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{
				botId: "bot_general",
				request: { name: "Helper", capabilities: [] },
			},
		]);
	});

	test("a Flux duplicate shows the FlowPilot card and moves without its abilities changing", async () => {
		const existing = bot({ capabilities: ["flowpilot"], location: "desktop" });
		const view = await renderDialog({ existing });
		await view.settle();
		await view.click("Abilities");
		expect(view.body.textContent).toContain("Always on");
		expect(view.body.querySelector("[role='switch']")).toBeNull();

		await view.click("Runs on");
		const cloud = view.body.querySelector<HTMLButtonElement>(
			"[role='radio'][value='server']",
		);
		if (!cloud) throw new Error("No cloud option");
		await view.press(cloud);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: existing.id, request: { location: "server" } },
		]);
	});
});

describe("BotEditorDialog model pins", () => {
	const pinField = (body: HTMLElement, label: string) => {
		const found = body.querySelector<HTMLButtonElement>(
			`button[aria-label^='${label}:']`,
		);
		if (!found) throw new Error(`No ${label} field`);
		return found;
	};

	test("Brain pins speech, image and vision models by Bit type and saves only the changes", async () => {
		const view = await renderDialog({ existing: bot() });
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(
			pinField(view.body, "Speech to text").getAttribute("aria-label"),
		).toBe("Speech to text: Let the host choose");

		await view.press(pinField(view.body, "Speech to text"));
		const listed = view.body.querySelector("fieldset")?.textContent;
		expect(listed).toContain("Whisper Small");
		expect(listed).not.toContain("Kokoro");
		expect(listed).not.toContain("Sight model");
		await view.click("Whisper Small");
		await view.settle();

		await view.press(pinField(view.body, "Vision"));
		await view.click("Sight model");
		await view.settle();

		await view.press(pinField(view.body, "Image generation"));
		await view.click("Flux Schnell");
		await view.settle();
		await view.press(pinField(view.body, "Text to speech"));
		await view.click("Kokoro");
		await view.settle();

		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{
				botId: "bot_1",
				request: {
					stt_model: "stt_1",
					tts_model: "tts_1",
					image_model: "image_1",
					vision_model: "vlm_1",
				},
			},
		]);
	});

	test("a pinned model is shown and cleared back to the host", async () => {
		const view = await renderDialog({
			existing: bot({ stt_model: "stt_1" }),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(
			pinField(view.body, "Speech to text").getAttribute("aria-label"),
		).toBe("Speech to text: Whisper Small");
		await view.press(pinField(view.body, "Speech to text"));
		const host = Array.from(
			view.body.querySelectorAll<HTMLButtonElement>("fieldset button"),
		).find((choice) =>
			choice.textContent?.includes("The app or server it runs on picks one"),
		);
		if (!host) throw new Error("No host choice");
		await view.press(host);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: "bot_1", request: { stt_model: null } },
		]);
	});
});

describe("BotEditorDialog memory", () => {
	const findMemoryField = (body: HTMLElement) =>
		body.querySelector<HTMLButtonElement>(
			"button[aria-label^='Memory model:']",
		);
	/** A boolean, so a failure doesn't print the whole document. */
	const showsMemoryField = (body: HTMLElement) =>
		findMemoryField(body) !== null;
	const memoryField = (body: HTMLElement) => {
		const found = findMemoryField(body);
		if (!found) throw new Error("No memory model field");
		return found;
	};
	const remembers = () => bot({ capabilities: ["memory"] });

	test("Brain and the Memory card edit the same saved memory model", async () => {
		const view = await renderDialog({ existing: remembers() });
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(showsMemoryField(view.body)).toBe(true);

		await view.click("Abilities");
		await view.settle();
		expect(switchFor(view.body, "Memory").getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(view.body.textContent).toContain(
			"Remembers what matters from your chats",
		);
		expect(memoryField(view.body).getAttribute("aria-label")).toBe(
			"Memory model: Let the host choose",
		);
		await view.press(memoryField(view.body));
		expect(() => view.button("Chat model")).toThrow();
		await view.click("Nomic Embed");
		await view.click("Brain");
		await view.settle();
		expect(memoryField(view.body).getAttribute("aria-label")).toBe(
			"Memory model: Nomic Embed",
		);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: "bot_1", request: { embedding_model: "embedding_1" } },
		]);
	});

	test("the Assistant saves a memory model selected in Brain without changing the chat default", async () => {
		const { useGlobalChatStore } = await import(
			"../../../state/global-chat/global-chat-store"
		);
		const chatEmbeddingModel = useGlobalChatStore.getState().embeddingModelId;
		const view = await renderDialog({
			existing: bot({
				id: "bot_general",
				system: "general",
				name: null,
				capabilities: ["internet", "memory"],
			}),
		});
		await view.settle();
		await view.click("Brain");
		await view.settle();
		expect(view.button("Save changes").disabled).toBe(true);
		expect(
			view.body.querySelector('[aria-label="Memory embedding model"]'),
		).toBeNull();
		await view.press(memoryField(view.body));
		await view.click("Nomic Embed");
		expect(view.button("Save changes").disabled).toBe(false);
		expect(useGlobalChatStore.getState().embeddingModelId).toBe(
			chatEmbeddingModel,
		);
		expect(view.calls.updated).toEqual([]);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: "bot_general", request: { embedding_model: "embedding_1" } },
		]);
		expect(view.calls.saved[0]?.embedding_model).toBe("embedding_1");
	});

	test.each([
		bot({ id: "account_assistant", system: "general", name: null }),
		bot({ id: "custom_bot" }),
	])("Brain reviews workspace facts for $id", async (existing) => {
		const facts = await import("../../../state/global-chat/bot-memory-facts");
		const memories = await import(
			"../../../state/global-chat/global-chat-memory"
		);
		const listFacts = spyOn(facts, "listBotFacts").mockResolvedValue([]);
		const listMemories = spyOn(
			memories,
			"listGlobalChatMemories",
		).mockResolvedValue([]);
		try {
			const view = await renderDialog({ existing });
			await view.settle();
			await view.click("Brain");
			await view.settle();
			expect(view.calls.workspaces).toEqual([]);
			await view.click("Review & manage saved memories");
			await view.settle();
			expect(view.calls.workspaces).toEqual([existing.id]);
			expect(listFacts).toHaveBeenCalledWith({
				botId: existing.id,
				appId: `workspace_${existing.id}`,
				token: undefined,
			});
			expect(listMemories).not.toHaveBeenCalled();
		} finally {
			listFacts.mockRestore();
			listMemories.mockRestore();
		}
	});

	test.each([
		bot({ id: "bot_flux", system: "flux", name: null }),
		bot({ id: "local_assistant", system: "general", local: true }),
	])("Brain keeps the existing memory store for $id", async (existing) => {
		const memories = await import(
			"../../../state/global-chat/global-chat-memory"
		);
		const listMemories = spyOn(
			memories,
			"listGlobalChatMemories",
		).mockResolvedValue([]);
		try {
			const view = await renderDialog({ existing });
			await view.settle();
			await view.click("Brain");
			await view.settle();
			await view.click("Review & manage saved memories");
			await view.settle();
			expect(view.calls.workspaces).toEqual([]);
			expect(listMemories).toHaveBeenCalledWith(
				{
					profileId: "profile_1",
					botId: existing.system === "flux" ? null : existing.id,
				},
				undefined,
			);
		} finally {
			listMemories.mockRestore();
		}
	});

	test("letting the host choose clears it", async () => {
		const view = await renderDialog({
			existing: bot({
				capabilities: ["memory"],
				embedding_model: "embedding_2",
			}),
		});
		await view.settle();
		await view.click("Abilities");
		await view.settle();
		expect(memoryField(view.body).getAttribute("aria-label")).toBe(
			"Memory model: BGE Small",
		);

		await view.press(memoryField(view.body));
		await view.click("Let the host choose");
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: "bot_1", request: { embedding_model: null } },
		]);
	});

	test("switching Memory off hides its model and saves without it", async () => {
		const view = await renderDialog({ existing: remembers() });
		await view.settle();
		await view.click("Abilities");
		await view.settle();
		await view.press(switchFor(view.body, "Memory"));
		expect(showsMemoryField(view.body)).toBe(false);
		await view.click("Brain");
		await view.settle();
		expect(showsMemoryField(view.body)).toBe(false);
		await view.click("Save changes");
		await view.settle();
		expect(view.calls.updated).toEqual([
			{ botId: "bot_1", request: { capabilities: [] } },
		]);
	});

	test("a new bot starts with Memory on", async () => {
		const view = await renderDialog();
		await view.click("Secretary");
		await view.click("Abilities");
		await view.settle();
		expect(switchFor(view.body, "Memory").getAttribute("aria-checked")).toBe(
			"true",
		);
		expect(showsMemoryField(view.body)).toBe(true);
		expect(view.body.textContent).toContain(
			"New bots start with Memory on and everything else off.",
		);
	});

	test("a FlowPilot bot keeps its one card, and its memory model stays in Brain", async () => {
		const view = await renderDialog({
			existing: bot({ capabilities: ["flowpilot"] }),
		});
		await view.settle();
		await view.click("Abilities");
		await view.settle();
		expect(view.body.querySelector("[role='switch']") === null).toBe(true);
		expect(showsMemoryField(view.body)).toBe(false);
		await view.click("Brain");
		await view.settle();
		expect(memoryField(view.body).getAttribute("aria-label")).toBe(
			"Memory model: Let the host choose",
		);
	});
});

describe("BotEditorDialog duplicate", () => {
	test("copies the saved bot, closes and hands the copy over", async () => {
		const existing = bot();
		const view = await renderDialog({ existing, clone: true });
		await view.settle();
		await view.click("Duplicate");
		await view.settle();
		expect(view.calls.cloned).toEqual([existing.id]);
		expect(view.calls.duplicated.map((copy) => copy.id)).toEqual(["bot_copy"]);
		expect(view.calls.openChanges).toEqual([false]);
		expect(view.toastSuccesses()).toEqual(["Duplicated as Scout copy"]);
		expect(
			view.client.getQueryData(["bots", "detail", "bot_copy"]),
		).toMatchObject({ id: "bot_copy", name: "Scout copy" });
	});

	test("Flux can be duplicated but not deleted", async () => {
		const view = await renderDialog({
			existing: bot({ id: "bot_flux", system: "flux", name: null }),
			clone: true,
		});
		await view.settle();
		expect(() => view.button("Delete bot")).toThrow();
		await view.click("Duplicate");
		await view.settle();
		expect(view.calls.cloned).toEqual(["bot_flux"]);
		expect(view.toastSuccesses()).toEqual(["Duplicated as Flux copy"]);
	});

	test("is hidden where the host can't copy bots", async () => {
		const view = await renderDialog({ existing: bot() });
		await view.settle();
		expect(view.button("Delete bot")).toBeDefined();
		expect(() => view.button("Duplicate")).toThrow();
	});

	test("isn't offered for a bot that doesn't exist yet", async () => {
		const view = await renderDialog({ clone: true });
		await view.click("Secretary");
		await view.click("Data & safety");
		expect(view.button("Create bot")).toBeDefined();
		expect(() => view.button("Duplicate")).toThrow();
	});

	test("waits until unsaved changes are saved or discarded", async () => {
		const view = await renderDialog({ existing: bot(), clone: true });
		await view.settle();
		await view.type(view.field("input[id$='-name']"), "Scout II");
		const duplicate = view.button("Duplicate");
		expect(duplicate.getAttribute("aria-disabled")).toBe("true");
		await view.press(duplicate);
		await view.settle();
		expect(view.calls.cloned).toEqual([]);
		expect(view.calls.openChanges).toEqual([]);
	});

	test("a failed copy is a toast with its cause, and the editor stays open", async () => {
		const view = await renderDialog({
			existing: bot(),
			clone: "Command local_bots_clone not found",
		});
		await view.settle();
		await view.click("Duplicate");
		await view.settle();
		expect(view.toastErrors()).toEqual(["The bot couldn't be duplicated."]);
		expect(view.toastError.mock.calls[0]?.[1]).toEqual({
			description: "Command local_bots_clone not found",
		});
		expect(view.calls.openChanges).toEqual([]);
		expect(view.calls.duplicated).toEqual([]);
		expect(view.button("Duplicate").disabled).toBe(false);
	});
});
