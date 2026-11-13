// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { useBackendStore } from "../../state/backend-state";
import {
	type GlobalChatRun,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { runGlobalChatTool } from "../../state/global-chat/global-chat-tool-registry";
import {
	type FrontendToolRequest,
	type FrontendToolResponse,
	GlobalToolBridge,
} from "./global-tool-bridge";

vi.mock("@flow-like/locales", async (importOriginal) => {
	const translate = (key: string, fallback?: unknown) =>
		typeof fallback === "string" ? fallback : key;
	return {
		...(await importOriginal<typeof import("@flow-like/locales")>()),
		useTranslation: () => ({ t: translate, i18n: { language: "en" } }),
		i18n: { t: translate, language: "en" },
	};
});
vi.mock("react-oidc-context", async (importOriginal) => ({
	...(await importOriginal<typeof import("react-oidc-context")>()),
	useAuth: () => ({
		isAuthenticated: true,
		isLoading: false,
		user: { access_token: "token", profile: { sub: "me" } },
	}),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: () => {}, replace: () => {} }),
	usePathname: () => "/library",
	useSearchParams: () => new URLSearchParams(),
}));
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

function liveRun(runId: string, conversationId: string): GlobalChatRun {
	return {
		runId,
		conversationId,
		status: "streaming",
		startedAt: Date.now(),
		label: "Turn",
		sourceAttachments: [],
		selection: {
			runId,
			provider: "bits",
			selectedModelId: "",
			reasoningEffort: "",
		},
		message: null,
		subPlanSteps: [],
		subAttachments: [],
		subUsageStats: [],
		subWidgets: [],
		pendingAppRefs: [],
		debugReport: null,
		steers: [],
	};
}

const RUNS = {
	flux: liveRun("run-flux", "flux-launch"),
	fluxElsewhere: liveRun("run-flux-2", "flux-notes"),
	scout: liveRun("run-scout", "bot:bot_scout"),
	writer: liveRun("run-writer", "bot:bot_writer"),
};

let root: Root;
let host: HTMLDivElement;
let client: QueryClient;
let requestCount = 0;

/** A call that runs an app event, which the host performs once this client consents. */
function appEventCall(runId: string, appId = "app_atlas"): FrontendToolRequest {
	requestCount += 1;
	return {
		requestId: `request-${requestCount}`,
		toolName: "call_app_event",
		arguments: { app_id: appId, event_id: "send_invoice", payload: {} },
		approval: {
			kind: "execute",
			title: "Approve app event execution",
			sessionKey: "call_app_event",
		},
		approvalOnly: true,
		context: { runId },
	};
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

/** Sends the call; resolves with the prompt it raised (if any) and how the call ended. */
async function ask(request: FrontendToolRequest) {
	let response: FrontendToolResponse | undefined;
	let pending: Promise<FrontendToolResponse> | undefined;
	await act(async () => {
		pending = runGlobalChatTool(request).then((value) => {
			response = value;
			return value;
		});
	});
	await settle();
	const prompt = useGlobalChatStore.getState().toolPrompt;
	return {
		prompt,
		answered: () => response,
		answer: async (approved: boolean, remember = false) => {
			await act(async () => {
				prompt?.respond({ approved, remember });
				await pending;
			});
			return response;
		},
	};
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	useBackendStore.getState().setBackend({
		userState: { getProfile: async () => ({ id: "profile-1" }) },
		boardState: {},
	} as never);
	useGlobalChatStore.setState({
		activeConversationId: "flux-launch",
		runs: Object.fromEntries(
			Object.values(RUNS).map((run) => [run.runId, run]),
		),
		toolPrompt: null,
		autoMode: false,
		botAutoMode: {},
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	client = new QueryClient();
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<GlobalToolBridge />
			</QueryClientProvider>,
		),
	);
	await settle();
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	useGlobalChatStore.setState({ toolPrompt: null, runs: {}, autoMode: false });
});

