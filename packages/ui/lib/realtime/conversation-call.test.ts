import { describe, expect, test } from "bun:test";
import {
	CALL_FIELD,
	type CallPeerLike,
	MAX_CALL_PARTICIPANTS,
	botVoiceSource,
	byteLevel,
	callIsFull,
	callLiveKey,
	createCallEngine,
	createSpeakingDetector,
	deriveLiveCall,
	formatClock,
	peerSubs,
	sanitizeCallPresence,
	splitCallDuration,
} from "./conversation-call";

const presence = (id: string, peer: string, extra = {}) => ({
	id,
	joined: true,
	muted: false,
	peer,
	...extra,
});

function states(entries: [number, unknown][]): ReadonlyMap<number, unknown> {
	return new Map(entries);
}

const member = (sub: string, call: unknown) => ({ sub, [CALL_FIELD]: call });

describe("sanitizeCallPresence", () => {
	test("keeps a well-formed field and coerces flags", () => {
		expect(
			sanitizeCallPresence({ id: "msg_1", peer: "p-1", joined: true }),
		).toEqual({
			id: "msg_1",
			joined: true,
			muted: false,
			peer: "p-1",
			listening: false,
		});
	});

	test("listening is on only when exactly true", () => {
		const base = { id: "c", peer: "p", joined: true };
		expect(sanitizeCallPresence({ ...base, listening: true })?.listening).toBe(
			true,
		);
		for (const bad of ["true", 1, null, undefined]) {
			expect(sanitizeCallPresence({ ...base, listening: bad })?.listening).toBe(
				false,
			);
		}
	});

	test("drops malformed fields", () => {
		for (const bad of [
			null,
			"call",
			[],
			{},
			{ id: 4, peer: "p" },
			{ id: "has space", peer: "p" },
			{ id: "x".repeat(200), peer: "p" },
			{ id: "ok", peer: "" },
			{ id: "ok", peer: { a: 1 } },
		]) {
			expect(sanitizeCallPresence(bad)).toBeNull();
		}
	});

	test("bounds, deduplicates and sorts advertised bot voices", () => {
		const value = sanitizeCallPresence({
			...presence("c", "p"),
			voiceBots: [
				"bot:z",
				"bot:a",
				"bot:z",
				"bad id",
				null,
				7,
				"x".repeat(161),
			],
			speakingBot: "bot:a",
		});
		expect(value?.voiceBots).toEqual(["bot:a", "bot:z"]);
		expect(value?.speakingBot).toBe("bot:a");
		expect(
			sanitizeCallPresence({
				...presence("c", "p"),
				voiceBots: Array.from({ length: 100 }, (_, i) => `bot-${i}`),
			})?.voiceBots,
		).toHaveLength(64);
	});

	test("accepts speaking state only for an advertised voice", () => {
		for (const voiceBots of [undefined, "bot-1", {}, [], ["bot-2"]]) {
			const value = sanitizeCallPresence({
				...presence("c", "p"),
				voiceBots,
				speakingBot: "bot-1",
			});
			expect(value?.speakingBot).toBeUndefined();
		}
		for (const speakingBot of [null, true, {}, "bad id"]) {
			expect(
				sanitizeCallPresence({
					...presence("c", "p"),
					voiceBots: ["bot-1"],
					speakingBot,
				})?.speakingBot,
			).toBeUndefined();
		}
	});
});

