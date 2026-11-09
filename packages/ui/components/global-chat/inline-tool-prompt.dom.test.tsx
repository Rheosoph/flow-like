// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import botsLocale from "@flow-like/locales/locales/en/bots.json";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { useAuthStatusStore, useBackendStore } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import type { GlobalToolPrompt } from "../../state/global-chat/global-chat-store";
import { InlineToolPrompt } from "./inline-tool-prompt";

function english(key: string): string | undefined {
	const value = key
		.split(".")
		.reduce<unknown>(
			(node, part) => (node as Record<string, unknown> | undefined)?.[part],
			botsLocale,
		);
	return typeof value === "string" ? value : undefined;
}

function translate(key: string, fallback?: unknown, options?: unknown) {
	const values = ((typeof fallback === "object" && fallback) ||
		options ||
		{}) as Record<string, unknown>;
	const template =
		typeof fallback === "string" ? fallback : (english(key) ?? key);
	return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(values[name] ?? ""),
	);
}

vi.mock("@flow-like/locales", async (importOriginal) => ({
	...(await importOriginal<typeof import("@flow-like/locales")>()),
	useTranslation: () => ({ t: translate, i18n: { language: "en" } }),
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({
		isAuthenticated: true,
		isLoading: false,
		user: { access_token: "token", profile: { sub: "me" } },
	}),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: () => {}, replace: () => {} }),
	usePathname: () => "/bots",
	useSearchParams: () => new URLSearchParams(),
}));
vi.mock("../../hooks/use-bot-expression", () => ({
	useBotExpression: () => ({ expression: "neutral", status: "idle" }),
}));
// happy-dom cancels Web Animations with an unhandled AbortError.
vi.mock("framer-motion", async (importOriginal) => {
	const { createElement } = await import("react");
	const still =
		(tag: string) =>
		({
			initial: _initial,
			animate: _animate,
			exit: _exit,
			transition: _transition,
			...props
		}: Record<string, unknown>) =>
			createElement(tag, props);
	return {
		...(await importOriginal<typeof import("framer-motion")>()),
		AnimatePresence: ({ children }: { children: ReactNode }) => children,
		motion: new Proxy({}, { get: (_, tag: string) => still(tag) }),
	};
});
// The package barrel reaches the editor, whose math plugin imports KaTeX's stylesheet.
vi.mock("@platejs/math", () => {
	const plugin = { withComponent: () => plugin, configure: () => plugin };
	return {
		BaseEquationPlugin: plugin,
		BaseInlineEquationPlugin: plugin,
		getEquationHtml: () => "",
		insertEquation: () => {},
		insertInlineEquation: () => {},
	};
});
vi.mock("@platejs/math/react", () => {
	const plugin = { withComponent: () => plugin, configure: () => plugin };
	return {
		EquationPlugin: plugin,
		InlineEquationPlugin: plugin,
		useEquationElement: () => {},
		useEquationInput: () => ({}),
	};
});

function bot(id: string, name: string): IBot {
	return {
		id,
		owner_id: "me",
		system: null,
		preset_id: null,
		name,
		description: null,
		instructions: null,
		appearance: {} as IBot["appearance"],
		model: { primary: null, fallbacks: [] },
		classification: {} as IBot["classification"],
		capabilities: [],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: "",
		updated_at: "",
	};
}

const appMeta = vi.hoisted(() => ({
	names: {} as Record<string, string>,
}));

let root: Root;
let host: HTMLDivElement;

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	appMeta.names = { app_social: "Social Desk" };
	useAuthStatusStore.getState().setSignedIn(true);
	useBackendStore.getState().setBackend({
		botState: { listBots: async () => [bot("bot_medi", "Medi")] },
		appState: {
			getAppMeta: async (appId: string) => {
				const name = appMeta.names[appId];
				if (!name) throw new Error("App not found");
				return { name };
			},
		},
	} as never);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	vi.restoreAllMocks();
});

function approval(overrides: Partial<GlobalToolPrompt> = {}): GlobalToolPrompt {
	return {
		id: "prompt-1",
		kind: "approval",
		runId: "run-1",
		conversationId: "bot:bot_medi",
		toolName: "call_app_event",
		title: "Approve app event execution",
		appId: "app_social",
		rememberable: true,
		rememberAppId: "app_social",
		respond: () => {},
		...overrides,
	};
}

async function show(prompt: GlobalToolPrompt) {
	vi.spyOn(console, "error").mockImplementation(() => {});
	await act(async () =>
		root.render(
			<QueryClientProvider
				client={
					new QueryClient({ defaultOptions: { queries: { retry: false } } })
				}
			>
				<InlineToolPrompt prompt={prompt} />
			</QueryClientProvider>,
		),
	);
	await settle();
}

const rememberLabel = () =>
	host.querySelector("label[for='inline-tool-prompt-remember']")?.textContent;

