// @vitest-environment happy-dom

import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
	clearIncomingCallJoin,
	registerIncomingCallScope,
	requestIncomingCallJoin,
} from "../lib/realtime/incoming-call-join";
import type { IConversation } from "../state/backend-state/inbox-state";
import type { ConversationRoom, RoomAwareness } from "./use-conversation-room";

const mocks = vi.hoisted(() => ({
	startCallListener: vi.fn(),
	sendEntry: vi.fn(),
	transcribe: vi.fn(),
	setLocalStream: vi.fn(),
	setLocalStreams: vi.fn(),
	setAllowedPeers: vi.fn(),
	disposeEngine: vi.fn(),
	warmAudio: vi.fn(async () => {}),
	warmListening: vi.fn(async () => {}),
	cancelRun: vi.fn(async () => true),
	posted: vi.fn(),
	firstText: vi.fn(),
	storeListeners: new Set<() => void>(),
	storeState: { runs: {} as Record<string, unknown> },
}));
vi.mock("../state/global-chat/global-chat-stream", () => ({
	cancelGlobalChatRun: mocks.cancelRun,
}));
vi.mock("../state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: {
		getState: () => mocks.storeState,
		subscribe: (listener: () => void) => {
			mocks.storeListeners.add(listener);
			return () => mocks.storeListeners.delete(listener);
		},
	},
}));
vi.mock("../lib/realtime/call-latency", () => ({
	callLatency: {
		transcriptPosted: mocks.posted,
		firstText: mocks.firstText,
	},
}));
vi.mock("../state/backend-state", () => {
	const backend = {
		audioState: { transcribe: mocks.transcribe, warm: mocks.warmAudio },
	};
	return { useBackend: () => backend };
});
vi.mock("./use-inbox", () => ({
	useSendInboxEntry: () => ({ mutateAsync: mocks.sendEntry }),
	newInboxMessageId: () => "message-id",
}));
vi.mock("../lib/realtime/call-listener", () => ({
	startCallListener: mocks.startCallListener,
	warmCallListening: mocks.warmListening,
}));
vi.mock("../lib/realtime/conversation-call", async (importOriginal) => ({
	...(await importOriginal<
		typeof import("../lib/realtime/conversation-call")
	>()),
	createCallEngine: () => ({
		setLocalStream: mocks.setLocalStream,
		setLocalStreams: mocks.setLocalStreams,
		setAllowedPeers: mocks.setAllowedPeers,
		onRemote: () => () => {},
		dispose: mocks.disposeEngine,
	}),
}));

import { useBackend } from "../state/backend-state";
import {
	type VoiceTurnSender,
	registerVoiceTurnSender,
} from "../state/global-chat/voice-turn";
import {
	type ConversationCallState,
	useConversationCall,
} from "./use-conversation-call";

function media(id: string) {
	const track = {
		kind: "audio",
		readyState: "live",
		enabled: true,
		stop: vi.fn(),
	};
	const stream = {
		id,
		getTracks: () => [track],
		getAudioTracks: () => [track],
		getVideoTracks: () => [],
	} as unknown as MediaStream;
	return { stream, track };
}

function node() {
	return { connect: vi.fn(), disconnect: vi.fn() };
}

class FakeBufferSource {
	buffer: unknown;
	onended: (() => void) | null = null;
	connect = vi.fn();
	disconnect = vi.fn();
	start = vi.fn();
	stop = vi.fn();
}

let initialContextState: AudioContextState;
const contexts: FakeAudioContext[] = [];
class FakeAudioContext extends EventTarget {
	state = initialContextState;
	destination = node();
	output = media("mixed-audio");
	sources: FakeBufferSource[] = [];
	resume = vi.fn(async () => {});
	close = vi.fn(async () => {
		this.state = "closed";
	});
	decodeAudioData = vi.fn(async () => ({}));
	currentTime = 0;
	createGain = vi.fn(() => ({
		...node(),
		gain: { setTargetAtTime: vi.fn() },
	}));
	createMediaStreamSource = vi.fn(() => node());
	createMediaStreamDestination = vi.fn(() => ({
		...node(),
		stream: this.output.stream,
	}));
	createAnalyser = vi.fn(() => ({
		...node(),
		fftSize: 512,
		getByteTimeDomainData: (buffer: Uint8Array) => buffer.fill(128),
	}));
	createBufferSource = vi.fn(() => {
		const source = new FakeBufferSource();
		this.sources.push(source);
		return source;
	});
	constructor() {
		super();
		contexts.push(this);
	}
}

function awareness() {
	const states = new Map<number, unknown>([[1, { sub: "me" }]]);
	const listeners = new Set<() => void>();
	const changed = () => {
		for (const listener of listeners) listener();
	};
	return {
		states,
		changed,
		api: {
			clientID: 1,
			getStates: () => states,
			getLocalState: () => states.get(1) as Record<string, unknown>,
			setLocalStateField: (field: string, value: unknown) => {
				states.set(1, { ...(states.get(1) as object), [field]: value });
				changed();
			},
			on: (_event: string, listener: () => void) => {
				listeners.add(listener);
			},
			off: (_event: string, listener: () => void) => {
				listeners.delete(listener);
			},
		} satisfies RoomAwareness,
	};
}

