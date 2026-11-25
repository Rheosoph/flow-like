// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type {
	BotEntity,
	BotFact,
} from "../../state/global-chat/bot-memory-facts";
import { BotFactsList } from "./bot-facts-list";

const mocks = vi.hoisted(() => ({
	listFacts: vi.fn(),
	forgetFact: vi.fn(),
	listEntities: vi.fn(),
	updateEntity: vi.fn(),
	mergeEntity: vi.fn(),
	forgetEntity: vi.fn(),
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
	listBotEntities: mocks.listEntities,
	updateBotEntity: mocks.updateEntity,
	mergeBotEntity: mocks.mergeEntity,
	forgetBotEntity: mocks.forgetEntity,
}));

function entity(id: string, patch: Partial<BotEntity> = {}): BotEntity {
	return {
		id,
		name: id,
		kind: "person",
		owner: false,
		aliases: [],
		fact_count: 0,
		created_at: "2026-11-01T00:00:00Z",
		updated_at: "2026-11-01T00:00:00Z",
		...patch,
	};
}

function fact(id: string, patch: Partial<BotFact> = {}): BotFact {
	return {
		id,
		text: id,
		kind: "profile",
		subject: "user",
		source: "user_stated",
		confidence: 0.9,
		recorded_at: "2026-10-20T12:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [],
		...patch,
	};
}

const entities = [
	entity("e_user", {
		name: "user",
		owner: true,
		aliases: ["user"],
		fact_count: 2,
	}),
	entity("e_mia", {
		name: "Mia",
		aliases: ["user.sister"],
		fact_count: 2,
	}),
	entity("e_hh", { name: "Hamburg", kind: "place", fact_count: 1 }),
];

const facts = [
	fact("f_city", {
		text: "Mia lives in Hamburg",
		subject_id: "e_mia",
		object_id: "e_hh",
		relation: "lives in",
	}),
	fact("f_sister", {
		text: "Mia is the user's sister",
		subject_id: "e_user",
		object_id: "e_mia",
		relation: "sister",
	}),
	fact("f_tea", { text: "Prefers green tea", subject_id: "e_user" }),
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
	const found = [...document.body.querySelectorAll("button")].find(
		(candidate) =>
			candidate.textContent?.trim() === label ||
			candidate.getAttribute("aria-label") === label,
	);
	if (!found) throw new Error(`no button "${label}" in ${host.innerHTML}`);
	return found as HTMLButtonElement;
}

function row(name: string): HTMLButtonElement {
	const found = [
		...host.querySelectorAll<HTMLButtonElement>("button[aria-pressed]"),
	].find((candidate) => candidate.textContent?.startsWith(name));
	if (!found) throw new Error(`no row "${name}"`);
	return found;
}

async function click(element: Element) {
	await act(async () => {
		element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
	await settle();
}

async function mount() {
	await render(
		<BotFactsList botId="bot_ada" appId="app_1" name="Ada" onJump={vi.fn()} />,
	);
	await click(button("People & things"));
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) {
		if (typeof mock === "function" && "mockReset" in mock) mock.mockReset();
	}
	mocks.listFacts.mockResolvedValue(facts);
	mocks.listEntities.mockResolvedValue(entities);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

describe("the People & things view", () => {
	test("is offered only when the bot knows entities", async () => {
		mocks.listEntities.mockResolvedValue([]);
		await render(
			<BotFactsList botId="b" appId="a" name="Ada" onJump={vi.fn()} />,
		);
		expect(host.querySelector("fieldset[aria-label='Show memories as']")).toBe(
			null,
		);
	});

	test("shows the owner as You and groups the rest by kind", async () => {
		await mount();
		expect(host.textContent).toContain("You");
		expect(host.textContent).not.toMatch(/\buser\b/);
		expect(host.textContent).toContain("also: You › sister");
		expect(host.textContent).toContain("People");
		expect(host.textContent).toContain("Places");
	});

	test("opens an entity with only the facts about it", async () => {
		await mount();
		await click(row("Hamburg"));
		expect(host.textContent).toContain("Mia lives in Hamburg");
		expect(host.textContent).not.toContain("Prefers green tea");
	});

	test("renames through the backend and refreshes", async () => {
		mocks.updateEntity.mockResolvedValue(entity("e_mia", { name: "Mia S." }));
		await mount();
		await click(row("Mia"));
		await click(button("Rename"));
		const input = host.querySelector<HTMLInputElement>(
			"input[aria-label='Rename']",
		);
		if (!input) throw new Error("rename field missing");
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, "Mia S.");
			input.dispatchEvent(new Event("input", { bubbles: true }));
		});
		await click(button("Save name"));
		expect(mocks.updateEntity).toHaveBeenCalledWith(
			expect.objectContaining({ botId: "bot_ada" }),
			{ id: "e_mia", scope: undefined },
			{ name: "Mia S." },
		);
		expect(mocks.listEntities.mock.calls.length).toBeGreaterThan(1);
	});

	test("forgetting an entity confirms with the fact count first", async () => {
		mocks.forgetEntity.mockResolvedValue([]);
		await mount();
		await click(row("Mia"));
		await click(button("Forget everything about Mia"));
		expect(mocks.forgetEntity).not.toHaveBeenCalled();
		expect(document.body.textContent).toContain(
			"2 facts that mention Mia are deleted",
		);
		await click(button("Forget"));
		expect(mocks.forgetEntity).toHaveBeenCalledWith(
			expect.objectContaining({ botId: "bot_ada" }),
			{ id: "e_mia", scope: undefined },
		);
	});

	test("the owner cannot be forgotten or merged", async () => {
		await mount();
		await click(row("You"));
		expect(
			[...document.body.querySelectorAll("button")].some((b) =>
				b.textContent?.includes("Forget everything"),
			),
		).toBe(false);
		expect(
			[...document.body.querySelectorAll("button")].some((b) =>
				b.textContent?.includes("Merge into"),
			),
		).toBe(false);
	});

	test("a neighbour in the graph selects that entity", async () => {
		await mount();
		await click(row("Mia"));
		const hamburg = host.querySelector("svg [aria-label='Open Hamburg']");
		if (!hamburg) throw new Error("neighbour missing");
		await click(hamburg);
		expect(host.querySelector("section[aria-label='Hamburg']")).not.toBe(null);
	});
});
