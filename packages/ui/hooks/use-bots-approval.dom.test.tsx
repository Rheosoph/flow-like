import {
	afterAll,
	afterEach,
	beforeAll,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import { act, createElement } from "react";
import {
	installDomGlobals,
	restoreDomGlobals,
	settle,
} from "../components/bots-tab/testing/dom-harness";

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useBackendStore } = await import("../state/backend-state");
const { useAnswerBotApproval } = await import("./use-bots");

type Variables = Parameters<
	ReturnType<typeof useAnswerBotApproval>["mutateAsync"]
>[0];

const CARD = { id: "ap-7", turn_id: "turn_1", tool: "shell_run" } as never;
const ENVELOPES = { u1: { v: 1, enc: "x" } };

let answers: unknown[][] = [];
let sealed: Array<[string, unknown]> = [];
let sealsEntries = true;
let encrypted = true;
let run: ((variables: Variables) => Promise<unknown>) | undefined;

const botState = {
	answerApproval: async (...args: unknown[]) => {
		answers.push(args);
	},
};

function inboxState() {
	return sealsEntries
		? {
				sealEntry: async (conversationId: string, request: unknown) => {
					sealed.push([conversationId, request]);
					return encrypted
						? { message_id: "ar-7", kind: "message", envelopes: ENVELOPES }
						: request;
				},
			}
		: {};
}

function Probe() {
	run = useAnswerBotApproval().mutateAsync;
	return null;
}

const previousBackend = useBackendStore.getState().backend;
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	useBackendStore.getState().setBackend(previousBackend as never);
	restoreDomGlobals();
});

async function mount() {
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	useBackendStore
		.getState()
		.setBackend({ botState, inboxState: inboxState() } as never);
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client: new QueryClient() },
				createElement(Probe),
			),
		),
	);
	await settle();
}

beforeEach(() => {
	answers = [];
	sealed = [];
	sealsEntries = true;
	encrypted = true;
});

afterEach(async () => {
	await act(async () => root.unmount());
});

const base = {
	botId: "bot_a",
	turnId: "turn_1",
	approvalId: "ap-7",
	decision: "once" as const,
};

describe("useAnswerBotApproval", () => {
	test("seals the answer entry of an encrypted chat and sends it with the card", async () => {
		await mount();
		await act(async () => {
			await run?.({ ...base, conversationId: "conv", card: CARD });
		});
		expect(sealed[0]?.[0]).toBe("conv");
		expect(sealed[0]?.[1]).toMatchObject({
			message_id: "ar-7",
			kind: "message",
			envelope: {
				body: {
					text: "",
					reply_to: "ap-7",
					approval_response: { approval_id: "ap-7", decision: "once" },
				},
			},
		});
		expect(answers).toEqual([
			[
				"bot_a",
				"turn_1",
				"ap-7",
				"once",
				undefined,
				{ card: CARD, envelopes: ENVELOPES },
			],
		]);
	});

	test("answers as before in a plain chat", async () => {
		encrypted = false;
		await mount();
		await act(async () => {
			await run?.({ ...base, conversationId: "conv", card: CARD });
		});
		expect(answers).toEqual([["bot_a", "turn_1", "ap-7", "once"]]);
	});

	test("answers as before without a card", async () => {
		await mount();
		await act(async () => {
			await run?.(base);
		});
		expect(sealed).toEqual([]);
		expect(answers).toEqual([["bot_a", "turn_1", "ap-7", "once"]]);
	});

	test("answers as before when the inbox cannot seal", async () => {
		sealsEntries = false;
		await mount();
		await act(async () => {
			await run?.({ ...base, conversationId: "conv", card: CARD });
		});
		expect(answers).toEqual([["bot_a", "turn_1", "ap-7", "once"]]);
	});
});
