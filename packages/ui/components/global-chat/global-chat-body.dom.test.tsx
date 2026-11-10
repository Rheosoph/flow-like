// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import botsLocale from "@flow-like/locales/locales/en/bots.json";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, type Ref, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { DEFAULT_CHAT_AI_DISCLOSURE } from "../../lib/chat-appearance";
import { IRole } from "../../lib/schema/llm/history";
import { useAuthStatusStore, useBackendStore } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	type IMessage,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";
import { useChatPreparationStore } from "../../state/global-chat/global-chat-preparation";
import {
	type GlobalChatRun,
	type GlobalToolPrompt,
	MAX_CONCURRENT_GLOBAL_CHAT_RUNS,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { IChatConcurrency } from "../interfaces/chat-default/chat";
import type { ISendMessageFunction } from "../interfaces/chat-default/chatbox";
import { GlobalChatBody } from "./global-chat-body";

const harness = vi.hoisted(() => ({
	send: undefined as ISendMessageFunction | undefined,
	upload: undefined as
		| {
				resolve: (attachments: unknown[]) => void;
		  }
		| undefined,
	bodies: [] as Record<string, unknown>[],
	uploads: [] as { resolve: (attachments: unknown[]) => void }[],
	disclosure: undefined as string | undefined,
	pushed: [] as unknown[],
	concurrency: undefined as IChatConcurrency | undefined,
	loadingText: undefined as string | undefined,
	isStreamActive: false,
	targetGate: undefined as Promise<null> | undefined,
	holdStream: false,
	stream: undefined as { resolve: () => void } | undefined,
	welcome: undefined as
		| { title: string; description?: string; examples?: string[] }
		| undefined,
}));

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
vi.mock("../interfaces/chat-default/chat", async () => {
	const { useImperativeHandle } = await import("react");
	return {
		Chat: (props: {
			ref?: Ref<unknown>;
			onSendMessage: ISendMessageFunction;
			inlinePrompt?: ReactNode;
			inlineContent?: ReactNode;
			concurrency?: IChatConcurrency;
			loadingText?: string;
			isStreamActive?: boolean;
			composerActions?: ReactNode;
			config?: { ai_disclosure?: string };
		}) => {
			useImperativeHandle(props.ref, () => ({
				pushCurrentMessageUpdate: (messages: unknown) =>
					harness.pushed.push(messages),
			}));
			harness.send = props.onSendMessage;
			harness.concurrency = props.concurrency;
			harness.loadingText = props.loadingText;
			harness.isStreamActive = Boolean(props.isStreamActive);
			harness.disclosure = props.config?.ai_disclosure;
			return (
				<>
					<div data-testid="inline-prompt">{props.inlinePrompt}</div>
					<div data-testid="inline-content">{props.inlineContent}</div>
					<div data-testid="composer-actions">{props.composerActions}</div>
				</>
			);
		},
	};
});
vi.mock("../interfaces/chat-default/welcome", () => ({
	ChatWelcome: (props: {
		greeting: { title: string; description?: string; mark?: ReactNode };
		composer: { actions?: ReactNode };
		onSendMessage: ISendMessageFunction;
		config?: { ai_disclosure?: string; example_messages?: string[] };
	}) => {
		harness.send = props.onSendMessage;
		harness.disclosure = props.config?.ai_disclosure;
		harness.welcome = {
			title: props.greeting.title,
			description: props.greeting.description,
			examples: props.config?.example_messages,
		};
		return (
			<>
				<div data-testid="welcome">{props.greeting.mark}</div>
				<div data-testid="composer-actions">{props.composer.actions}</div>
			</>
		);
	},
}));
vi.mock("../interfaces/chat-default/attachment", async (importOriginal) => ({
	...(await importOriginal<
		typeof import("../interfaces/chat-default/attachment")
	>()),
	fileToAttachment: () =>
		new Promise((resolve) => {
			harness.upload = { resolve };
			harness.uploads.push(harness.upload);
		}),
}));
vi.mock("../../state/global-chat/bot-inbox", async (importOriginal) => ({
	...(await importOriginal<
		typeof import("../../state/global-chat/bot-inbox")
	>()),
	ensureBotTarget: () => harness.targetGate ?? Promise.resolve(null),
}));
vi.mock(
	"../../state/global-chat/global-chat-web-transport",
	async (importOriginal) => ({
		...(await importOriginal<
			typeof import("../../state/global-chat/global-chat-web-transport")
		>()),
		webGlobalChatStart:
			(options: { body: Record<string, unknown> }) => async () => {
				harness.bodies.push(options.body);
				if (harness.holdStream)
					await new Promise<void>((resolve) => {
						harness.stream = { resolve };
					});
				return { message: "The answer." };
			},
	}),
);
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

const BOTS = [
	bot("bot_atlas", "Atlas"),
	bot("bot_bard", "Bard"),
	bot("bot_cleo", "Cleo"),
];

/** A turn of `conversationId` that is still streaming. */
function liveRun(runId: string, conversationId: string): GlobalChatRun {
	return {
		runId,
		conversationId,
		status: "streaming",
		startedAt: Date.now(),
		label: "Busy",
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

/** A backend whose profile offers `models` as its own chat models. */
function backendWith(models: unknown[]) {
	return {
		botState: { listBots: async () => BOTS },
		userState: {
			getSettingsProfile: async () => ({
				hub_profile: { id: "profile-1", bits: [] },
			}),
			getProfile: async () => ({ id: "profile-1" }),
		},
		bitState: {
			searchBits: async () => [],
			listCustomBits: async () => models,
			getProfileBits: async () => [],
		},
		capabilities: () => ({}),
	} as never;
}

let root: Root;
let host: HTMLDivElement;

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	await globalChatDb.messages.clear();
	await globalChatDb.sessions.clear();
	harness.send = undefined;
	harness.upload = undefined;
	harness.bodies = [];
	harness.uploads = [];
	harness.pushed = [];
	harness.concurrency = undefined;
	harness.loadingText = undefined;
	harness.isStreamActive = false;
	harness.targetGate = undefined;
	harness.holdStream = false;
	harness.stream = undefined;
	useChatPreparationStore.setState({ replies: {} });
	harness.welcome = undefined;
	useAuthStatusStore.getState().setSignedIn(true);
	useBackendStore.getState().setBackend(backendWith([]));
	useGlobalChatStore.setState({
		activeConversationId: "bot:bot_atlas",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		nativeDrafts: [],
		isStreaming: false,
		streamingMessages: [],
		toolPrompt: null,
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<GlobalChatBody variant="overlay" botId="bot_atlas" />
			</QueryClientProvider>,
		),
	);
	await settle();
});

afterEach(async () => {
	useChatPreparationStore.setState({ replies: {} });
	await act(async () => root.unmount());
	host.remove();
	useGlobalChatStore.setState({
		toolPrompt: null,
		runs: {},
		conversationLoading: false,
	});
});

/** Sends with a file, and while it uploads the user opens `conversationId`. */
async function sendWhileOpening(conversationId: string) {
	expect(harness.send).toBeDefined();
	let sending: Promise<void> | undefined;
	await act(async () => {
		sending = harness.send?.("Summarize the notes", [
			new File(["notes"], "notes.txt", { type: "text/plain" }),
		]);
	});
	expect(harness.upload).toBeDefined();
	await act(async () => {
		useGlobalChatStore.getState().loadConversation(conversationId, []);
	});
	await settle();
	await act(async () => {
		harness.upload?.resolve([
			{ url: "https://files.test/notes.txt", name: "notes.txt" },
		]);
		await sending;
	});
	await settle();
}

describe("immediate reply feedback", () => {
	test("setup is busy with no block of its own, and the empty stream gets the status text", async () => {
		let ready!: () => void;
		harness.targetGate = new Promise<null>((resolve) => {
			ready = () => resolve(null);
		});
		harness.holdStream = true;
		let sending: Promise<void> | undefined;
		await act(async () => {
			sending = harness.send?.("Hello");
		});
		expect(host.textContent).not.toContain("getting started");
		expect(harness.isStreamActive).toBe(true);
		expect(harness.concurrency?.runs).toEqual([]);
		expect(harness.bodies).toEqual([]);
		await act(async () => {
			ready();
		});
		await settle();
		expect(
			Object.keys(useChatPreparationStore.getState().replies),
		).toHaveLength(0);
		expect(Object.keys(useGlobalChatStore.getState().runs)).toHaveLength(1);
		expect(harness.loadingText).toBe("Got it. I’m getting started.");
		expect(harness.isStreamActive).toBe(true);
		await act(async () => {
			harness.stream?.resolve();
			await sending;
		});
		expect(harness.isStreamActive).toBe(false);
		expect(
			useGlobalChatStore
				.getState()
				.messages.map((message) => message.inner.content),
		).toEqual(["Hello", "The answer."]);
		const saved = await globalChatDb.messages.toArray();
		expect(
			saved.some((message) =>
				JSON.stringify(message).includes("getting started"),
			),
		).toBe(false);
	});

	test("reserves preparation capacity without requeueing its own last slot", async () => {
		const sending: (Promise<void> | undefined)[] = [];
		for (let index = 0; index < MAX_CONCURRENT_GLOBAL_CHAT_RUNS; index += 1) {
			await act(async () => {
				sending.push(
					harness.send?.(`Read file ${index}`, [
						new File(["notes"], `notes-${index}.txt`),
					]),
				);
			});
		}
		expect(harness.concurrency?.atCapacity).toBe(true);
		expect(useGlobalChatStore.getState().queue).toEqual([]);
		await act(async () => {
			await harness.send?.("Next question");
		});
		expect(
			useGlobalChatStore.getState().queue.map((entry) => entry.content),
		).toEqual(["Next question"]);
		await act(async () => {
			for (const upload of harness.uploads)
				upload.resolve([
					{ url: "https://files.test/notes.txt", name: "notes.txt" },
				]);
			await Promise.all(sending);
		});
		await settle();
		expect(harness.bodies).toHaveLength(MAX_CONCURRENT_GLOBAL_CHAT_RUNS + 1);
		expect(useGlobalChatStore.getState().queue).toEqual([]);
		expect(
			Object.keys(useChatPreparationStore.getState().replies),
		).toHaveLength(0);
	});

	test("a setup failure clears the acknowledgement and busy controls", async () => {
		let fail!: (reason: Error) => void;
		harness.targetGate = new Promise<null>((_resolve, reject) => {
			fail = reject;
		});
		let sending: Promise<void> | undefined;
		await act(async () => {
			sending = harness.send?.("Hello");
		});
		const result = sending?.catch((error) => error);
		await act(async () => {
			fail(new Error("Inbox unavailable"));
			await result;
		});
		expect(await result).toMatchObject({ message: "Inbox unavailable" });
		expect(host.querySelector("[data-chat-preparation]")).toBeNull();
		expect(harness.isStreamActive).toBe(false);
		expect(harness.concurrency?.runs).toEqual([]);
		expect(harness.bodies).toEqual([]);
	});
});

describe("a send goes where it was typed", () => {
	test("another chat at its run limit doesn't hold it back", async () => {
		const runs: Record<string, GlobalChatRun> = {};
		for (let index = 0; index < MAX_CONCURRENT_GLOBAL_CHAT_RUNS; index += 1) {
			runs[`busy-${index}`] = liveRun(`busy-${index}`, "bot:bot_bard");
		}
		await act(async () => useGlobalChatStore.setState({ runs }));

		await sendWhileOpening("bot:bot_bard");
		expect(useGlobalChatStore.getState().queue).toEqual([]);
		expect(harness.bodies.map((body) => body.bot_id)).toEqual(["bot_atlas"]);
	});

	test("its chat is named after its own bot, not the one opened meanwhile", async () => {
		await sendWhileOpening("bot:bot_bard");
		expect(
			(await globalChatDb.sessions.get("bot:bot_atlas"))?.summarization,
		).toBe("Atlas");
		expect(await globalChatDb.sessions.get("bot:bot_bard")).toBeUndefined();
	});

	test("a bot that moved to the account meanwhile answers under its new id", async () => {
		await act(async () =>
			useGlobalChatStore.setState({ activeConversationId: "bot:bot_cleo" }),
		);
		await settle();
		let sending: Promise<void> | undefined;
		await act(async () => {
			sending = harness.send?.("Draft the memo", [
				new File(["memo"], "memo.txt", { type: "text/plain" }),
			]);
		});
		await act(async () => {
			await useGlobalChatStore
				.getState()
				.rebindBotChat("bot_cleo", "bot_cleo_account");
		});
		await act(async () => {
			harness.upload?.resolve([
				{ url: "https://files.test/memo.txt", name: "memo.txt" },
			]);
			await sending;
		});
		await settle();
		expect(harness.bodies.map((body) => body.bot_id)).toEqual([
			"bot_cleo_account",
		]);
		const saved = await globalChatDb.messages.orderBy("timestamp").toArray();
		expect(saved.map((message) => message.sessionId)).toEqual([
			"bot:bot_cleo_account",
			"bot:bot_cleo_account",
		]);
		expect(saved.map((message) => message.inner.role)).toEqual([
			IRole.User,
			IRole.Assistant,
		]);
	});
});

describe("a tool prompt's card", () => {
	test("turning on auto mode leaves a configuration approval pending", async () => {
		const respond = vi.fn();
		await act(async () => {
			useGlobalChatStore.setState({
				runs: { "run-atlas": liveRun("run-atlas", "bot:bot_atlas") },
				toolPrompt: {
					id: "configure-atlas",
					kind: "approval",
					runId: "run-atlas",
					conversationId: "bot:bot_atlas",
					toolName: "bot_update_configuration",
					title: "Approve bot settings",
					respond,
				},
			});
			useGlobalChatStore.getState().setAutoMode(true, "bot:bot_atlas");
		});
		await settle();
		expect(respond).not.toHaveBeenCalled();
		useGlobalChatStore.getState().setAutoMode(false, "bot:bot_atlas");
	});
	/** A prompt from a run of `conversationId`; without one, no run owns it. */
	async function asks(conversationId?: string) {
		const runId = conversationId && `run-${conversationId}`;
		const prompt: GlobalToolPrompt = {
			id: `prompt-${conversationId ?? "unowned"}`,
			kind: "approval",
			runId: runId || undefined,
			conversationId,
			toolName: "call_app_event",
			title: "Approve app event execution",
			respond: () => {},
		};
		await act(async () => {
			useGlobalChatStore.setState({
				runs: runId ? { [runId]: liveRun(runId, conversationId) } : {},
				toolPrompt: prompt,
			});
		});
		await settle();
	}

	const slot = () =>
		document.body.querySelector("[data-testid='inline-prompt']")?.textContent ??
		"";
	const cardShown = () => slot().includes("Approve app event execution");

	test("shows only in the chat of the run that asks; another chat points there", async () => {
		await asks("bot:bot_bard");
		expect(cardShown()).toBe(false);
		expect(slot()).toContain(
			"Bard is waiting for your answer in another chat.",
		);

		const open = Array.from(
			document.body.querySelectorAll<HTMLButtonElement>(
				"[data-testid='inline-prompt'] button",
			),
		).find((button) => button.textContent === "Open chat");
		await act(async () => open?.click());
		await settle();
		expect(useGlobalChatStore.getState().activeConversationId).toBe(
			"bot:bot_bard",
		);
		expect(cardShown()).toBe(true);
	});

	test("a card of one Flux chat never shows in another Flux chat", async () => {
		await show("flux-notes");
		await asks("flux-launch");
		expect(cardShown()).toBe(false);
		expect(slot()).toContain(
			"Flux is waiting for your answer in another chat.",
		);

		await show("flux-launch");
		expect(cardShown()).toBe(true);
	});

	test("a prompt no run owns shows in whichever chat is open, as before", async () => {
		await asks(undefined);
		expect(cardShown()).toBe(true);
		await show("bot:bot_bard");
		expect(cardShown()).toBe(true);
		await show("flux-notes");
		expect(cardShown()).toBe(true);
	});
});

async function show(conversationId: string, messages: IMessage[] = []) {
	await act(async () => {
		useGlobalChatStore.getState().loadConversation(conversationId, messages);
	});
	await settle();
}

const welcomeShown = () =>
	document.body.querySelector("[data-testid='welcome']") !== null;
const transcriptShown = () =>
	document.body.querySelector("[data-testid='inline-prompt']") !== null;

function message(id: string, sessionId: string): IMessage {
	return {
		id,
		sessionId,
		appId: "global",
		files: [],
		inner: { role: IRole.User, content: "Hi" },
		timestamp: 1,
	};
}

describe("an empty chat opens on the standard welcome", () => {
	test("a bot's welcome greets with its name and face and suggests nothing", async () => {
		expect(welcomeShown()).toBe(true);
		expect(transcriptShown()).toBe(false);
		expect(harness.welcome?.title).toBe("Atlas");
		expect(harness.welcome?.examples).toBeUndefined();
		expect(
			document.body.querySelector("[data-testid='welcome'] svg") !== null,
		).toBe(true);
	});

	test("Flux's welcome suggests what it builds", async () => {
		await show("flux-notes");
		expect(harness.welcome?.title).toBe("Flux");
		expect(harness.welcome?.examples).toEqual([
			"Create a new app",
			"Show me the package store",
			"What can I build with Flow-Like?",
		]);
	});

	test("a chat with messages shows its transcript", async () => {
		await show("bot:bot_bard", [message("hello", "bot:bot_bard")]);
		expect(welcomeShown()).toBe(false);
		expect(transcriptShown()).toBe(true);
	});

	test("the transcript that takes over shows the replies already streaming", async () => {
		const reply = message("reply", "bot:bot_atlas");
		await act(async () => {
			useGlobalChatStore.setState({
				runs: { reply: liveRun("reply", "bot:bot_atlas") },
				streamingMessages: [reply],
			});
		});
		await settle();
		expect(transcriptShown()).toBe(true);
		expect(harness.pushed.at(-1)).toEqual([reply]);
	});
});

describe("switching to another conversation", () => {
	const loadingShown = () =>
		document.body.textContent?.includes("Loading conversation...") ?? false;

	test("shows the chat's loading state instead of the previous chat until the next one lands", async () => {
		await show("bot:bot_atlas", [message("hello", "bot:bot_atlas")]);
		expect(transcriptShown()).toBe(true);

		await act(async () =>
			useGlobalChatStore.getState().setConversationLoading(true),
		);
		expect(loadingShown()).toBe(true);
		expect(transcriptShown()).toBe(false);
		expect(welcomeShown()).toBe(false);

		await show("bot:bot_bard");
		expect(loadingShown()).toBe(false);
		expect(welcomeShown()).toBe(true);
	});

	test("an empty chat shows it too, not its welcome", async () => {
		expect(welcomeShown()).toBe(true);
		await act(async () =>
			useGlobalChatStore.getState().setConversationLoading(true),
		);
		expect(loadingShown()).toBe(true);
		expect(welcomeShown()).toBe(false);
	});
});

describe("the composer names the model that answers", () => {
	const GPT = {
		id: "bit-gpt",
		type: "Llm",
		meta: { en: { name: "GPT Test", tags: [] } },
	};
	const actions = () =>
		document.body.querySelector("[data-testid='composer-actions']")
			?.textContent ?? "";

	async function remountWith(models: unknown[]) {
		await act(async () => root.unmount());
		useBackendStore.getState().setBackend(backendWith(models));
		root = createRoot(host);
		await act(async () =>
			root.render(
				<QueryClientProvider
					client={
						new QueryClient({ defaultOptions: { queries: { retry: false } } })
					}
				>
					<GlobalChatBody variant="overlay" botId="bot_atlas" />
				</QueryClientProvider>,
			),
		);
		await settle();
	}

	test("a bot without a model of its own shows the chat's default one", async () => {
		await remountWith([GPT]);
		expect(actions()).toContain("GPT Test");
		expect(actions()).toContain(
			"Atlas has no model of its own, so it uses the chat's default model",
		);
	});

	test("so does its chat once it has messages, and Flux's", async () => {
		await remountWith([GPT]);
		await show("bot:bot_atlas", [message("hello", "bot:bot_atlas")]);
		expect(transcriptShown()).toBe(true);
		expect(actions()).toContain("GPT Test");
		await show("flux-notes");
		expect(actions()).toContain("GPT Test");
	});

	test("without a chat model there is nothing to show", async () => {
		await remountWith([]);
		expect(actions()).not.toContain("GPT Test");
		expect(actions()).not.toContain("no model of its own");
	});
});

describe("the AI disclosure under the composer", () => {
	test("names the bot the open chat talks to, never FlowPilot in another bot's chat", async () => {
		expect(harness.disclosure).toBe(
			"Atlas is an AI assistant. Responses may be inaccurate—check important information.",
		);
		await show("bot:bot_bard");
		expect(harness.disclosure).toBe(
			"Bard is an AI assistant. Responses may be inaccurate—check important information.",
		);
		await show("bot:general");
		expect(harness.disclosure).toBe(
			"Assistant is an AI assistant. Responses may be inaccurate—check important information.",
		);
		await show("flux-notes");
		expect(harness.disclosure).toBe(
			"Flux is an AI assistant. Responses may be inaccurate—check important information.",
		);
	});

	test("is the neutral default in the chat of a bot that's gone", async () => {
		await show("bot:bot_gone");
		expect(harness.disclosure).toBe(DEFAULT_CHAT_AI_DISCLOSURE);
		expect(harness.disclosure).not.toContain("FlowPilot");
	});
});