describe('what "Don\'t ask again" covers', () => {
	test("a bot's approval for an app tool: that bot, in that app", async () => {
		await show(approval());
		expect(rememberLabel()).toBe("Don't ask again for Medi in Social Desk");
	});

	test("Flux's chats share one scope, named Flux", async () => {
		await show(approval({ conversationId: "flux-launch" }));
		expect(rememberLabel()).toBe("Don't ask again for Flux in Social Desk");
	});

	test("a tool that isn't limited to one app names the bot alone", async () => {
		await show(
			approval({
				toolName: "flowpilot_board",
				appId: "app_social",
				rememberAppId: undefined,
			}),
		);
		expect(rememberLabel()).toBe("Don't ask again for Medi");
	});

	test("an app the library can't name reads as this app", async () => {
		await show(approval({ rememberAppId: "app_gone", appId: "app_gone" }));
		expect(rememberLabel()).toBe("Don't ask again for Medi in this app");
	});

	test("offers nothing when the approval can't be remembered", async () => {
		await show(approval({ rememberable: false, rememberAppId: undefined }));
		expect(rememberLabel()).toBeUndefined();
		expect(host.querySelector("#inline-tool-prompt-remember")).toBeNull();
	});

	test("ticking it remembers the approval", async () => {
		const respond = vi.fn();
		await show(approval({ respond }));
		await act(async () =>
			host
				.querySelector<HTMLButtonElement>("#inline-tool-prompt-remember")
				?.click(),
		);
		const approve = Array.from(host.querySelectorAll("button")).find(
			(button) => button.textContent === "Approve",
		);
		await act(async () => approve?.click());
		expect(respond).toHaveBeenCalledWith({ approved: true, remember: true });
	});
});

test("configuration approval shows the complete prompt and model mapping without a remember option", async () => {
	const instructions = `${"Follow these instructions.\n".repeat(100)}Final instruction.`;
	await show(
		approval({
			toolName: "bot_update_configuration",
			appId: undefined,
			rememberable: false,
			rememberAppId: undefined,
			configurationChanges: {
				name: "Atlas",
				instructions,
				image_model: "profile:image-1",
			},
		}),
	);
	expect(
		host.querySelector("[data-slot='bot-configuration-changes']")?.textContent,
	).toContain(instructions);
	expect(host.textContent).toContain("System prompt");
	expect(host.textContent).toContain("profile:image-1");
	expect(host.querySelector("#inline-tool-prompt-remember")).toBeNull();
});

test.each([
	["Approve changes", true],
	["Reject changes", false],
] as const)(
	"configuration action %s answers only this proposal",
	async (label, approved) => {
		const respond = vi.fn();
		await show(
			approval({
				toolName: "bot_update_configuration",
				title: "Approve bot_update_configuration",
				description: "The bot wants to change its image_model.",
				appId: undefined,
				configurationChanges: { image_model: "profile:image-1" },
				respond,
			}),
		);
		expect(host.querySelector("header")?.textContent).toContain(
			"Update bot settings",
		);
		expect(host.querySelector("header")?.textContent).not.toContain(
			"bot_update_configuration",
		);
		expect(host.textContent).not.toContain(
			"The bot wants to change its image_model.",
		);
		expect(host.querySelector("#inline-tool-prompt-remember")).toBeNull();
		const action = Array.from(host.querySelectorAll("button")).find(
			(button) => button.textContent === label,
		);
		expect(action).toBeDefined();
		await act(async () => action?.click());
		expect(respond).toHaveBeenCalledWith({ approved, remember: false });
	},
);

describe("secret prompt", () => {
	const secretPrompt = (
		store: (value: string) => Promise<void>,
		respond: GlobalToolPrompt["respond"],
	) =>
		approval({
			kind: "secret",
			toolName: "secret_request",
			title: "Secret requested",
			rememberable: false,
			rememberAppId: undefined,
			appId: undefined,
			secret: { name: "API_KEY", description: "For the weather API", store },
			respond,
		});
	const buttonLabelled = (label: string) =>
		Array.from(host.querySelectorAll("button")).find(
			(button) => button.textContent?.trim() === label,
		);
	const typeValue = async (value: string) => {
		const input = host.querySelector("input") as HTMLInputElement;
		const setter = Object.getOwnPropertyDescriptor(
			Object.getPrototypeOf(input),
			"value",
		)?.set;
		await act(async () => {
			setter?.call(input, value);
			input.dispatchEvent(new Event("input", { bubbles: true }));
		});
	};

	test("masks the input and offers no remember option", async () => {
		await show(
			secretPrompt(
				async () => {},
				() => {},
			),
		);
		expect(host.querySelector("input")?.getAttribute("type")).toBe("password");
		expect(host.textContent).toContain("API_KEY");
		expect(host.textContent).toContain("For the weather API");
		expect(rememberLabel()).toBeUndefined();
	});

	test("stores the value, then resolves without carrying it", async () => {
		const store = vi.fn(async () => {});
		const respond = vi.fn();
		await show(secretPrompt(store, respond));
		await typeValue("hunter2");
		await act(async () => buttonLabelled("Save secret")?.click());
		await settle();
		expect(store).toHaveBeenCalledWith("hunter2");
		expect(respond).toHaveBeenCalledWith({ approved: true, remember: false });
		expect(JSON.stringify(respond.mock.calls)).not.toContain("hunter2");
	});

	test("a failed store keeps the prompt open", async () => {
		const respond = vi.fn();
		await show(
			secretPrompt(async () => {
				throw new Error("offline");
			}, respond),
		);
		await typeValue("hunter2");
		await act(async () => buttonLabelled("Save secret")?.click());
		await settle();
		expect(respond).not.toHaveBeenCalled();
		expect(host.querySelector('[role="alert"]')).not.toBeNull();
	});

	test("Decline resolves as not approved without storing", async () => {
		const store = vi.fn(async () => {});
		const respond = vi.fn();
		await show(secretPrompt(store, respond));
		await act(async () => buttonLabelled("Decline")?.click());
		expect(store).not.toHaveBeenCalled();
		expect(respond).toHaveBeenCalledWith({ approved: false, remember: false });
	});
});