describe("interrupt presence", () => {
	const record = {
		message_id: "msg_abc",
		heard_chars: 10,
		spoken_chars: 40,
		at: 5,
	};

	test("keeps well-formed interrupt requests and reports", () => {
		const value = sanitizeCallPresence({
			...presence("c", "p"),
			interruptBot: { botId: "bot:a", at: 7 },
			interruptedSpeech: record,
		});
		expect(value?.interruptBot).toEqual({ botId: "bot:a", at: 7 });
		expect(value?.interruptedSpeech).toEqual(record);
	});

	test("drops malformed or out-of-range values", () => {
		for (const interruptBot of [
			null,
			"bot",
			{ botId: "bad id", at: 1 },
			{ botId: "bot", at: "1" },
			{ botId: "bot", at: Number.NaN },
			{ botId: "bot", at: 0 },
		]) {
			expect(
				sanitizeCallPresence({ ...presence("c", "p"), interruptBot })
					?.interruptBot,
			).toBeUndefined();
		}
		for (const patch of [
			{ message_id: "bad id" },
			{ spoken_chars: 0, heard_chars: 0 },
			{ heard_chars: 41 },
			{ heard_chars: -1 },
			{ spoken_chars: 64_001 },
			{ heard_chars: 1.5 },
			{ at: -1 },
		]) {
			expect(
				sanitizeCallPresence({
					...presence("c", "p"),
					interruptedSpeech: { ...record, ...patch },
				})?.interruptedSpeech,
			).toBeUndefined();
		}
	});

	test("reach derived participants", () => {
		const live = deriveLiveCall(
			states([
				[
					3,
					{
						sub: "me",
						[CALL_FIELD]: {
							...presence("c", "pm"),
							interruptBot: { botId: "bot-1", at: 9 },
						},
					},
				],
			]),
			3,
			new Set(["me"]),
		);
		expect(live?.participants[0]?.interruptBot).toEqual({
			botId: "bot-1",
			at: 9,
		});
	});
});

describe("deriveLiveCall", () => {
	const members = new Set(["anna", "paul", "me"]);

	test("carries who lets bots listen and changes the live key", () => {
		const read = (listening: boolean) =>
			deriveLiveCall(
				states([[1, member("anna", presence("c", "pa", { listening }))]]),
				3,
				new Set(["anna"]),
			);
		expect(read(true)?.participants[0].listening).toBe(true);
		expect(read(false)?.participants[0].listening).toBe(false);
		expect(callLiveKey(read(true))).not.toBe(callLiveKey(read(false)));
	});

	test("lists joined members in client order and marks the local one", () => {
		const live = deriveLiveCall(
			states([
				[9, member("paul", presence("c1", "pp", { muted: true }))],
				[3, member("me", presence("c1", "pm"))],
				[5, member("anna", { ...presence("c1", "pa"), joined: false })],
			]),
			3,
			members,
		);
		expect(live?.callId).toBe("c1");
		expect(live?.participants.map((p) => [p.sub, p.local, p.muted])).toEqual([
			["me", true, false],
			["paul", false, true],
		]);
	});

	test("ignores non-members, missing subs and malformed states", () => {
		const live = deriveLiveCall(
			states([
				[1, member("stranger", presence("c1", "ps"))],
				[2, { [CALL_FIELD]: presence("c1", "px") }],
				[3, "garbage"],
				[4, member("anna", { id: 7 })],
				[5, member("paul", presence("c1", "pp"))],
			]),
			0,
			members,
		);
		expect(live?.participants.map((p) => p.sub)).toEqual(["paul"]);
	});

	test("returns null when nobody is in a call", () => {
		expect(
			deriveLiveCall(states([[1, { sub: "anna" }]]), 0, members),
		).toBeNull();
		expect(deriveLiveCall(states([]), 0, members)).toBeNull();
	});

	test("counts a person once even with several tabs", () => {
		const live = deriveLiveCall(
			states([
				[1, member("anna", presence("c1", "a1"))],
				[2, member("anna", presence("c1", "a2"))],
			]),
			0,
			members,
		);
		expect(live?.participants).toHaveLength(1);
		expect(live?.participants[0].peer).toBe("a1");
	});

	test("caps a call at the participant limit, lowest client ids first", () => {
		const many = new Set(Array.from({ length: 12 }, (_, i) => `u${i}`));
		const live = deriveLiveCall(
			states(
				Array.from({ length: 12 }, (_, i): [number, unknown] => [
					100 - i,
					member(`u${i}`, presence("c1", `p${i}`)),
				]),
			),
			0,
			many,
		);
		expect(live?.participants).toHaveLength(MAX_CALL_PARTICIPANTS);
		expect(live?.participants[0].clientId).toBe(89);
	});

	test("with two call ids the bigger call wins, the smaller id breaks ties", () => {
		const big = deriveLiveCall(
			states([
				[1, member("anna", presence("b", "a"))],
				[2, member("paul", presence("a", "p"))],
				[3, member("me", presence("a", "m"))],
			]),
			0,
			members,
		);
		expect(big?.callId).toBe("a");
		const tie = deriveLiveCall(
			states([
				[1, member("anna", presence("b", "a"))],
				[2, member("paul", presence("a", "p"))],
			]),
			0,
			members,
		);
		expect(tie?.callId).toBe("a");
		expect(tie?.participants.map((p) => p.sub)).toEqual(["paul"]);
	});
});