const conversation = {
	id: "conversation",
	members: [
		{ kind: "user", principal_id: "me" },
		{ kind: "user", principal_id: "other" },
		{ kind: "bot", principal_id: "bot" },
	],
} as unknown as IConversation;
let root: Root | undefined;
let latest: ConversationCallState;
let microphone: ReturnType<typeof media>;
let getUserMedia: ReturnType<typeof vi.fn>;
let retireScope: () => void;
let speechModel: { bitId: string; route: "remote" } | null;
let listener: {
	setPaused: ReturnType<typeof vi.fn>;
	setGuarded: ReturnType<typeof vi.fn>;
	retryBargeIn: ReturnType<typeof vi.fn>;
	endBargeIn: ReturnType<typeof vi.fn>;
	stop: ReturnType<typeof vi.fn>;
};

beforeEach(() => {
	vi.clearAllMocks();
	vi.useFakeTimers();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("AudioContext", FakeAudioContext);
	initialContextState = "running";
	speechModel = { bitId: "stt-model", route: "remote" };
	contexts.length = 0;
	retireScope = registerIncomingCallScope(useBackend(), "hub:me");
	microphone = media("microphone");
	getUserMedia = vi.fn(async () => microphone.stream);
	Object.defineProperty(navigator, "mediaDevices", {
		configurable: true,
		value: { getUserMedia },
	});
	listener = {
		setPaused: vi.fn(),
		setGuarded: vi.fn(),
		retryBargeIn: vi.fn(() => false),
		endBargeIn: vi.fn(),
		stop: vi.fn(),
	};
	mocks.startCallListener.mockResolvedValue(listener);
	mocks.sendEntry.mockResolvedValue(undefined);
	mocks.storeState.runs = {};
	mocks.storeListeners.clear();
});

