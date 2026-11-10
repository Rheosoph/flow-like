// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { modelRequiredError } from "../../../lib/flowpilot/chat-error";
import { type IBit, IBitTypes } from "../../../lib/schema/bit/bit";

const mocks = vi.hoisted(() => ({
	profile: vi.fn(),
	catalog: vi.fn(),
	gate: vi.fn(),
	allows: vi.fn(),
	add: vi.fn(),
}));

vi.mock("../../bots-tab/editor/steps/use-bot-models", () => ({
	LLM_CATALOG: { types: ["Llm", "Vlm"] },
	useBotBitOptions: mocks.profile,
	useCatalogModelOptions: mocks.catalog,
}));
vi.mock("../../../hooks/use-add-model-to-profile", () => ({
	useAddModelToProfile: () => ({
		gate: mocks.gate,
		allows: mocks.allows,
		add: mocks.add,
	}),
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({
		capabilities: () => ({ canHostLlamaCPP: true, canHostMLX: false }),
	}),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: vi.fn() }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown) =>
			typeof fallback === "string" ? fallback : key,
		i18n: { language: "en" },
	}),
}));

import { ChatMessageError } from "./chat-message-error";

function model(id: string, name: string, provider: string, tier?: string) {
	const bit = {
		id,
		type: IBitTypes.Llm,
		meta: { en: { name, description: `${name} answers.` } },
		parameters: {
			provider: { provider_name: provider, params: tier ? { tier } : {} },
		},
	} as unknown as IBit;
	return { id, label: name, bit, local: false, free: false };
}

const HOSTED = model("sonnet", "Hosted Sonnet", "hosted:openrouter", "PRO");
const ABOVE_PLAN = model("opus", "Hosted Opus", "Premium", "MAX");
const ON_DEVICE = model("gguf", "Local Llama", "Local");
const OWN_PROVIDER = model("own", "My OpenAI", "custom:openai");
const AGENT_ACCOUNT = model("cc", "Claude Code", "custom:claude-code");

const CLOUD = modelRequiredError("Nothing here runs in the cloud.", true);
const DEVICE = modelRequiredError("Nothing here runs on this device.", false);

let root: Root;
let host: HTMLDivElement;
let settleAdd: (added: boolean) => void;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	vi.clearAllMocks();
	mocks.profile.mockReturnValue({ options: [], isLoading: false });
	mocks.catalog.mockReturnValue({
		options: [HOSTED],
		isLoading: false,
		isError: false,
	});
	mocks.gate.mockImplementation((bit: IBit) => ({
		isRestricted: bit.id === ABOVE_PLAN.id,
		requiredTier: null,
	}));
	mocks.allows.mockReturnValue(true);
	mocks.add.mockImplementation(
		() =>
			new Promise<boolean>((resolve) => {
				settleAdd = resolve;
			}),
	);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	document.body.innerHTML = "";
});

async function mount(error = CLOUD) {
	await act(async () => {
		root.render(<ChatMessageError error={error} />);
	});
}

function setup(): HTMLElement {
	const found = host.querySelector<HTMLElement>(
		"[data-fl-chat-error-model-setup]",
	);
	if (!found) throw new Error("The card offers no model picker");
	return found;
}

function state(): string | undefined {
	return setup().dataset.flChatErrorModelSetup;
}

async function pickFromCatalog(name: string) {
	await act(async () => {
		(setup().querySelector("button") as HTMLButtonElement).click();
	});
	const choice = [...document.body.querySelectorAll("button")].find(
		(node) => node.textContent?.includes(name) && !host.contains(node),
	);
	if (!choice) throw new Error(`Missing choice: ${name}`);
	await act(async () => choice.click());
}

/** What the card hands the profile query and the catalog to decide which models help. */
function predicates() {
	const usable = mocks.profile.mock.calls.at(-1)?.[1] as (bit: IBit) => boolean;
	return { usable, catalog: mocks.catalog.mock.calls.at(-1)?.[0] };
}

async function openCatalog() {
	await act(async () => {
		(setup().querySelector("button") as HTMLButtonElement).click();
	});
}

test("a profile without a usable model shows the picker instead of a link to settings", async () => {
	await mount();
	expect(host.textContent).toContain("Nothing here runs in the cloud.");
	expect(host.textContent).toContain("Choose a model");
	expect(host.textContent).not.toContain("Open model settings");
	expect(host.textContent).not.toContain("See plans");
	expect(state()).toBe("needed");
});

test("a pick is added to the profile and the card says to send the message again", async () => {
	await mount();
	await pickFromCatalog("Hosted Sonnet");
	expect(mocks.add).toHaveBeenCalledExactlyOnceWith(HOSTED.bit);
	expect(state()).toBe("adding");
	expect(host.textContent).toContain("Hosted Sonnet");

	await act(async () => settleAdd(true));
	expect(state()).toBe("ready");
	expect(host.textContent).toContain("Send your message again.");
});

test("a pick that could not be added leaves the picker asking again", async () => {
	await mount();
	await pickFromCatalog("Hosted Sonnet");
	await act(async () => settleAdd(false));
	expect(state()).toBe("needed");
	expect(host.textContent).not.toContain("Send your message again.");
	expect(host.textContent).toContain("Choose a model");
});

test("a model the profile gained elsewhere settles the card, which still lets another be chosen", async () => {
	mocks.profile.mockReturnValue({ options: [HOSTED], isLoading: false });
	await mount();
	expect(state()).toBe("ready");
	expect(host.textContent).toContain("Choose another model");
});

test("an answer from the cloud counts hosted models in the plan and own providers only", async () => {
	await mount();
	await openCatalog();
	const { usable, catalog } = predicates();
	expect(
		[HOSTED, OWN_PROVIDER, ABOVE_PLAN, ON_DEVICE, AGENT_ACCOUNT].map((item) =>
			usable(item.bit),
		),
	).toEqual([true, true, false, false, false]);
	expect(
		[HOSTED, ABOVE_PLAN, ON_DEVICE, AGENT_ACCOUNT].map((item) =>
			catalog.accepts(item.bit),
		),
	).toEqual([true, true, false, false]);
});

test("an answer from this device counts what it can host and lists the whole catalog", async () => {
	await mount(DEVICE);
	await openCatalog();
	const { usable, catalog } = predicates();
	expect(usable(ON_DEVICE.bit)).toBe(true);
	expect(usable(AGENT_ACCOUNT.bit)).toBe(false);
	expect(catalog.accepts).toBeUndefined();
});

test("any other failure keeps its single button and has no picker", async () => {
	await act(async () => {
		root.render(
			<ChatMessageError
				error={{
					kind: "billing",
					title: "Your plan does not cover these models",
					message: "Plan used up",
					action: { kind: "upgrade", label: "See plans" },
					retryable: false,
				}}
			/>,
		);
	});
	expect(host.textContent).toContain("See plans");
	expect(host.querySelector("[data-fl-chat-error-model-setup]")).toBeNull();
});

test("a card without a title of its own gets the default one", async () => {
	await mount({ ...CLOUD, title: "" });
	expect(host.textContent).toContain("Couldn't answer");
});
