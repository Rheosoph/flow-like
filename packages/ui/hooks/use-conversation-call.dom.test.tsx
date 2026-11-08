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
}));
vi.mock("../state/backend-state", () => {
	const backend = { audioState: { transcribe: mocks.transcribe } };
	return { useBackend: () => backend };
});
vi.mock("./use-inbox", () => ({
	useSendInboxEntry: () => ({ mutateAsync: mocks.sendEntry }),
	newInboxMessageId: () => "message-id",
}));
vi.mock("../lib/realtime/call-listener", () => ({
	startCallListener: mocks.startCallListener,
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
let listener: {
	setPaused: ReturnType<typeof vi.fn>;
	stop: ReturnType<typeof vi.fn>;
};

beforeEach(() => {
	vi.clearAllMocks();
	vi.useFakeTimers();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("AudioContext", FakeAudioContext);
	initialContextState = "running";
	contexts.length = 0;
	retireScope = registerIncomingCallScope(useBackend(), "hub:me");
	microphone = media("microphone");
	getUserMedia = vi.fn(async () => microphone.stream);
	Object.defineProperty(navigator, "mediaDevices", {
		configurable: true,
		value: { getUserMedia },
	});
	listener = { setPaused: vi.fn(), stop: vi.fn() };
	mocks.startCallListener.mockResolvedValue(listener);
	mocks.sendEntry.mockResolvedValue(undefined);
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

async function setup() {
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
			conversation,
			"me",
			room,
			undefined,
			{
				stt: { bitId: "stt-model", route: "remote" },
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

async function joined() {
	const presence = await setup();
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
		{ context: contexts[0], paused: false },
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
		const oldListener = { setPaused: vi.fn(), stop: vi.fn() };
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
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
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
		expect(listener.setPaused).toHaveBeenLastCalledWith(true);
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
