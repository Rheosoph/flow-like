import { afterAll, beforeAll, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { foldInboxEntries } from "../components/bots-tab/fold-entries";
import {
	installDomGlobals,
	restoreDomGlobals,
	settle,
	domWindow as window,
} from "../components/bots-tab/testing/dom-harness";
import type { CrewDirectory } from "../components/bots-tab/use-crew-directory";
import { useBackendStore } from "../state/backend-state";
import type {
	IConversation,
	IInboxEntry,
} from "../state/backend-state/inbox-state";
import { useBotCallSpeech } from "./use-bot-call-speech";
import type { ConversationCallState } from "./use-conversation-call";

installDomGlobals();
const { createRoot } = await import("react-dom/client");
const previousBackend = useBackendStore.getState().backend;
beforeAll(installDomGlobals);
afterAll(() => {
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});
const conversation = {
	id: "c1",
	kind: "direct",
	members: [{ principal_id: "bot", kind: "bot" }],
} as IConversation;
function entry(id: string, seq: number, reply = false, text = id): IInboxEntry {
	return {
		id,
		message_id: id,
		conversation_id: "c1",
		sender_id: reply ? "bot" : "me",
		kind: reply ? "message" : "transcript",
		envelope: {
			v: 1,
			enc: "none",
			body: reply
				? { reply_to: "spoken", text }
				: { call: "call", text: "Hello", offset_ms: 0 },
		},
		cursor: `${seq}:e_${id}`,
		created_at: new Date(seq * 1000).toISOString(),
	};
}
const directory: CrewDirectory = (id) => ({
	id,
	name: "Bot",
	isMe: false,
	isBot: true,
	isPending: false,
	ttsModel: "voice",
});

async function fixture(
	desktop = false,
	provider = "hosted:openrouter",
	defaultVoice?: string,
	maxInputCharacters?: unknown,
) {
	const requests: unknown[] = [];
	const played: string[] = [];
	let capabilities: readonly string[] = [];
	let lookups = 0;
	let stops = 0;
	useBackendStore.getState().setBackend({
		capabilities: () => ({ canExecuteLocally: desktop }),
		bitState: {
			getBit: async () => {
				lookups++;
				return {
					id: "voice",
					hub: "hub",
					type: "Tts",
					parameters: {
						provider: { provider_name: provider },
						default_voice: defaultVoice,
						max_input_characters: maxInputCharacters,
					},
				};
			},
			isBitInstalled: async () => true,
		},
		audioState: {
			speak: async (request: { text: string }) => {
				requests.push(request);
				return new Blob([request.text]);
			},
		},
	} as never);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const host = window.document.createElement("div");
	window.document.body.append(host);
	const root = createRoot(host as never);
	let joined = false;
	let entries = [entry("spoken", 1), entry("old", 2, true)];
	const controls = {
		setBotVoiceCapabilities: (ids: readonly string[]) => {
			capabilities = ids;
		},
		canSpeakForBot: () => capabilities.includes("bot"),
		playBotSpeech: async (blob: Blob) => {
			played.push(await blob.text());
		},
		stopBotSpeech: () => {
			stops++;
		},
	};
	function Harness() {
		useBotCallSpeech({
			conversation,
			entries,
			items: foldInboxEntries(entries),
			historyReady: true,
			directory,
			call: {
				...controls,
				joined,
				live: joined ? { callId: "call" } : null,
			} as unknown as ConversationCallState,
		});
		return null;
	}
	async function render() {
		await act(async () =>
			root.render(
				<QueryClientProvider client={client}>
					<Harness />
				</QueryClientProvider>,
			),
		);
		await settle();
	}
	await render();
	return {
		requests,
		played,
		get capabilities() {
			return capabilities;
		},
		get lookups() {
			return lookups;
		},
		get stops() {
			return stops;
		},
		async join() {
			joined = true;
			await render();
		},
		async add(id: string, seq: number, text = id) {
			entries = [...entries, entry(id, seq, true, text)];
			await render();
		},
		async leave() {
			joined = false;
			await render();
		},
		async dispose() {
			await act(async () => root.unmount());
			client.clear();
			host.remove();
		},
	};
}

async function waitForVoice() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 400));
	});
}

test("joined calls synthesize only new replies with the pinned hosted voice and stop on leave", async () => {
	const f = await fixture(false, "hosted:openrouter", "af_heart");
	try {
		expect(f.lookups).toBe(0);
		await f.join();
		expect(f.capabilities).toEqual(["bot"]);
		expect(f.requests).toEqual([]);
		await f.add("answer", 3);
		await waitForVoice();
		expect(f.requests).toEqual([
			{
				bit_id: "voice",
				text: "answer",
				local: false,
				voice: "af_heart",
				format: "wav",
			},
		]);
		expect(f.played).toEqual(["answer"]);
		await f.leave();
		expect(f.capabilities).toEqual([]);
		expect(f.stops).toBeGreaterThan(0);
		await f.add("after-leave", 4);
		await waitForVoice();
		expect(f.requests).toHaveLength(1);
	} finally {
		await f.dispose();
	}
});

test("installed local voices are advertised only by a desktop and use a qualified Bit ID", async () => {
	for (const desktop of [false, true]) {
		const f = await fixture(desktop, "local:any-tts");
		try {
			await f.join();
			expect(f.capabilities).toEqual(desktop ? ["bot"] : []);
			await f.add("answer", 3);
			await waitForVoice();
			expect(f.requests).toEqual(
				desktop
					? [
							{
								bit_id: "hub:voice",
								text: "answer",
								local: true,
								format: "wav",
							},
						]
					: [],
			);
		} finally {
			await f.dispose();
		}
	}
});

test("catalog speech limits split long call replies without dropping text", async () => {
	const f = await fixture(false, "hosted:openrouter", "aura-2-thalia-en", 2000);
	try {
		await f.join();
		const text = "😀".repeat(4100);
		await f.add("long-answer", 3, text);
		await waitForVoice();
		expect(f.played.map((chunk) => Array.from(chunk).length)).toEqual([
			2000, 2000, 100,
		]);
		expect(f.played.join("")).toBe(text);
		expect(f.requests).toHaveLength(3);
	} finally {
		await f.dispose();
	}
});
