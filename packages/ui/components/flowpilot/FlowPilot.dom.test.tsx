// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { FlowPilot } from "./FlowPilot";
import type { ProviderModelReasoningPickerProps } from "./provider-model-reasoning-picker";
import type { FlowPilotProps } from "./types";

const harness = vi.hoisted(() => ({
	backend: {} as Record<string, unknown>,
	start: vi.fn(async () => {}),
	sdkModels: [{ id: "default", name: "CLI default" }],
}));

vi.mock("../../state/backend-state", () => ({
	useBackend: () => harness.backend,
}));
vi.mock("../../hooks", async () => ({
	useInvoke: (await import("../../hooks/use-invoke")).useInvoke,
	useCopilotSDK: () => ({
		models: harness.sdkModels,
		isRunning: true,
		isConnecting: false,
		start: harness.start,
		stop: async () => {},
	}),
}));
vi.mock("../../lib", async () => ({
	...(await import("../../lib/bit/local-model-filter")),
	IBitTypes: (await import("../../lib/schema/bit/bit")).IBitTypes,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string) => fallback ?? key,
	}),
}));
vi.mock("../../hooks/use-frontend-runtime-tool-executor", () => ({
	useFrontendRuntimeToolExecutor: () => async () => undefined,
}));
vi.mock("../../lib/flowpilot/board-context-manifest", () => ({
	buildFlowPilotBoardContextAugmentation: async () => undefined,
}));
vi.mock("../../lib/flowpilot/window-scoped-listen", () => ({
	listenInCurrentWindow: async () => () => {},
}));
vi.mock("../../lib/flowpilot-db", () => ({
	createConversation: async () => ({ id: "board-conversation" }),
	addMessage: async () => ({ id: "board-message" }),
	getMessages: async () => [],
	updateConversation: async () => {},
	updateMessage: async () => {},
}));
vi.mock("./generation-metrics", () => ({
	FlowPilotGenerationMetricsRun: class {
		push() {}
		finish() {}
		disposeReview() {}
		observeFinalBoardNodeCount() {}
		abandon() {}
	},
}));
vi.mock("./HistoryPanel", () => ({ HistoryPanel: () => null }));
vi.mock("./ContextNodes", () => ({ ContextNodes: () => null }));
vi.mock("./MessageContent", () => ({
	MessageContent: ({ content }: { content: string }) => <p>{content}</p>,
}));
vi.mock("./PendingCommandsView", () => ({ PendingCommandsView: () => null }));
vi.mock("./PendingComponentsView", () => ({
	PendingComponentsView: () => null,
}));
vi.mock("./flowscript-workspace-panel", () => ({
	FlowScriptWorkspacePanel: () => null,
	formatLineCount: () => "",
}));
// Keep the actual picker and its click callbacks; expose its controlled selection for assertions.
vi.mock("./provider-model-reasoning-picker", async (importOriginal) => {
	const original =
		await importOriginal<typeof import("./provider-model-reasoning-picker")>();
	return {
		...original,
		ProviderModelReasoningPicker: (
			props: ProviderModelReasoningPickerProps,
		) => (
			<div
				data-testid="board-model-selection"
				data-model-id={props.selectedModelId}
				data-provider={props.provider}
			>
				<original.ProviderModelReasoningPicker {...props} />
			</div>
		),
	};
});
// happy-dom cancels Web Animations with an unhandled AbortError.
vi.mock("framer-motion", async () => {
	const { createElement } = await import("react");
	const components = new Map();
	return {
		AnimatePresence: ({ children }: { children: ReactNode }) => children,
		motion: new Proxy(
			{},
			{
				get: (_, tag: string) => {
					if (!components.has(tag)) {
						components.set(
							tag,
							({
								initial,
								animate,
								exit,
								transition,
								layoutId,
								...props
							}: Record<string, unknown>) => createElement(tag, props),
						);
					}
					return components.get(tag);
				},
			},
		),
	};
});

const claude = {
	id: "custom-claude-code",
	type: "Llm",
	meta: { en: { name: "My Claude Code" } },
	parameters: { provider: { provider_name: "custom:claude-code" } },
};
const qwen = {
	id: "custom-qwen",
	type: "Llm",
	meta: { en: { name: "Custom Qwen" } },
	parameters: { provider: { provider_name: "custom:openai" } },
};
const board = {
	id: "review-board",
	name: "Review board",
	nodes: {},
	layers: {},
	variables: {},
	comments: {},
} as FlowPilotProps["board"];

