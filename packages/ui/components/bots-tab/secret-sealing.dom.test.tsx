import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type { IUpsertBotSecretRequest } from "../../state/backend-state/bot-state";
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
const { ApprovalCard } = await import("./approval-card");

const ALGORITHM = "hpke-x25519-sha256-chacha20poly1305";
const ENVELOPE = { v: 1, enc: ALGORITHM, ct: "Y3Q", keys: [] };

let capabilities: string[] = [];
let published = true;
let canSeal = true;
const stored: Array<[string, string, IUpsertBotSecretRequest]> = [];
const sealedCalls: Array<[string, string, string]> = [];

const botState = {
	getBot: async (id: string) => ({
		id,
		name: "Scout",
		system: null,
		location: "desktop",
		capabilities,
	}),
	answerApproval: async () => {},
	upsertBotSecret: async (
		botId: string,
		name: string,
		request: IUpsertBotSecretRequest,
	) => {
		stored.push([botId, name, request]);
		return { name };
	},
};

function inboxState() {
	return {
		getKeys: async (ids: string[]) =>
			published
				? ids.map((id) => ({
						principal_id: id,
						key_id: "ik_bot",
						algorithm: ALGORITHM,
						public_key: "pk",
						created_at: "2026-11-12T00:00:00Z",
					}))
				: [],
		...(canSeal
			? {
					sealBotSecret: async (botId: string, name: string, value: string) => {
						sealedCalls.push([botId, name, value]);
						return ENVELOPE;
					},
				}
			: {}),
	};
}

const previousBackend = useBackendStore.getState().backend;
let client: InstanceType<typeof QueryClient>;
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

const secretCard = createElement(ApprovalCard, {
	approval: {
		id: "ap-s",
		turnId: "turn_s",
		tool: "secret_request",
		summary: "Ask for a secret",
		scopes: [],
		secret: { name: "API_KEY", description: "Weather API" },
	} as never,
	botId: "bot_a",
	botName: "Scout",
	readOnly: false,
});

async function show() {
	useBackendStore
		.getState()
		.setBackend({ botState, inboxState: inboxState() } as never);
	await act(async () =>
		root.render(createElement(QueryClientProvider, { client }, secretCard)),
	);
	await settle();
}

function sealSwitch(): HTMLButtonElement | null {
	return container.querySelector('[data-slot="secret-seal"] button');
}

async function click(element: HTMLElement | null | undefined) {
	expect(element).toBeTruthy();
	await act(async () => {
		element?.click();
	});
	await settle();
}

async function saveValue(value: string) {
	const input = container.querySelector("input") as HTMLInputElement;
	const setter = Object.getOwnPropertyDescriptor(
		Object.getPrototypeOf(input),
		"value",
	)?.set;
	await act(async () => {
		setter?.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
	});
	await settle();
	const save = [...container.querySelectorAll("button")].find(
		(button) => button.textContent?.trim() === "Save secret",
	);
	await click(save as HTMLButtonElement);
}

beforeAll(() => {
	installDomGlobals();
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
});

afterAll(async () => {
	await act(async () => root.unmount());
	mock.module("@flow-like/locales", () => actualLocales);
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});

beforeEach(async () => {
	await act(async () => root.render(null));
	capabilities = ["encryption"];
	published = true;
	canSeal = true;
	stored.length = 0;
	sealedCalls.length = 0;
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

describe("sealing a requested secret to the bot's key", () => {
	test("is offered and on by default; the server only gets the envelope", async () => {
		await show();
		expect(container.textContent).toContain("Seal to Scout's key");
		expect(sealSwitch()?.getAttribute("aria-checked")).toBe("true");
		await saveValue("sk-123");
		expect(sealedCalls).toEqual([["bot_a", "API_KEY", "sk-123"]]);
		expect(stored).toEqual([
			["bot_a", "API_KEY", { sealed: ENVELOPE, description: "Weather API" }],
		]);
		expect(JSON.stringify(stored)).not.toContain("sk-123");
	});

	test("the owner can turn it off and store a server-encrypted value", async () => {
		await show();
		await click(sealSwitch());
		expect(sealSwitch()?.getAttribute("aria-checked")).toBe("false");
		await saveValue("sk-123");
		expect(sealedCalls).toEqual([]);
		expect(stored).toEqual([
			["bot_a", "API_KEY", { value: "sk-123", description: "Weather API" }],
		]);
	});

	for (const [why, setup] of [
		[
			"the bot has no encryption",
			() => {
				capabilities = [];
			},
		],
		[
			"the bot published no key",
			() => {
				published = false;
			},
		],
		[
			"this app cannot seal",
			() => {
				canSeal = false;
			},
		],
	] as const) {
		test(`is not offered when ${why}`, async () => {
			setup();
			await show();
			expect(sealSwitch()).toBeNull();
			await saveValue("sk-123");
			expect(sealedCalls).toEqual([]);
			expect(stored[0]?.[2]).toEqual({
				value: "sk-123",
				description: "Weather API",
			});
		});
	}
});