afterEach(async () => {
	await act(async () => root?.unmount());
	clearIncomingCallJoin();
	retireScope();
	root = undefined;
	document.body.innerHTML = "";
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

async function setup(target: IConversation = conversation) {
	const presence = awareness();
	const room: ConversationRoom = {
		awareness: presence.api,
		provider: {
			room: { peerId: "local-peer", webrtcConns: new Map() },
			on() {},
			off() {},
		},
	};
	function Probe() {
		latest = useConversationCall(
			target,
			"me",
			room,
			undefined,
			{
				stt: speechModel,
				hasBots: true,
				mentionsIn: () => ["bot"],
			},
			{ stableCallId: "call-one" },
		);
		return null;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const mountedRoot = createRoot(container);
	root = mountedRoot;
	await act(async () => mountedRoot.render(createElement(Probe)));
	return presence;
}

async function joined(target?: IConversation) {
	const presence = await setup(target);
	await act(async () => {
		await latest.join();
	});
	return presence;
}

test("an incoming Join waits for live presence, reuses click audio, and joins only once", async () => {
	requestIncomingCallJoin({
		scope: "hub:me",
		userId: "me",
		conversationId: "conversation",
		callId: "call-one",
	});
	const presence = await setup();
	expect(getUserMedia).not.toHaveBeenCalled();
	await act(async () => {
		presence.states.set(2, {
			sub: "other",
			call: { id: "call-one", peer: "remote-peer", joined: true },
		});
		presence.changed();
	});
	expect(latest.joined).toBe(true);
	expect(getUserMedia).toHaveBeenCalledOnce();
	expect(contexts).toHaveLength(1);
	await act(async () => {
		presence.changed();
	});
	expect(getUserMedia).toHaveBeenCalledOnce();
});

test("incoming Join reports denied microphone access without retrying automatically", async () => {
	getUserMedia.mockRejectedValue(new DOMException("Denied", "NotAllowedError"));
	const presence = await setup();
	await act(async () => {
		presence.states.set(2, {
			sub: "other",
			call: { id: "call-one", peer: "remote-peer", joined: true },
		});
		presence.changed();
		requestIncomingCallJoin({
			scope: "hub:me",
			userId: "me",
			conversationId: "conversation",
			callId: "call-one",
		});
	});
	expect(latest.joined).toBe(false);
	expect(latest.error).toBe("denied");
	expect(contexts[0].close).toHaveBeenCalledOnce();
	await act(async () => {
		presence.changed();
	});
	expect(getUserMedia).toHaveBeenCalledOnce();
});

test("a call ending during microphone permission does not leave an orphaned call", async () => {
	let allow!: (stream: MediaStream) => void;
	getUserMedia.mockImplementation(
		() =>
			new Promise<MediaStream>((resolve) => {
				allow = resolve;
			}),
	);
	const presence = await setup();
	await act(async () => {
		presence.states.set(2, {
			sub: "other",
			call: { id: "call-one", peer: "remote-peer", joined: true },
		});
		presence.changed();
		requestIncomingCallJoin({
			scope: "hub:me",
			userId: "me",
			conversationId: "conversation",
			callId: "call-one",
		});
	});
	await act(async () => {
		presence.states.delete(2);
		presence.changed();
	});
	await act(async () => {
		allow(microphone.stream);
	});
	expect(latest.joined).toBe(false);
	expect(microphone.track.stop).toHaveBeenCalledOnce();
	expect(contexts[0].close).toHaveBeenCalledOnce();
});

async function listen() {
	await act(async () => {
		latest.setListening(true);
	});
	expect(mocks.startCallListener).toHaveBeenCalledWith(
		microphone.stream,
		expect.any(Function),
		expect.objectContaining({
			context: contexts[0],
			paused: false,
			onPartial: expect.any(Function),
			onReset: expect.any(Function),
		}),
	);
	expect(latest.listening).toBe(true);
}

async function play() {
	let playback: Promise<void> = Promise.resolve();
	await act(async () => {
		latest.setBotVoiceCapabilities(["bot"]);
		playback = latest.playBotSpeech(
			{ arrayBuffer: async () => new ArrayBuffer(4) } as Blob,
			"bot",
		);
	});
	return { playback, source: contexts[0].sources[0] };
}

function localCall(presence: ReturnType<typeof awareness>) {
	return (
		presence.states.get(1) as {
			call?: { muted: boolean; listening: boolean; speakingBot: string | null };
		}
	).call;
}

describe("call transcription delivery", () => {
	const utterance = {
		samples: new Float32Array(16_000).fill(0.1),
		offsetMs: 0,
		durationMs: 1000,
	};

	test("a spoken utterance posts the transcript and its addressed bot", async () => {
		mocks.transcribe.mockResolvedValue({ text: "  Bot, check the plan.  " });
		await joined();
		await listen();
		await act(async () => {
			mocks.startCallListener.mock.calls[0][1](utterance);
		});
		expect(mocks.transcribe).toHaveBeenCalledWith({
			bit_id: "stt-model",
			audio_base64: expect.any(String),
			mime: "audio/wav",
			local: false,
		});
		expect(mocks.sendEntry).toHaveBeenCalledWith({
			conversationId: "conversation",
			request: {
				message_id: "message-id",
				kind: "transcript",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						call: "call-one",
						text: "Bot, check the plan.",
						offset_ms: 0,
						duration_ms: 1000,
						mentions: ["bot"],
					},
				},
			},
		});
	});

	test("drafts update while speaking and only the final transcript wakes the bot", async () => {
		mocks.transcribe
			.mockResolvedValueOnce({ text: "Bot, check" })
			.mockResolvedValueOnce({ text: "Bot, check the plan" })
			.mockResolvedValueOnce({ text: "Bot, check the plan." });
		await joined();
		await listen();
		const [, final, capture] = mocks.startCallListener.mock.calls[0];
		await act(async () => capture.onPartial(utterance));
		expect(latest.transcriptDraft).toBe("Bot, check");
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		await act(async () =>
			capture.onPartial({ ...utterance, durationMs: 2000 }),
		);
		expect(latest.transcriptDraft).toBe("Bot, check the plan");
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		await act(async () => final({ ...utterance, durationMs: 2200 }));
		expect(latest.transcriptDraft).toBe("");
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
		expect(
			mocks.sendEntry.mock.calls[0][0].request.envelope.body,
		).toMatchObject({
			text: "Bot, check the plan.",
			mentions: ["bot"],
		});
	});

	test("muting invalidates a pending draft without disabling listening", async () => {
		let finish!: (value: { text: string }) => void;
		mocks.transcribe.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		await joined();
		await listen();
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onPartial(utterance));
		await act(async () => latest.toggleMute());
		await act(async () => finish({ text: "stale draft" }));
		expect(latest.transcriptDraft).toBe("");
		expect(latest.listening).toBe(true);
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		mocks.transcribe.mockResolvedValue({ text: "new draft" });
		await act(async () => latest.toggleMute());
		await act(async () => capture.onPartial({ ...utterance, offsetMs: 3000 }));
		expect(latest.transcriptDraft).toBe("new draft");
	});

	test("turning listening off clears drafts and rejects late preview results", async () => {
		let finish!: (value: { text: string }) => void;
		mocks.transcribe
			.mockResolvedValueOnce({ text: "first draft" })
			.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						finish = resolve;
					}),
			);
		await joined();
		await listen();
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onPartial(utterance));
		expect(latest.transcriptDraft).toBe("first draft");
		await act(async () =>
			capture.onPartial({ ...utterance, durationMs: 2000 }),
		);
		await act(async () => latest.setListening(false));
		expect(latest.transcriptDraft).toBe("");
		await act(async () => {
			latest.setListening(true);
			capture.onPartial(utterance);
			finish({ text: "stale draft" });
		});
		expect(latest.transcriptDraft).toBe("");
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		expect(mocks.transcribe).toHaveBeenCalledTimes(2);
	});

	test("transcript offsets use capture time instead of the time recognition completes", async () => {
		mocks.transcribe.mockResolvedValue({ text: "After a pause" });
		await joined();
		await listen();
		const capturedAt = Date.now() + 3000;
		await act(async () => {
			vi.advanceTimersByTime(20_000);
			mocks.startCallListener.mock.calls[0][1]({ ...utterance, capturedAt });
		});
		expect(
			mocks.sendEntry.mock.calls[0][0].request.envelope.body.offset_ms,
		).toBe(3000);
	});

	test("speech model changes take effect while listening stays on", async () => {
		mocks.transcribe.mockResolvedValue({ text: "Draft" });
		await joined();
		await listen();
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onPartial(utterance));
		speechModel = { bitId: "replacement", route: "remote" };
		await act(async () => latest.toggleMute());
		await act(async () => latest.toggleMute());
		await act(async () => capture.onPartial({ ...utterance, offsetMs: 3000 }));
		expect(mocks.transcribe).toHaveBeenLastCalledWith(
			expect.objectContaining({ bit_id: "replacement" }),
		);
		speechModel = null;
		await act(async () => latest.toggleMute());
		await act(async () => latest.toggleMute());
		await act(async () => capture.onPartial({ ...utterance, offsetMs: 5000 }));
		expect(mocks.transcribe).toHaveBeenCalledTimes(2);
		expect(latest.canListen).toBe(false);
	});

	test("switching listening off and back on discards the old transcription and callback", async () => {
		let finish!: (value: { text: string }) => void;
		mocks.transcribe.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		mocks.transcribe.mockResolvedValue({ text: "new utterance" });
		await joined();
		await listen();
		const previousCapture = mocks.startCallListener.mock.calls[0][1];
		await act(async () => previousCapture(utterance));
		await act(async () => {
			latest.setListening(false);
			latest.setListening(true);
			previousCapture(utterance);
			finish({ text: "discard this old audio" });
		});
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		expect(mocks.transcribe).toHaveBeenCalledOnce();
		await act(async () => {
			mocks.startCallListener.mock.calls[1][1](utterance);
		});
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
		expect(mocks.sendEntry.mock.calls[0][0].request.envelope.body.text).toBe(
			"new utterance",
		);
	});

	test("a stale capture startup cannot replace the current listener", async () => {
		const oldListener = {
			setPaused: vi.fn(),
			setGuarded: vi.fn(),
			retryBargeIn: vi.fn(() => false),
			endBargeIn: vi.fn(),
			stop: vi.fn(),
		};
		let finish!: (value: typeof oldListener) => void;
		mocks.startCallListener.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		await joined();
		await listen();
		await act(async () => {
			latest.setListening(false);
			latest.setListening(true);
			finish(oldListener);
		});
		expect(oldListener.stop).toHaveBeenCalledOnce();
		expect(listener.stop).not.toHaveBeenCalled();
		await act(async () => latest.toggleMute());
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
		expect(latest.listening).toBe(true);
	});

	test("capture and transcription failures become visible call errors", async () => {
		mocks.startCallListener.mockRejectedValueOnce(new Error("capture failed"));
		await joined();
		await act(async () => latest.setListening(true));
		expect(latest.listening).toBe(false);
		expect(latest.error).toBe("transcription");
		await listen();
		expect(latest.error).toBeNull();
		mocks.transcribe.mockRejectedValueOnce(new Error("model unavailable"));
		await act(async () => {
			mocks.startCallListener.mock.calls[1][1](utterance);
		});
		expect(latest.error).toBe("transcription");
		expect(mocks.sendEntry).not.toHaveBeenCalled();
	});
});