describe("botVoiceSource", () => {
	const members = new Set(["anna", "paul", "me"]);
	const read = (entries: [number, unknown][], localClientId = 3) =>
		deriveLiveCall(states(entries), localClientId, members);
	const voice = (sub: string, peer: string, extra = {}) =>
		member(sub, presence("c", peer, { voiceBots: ["bot-1"], ...extra }));

	test("elects the lowest capable client consistently for every participant", () => {
		const entries: [number, unknown][] = [
			[9, voice("paul", "pp")],
			[3, voice("me", "pm")],
			[1, member("anna", presence("c", "pa"))],
		];
		for (const local of [1, 3, 9]) {
			expect(botVoiceSource(read(entries, local), "bot-1")?.sub).toBe("me");
		}
		expect(botVoiceSource(read(entries), "other-bot")).toBeNull();
		expect(botVoiceSource(null, "bot-1")).toBeNull();
	});

	test("a muted participant can supply the bot voice without enabling their microphone", () => {
		const live = read([
			[3, voice("me", "pm", { muted: true, listening: false })],
		]);
		expect(botVoiceSource(live, "bot-1")?.sub).toBe("me");
	});

	test("fails over when the source leaves or withdraws its model", () => {
		const other: [number, unknown] = [9, voice("paul", "pp")];
		for (const extra of [{ joined: false }, { voiceBots: [] }]) {
			expect(
				botVoiceSource(read([[3, voice("me", "pm", extra)], other]), "bot-1")
					?.sub,
			).toBe("paul");
		}
	});

	test("ignores non-members, another call and duplicate peer claims", () => {
		const live = read([
			[0, voice("outsider", "po")],
			[1, voice("anna", "shared")],
			[2, voice("paul", "shared")],
			[3, voice("me", "pm")],
			[4, voice("anna", "other-call", { id: "other" })],
		]);
		expect(botVoiceSource(live, "bot-1")?.sub).toBe("me");
	});

	test("a second tab cannot supply a voice for a person's admitted first tab", () => {
		const live = read([
			[1, member("anna", presence("c", "first"))],
			[2, voice("anna", "second")],
			[3, voice("me", "pm")],
		]);
		expect(botVoiceSource(live, "bot-1")?.sub).toBe("me");
	});

	test("voice capability and speaking updates change the live key", () => {
		const base = read([[3, voice("me", "pm")]]);
		const speaking = read([[3, voice("me", "pm", { speakingBot: "bot-1" })]]);
		const withdrawn = read([[3, voice("me", "pm", { voiceBots: [] })]]);
		expect(base?.participants[0]?.voiceBots).toEqual(["bot-1"]);
		expect(speaking?.participants[0]?.speakingBot).toBe("bot-1");
		expect(callLiveKey(base)).not.toBe(callLiveKey(speaking));
		expect(callLiveKey(base)).not.toBe(callLiveKey(withdrawn));
	});
});