describe("standing approvals belong to the run that asks", () => {
	test("a successful configuration tool refreshes the bot once after execution", async () => {
		const invalidate = vi.spyOn(client, "invalidateQueries");
		const update = async (status: "progress" | "failed" | "done") => {
			await act(async () =>
				useGlobalChatStore.setState((state) => ({
					runs: {
						...state.runs,
						[RUNS.scout.runId]: {
							...RUNS.scout,
							message: {
								id: "configuration-message",
								plan_steps: [
									{
										id: "configuration-call",
										toolName: "bot_update_configuration",
										title: "Update settings",
										status,
									},
								],
							} as never,
						},
					},
				})),
			);
		};
		await update("progress");
		expect(invalidate).not.toHaveBeenCalled();
		await update("failed");
		expect(invalidate).not.toHaveBeenCalled();
		await update("done");
		expect(invalidate).toHaveBeenCalledWith({ queryKey: ["bots", "list"] });
		expect(invalidate).toHaveBeenCalledWith({
			queryKey: ["bots", "detail", "bot_scout"],
		});
		expect(invalidate).toHaveBeenCalledWith({
			queryKey: ["lookupUserBatched", "bot_scout"],
		});
		const count = invalidate.mock.calls.length;
		await update("done");
		expect(invalidate).toHaveBeenCalledTimes(count);
	});
	test("configuration changes require a fresh answer despite auto mode or standing grants", async () => {
		const putToolGrant = vi.fn();
		useBackendStore.getState().setBackend({
			userState: { getProfile: async () => ({ id: "profile-1" }) },
			boardState: {},
			botState: {
				listToolGrants: async () => [
					{ tool: "bot_update_configuration", scope: "*" },
				],
				putToolGrant,
			},
		} as never);
		useGlobalChatStore.getState().setAutoMode(true, "bot:bot_scout");
		const changes = {
			name: "Atlas",
			instructions: "Keep the full prompt.\n".repeat(100),
		};
		const request = (): FrontendToolRequest => ({
			...appEventCall(RUNS.scout.runId),
			toolName: "bot_update_configuration",
			arguments: { changes },
			approval: {
				kind: "mutating",
				title: "Approve bot settings",
				sessionKey: "shared",
			},
		});
		const first = await ask(request());
		expect(first.answered()).toBeUndefined();
		expect(first.prompt).toMatchObject({
			rememberable: false,
			configurationChanges: changes,
		});
		expect((await first.answer(true, true))?.approved).toBe(true);
		expect(putToolGrant).not.toHaveBeenCalled();

		const second = await ask(request());
		expect(second.answered()).toBeUndefined();
		expect(second.prompt?.rememberable).toBe(false);
		expect((await second.answer(false))?.approved).toBe(false);
	});
	test("Don't ask again for Flux holds for every Flux chat but no bot", async () => {
		const first = await ask(appEventCall(RUNS.flux.runId));
		expect(first.prompt).not.toBeNull();
		expect((await first.answer(true, true))?.approved).toBe(true);

		const otherFluxChat = await ask(appEventCall(RUNS.fluxElsewhere.runId));
		expect(otherFluxChat.prompt).toBeNull();
		expect(otherFluxChat.answered()?.approved).toBe(true);

		const bot = await ask(appEventCall(RUNS.scout.runId));
		expect(bot.prompt).not.toBeNull();
		await bot.answer(false);
	});

	test("one bot's Don't ask again covers neither another bot nor another app", async () => {
		const scout = await ask(appEventCall(RUNS.scout.runId));
		await scout.answer(true, true);
		expect((await ask(appEventCall(RUNS.scout.runId))).prompt).toBeNull();

		const otherApp = await ask(appEventCall(RUNS.scout.runId, "app_ledger"));
		expect(otherApp.prompt).not.toBeNull();
		await otherApp.answer(false);

		const writer = await ask(appEventCall(RUNS.writer.runId));
		expect(writer.prompt).not.toBeNull();
		await writer.answer(false);
	});

	test("a bot's Don't ask again is saved as a grant, and a fresh session never asks again", async () => {
		const saved: Array<[string, string, string]> = [];
		useBackendStore.getState().setBackend({
			userState: { getProfile: async () => ({ id: "profile-1" }) },
			boardState: {},
			botState: {
				listToolGrants: async () =>
					saved.map(([, tool, scope]) => ({ tool, scope })),
				putToolGrant: async (botId: string, tool: string, scope: string) => {
					saved.push([botId, tool, scope]);
					return { tool, scope };
				},
			},
		} as never);
		const remount = async () => {
			await act(async () => root.unmount());
			root = createRoot(host);
			await act(async () =>
				root.render(
					<QueryClientProvider client={new QueryClient()}>
						<GlobalToolBridge />
					</QueryClientProvider>,
				),
			);
			await settle();
		};
		await remount();

		const scout = await ask(appEventCall(RUNS.scout.runId));
		await scout.answer(true, true);
		expect(saved).toEqual([["bot_scout", "call_app_event", "app:app_atlas"]]);

		await remount();
		const later = await ask(appEventCall(RUNS.scout.runId));
		expect(later.prompt).toBeNull();
		expect(later.answered()?.approved).toBe(true);

		const otherApp = await ask(appEventCall(RUNS.scout.runId, "app_ledger"));
		expect(otherApp.prompt).not.toBeNull();
		await otherApp.answer(false);
		expect(saved).toHaveLength(1);
	});

	test("a call asked again after web content skips the standing grant and the session's answer", async () => {
		const putToolGrant = vi.fn(async () => ({}));
		useBackendStore.getState().setBackend({
			userState: { getProfile: async () => ({ id: "profile-1" }) },
			boardState: {},
			botState: {
				listToolGrants: async () => [
					{ tool: "call_app_event", scope: "app:app_atlas" },
				],
				putToolGrant,
			},
		} as never);
		const covered = await ask(appEventCall(RUNS.scout.runId));
		expect(covered.prompt).toBeNull();
		expect(covered.answered()?.approved).toBe(true);

		const askedAgain = (): FrontendToolRequest => {
			const request = appEventCall(RUNS.scout.runId);
			return {
				...request,
				approval: {
					...(request.approval as NonNullable<FrontendToolRequest["approval"]>),
					asksAgain: "web_content",
				},
			};
		};
		const first = await ask(askedAgain());
		expect(first.prompt).toMatchObject({
			asksAgain: "web_content",
			rememberable: false,
		});
		expect((await first.answer(true, true))?.approved).toBe(true);
		expect(putToolGrant).not.toHaveBeenCalled();

		const again = await ask({
			...askedAgain(),
			approval: {
				kind: "execute",
				sessionKey: "call_app_event",
				asks_again: "web_content",
			},
		});
		expect(again.prompt).not.toBeNull();
		expect((await again.answer(false))?.approved).toBe(false);
	});

	test("a bot's auto mode does not waive a call asked again after web content", async () => {
		useGlobalChatStore.getState().setAutoMode(true, "bot:bot_scout");
		const waived = await ask(appEventCall(RUNS.scout.runId));
		expect(waived.prompt).toBeNull();
		expect(waived.answered()?.approved).toBe(true);

		const request = appEventCall(RUNS.scout.runId);
		const askedAgain = await ask({
			...request,
			approval: {
				...(request.approval as NonNullable<FrontendToolRequest["approval"]>),
				asks_again: "web_content",
			},
		});
		expect(askedAgain.prompt).toMatchObject({
			asksAgain: "web_content",
			rememberable: false,
		});
		expect((await askedAgain.answer(false))?.approved).toBe(false);
	});

	test("auto mode switched on in Flux leaves bots asking", async () => {
		useGlobalChatStore.getState().setAutoMode(true);
		const flux = await ask(appEventCall(RUNS.flux.runId));
		expect(flux.prompt).toBeNull();
		expect(flux.answered()?.approved).toBe(true);

		const bot = await ask(appEventCall(RUNS.scout.runId));
		expect(bot.prompt).not.toBeNull();
		await bot.answer(false);
	});

	test("the prompt names the run and the chat that ask", async () => {
		useGlobalChatStore.setState({ activeConversationId: "bot:bot_writer" });
		const scout = await ask(appEventCall(RUNS.scout.runId));
		expect(scout.prompt).toMatchObject({
			runId: "run-scout",
			conversationId: "bot:bot_scout",
		});
		await scout.answer(false);
	});

	test("the prompt says which app a remembered approval is limited to", async () => {
		const scout = await ask(appEventCall(RUNS.scout.runId, "app_ledger"));
		expect(scout.prompt).toMatchObject({
			rememberable: true,
			rememberAppId: "app_ledger",
		});
		await scout.answer(false);
	});

	test("a call no run owns offers nothing to remember", async () => {
		const unowned = await ask(appEventCall("run-gone"));
		expect(unowned.prompt?.rememberable).toBe(false);
		expect(unowned.prompt?.rememberAppId).toBeUndefined();
		await unowned.answer(false);
	});

	test("a call no run owns waives nothing, even beside a lone Flux run in auto mode", async () => {
		useGlobalChatStore.setState({
			runs: { [RUNS.flux.runId]: RUNS.flux },
		});
		useGlobalChatStore.getState().setAutoMode(true);
		const unowned = await ask(appEventCall("run-gone"));
		expect(unowned.prompt).not.toBeNull();
		expect(unowned.prompt?.runId).toBe("run-gone");
		expect(unowned.prompt?.conversationId).toBeUndefined();
		await unowned.answer(false);
	});

	test("a bot's own auto mode lets its runs act and no one else's", async () => {
		useGlobalChatStore.getState().setAutoMode(true, "bot:bot_scout");
		expect(useGlobalChatStore.getState().autoMode).toBe(false);
		const scout = await ask(appEventCall(RUNS.scout.runId));
		expect(scout.prompt).toBeNull();
		expect(scout.answered()?.approved).toBe(true);

		for (const run of [RUNS.writer, RUNS.flux]) {
			const other = await ask(appEventCall(run.runId));
			expect(other.prompt).not.toBeNull();
			await other.answer(false);
		}
	});
});