let root: Root;
let host: HTMLDivElement;
let client: QueryClient;

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

function selection() {
	const element = host.querySelector("[data-testid='board-model-selection']");
	if (!element) throw new Error("Board model picker is missing");
	return element;
}

async function clickButton(label: string) {
	const button = [...document.querySelectorAll("button")].find(
		(element) =>
			element.getAttribute("aria-label") === label ||
			element.textContent?.trim() === label,
	);
	if (!button) throw new Error(`Button ${label} is missing`);
	expect(button.disabled).toBe(false);
	await act(async () => button.click());
}

async function openPicker() {
	const trigger = host.querySelector<HTMLButtonElement>(
		"button[aria-label^='Provider and model:']",
	);
	if (!trigger) throw new Error("Board model picker trigger is missing");
	await act(async () => trigger.click());
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.stubGlobal("__TAURI__", {});
	vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(
		() => {},
	);
	harness.start.mockClear();
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	host.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

test.each(["partial", "empty"] as const)(
	"Board FlowPilot sends the explicitly chosen Claude Bit after a successful %s catalog refresh",
	async (catalogState) => {
		let customCatalog = [qwen, claude];
		const libraryReads = vi.fn();
		async function listCustomBits() {
			libraryReads();
			return customCatalog;
		}
		const copilotChat = vi.fn(async (..._args: unknown[]) => ({
			message: "The board is ready.",
			commands: [],
			components: [],
			suggestions: [],
		}));
		harness.backend = {
			userState: {
				getSettingsProfile: async () => ({
					hub_profile: { id: "board-profile", bits: [qwen.id] },
				}),
			},
			bitState: { searchBits: async () => [qwen], listCustomBits },
			boardState: { copilot_chat: copilotChat },
			capabilities: () => ({ canUseNativeAgentProviders: true }),
		};
		await act(async () =>
			root.render(
				<QueryClientProvider client={client}>
					<FlowPilot agentMode="board" board={board} appId="review-app" />
				</QueryClientProvider>,
			),
		);
		await vi.waitFor(async () => {
			await settle();
			expect(selection().getAttribute("data-model-id")).toBe(claude.id);
		});
		await openPicker();
		await clickButton("Custom Qwen");
		expect(selection().getAttribute("data-model-id")).toBe(qwen.id);
		await openPicker();
		await clickButton("My Claude Code");
		expect(selection().getAttribute("data-model-id")).toBe(claude.id);

		customCatalog = catalogState === "partial" ? [qwen] : [];
		await act(async () => {
			await client.invalidateQueries({ queryKey: ["listCustomBits"] });
		});
		await settle();
		expect(libraryReads).toHaveBeenCalledTimes(2);
		expect(
			client.getQueryState(["listCustomBits", "board-profile"])?.status,
		).toBe("success");
		expect(selection().getAttribute("data-model-id")).toBe(claude.id);

		await openPicker();
		await clickButton("Codex");
		expect(selection().getAttribute("data-provider")).toBe("codex");
		expect(selection().getAttribute("data-model-id")).toBe("default");
		expect(harness.start).toHaveBeenCalledWith(
			expect.objectContaining({ backend: "codex" }),
		);
		await clickButton("Bits");
		expect(selection().getAttribute("data-provider")).toBe("bits");
		expect(selection().getAttribute("data-model-id")).toBe(claude.id);
		await openPicker();

		await clickButton("Add logging");
		const send = host.querySelector("svg.lucide-send")?.closest("button");
		if (!send) throw new Error("Send button is missing");
		expect(send.disabled).toBe(false);
		await act(async () => send.click());
		await settle();
		expect(copilotChat).toHaveBeenCalledTimes(1);
		expect(copilotChat.mock.calls[0]?.[0]).toBe("Board");
		expect(copilotChat.mock.calls[0]?.[1]).toBe(board);
		expect(copilotChat.mock.calls[0]?.[11]).toBe(claude.id);
		expect(copilotChat.mock.calls[0]?.[20]).toBe("Add logging");
		expect(host.textContent).toContain("The board is ready.");
	},
);