describe("call helpers", () => {
	test("callIsFull is false for someone already in the call", () => {
		const subs = Array.from(
			{ length: MAX_CALL_PARTICIPANTS },
			(_, i) => `u${i}`,
		);
		const live = deriveLiveCall(
			states(
				subs.map((s, i): [number, unknown] => [
					i,
					member(s, presence("c", `p${i}`)),
				]),
			),
			0,
			new Set(subs),
		);
		expect(callIsFull(live, "outsider")).toBe(true);
		expect(callIsFull(live, "u3")).toBe(false);
		expect(callIsFull(null, "me")).toBe(false);
	});

	test("callLiveKey changes with mute and peers", () => {
		const members = new Set(["anna"]);
		const key = (muted: boolean) =>
			callLiveKey(
				deriveLiveCall(
					states([[1, member("anna", presence("c", "p", { muted }))]]),
					0,
					members,
				),
			);
		expect(key(false)).toBe(key(false));
		expect(key(false)).not.toBe(key(true));
		expect(callLiveKey(null)).toBe("");
	});

	test("peerSubs drops a peer id two people claim", () => {
		const participants = [
			{
				sub: "anna",
				peer: "p1",
				muted: false,
				listening: false,
				clientId: 1,
				local: false,
			},
			{
				sub: "paul",
				peer: "p1",
				muted: false,
				listening: false,
				clientId: 2,
				local: false,
			},
			{
				sub: "me",
				peer: "p2",
				muted: false,
				listening: false,
				clientId: 3,
				local: true,
			},
		];
		expect([...peerSubs(participants)]).toEqual([["p2", "me"]]);
	});

	test("durations and clocks", () => {
		expect(splitCallDuration(12 * 60_000 + 5000)).toEqual({
			hours: 0,
			minutes: 12,
		});
		expect(splitCallDuration(95 * 60_000)).toEqual({ hours: 1, minutes: 35 });
		expect(splitCallDuration(Number.NaN)).toEqual({ hours: 0, minutes: 0 });
		expect(formatClock(75_000)).toBe("1:15");
		expect(formatClock(3_725_000)).toBe("1:02:05");
		expect(formatClock(-5)).toBe("0:00");
	});

	test("the speaking detector rises at once and holds briefly", () => {
		const detector = createSpeakingDetector({ threshold: 0.1, holdMs: 300 });
		expect(detector.update(0.01, 0)).toBe(false);
		expect(detector.update(0.5, 100)).toBe(true);
		expect(detector.update(0, 350)).toBe(true);
		expect(detector.update(0, 500)).toBe(false);
	});

	test("byteLevel is zero for silence", () => {
		expect(byteLevel(new Uint8Array(64).fill(128))).toBe(0);
		expect(byteLevel(new Uint8Array(64).fill(255))).toBeGreaterThan(0.9);
		expect(byteLevel([])).toBe(0);
	});
});

class FakePeer implements CallPeerLike {
	destroyed = false;
	added: MediaStream[] = [];
	removed: MediaStream[] = [];
	private handlers = new Map<string, Set<(s?: MediaStream) => void>>();
	addStream(stream: MediaStream) {
		this.added.push(stream);
	}
	removeStream(stream: MediaStream) {
		this.removed.push(stream);
	}
	on(
		event: "stream" | "close" | "connect",
		listener: (s?: MediaStream) => void,
	) {
		const set = this.handlers.get(event) ?? new Set();
		set.add(listener);
		this.handlers.set(event, set);
	}
	off(
		event: "stream" | "close" | "connect",
		listener: (s?: MediaStream) => void,
	) {
		this.handlers.get(event)?.delete(listener);
	}
	emit(event: "stream" | "close" | "connect", stream?: MediaStream) {
		for (const l of [...(this.handlers.get(event) ?? [])]) l(stream);
	}
	listeners(event: string) {
		return this.handlers.get(event)?.size ?? 0;
	}
}

interface FakeConn {
	peer?: unknown;
	connected?: boolean;
	glareToken?: number;
}

class FakeProvider {
	room = { webrtcConns: new Map<string, FakeConn>() };
	private peersListeners = new Set<(change: unknown) => void>();
	on(_: "peers", l: (change: unknown) => void) {
		this.peersListeners.add(l);
	}
	off(_: "peers", l: (change: unknown) => void) {
		this.peersListeners.delete(l);
	}
	connect(id: string): FakePeer {
		const peer = new FakePeer();
		this.room.webrtcConns.set(id, { peer });
		this.fire();
		return peer;
	}
	drop(id: string) {
		this.room.webrtcConns.delete(id);
		this.fire();
	}
	fire() {
		for (const l of [...this.peersListeners]) l({});
	}
	get listenerCount() {
		return this.peersListeners.size;
	}
}