describe("conversation call bot audio", () => {
	test("a suspended AudioContext preserves the microphone and enables bot voice after resuming", async () => {
		initialContextState = "suspended";
		getUserMedia.mockImplementation(async () => {
			expect(contexts).toHaveLength(1);
			expect(contexts[0].resume).toHaveBeenCalledOnce();
			return microphone.stream;
		});
		await joined();
		expect(latest.joined).toBe(true);
		expect(mocks.setLocalStreams).toHaveBeenLastCalledWith([microphone.stream]);
		expect(contexts[0].createMediaStreamDestination).toHaveBeenCalledOnce();
		await act(async () => latest.setBotVoiceCapabilities(["bot"]));
		expect(latest.canSpeakForBot("bot")).toBe(true);
		await act(async () => {
			contexts[0].state = "running";
			contexts[0].dispatchEvent(new Event("statechange"));
		});
		expect(mocks.setLocalStreams).toHaveBeenLastCalledWith([
			contexts[0].output.stream,
		]);
		const { playback, source } = await play();
		expect(source.start).toHaveBeenCalledOnce();
		await act(async () => {
			source.onended?.();
			await playback;
		});
		await act(async () => latest.leave());
		expect(contexts[0].close).toHaveBeenCalledOnce();
		expect(microphone.track.stop).toHaveBeenCalledOnce();
	});

	test("local bot speech pauses transcription and preserves listening consent and mute", async () => {
		const presence = await joined();
		await listen();
		expect(mocks.setLocalStreams).toHaveBeenLastCalledWith([
			contexts[0].output.stream,
		]);
		const { playback, source } = await play();
		expect(source.start).toHaveBeenCalledOnce();
		expect(listener.setGuarded).toHaveBeenLastCalledWith(true);
		expect(listener.setPaused).not.toHaveBeenCalledWith(true);
		expect(localCall(presence)).toMatchObject({
			listening: true,
			muted: false,
			speakingBot: "bot",
		});
		await act(async () => latest.toggleMute());
		expect(microphone.track.enabled).toBe(false);
		expect(contexts[0].output.track.enabled).toBe(true);
		await act(async () => {
			source.onended?.();
			await playback;
		});
		await act(async () => {
			vi.advanceTimersByTime(351);
		});
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
		expect(localCall(presence)).toMatchObject({
			listening: true,
			muted: true,
			speakingBot: null,
		});
		await act(async () => latest.toggleMute());
		expect(listener.setPaused).toHaveBeenLastCalledWith(false);
		expect(latest.listening).toBe(true);
		expect(mocks.transcribe).not.toHaveBeenCalled();
	});

	test.each([false, true])(
		"the next local speech chunk restores capture after the echo tail, muted=%s",
		async (muted) => {
			await joined();
			await listen();
			if (muted) await act(async () => latest.toggleMute());
			const first = await play();
			await act(async () => {
				first.source.onended?.();
				await first.playback;
			});
			expect(listener.setPaused).toHaveBeenLastCalledWith(true);
			await act(async () => vi.advanceTimersByTime(100));
			await play();
			expect(listener.setGuarded).toHaveBeenLastCalledWith(true);
			expect(listener.setPaused).toHaveBeenLastCalledWith(muted);
			await act(async () => vi.advanceTimersByTime(350));
			expect(listener.setGuarded).toHaveBeenLastCalledWith(true);
			expect(listener.setPaused).toHaveBeenLastCalledWith(muted);
		},
	);

	test("the next remote speech chunk restores capture before the echo tail expires", async () => {
		const presence = await joined();
		await listen();
		const remote = {
			id: "call-one",
			joined: true,
			muted: false,
			peer: "remote-peer",
			voiceBots: ["bot"],
		};
		const speaking = async (bot: string | null) => {
			await act(async () => {
				presence.states.set(2, {
					sub: "other",
					call: { ...remote, speakingBot: bot },
				});
				presence.changed();
			});
		};
		await speaking("bot");
		await speaking(null);
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
		await act(async () => vi.advanceTimersByTime(100));
		await speaking("bot");
		expect(listener.setPaused).toHaveBeenLastCalledWith(false);
		await act(async () => vi.advanceTimersByTime(350));
		expect(listener.setGuarded).toHaveBeenLastCalledWith(true);
		expect(listener.setPaused).toHaveBeenLastCalledWith(false);
	});

	test("remote bot speech pauses only transcription and resumes after its echo tail", async () => {
		const presence = await joined();
		await listen();
		const remote = {
			id: "call-one",
			joined: true,
			muted: false,
			peer: "remote-peer",
			voiceBots: ["bot"],
		};
		await act(async () => {
			presence.states.set(2, {
				sub: "other",
				call: { ...remote, speakingBot: "bot" },
			});
			presence.changed();
		});
		expect(listener.setGuarded).toHaveBeenLastCalledWith(true);
		expect(microphone.track.enabled).toBe(true);
		expect(localCall(presence)).toMatchObject({
			listening: true,
			muted: false,
		});
		await act(async () => {
			presence.states.set(2, { sub: "other", call: remote });
			presence.changed();
		});
		await act(async () => {
			vi.advanceTimersByTime(349);
		});
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
		await act(async () => {
			vi.advanceTimersByTime(1);
		});
		expect(listener.setPaused).toHaveBeenLastCalledWith(false);
		await act(async () => {
			presence.states.set(2, {
				sub: "other",
				call: { ...remote, speakingBot: "bot" },
			});
			presence.changed();
			latest.setListening(false);
		});
		await act(async () => {
			presence.states.set(2, { sub: "other", call: remote });
			presence.changed();
			vi.advanceTimersByTime(351);
		});
		expect(latest.listening).toBe(false);
		expect(listener.stop).toHaveBeenCalledOnce();
		expect(mocks.startCallListener).toHaveBeenCalledOnce();
	});

	test("leaving stops active speech, microphone capture and transcription", async () => {
		await joined();
		await listen();
		const { playback, source } = await play();
		await act(async () => {
			await latest.leave();
			await playback;
		});
		expect(latest.joined).toBe(false);
		expect(source.stop).toHaveBeenCalledOnce();
		expect(source.disconnect).toHaveBeenCalledOnce();
		expect(microphone.track.stop).toHaveBeenCalledOnce();
		expect(contexts[0].output.track.stop).toHaveBeenCalledOnce();
		expect(contexts[0].close).toHaveBeenCalledOnce();
		expect(listener.stop).toHaveBeenCalledOnce();
		expect(mocks.setLocalStream).toHaveBeenLastCalledWith(null);
	});

	test("cancelling a pending microphone request closes its context and discards the stream", async () => {
		let resolveMedia: (stream: MediaStream) => void = () => {};
		getUserMedia.mockReturnValue(
			new Promise<MediaStream>((resolve) => {
				resolveMedia = resolve;
			}),
		);
		await setup();
		let joining: Promise<void> = Promise.resolve();
		await act(async () => {
			joining = latest.join();
		});
		await act(async () => latest.leave());
		await act(async () => {
			resolveMedia(microphone.stream);
			await joining;
		});
		expect(latest.joined).toBe(false);
		expect(contexts[0].close).toHaveBeenCalledOnce();
		expect(microphone.track.stop).toHaveBeenCalledOnce();
		expect(mocks.setLocalStreams).not.toHaveBeenCalled();
	});
});

