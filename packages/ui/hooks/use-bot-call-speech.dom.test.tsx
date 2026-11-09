import { afterAll, beforeAll, expect, test } from "bun:test";
import { i18n } from "@flow-like/locales";
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
import { type IBit, IBitTypes } from "../lib/schema/bit/bit";
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
	isResolved: true,
	ttsModel: "voice",
});

async function fixture(
	desktop = false,
	provider = "hosted:openrouter",
	defaultVoice?: string,
	maxInputCharacters?: unknown,
	options: {
		pin?: string | null;
		profileBits?: IBit[] | Promise<IBit[]>;
		installed?: boolean;
	} = {},
) {
	const requests: unknown[] = [];
	const played: string[] = [];
	let capabilities: readonly string[] = [];
	let lookups = 0;
	let profileLookups = 0;
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
			isBitInstalled: async () => options.installed ?? true,
			getProfileBits: async () => {
				profileLookups++;
				return options.profileBits ?? [];
			},
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
			directory: (id) => ({
				...directory(id),
				ttsModel: options.pin === undefined ? "voice" : options.pin,
			}),
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
		get profileLookups() {
			return profileLookups;
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
		expect(f.profileLookups).toBe(0);
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
	const f = await fixture(false, "hosted:openrouter", "aura-2-thalia-en", 100);
	try {
		await f.join();
		const text = "😀".repeat(4100);
		await f.add("long-answer", 3, text);
		await waitForVoice();
		expect(f.played.map((chunk) => Array.from(chunk).length)).toEqual(
			Array(41).fill(100),
		);
		expect(f.played.join("")).toBe(text);
		expect(f.requests).toHaveLength(41);
	} finally {
		await f.dispose();
	}
});

function profileVoice(id: string, provider: string): IBit {
	return {
		id,
		hub: "hub",
		type: IBitTypes.Tts,
		parameters: {
			provider: { provider_name: provider },
			default_voice: "profile-voice",
		},
	} as IBit;
}

test("unpinned bots use an installed profile voice on desktop or a hosted voice elsewhere", async () => {
	for (const desktop of [false, true]) {
		const f = await fixture(desktop, undefined, undefined, undefined, {
			pin: null,
			profileBits: [
				profileVoice("local-voice", "local:any-tts"),
				profileVoice("hosted-voice", "hosted:openrouter"),
			],
		});
		try {
			expect(f.profileLookups).toBe(0);
			await f.join();
			expect(f.capabilities).toEqual(["bot"]);
			expect(f.lookups).toBe(0);
			await f.add("answer", 3);
			await waitForVoice();
			expect(f.requests).toEqual([
				{
					bit_id: desktop ? "hub:local-voice" : "hosted-voice",
					text: "answer",
					local: desktop,
					voice: "profile-voice",
					format: "wav",
				},
			]);
			expect(f.played).toEqual(["answer"]);
		} finally {
			await f.dispose();
		}
	}
});

test("uninstalled local defaults fall back to hosted speech and absent voices leave text alone", async () => {
	for (const hosted of [false, true]) {
		const f = await fixture(true, undefined, undefined, undefined, {
			pin: null,
			installed: false,
			profileBits: [
				profileVoice("local-voice", "local:any-tts"),
				...(hosted ? [profileVoice("hosted-voice", "hosted:openrouter")] : []),
			],
		});
		try {
			await f.join();
			expect(f.capabilities).toEqual(hosted ? ["bot"] : []);
			await f.add("answer", 3);
			await waitForVoice();
			expect(f.played).toEqual(hosted ? ["answer"] : []);
			if (hosted)
				expect(f.requests[0]).toMatchObject({
					bit_id: "hosted-voice",
					local: false,
				});
		} finally {
			await f.dispose();
		}
	}
});

test("an explicit unavailable voice stays pinned even when the profile has another voice", async () => {
	const f = await fixture(false, "local:any-tts", undefined, undefined, {
		profileBits: [profileVoice("hosted-voice", "hosted:openrouter")],
	});
	try {
		await f.join();
		await f.add("answer", 3);
		await waitForVoice();
		expect(f.capabilities).toEqual([]);
		expect(f.profileLookups).toBe(0);
		expect(f.requests).toEqual([]);
	} finally {
		await f.dispose();
	}
});

test("a default voice resolving after a reply speaks that reply without replaying old history", async () => {
	let resolve!: (bits: IBit[]) => void;
	const profileBits = new Promise<IBit[]>((done) => {
		resolve = done;
	});
	const f = await fixture(false, undefined, undefined, undefined, {
		pin: null,
		profileBits,
	});
	try {
		await f.join();
		await f.add("answer", 3);
		expect(f.requests).toEqual([]);
		await act(async () => {
			resolve([profileVoice("hosted-voice", "hosted:openrouter")]);
		});
		await settle();
		await waitForVoice();
		expect(f.played).toEqual(["answer"]);
	} finally {
		await f.dispose();
	}
});

test("a language change does not rebuild the speech scheduler", async () => {
	const f = await fixture(false, "hosted:openrouter", "af_heart");
	const language = i18n.language;
	try {
		await f.join();
		const stops = f.stops;
		await act(async () => {
			await i18n.changeLanguage(language === "de" ? "en" : "de");
		});
		await f.add("answer", 3);
		await waitForVoice();
		expect(f.stops).toBe(stops);
		expect(f.played).toEqual(["answer"]);
	} finally {
		await act(async () => {
			await i18n.changeLanguage(language);
		});
		await f.dispose();
	}
});