const stream = (name: string) =>
	({ name, getTracks: () => [{}] }) as unknown as MediaStream;

describe("createCallEngine", () => {
	test("attaches the local stream to current and later connections", () => {
		const provider = new FakeProvider();
		const early = provider.connect("a");
		const engine = createCallEngine(provider);
		const mic = stream("mic");
		engine.setLocalStream(mic);
		expect(early.added).toEqual([mic]);
		const late = provider.connect("b");
		expect(late.added).toEqual([mic]);
		provider.fire();
		expect(early.added).toHaveLength(1);
		expect(late.added).toHaveLength(1);
	});

	test("re-attaches to a replacement connection after ICE rotation", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const mic = stream("mic");
		const first = provider.connect("a");
		engine.setLocalStream(mic);
		first.emit("close");
		first.destroyed = true;
		provider.drop("a");
		const second = provider.connect("a");
		expect(second).not.toBe(first);
		expect(second.added).toEqual([mic]);
		expect(first.added).toEqual([mic]);
	});

	test("detaches on leave and swaps streams", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const peer = provider.connect("a");
		const one = stream("one");
		const two = stream("two");
		engine.setLocalStream(one);
		engine.setLocalStream(two);
		expect(peer.removed).toEqual([one]);
		expect(peer.added).toEqual([one, two]);
		engine.setLocalStream(null);
		expect(peer.removed).toEqual([one, two]);
		const newcomer = provider.connect("b");
		expect(newcomer.added).toEqual([]);
	});

	test("collects remote streams by peer id and forgets them on close or removal", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const seen: string[][] = [];
		engine.onRemote((streams) => seen.push([...streams.keys()]));
		const a = provider.connect("a");
		const b = provider.connect("b");
		a.emit("stream", stream("sa"));
		b.emit("stream", stream("sb"));
		expect(seen.at(-1)).toEqual(["a", "b"]);
		a.emit("close");
		expect(seen.at(-1)).toEqual(["b"]);
		provider.drop("b");
		expect(seen.at(-1)).toEqual([]);
	});

	test("skips destroyed peers and keeps going", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const dead = provider.connect("a");
		dead.destroyed = true;
		const live = provider.connect("b");
		engine.setLocalStream(stream("mic"));
		expect(dead.added).toEqual([]);
		expect(live.added).toHaveLength(1);
	});

	test("dispose removes listeners and the local stream", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const peer = provider.connect("a");
		const mic = stream("mic");
		engine.setLocalStream(mic);
		engine.dispose();
		expect(peer.removed).toEqual([mic]);
		expect(provider.listenerCount).toBe(0);
		expect(peer.listeners("stream")).toBe(0);
		expect(peer.listeners("close")).toBe(0);
		expect(peer.listeners("connect")).toBe(0);
	});

	test("a peer keeps its glare token for the first handshake and loses it once connected", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const peer = provider.connect("a");
		const conn = provider.room.webrtcConns.get("a") as FakeConn;
		conn.glareToken = 1_731_312_000_000.5;
		provider.fire();
		expect(conn.glareToken).toBe(1_731_312_000_000.5);

		conn.connected = true;
		peer.emit("connect");
		expect(conn.glareToken).toBeUndefined();
		conn.glareToken = 1_731_312_003_000.5;
		expect(conn.glareToken).toBeUndefined();
		expect(JSON.parse(JSON.stringify({ token: conn.glareToken }))).toEqual({});
		engine.dispose();
	});

	test("a peer that was connected before the call engine carries no glare token", () => {
		const provider = new FakeProvider();
		provider.connect("a");
		const conn = provider.room.webrtcConns.get("a") as FakeConn;
		conn.connected = true;
		conn.glareToken = 1_731_312_000_000.5;
		const engine = createCallEngine(provider);
		expect(conn.glareToken).toBeUndefined();
		engine.dispose();
	});
});

