// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IConversationSegment } from "../../state/backend-state/inbox-state";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import { BotFactsList } from "./bot-facts-list";
import { BotHistoryPanel } from "./bot-history-panel";

const mocks = vi.hoisted(() => ({
	listFacts: vi.fn(),
	forgetFact: vi.fn(),
	segments: vi.fn(),
	jump: vi.fn(),
	topic: vi.fn(),
	latest: vi.fn(),
	hasHistory: vi.fn(() => true),
	toast: { info: vi.fn(), success: vi.fn(), error: vi.fn() },
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
	i18n: { t: translate },
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ user: { access_token: "token" } }),
}));
vi.mock("sonner", () => ({ toast: mocks.toast }));
vi.mock("../../state/global-chat/bot-memory-facts", () => ({
	listBotFacts: mocks.listFacts,
	forgetBotFact: mocks.forgetFact,
}));
vi.mock("../../state/global-chat/bot-inbox", () => ({
	botHasInboxHistory: mocks.hasHistory,
	loadBotSegments: mocks.segments,
	jumpBotChat: mocks.jump,
	startBotTopic: mocks.topic,
	showLatestBotChat: mocks.latest,
}));

const facts: BotFact[] = [
	{
		id: "f_tea",
		text: "Prefers green tea",
		kind: "preference",
		subject: "user",
		source: "user_stated",
		confidence: 0.9,
		recorded_at: "2026-10-20T12:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [
			{
				seq: 12,
				message_id: "m12",
				quote: "I only drink green tea",
				at: "2026-10-20T12:00:00Z",
			},
		],
	},
	{
		id: "f_city",
		text: "Lives in Hamburg",
		kind: "profile",
		subject: "user",
		source: "user_stated",
		confidence: 0.8,
		recorded_at: "2026-10-21T12:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [
			{ seq: 40, message_id: "m40", quote: "I live in Hamburg", at: "x" },
		],
	},
];

let root: Root;
let host: HTMLDivElement;

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

async function render(node: ReactNode) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>{node}</QueryClientProvider>,
		),
	);
	await settle();
}

function button(label: string): HTMLButtonElement {
	const found = [...host.querySelectorAll("button")].find(
		(candidate) =>
			candidate.textContent?.includes(label) ||
			candidate.getAttribute("aria-label") === label,
	);
	if (!found) throw new Error(`no button "${label}" in ${host.innerHTML}`);
	return found as HTMLButtonElement;
}

async function click(element: HTMLElement) {
	await act(async () => {
		element.click();
	});
	await settle();
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) {
		if (typeof mock === "function" && "mockReset" in mock) mock.mockReset();
	}
	mocks.hasHistory.mockReturnValue(true);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

describe("the Memory tab's facts", () => {
	test("lists each fact with its quote and a link to its position", async () => {
		mocks.listFacts.mockResolvedValue(facts);
		const onJump = vi.fn();
		await render(
			<BotFactsList botId="bot_ada" appId="app_1" name="Ada" onJump={onJump} />,
		);

		expect(host.textContent).toContain("Prefers green tea");
		expect(host.textContent).toContain("I only drink green tea");
		await click(button("#12"));
		expect(onJump).toHaveBeenCalledWith(12);
		expect(mocks.listFacts).toHaveBeenCalledWith(
			expect.objectContaining({ botId: "bot_ada", appId: "app_1" }),
		);
	});

	test("forgetting deletes through the backend and removes the row", async () => {
		mocks.listFacts.mockResolvedValue(facts);
		mocks.forgetFact.mockResolvedValue(undefined);
		await render(
			<BotFactsList
				botId="bot_ada"
				appId="app_1"
				name="Ada"
				onJump={vi.fn()}
			/>,
		);

		await click(
			host.querySelectorAll<HTMLButtonElement>(
				"button[aria-label='Forget this fact']",
			)[0],
		);

		expect(mocks.forgetFact).toHaveBeenCalledWith(
			expect.objectContaining({ botId: "bot_ada" }),
			"f_tea",
		);
		expect(host.textContent).not.toContain("Prefers green tea");
		expect(host.textContent).toContain("Lives in Hamburg");
	});

	test("a failed delete keeps the row and says why", async () => {
		mocks.listFacts.mockResolvedValue(facts);
		mocks.forgetFact.mockRejectedValue(new Error("busy"));
		await render(
			<BotFactsList
				botId="bot_ada"
				appId="app_1"
				name="Ada"
				onJump={vi.fn()}
			/>,
		);
		await click(
			host.querySelectorAll<HTMLButtonElement>(
				"button[aria-label='Forget this fact']",
			)[0],
		);
		expect(host.textContent).toContain("Prefers green tea");
		expect(mocks.toast.error).toHaveBeenCalledWith("busy");
	});

	test("shows the empty state", async () => {
		mocks.listFacts.mockResolvedValue([]);
		await render(
			<BotFactsList
				botId="bot_ada"
				appId="app_1"
				name="Ada"
				onJump={vi.fn()}
			/>,
		);
		expect(host.textContent).toContain("Ada writes down facts");
	});
});

describe("the history jump list", () => {
	const segment = (
		id: string,
		from: number,
		to: number,
		title: string | null,
	): IConversationSegment => ({
		id,
		author_id: "bot_ada",
		level: 0,
		seq_from: from,
		seq_to: to,
		t_from: "2026-10-01T10:00:00Z",
		t_to: "2026-10-02T10:00:00Z",
		title,
		summary: "s",
		parent_id: null,
		version: 1,
		created_at: "2026-10-02T10:00:00Z",
	});

	test("lists segments newest first and a click jumps to the position", async () => {
		mocks.segments.mockResolvedValue([
			segment("s0-1-10", 1, 10, "Setup"),
			segment("s0-11-20", 11, 20, "Trip planning"),
		]);
		mocks.jump.mockResolvedValue(true);
		await render(<BotHistoryPanel botId="bot_ada" name="Ada" />);

		const titles = [...host.querySelectorAll("li button span:first-child")].map(
			(node) => node.textContent,
		);
		expect(titles).toEqual(["Trip planning", "Setup"]);
		await click(button("Setup"));
		expect(mocks.jump).toHaveBeenCalledWith("bot_ada", 1);
	});

	test("New topic writes the marker", async () => {
		mocks.segments.mockResolvedValue([]);
		mocks.topic.mockResolvedValue(true);
		await render(<BotHistoryPanel botId="bot_ada" name="Ada" />);

		await click(button("New topic"));

		expect(mocks.topic).toHaveBeenCalledWith("bot_ada");
		expect(mocks.toast.success).toHaveBeenCalled();
	});

	test("renders nothing for a bot without inbox history", async () => {
		mocks.hasHistory.mockReturnValue(false);
		await render(<BotHistoryPanel botId="bot_ada" name="Ada" />);
		expect(host.textContent).toBe("");
	});
});
