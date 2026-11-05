// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IBot } from "../../state/backend-state/bot-state";
import { BotFactsList } from "./bot-facts-list";
import { BotMemoryNotice, useBotMemoryOff } from "./bot-memory-notice";

const mocks = vi.hoisted(() => ({
	update: vi.fn(),
	chatModel: { value: "" },
	listFacts: vi.fn(),
}));

function translate(
	key: string,
	fallback?: unknown,
	options?: Record<string, unknown>,
): string {
	const values =
		(typeof fallback === "object" && fallback !== null
			? (fallback as Record<string, unknown>)
			: options) ?? {};
	const template = typeof fallback === "string" ? fallback : key;
	return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(values[name] ?? ""),
	);
}

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate, i18n: { language: "en" } }),
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ user: { access_token: "token" } }),
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
vi.mock("../../hooks/use-bots", () => ({
	useUpdateBot: () => ({ mutate: mocks.update, isPending: false }),
}));
vi.mock("../../state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: (select: (s: { embeddingModelId: string }) => unknown) =>
		select({ embeddingModelId: mocks.chatModel.value }),
}));
vi.mock("../global-chat/memory-model-picker", () => ({
	MemoryModelPicker: () => <div data-testid="picker">picker</div>,
}));
vi.mock("../../state/global-chat/bot-memory-facts", () => ({
	listBotFacts: mocks.listFacts,
	forgetBotFact: vi.fn(),
}));

const bot = (overrides: Partial<IBot>) =>
	({
		id: "bot_ada",
		system: null,
		capabilities: ["memory"],
		embedding_model: null,
		...overrides,
	}) as IBot;

function Harness({ target }: Readonly<{ target: IBot }>) {
	const off = useBotMemoryOff(target, target.embedding_model);
	return off ? (
		<BotMemoryNotice reason={off} bot={target} botId={target.id} name="Ada" />
	) : (
		<p>memory on</p>
	);
}

let root: Root;
let host: HTMLDivElement;

async function render(target: IBot) {
	const client = new QueryClient();
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<Harness target={target} />
			</QueryClientProvider>,
		),
	);
}

beforeEach(() => {
	mocks.update.mockReset();
	mocks.chatModel.value = "";
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

describe("the Memory tab notice", () => {
	test("offers to turn on the capability, adding it to the bot's own", async () => {
		await render(bot({ capabilities: ["internet"] }));
		expect(host.textContent).toContain("Memory is switched off for Ada");
		expect(host.querySelector("[data-testid=picker]")).toBeNull();
		const button = [...host.querySelectorAll("button")].find((b) =>
			b.textContent?.includes("Turn on memory"),
		);
		await act(async () => button?.click());
		expect(mocks.update).toHaveBeenCalledWith(
			{ botId: "bot_ada", request: { capabilities: ["internet", "memory"] } },
			expect.anything(),
		);
	});

	test("shows the capability notice first when the model is missing too", async () => {
		await render(bot({ capabilities: [] }));
		expect(host.textContent).toContain("Memory is switched off");
		expect(host.querySelector("[data-testid=picker]")).toBeNull();
	});

	test("offers the memory model picker when no model is selected", async () => {
		await render(bot({}));
		expect(host.textContent).toContain("can't remember anything yet");
		expect(host.querySelector("[data-testid=picker]")).not.toBeNull();
	});

	test("stays quiet when the chat names a memory model", async () => {
		mocks.chatModel.value = "bit_embed";
		await render(bot({}));
		expect(host.textContent).toBe("memory on");
	});

	test("stays quiet when the bot has its own memory model", async () => {
		await render(bot({ embedding_model: "bit_own" }));
		expect(host.textContent).toBe("memory on");
	});

	test("a Flux bot always remembers given a model", async () => {
		await render(
			bot({ system: "flux", capabilities: [], embedding_model: "m" }),
		);
		expect(host.textContent).toBe("memory on");
	});
});

describe("the facts empty state", () => {
	const renderList = async (memoryOff: boolean) => {
		mocks.listFacts.mockResolvedValue([]);
		await act(async () =>
			root.render(
				<QueryClientProvider client={new QueryClient()}>
					<BotFactsList
						botId="bot_ada"
						appId="app_1"
						name="Ada"
						memoryOff={memoryOff}
						onJump={vi.fn()}
					/>
				</QueryClientProvider>,
			),
		);
		await act(async () => new Promise((r) => setTimeout(r, 20)));
	};

	test("does not claim facts are written down while memory is off", async () => {
		await renderList(true);
		expect(host.textContent).toContain("Nothing is saved while memory is off");
		expect(host.textContent).not.toContain("as you chat");
	});

	test("keeps the normal text when memory is on", async () => {
		await renderList(false);
		expect(host.textContent).toContain("Ada writes down facts");
	});
});