describe("call media streams", () => {
	test("retains microphone, camera and screen separately through connection rotation", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const first = provider.connect("a");
		const mic = stream("mic");
		const camera = stream("camera");
		const screen = stream("screen");
		engine.setLocalStreams([mic, camera, screen]);
		expect(first.added).toEqual([mic, camera, screen]);
		engine.setLocalStreams([mic, screen]);
		expect(first.removed).toEqual([camera]);
		const next = provider.connect("a");
		expect(next.added).toEqual([mic, screen]);
		expect(first.listeners("stream")).toBe(0);
		engine.dispose();
		expect(next.removed).toEqual([mic, screen]);
	});

	test("only admitted call peers receive media, and removal detaches every stream", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		engine.setAllowedPeers(new Set(["joined"]));
		const joined = provider.connect("joined");
		const waiting = provider.connect("waiting");
		const mic = stream("mic");
		const camera = stream("camera");
		engine.setLocalStreams([mic, camera]);
		expect(joined.added).toEqual([mic, camera]);
		expect(waiting.added).toEqual([]);
		engine.setAllowedPeers(new Set(["waiting"]));
		expect(joined.removed).toEqual([mic, camera]);
		expect(waiting.added).toEqual([mic, camera]);
		engine.dispose();
	});

	test("multiple remote streams from one peer do not overwrite each other", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const peer = provider.connect("a");
		let remote: readonly MediaStream[] = [];
		engine.onRemote((streams) => {
			remote = streams.get("a") ?? [];
		});
		const mic = stream("mic");
		const camera = stream("camera");
		const screen = stream("screen");
		peer.emit("stream", mic);
		peer.emit("stream", camera);
		peer.emit("stream", screen);
		expect(remote).toEqual([mic, camera, screen]);
		peer.emit("close");
		expect(remote).toEqual([]);
	});

	test("removing an ended remote stream retains the peer's remaining streams", () => {
		const provider = new FakeProvider();
		const engine = createCallEngine(provider);
		const peer = provider.connect("a");
		let remote: readonly MediaStream[] = [];
		engine.onRemote((streams) => {
			remote = streams.get("a") ?? [];
		});
		const screen = new EventTarget() as EventTarget & {
			getTracks: () => unknown[];
		};
		const track = Object.assign(new EventTarget(), { readyState: "live" });
		screen.getTracks = () => [track];
		const mic = stream("mic");
		peer.emit("stream", mic);
		peer.emit("stream", screen as unknown as MediaStream);
		track.readyState = "ended";
		track.dispatchEvent(new Event("ended"));
		expect(remote).toEqual([mic]);
		engine.dispose();
	});

	test("advertised camera and screen ids are validated and update the call view", () => {
		const read = (extra: object) =>
			deriveLiveCall(
				states([[1, member("anna", presence("c", "p", extra))]]),
				0,
				new Set(["anna"]),
			);
		const live = read({ camera: "camera-1", screen: "screen_1" });
		expect(live?.participants[0].camera).toBe("camera-1");
		expect(live?.participants[0].screen).toBe("screen_1");
		expect(callLiveKey(live)).not.toBe(callLiveKey(read({})));
		expect(
			read({ camera: "bad id", screen: {} })?.participants[0].camera,
		).toBeUndefined();
	});
});

test("pauses and resumes an existing simple-peer sender when a participant rejoins", () => {
	const provider = new FakeProvider();
	const peer = provider.connect("a") as FakePeer & {
		replaceTrack: (old: unknown, next: unknown, stream: MediaStream) => void;
	};
	const replacements: unknown[][] = [];
	peer.replaceTrack = (old, next, stream) => {
		replacements.push([old, next, stream]);
	};
	const mic = stream("mic");
	const engine = createCallEngine(provider);
	engine.setAllowedPeers(new Set(["a"]));
	engine.setLocalStreams([mic]);
	engine.setAllowedPeers(new Set());
	expect(replacements[0]?.[1]).toBeNull();
	expect(peer.removed).toEqual([]);
	engine.setAllowedPeers(new Set(["a"]));
	expect(replacements[1]?.[0]).toBe(replacements[1]?.[1]);
	expect(peer.added).toEqual([mic]);
	engine.dispose();
	expect(peer.removed).toEqual([mic]);
});