describe("interrupting bot speech", () => {
	const utterance = {
		samples: new Float32Array(16_000).fill(0.1),
		offsetMs: 0,
		durationMs: 1000,
	};
	const heard = { message_id: "msg_bot", heard_chars: 12, spoken_chars: 80 };
	const remote = {
		id: "call-one",
		joined: true,
		muted: false,
		peer: "remote-peer",
		voiceBots: ["bot"],
	};
	const control = (echo: string | null = null, record = heard) => ({
		interrupt: vi.fn(() => record),
		echoText: vi.fn(() => echo),
	});
	const bodyOf = (index: number) =>
		mocks.sendEntry.mock.calls[index][0].request.envelope.body;

	async function bargeIn(text: string, echo: string | null) {
		const presence = await joined();
		await listen();
		const speech = control(echo);
		await act(async () => latest.registerSpeechControl(speech));
		await play();
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onBargeInStart());
		mocks.transcribe.mockResolvedValueOnce({ text });
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 700 }),
		);
		return { presence, speech };
	}

	async function say(text: string, offsetMs: number) {
		mocks.transcribe.mockResolvedValueOnce({ text });
		await act(async () =>
			mocks.startCallListener.mock.calls[0][1]({ ...utterance, offsetMs }),
		);
	}

	test("talking over a bot interrupts it, and the next transcript carries what was heard, once", async () => {
		const { speech } = await bargeIn("wait, stop", "The plan has three steps");
		expect(speech.interrupt).toHaveBeenCalledOnce();
		expect(listener.endBargeIn).toHaveBeenCalledWith(true);
		await say("Please stop", 0);
		await say("And another thing", 5000);
		expect(bodyOf(0).interrupted).toEqual(heard);
		expect(bodyOf(1).interrupted).toBeUndefined();
	});

	test("an echo of the bot's own words restores the volume and interrupts nothing", async () => {
		const { speech } = await bargeIn(
			"the plan has three",
			"The plan has three steps",
		);
		expect(speech.interrupt).not.toHaveBeenCalled();
		expect(listener.retryBargeIn).not.toHaveBeenCalled();
		expect(listener.endBargeIn).toHaveBeenCalledWith(false);
		await say("Hello", 0);
		expect(bodyOf(0).interrupted).toBeUndefined();
	});

	test("silence from the speech model rejects the barge-in", async () => {
		const { speech } = await bargeIn("", "The plan");
		expect(speech.interrupt).not.toHaveBeenCalled();
		expect(listener.endBargeIn).toHaveBeenCalledWith(false);
	});

	test("an interrupt older than thirty seconds is not attached", async () => {
		await joined();
		await listen();
		await act(async () => latest.registerSpeechControl(control()));
		await act(async () => latest.interruptSpeech());
		await act(async () => {
			vi.advanceTimersByTime(31_000);
		});
		await say("Hello", 0);
		expect(bodyOf(0).interrupted).toBeUndefined();
	});

	test("the escape hatch stops locally played speech through the control", async () => {
		await joined();
		const speech = control();
		await act(async () => latest.registerSpeechControl(speech));
		await act(async () => latest.interruptSpeech());
		expect(speech.interrupt).toHaveBeenCalledOnce();
	});

	test("a participant whose device is not playing asks the playing device and attaches its report", async () => {
		const presence = await joined();
		await listen();
		await act(async () => {
			presence.states.set(2, {
				sub: "other",
				call: { ...remote, speakingBot: "bot" },
			});
			presence.changed();
		});
		expect(latest.speakingBot).toBe("bot");
		await act(async () => latest.interruptSpeech());
		const request = (
			presence.states.get(1) as {
				call: { interruptBot?: { botId: string; at: number } };
			}
		).call.interruptBot;
		expect(request?.botId).toBe("bot");
		await act(async () => {
			presence.states.set(2, {
				sub: "other",
				call: {
					...remote,
					interruptedSpeech: { ...heard, at: (request?.at ?? 0) + 40 },
				},
			});
			presence.changed();
		});
		await say("Stop please", 0);
		expect(bodyOf(0).interrupted).toEqual(heard);
	});

	test("the playing device stops for a newer remote request, reports it once and ignores replays", async () => {
		const presence = await joined();
		const speech = control();
		await act(async () => latest.registerSpeechControl(speech));
		const asking = {
			sub: "other",
			call: { ...remote, interruptBot: { botId: "bot", at: 50 } },
		};
		await act(async () => {
			presence.states.set(2, asking);
			presence.changed();
		});
		expect(speech.interrupt).toHaveBeenCalledWith({ botId: "bot", since: 50 });
		const report = (
			presence.states.get(1) as {
				call: { interruptedSpeech?: Record<string, unknown> };
			}
		).call.interruptedSpeech;
		expect(report).toMatchObject(heard);
		expect(typeof report?.at).toBe("number");
		await act(async () => {
			presence.states.set(2, { ...asking });
			presence.changed();
		});
		expect(speech.interrupt).toHaveBeenCalledOnce();
	});

	test("a request for a bot this device does not voice is ignored when nothing plays", async () => {
		const presence = await joined();
		const speech = {
			interrupt: vi.fn(() => null),
			echoText: vi.fn(() => null),
		};
		await act(async () => latest.registerSpeechControl(speech));
		await act(async () => {
			presence.states.set(2, {
				sub: "other",
				call: { ...remote, interruptBot: { botId: "bot", at: 50 } },
			});
			presence.changed();
		});
		expect(
			(presence.states.get(1) as { call: Record<string, unknown> }).call
				.interruptedSpeech,
		).toBeUndefined();
	});
});

