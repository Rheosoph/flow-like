// @vitest-environment happy-dom

import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
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
vi.mock("../state/backend-state", () => ({
	useBackend: () => ({ audioState: { transcribe: mocks.transcribe } }),
}));
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
class FakeAudioContext {
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

async function listen() {
	await act(async () => {
		latest.setListening(true);
	});
	expect(mocks.startCallListener).toHaveBeenCalledWith(
		microphone.stream,
		expect.any(Function),
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

describe("conversation call bot audio", () => {
	test("a suspended AudioContext preserves the ordinary microphone stream", async () => {
		initialContextState = "suspended";
		getUserMedia.mockImplementation(async () => {
			expect(contexts).toHaveLength(1);
			expect(contexts[0].resume).toHaveBeenCalledOnce();
			return microphone.stream;
		});
		await joined();
		expect(latest.joined).toBe(true);
		expect(mocks.setLocalStreams).toHaveBeenLastCalledWith([microphone.stream]);
		expect(contexts[0].createMediaStreamDestination).not.toHaveBeenCalled();
		await act(async () => latest.setBotVoiceCapabilities(["bot"]));
		expect(latest.canSpeakForBot("bot")).toBe(false);
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
