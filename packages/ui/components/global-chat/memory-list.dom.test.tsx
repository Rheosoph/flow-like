// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { useAuthStatusStore, useBackendStore } from "../../state/backend-state";
import type {
	MemoryEntry,
	MemoryOwner,
} from "../../state/global-chat/global-chat-memory";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { ChatMemorySettings } from "./chat-memory-settings";
import { MemoryList } from "./memory-list";

const memory = vi.hoisted(() => ({
	list: vi.fn(),
	remove: vi.fn(),
	clear: vi.fn(),
	status: vi.fn(),
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
	const plural =
		values.count === 1 ? values.defaultValue_one : values.defaultValue_other;
	const template =
		typeof values.count === "number" && typeof plural === "string"
			? plural
			: typeof fallback === "string"
				? fallback
				: key;
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
vi.mock("../../state/global-chat/global-chat-memory", () => ({
	listGlobalChatMemories: memory.list,
	deleteGlobalChatMemory: memory.remove,
	clearGlobalChatMemory: memory.clear,
	globalChatMemoryStatus: memory.status,
}));

const SCOUT: MemoryOwner = { profileId: "profile-1", botId: "bot_scout" };

const entries: MemoryEntry[] = [
	{
		id: "m1",
		content: "Prefers the digest on Mondays",
		role: "preference",
		timestamp: Date.now() - 120_000,
	},
	{
		id: "m2",
		content: "Works on the Q4 launch",
		role: "observation",
		timestamp: Date.now() - 7_200_000,
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

function button(label: string) {
	return Array.from(document.body.querySelectorAll("button")).find(
		(item) =>
			item.getAttribute("aria-label") === label ||
			item.textContent?.trim() === label,
	);
}

async function click(element: HTMLElement | undefined) {
	expect(element).toBeDefined();
	await act(async () => element?.click());
	await settle();
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	for (const mock of Object.values(memory)) mock.mockReset();
	memory.list.mockResolvedValue(entries);
	memory.remove.mockResolvedValue(undefined);
	memory.clear.mockResolvedValue(undefined);
	useBackendStore.getState().setBackend({
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "profile-1" } }),
		},
		bitState: { getProfileBits: async () => [] },
	} as never);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

describe("saved memories", () => {
	test("the bot settings dialog reviews the open bot's memories and forgets one", async () => {
		await render(<ChatMemorySettings botId="bot_scout" />);
		await click(button("Review & manage saved memories"));

		const dialog = document.body.querySelector("[role='dialog']");
		expect(dialog?.textContent).toContain("Saved memories");
		expect(dialog?.textContent).toContain("Memory is off.");
		expect(memory.list).toHaveBeenCalledWith(SCOUT, "token");
		expect(dialog?.textContent).toContain("Prefers the digest on Mondays");
		expect(dialog?.textContent).toContain("2 memories");

		await click(
			dialog?.querySelector<HTMLButtonElement>(
				"button[aria-label='Forget this memory']",
			) ?? undefined,
		);
		expect(memory.remove).toHaveBeenCalledWith(SCOUT, "m1", "token");
		expect(dialog?.textContent).not.toContain("Prefers the digest on Mondays");
		expect(dialog?.textContent).toContain("1 memory");
	});

	test("Flux's memories belong to the profile", async () => {
		await render(
			<MemoryList owner={{ profileId: "profile-1", botId: null }} />,
		);
		expect(memory.list).toHaveBeenCalledWith(
			{ profileId: "profile-1", botId: null },
			"token",
		);
		expect(host.textContent).not.toContain("Memory is off.");
	});

	test("a failed read can be tried again, and Clear all empties the list", async () => {
		memory.list.mockRejectedValueOnce(new Error("list memories failed: 503"));
		await render(
			<MemoryList owner={SCOUT} emptyHint="Scout saves facts here." />,
		);
		expect(host.textContent).toContain("Couldn't load memories");
		expect(host.textContent).toContain("list memories failed: 503");
		expect(host.textContent).not.toContain("No memories saved yet.");

		await click(button("Try again"));
		expect(memory.list).toHaveBeenCalledTimes(2);
		expect(host.textContent).toContain("Works on the Q4 launch");

		await click(button("Clear all"));
		expect(memory.clear).toHaveBeenCalledWith(SCOUT, "token");
		expect(host.textContent).toContain("No memories saved yet.");
		expect(host.textContent).toContain("Scout saves facts here.");
	});

	test("a delete the backend refuses keeps the memory", async () => {
		memory.remove.mockRejectedValueOnce(new Error("delete memory failed: 500"));
		await render(<MemoryList owner={SCOUT} />);
		await click(button("Forget this memory"));
		expect(host.textContent).toContain("Prefers the digest on Mondays");
		expect(host.textContent).toContain("2 memories");
	});
});

describe("the shared memory embedding model", () => {
	const owner = (botId: string | null): MemoryOwner => ({
		profileId: "profile-1",
		botId,
	});
	/** Saved memories per owner (`flux` for Flux), all from `embed-a`. */
	const saved: Record<string, number> = {
		flux: 3,
		bot_scout: 2,
		bot_writer_here: 1,
	};

	beforeEach(() => {
		const proto = Element.prototype as unknown as Record<string, unknown>;
		proto.scrollIntoView ??= () => {};
		proto.hasPointerCapture ??= () => false;
		proto.releasePointerCapture ??= () => {};
		useAuthStatusStore.getState().setSignedIn(true);
		useGlobalChatStore.setState({ embeddingModelId: "embed-a" });
		memory.status.mockImplementation(async ({ botId }: MemoryOwner) => {
			const count = saved[botId ?? "flux"] ?? 0;
			return { count, embedding_model_id: count ? "embed-a" : null };
		});
		useBackendStore.getState().setBackend({
			userState: {
				getSettingsProfile: async () => ({ hub_profile: { id: "profile-1" } }),
			},
			bitState: {
				getProfileBits: async () =>
					["embed-a", "embed-b"].map((id) => ({
						id,
						type: IBitTypes.Embedding,
						meta: { en: { name: id === "embed-a" ? "Embed A" : "Embed B" } },
					})),
			},
			botState: {
				local: {},
				listBots: async () => [
					{ id: "bot_general", system: "general" },
					{ id: "bot_flux", system: "flux" },
					{ id: "bot_scout", system: null },
					{ id: "bot_writer_here", system: null, local: true },
				],
			},
		} as never);
	});

	async function pickModel(name: string) {
		const picker = document.body.querySelector<HTMLElement>(
			'[role="combobox"][aria-label="Memory embedding model"]',
		);
		await click(picker ?? undefined);
		await click(
			Array.from(
				document.body.querySelectorAll<HTMLElement>('[role="option"]'),
			).find((option) => option.textContent?.trim() === name),
		);
	}

	test("switching it checks Flux and every bot, shows the total and clears them all", async () => {
		await render(<ChatMemorySettings botId="bot_scout" />);
		await pickModel("Embed B");

		const checked = memory.status.mock.calls.map(([asked]) => asked);
		for (const botId of [
			null,
			"general",
			"bot_general",
			"bot_scout",
			"bot_writer_here",
		]) {
			expect(checked).toContainEqual(owner(botId));
		}
		expect(checked).not.toContainEqual(owner("bot_flux"));
		const alert = document.body.querySelector("[role='alertdialog']");
		expect(alert?.textContent).toContain("6 saved memories");

		await click(button("Delete & switch"));
		expect(memory.clear.mock.calls.map(([cleared]) => cleared)).toEqual([
			owner(null),
			owner("bot_scout"),
			owner("bot_writer_here"),
		]);
		expect(useGlobalChatStore.getState().embeddingModelId).toBe("embed-b");
	});

	test("keeping the model clears nothing", async () => {
		await render(<ChatMemorySettings botId={null} />);
		await pickModel("Embed B");
		await click(button("Keep current model"));
		expect(memory.clear).not.toHaveBeenCalled();
		expect(useGlobalChatStore.getState().embeddingModelId).toBe("embed-a");
	});
});