describe("streaming voice turns", () => {
	const directConversation = {
		id: "conversation",
		kind: "direct",
		members: [
			{ kind: "user", principal_id: "me" },
			{ kind: "bot", principal_id: "bot" },
		],
	} as unknown as IConversation;
	const utterance = {
		samples: new Float32Array(16_000).fill(0.1),
		offsetMs: 0,
		durationMs: 1000,
	};
	let unregister: (() => void) | undefined;
	afterEach(() => unregister?.());

	function reply() {
		return {
			update: vi.fn(),
			flush: vi.fn(),
			end: vi.fn(),
			cancel: vi.fn(),
		};
	}

	function control(streamed: ReturnType<typeof reply> | null) {
		return {
			interrupt: vi.fn(() => null),
			echoText: vi.fn(() => null),
			stream: vi.fn(() => streamed),
			pause: vi.fn(() => true),
			resume: vi.fn(),
		};
	}

	function sender(runId = "run-1") {
		const done = Promise.withResolvers<"completed" | "failed" | "cancelled">();
		const send = vi.fn<VoiceTurnSender>(async () => ({
			runId,
			done: done.promise,
		}));
		unregister = registerVoiceTurnSender("bot", send);
		return { send, done };
	}

	async function ready(speech: ReturnType<typeof control>) {
		await joined(directConversation);
		await act(async () => latest.registerSpeechControl(speech));
		await act(async () => latest.setListening(true));
	}

	async function say(text: string, offsetMs = 0) {
		mocks.transcribe.mockResolvedValueOnce({ text });
		await act(async () =>
			mocks.startCallListener.mock.calls[0][1]({ ...utterance, offsetMs }),
		);
	}

	async function setRun(runId: string, text: string, tool = false) {
		mocks.storeState.runs = {
			[runId]: {
				message: { inner: { content: text }, current_step_id: tool ? "s" : "" },
			},
		};
		await act(async () => {
			for (const listener of mocks.storeListeners) listener();
		});
	}

	test("the dock takes the turn: no plain transcript, text streams into the reply, done ends it", async () => {
		const streamed = reply();
		const speech = control(streamed);
		const { send, done } = sender();
		await ready(speech);
		await say("Bot, what is next?");
		expect(send).toHaveBeenCalledWith({
			messageId: "message-id",
			text: "Bot, what is next?",
			transcript: {
				call: "call-one",
				offset_ms: 0,
				duration_ms: 1000,
				mentions: ["bot"],
			},
		});
		expect(speech.stream).toHaveBeenCalledWith("message-id", "bot");
		expect(mocks.sendEntry).not.toHaveBeenCalled();
		expect(mocks.posted).toHaveBeenCalledWith(
			"message-id",
			expect.objectContaining({ route: "stream" }),
		);
		await setRun("run-1", "Next up");
		expect(streamed.update).toHaveBeenLastCalledWith("Next up");
		expect(mocks.firstText).toHaveBeenCalledWith("message-id");
		await setRun("run-1", "Next up is");
		expect(streamed.update).toHaveBeenLastCalledWith("Next up is");
		await setRun("run-1", "Next up is", true);
		expect(streamed.flush).toHaveBeenCalledOnce();
		await act(async () => done.resolve("completed"));
		expect(streamed.end).toHaveBeenCalledWith("Next up is");
		expect(streamed.cancel).not.toHaveBeenCalled();
		expect(mocks.storeListeners.size).toBe(0);
	});

	test("a failed run cancels the spoken reply", async () => {
		const streamed = reply();
		const { done } = sender();
		await ready(control(streamed));
		await say("Hello");
		await setRun("run-1", "Hel");
		await act(async () => done.resolve("failed"));
		expect(streamed.cancel).toHaveBeenCalledOnce();
		expect(streamed.end).not.toHaveBeenCalled();
		expect(mocks.storeListeners.size).toBe(0);
	});

	test("when the dock does not take the turn the plain transcript is posted", async () => {
		const streamed = reply();
		const speech = control(streamed);
		const send = vi.fn<VoiceTurnSender>(async () => null);
		unregister = registerVoiceTurnSender("bot", send);
		await ready(speech);
		await say("Hello");
		expect(send).toHaveBeenCalledOnce();
		expect(speech.stream).not.toHaveBeenCalled();
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
		expect(mocks.sendEntry.mock.calls[0][0].request.kind).toBe("transcript");
		expect(mocks.posted).toHaveBeenCalledWith(
			"message-id",
			expect.objectContaining({ route: "queued" }),
		);
	});

	test("without a registered sender the call behaves as before", async () => {
		const speech = control(reply());
		await ready(speech);
		await say("Hello");
		expect(speech.stream).not.toHaveBeenCalled();
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
	});

	test("a sender that throws falls back to the plain transcript once", async () => {
		unregister = registerVoiceTurnSender("bot", async () => {
			throw new Error("boom");
		});
		await ready(control(reply()));
		await say("Hello");
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
	});

	test("a group conversation never uses the dock", async () => {
		const { send } = sender();
		await joined();
		await act(async () => latest.registerSpeechControl(control(reply())));
		await act(async () => latest.setListening(true));
		await say("Hello");
		expect(send).not.toHaveBeenCalled();
		expect(mocks.sendEntry).toHaveBeenCalledOnce();
	});

	test("a new utterance cancels the run that still generates", async () => {
		const first = reply();
		const second = reply();
		const speech = control(first);
		speech.stream.mockReturnValueOnce(first).mockReturnValueOnce(second);
		const done = Promise.withResolvers<"completed" | "failed" | "cancelled">();
		let count = 0;
		unregister = registerVoiceTurnSender("bot", async () => ({
			runId: `run-${++count}`,
			done: done.promise,
		}));
		await ready(speech);
		await say("One");
		await say("Two", 3000);
		expect(mocks.cancelRun).toHaveBeenCalledWith("run-1");
		expect(first.cancel).toHaveBeenCalledOnce();
		expect(speech.stream).toHaveBeenCalledTimes(2);
		expect(second.cancel).not.toHaveBeenCalled();
	});

	test("barge-in pauses, a confirmed one interrupts and cancels the run", async () => {
		const streamed = reply();
		const speech = control(streamed);
		sender();
		await ready(speech);
		await say("Hello");
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onBargeInStart());
		expect(speech.pause).toHaveBeenCalledOnce();
		mocks.transcribe.mockResolvedValueOnce({ text: "wait, stop" });
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 700 }),
		);
		expect(speech.interrupt).toHaveBeenCalledOnce();
		expect(speech.resume).not.toHaveBeenCalled();
		expect(mocks.cancelRun).toHaveBeenCalledWith("run-1");
		expect(streamed.cancel).toHaveBeenCalledOnce();
	});

	test("a rejected barge-in resumes the paused reply and leaves the run alone", async () => {
		const streamed = reply();
		const speech = control(streamed);
		sender();
		await ready(speech);
		await say("Hello");
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onBargeInStart());
		mocks.transcribe.mockResolvedValueOnce({ text: "" });
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 700 }),
		);
		expect(speech.resume).toHaveBeenCalledOnce();
		expect(speech.interrupt).not.toHaveBeenCalled();
		expect(mocks.cancelRun).not.toHaveBeenCalled();
		expect(streamed.cancel).not.toHaveBeenCalled();
	});

	test("an empty initial probe keeps speech paused while a longer probe confirms the interruption", async () => {
		const streamed = reply();
		const speech = control(streamed);
		sender();
		await ready(speech);
		await say("Hello");
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onBargeInStart());
		listener.retryBargeIn.mockReturnValueOnce(true);
		mocks.transcribe.mockResolvedValueOnce({ text: "" });
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 500 }),
		);
		expect(listener.retryBargeIn).toHaveBeenCalledOnce();
		expect(listener.endBargeIn).not.toHaveBeenCalled();
		expect(speech.resume).not.toHaveBeenCalled();
		expect(speech.interrupt).not.toHaveBeenCalled();
		mocks.transcribe.mockResolvedValueOnce({ text: "wait, stop" });
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 1000 }),
		);
		expect(listener.endBargeIn).toHaveBeenCalledWith(true);
		expect(speech.interrupt).toHaveBeenCalledOnce();
		expect(speech.resume).not.toHaveBeenCalled();
		expect(mocks.cancelRun).toHaveBeenCalledWith("run-1");
		expect(streamed.cancel).toHaveBeenCalledOnce();
	});

	test("a stalled probe resumes speech after two seconds and ignores a late transcription", async () => {
		const streamed = reply();
		const speech = control(streamed);
		sender();
		await ready(speech);
		await say("Hello");
		const capture = mocks.startCallListener.mock.calls[0][2];
		await act(async () => capture.onBargeInStart());
		const probe = Promise.withResolvers<{ text: string }>();
		mocks.transcribe.mockReturnValueOnce(probe.promise);
		await act(async () =>
			capture.onBargeInProbe({ ...utterance, durationMs: 500 }),
		);
		await act(async () => vi.advanceTimersByTime(1999));
		expect(speech.resume).not.toHaveBeenCalled();
		expect(listener.endBargeIn).not.toHaveBeenCalled();
		await act(async () => vi.advanceTimersByTime(1));
		expect(speech.resume).toHaveBeenCalledOnce();
		expect(listener.endBargeIn).toHaveBeenCalledWith(false);
		expect(listener.retryBargeIn).not.toHaveBeenCalled();
		await act(async () => probe.resolve({ text: "wait, stop" }));
		expect(speech.resume).toHaveBeenCalledOnce();
		expect(listener.endBargeIn).toHaveBeenCalledOnce();
		expect(speech.interrupt).not.toHaveBeenCalled();
		expect(mocks.cancelRun).not.toHaveBeenCalled();
		expect(streamed.cancel).not.toHaveBeenCalled();
	});

	test.each(["mute", "stop listening"])(
		"%s releases a pending barge-in pause and ignores its late confirmation",
		async (action) => {
			const streamed = reply();
			const speech = control(streamed);
			sender();
			await ready(speech);
			await say("Hello");
			const capture = mocks.startCallListener.mock.calls[0][2];
			await act(async () => capture.onBargeInStart());
			expect(speech.pause).toHaveBeenCalledOnce();
			const probe = Promise.withResolvers<{ text: string }>();
			mocks.transcribe.mockReturnValueOnce(probe.promise);
			await act(async () =>
				capture.onBargeInProbe({ ...utterance, durationMs: 700 }),
			);
			await act(async () => {
				if (action === "mute") latest.toggleMute();
				else latest.setListening(false);
			});
			expect(speech.resume).toHaveBeenCalledOnce();
			await act(async () => probe.resolve({ text: "wait, stop" }));
			expect(speech.resume).toHaveBeenCalledOnce();
			expect(speech.interrupt).not.toHaveBeenCalled();
			expect(listener.endBargeIn).not.toHaveBeenCalled();
			expect(mocks.cancelRun).not.toHaveBeenCalled();
			expect(streamed.cancel).not.toHaveBeenCalled();
		},
	);

	test("the manual interrupt cancels an unfinished run", async () => {
		const streamed = reply();
		sender();
		await ready(control(streamed));
		await say("Hello");
		await act(async () => latest.interruptSpeech());
		expect(mocks.cancelRun).toHaveBeenCalledWith("run-1");
		expect(streamed.cancel).toHaveBeenCalledOnce();
	});

	test("joining warms the listener and the speech model", async () => {
		await joined();
		expect(mocks.warmListening).toHaveBeenCalled();
		expect(mocks.warmAudio).toHaveBeenCalledWith({
			stt: { bit_id: "stt-model", local: false },
		});
	});

	test("local speech models get the raw bytes instead of base64", async () => {
		speechModel = { bitId: "local-stt", route: "local" } as never;
		mocks.transcribe.mockResolvedValue({ text: "hi" });
		await joined();
		await listen();
		await act(async () => {
			mocks.startCallListener.mock.calls[0][1](utterance);
		});
		expect(mocks.transcribe).toHaveBeenCalledWith(
			expect.objectContaining({
				audio_base64: "",
				audio_bytes: expect.any(Uint8Array),
				local: true,
			}),
		);
	});
});
