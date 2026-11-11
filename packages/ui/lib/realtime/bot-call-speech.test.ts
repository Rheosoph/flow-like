import { expect, test } from "bun:test";
import { foldInboxEntries } from "../../components/bots-tab/fold-entries";
import type {
	IInboxEntry,
	InboxEntryKind,
} from "../../state/backend-state/inbox-state";
import {
	type BotSpeechVoice,
	callSpeechChunks,
	createBotCallSpeech,
	isBotEcho,
	nextSpeechPiece,
	speechBitReference,
	speechCharacterLimit,
	speechChunks,
} from "./bot-call-speech";
import { createCallLatencyTrace } from "./call-latency";

const voice: BotSpeechVoice = { bitId: "voice", local: false };
function entry(
	id: string,
	seq: number,
	kind: InboxEntryKind,
	body: Record<string, unknown>,
	sender = "bot",
): IInboxEntry {
	return {
		id,
		message_id: id,
		conversation_id: "room",
		sender_id: sender,
		kind,
		envelope: { v: 1, enc: "none", body },
		created_at: new Date(seq * 1000).toISOString(),
		cursor: `${seq}:e_${id}`,
	};
}
const transcript = entry(
	"spoken",
	1,
	"transcript",
	{ call: "call", text: "Hello", offset_ms: 0 },
	"person",
);
function reply(
	id: string,
	seq: number,
	text = id,
	extra: Record<string, unknown> = {},
) {
	return entry(id, seq, "message", { text, reply_to: "spoken", ...extra });
}
function snapshot(
	entries: IInboxEntry[],
	voices = new Map<string, BotSpeechVoice | null>([["bot", voice]]),
) {
	return { entries, items: foldInboxEntries(entries), voices };
}
async function settle() {
	for (let i = 0; i < 24; i++) await Promise.resolve();
}
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}
function harness(
	initialEntries: IInboxEntry[] = [transcript],
	overrides: Partial<Parameters<typeof createBotCallSpeech>[0]> = {},
) {
	const spoken: string[] = [];
	const played: string[] = [];
	const errors: unknown[] = [];
	let stops = 0;
	const controller = createBotCallSpeech({
		callId: "call",
		initialEntries,
		synthesize: async (_voice, text) => {
			spoken.push(text);
			return new Blob([text]);
		},
		play: async (blob) => {
			played.push(await blob.text());
		},
		stop: () => {
			stops++;
		},
		canSpeak: () => true,
		onError: (error) => errors.push(error),
		trace: createCallLatencyTrace({ log: () => {} }),
		...overrides,
	});
	return { controller, spoken, played, errors, stops: () => stops };
}

test("only new final bot replies to this call speak, once, in chronological order", async () => {
	const old = reply("old", 2);
	const h = harness([transcript, old]);
	const entries = [
		reply("second", 6),
		reply("first", 5),
		old,
		transcript,
		reply("backfill", 0),
		reply("unrelated", 7, "other", { reply_to: "typed" }),
		reply("error", 8, "failed", { error: { code: "provider" } }),
		entry(
			"human",
			9,
			"message",
			{ text: "person", reply_to: "spoken" },
			"person",
		),
		entry(
			"other-call",
			3,
			"transcript",
			{ call: "elsewhere", text: "Other", offset_ms: 0 },
			"person",
		),
		reply("other-reply", 10, "other", { reply_to: "other-call" }),
		reply("deleted", 11),
		entry("delete", 12, "delete", { target: "deleted" }),
	];
	h.controller.update(snapshot(entries));
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toEqual(["first", "second"]);
	expect(h.played).toEqual(h.spoken);
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toHaveLength(2);
	h.controller.dispose();
});

test("one participant supplies a reply and later election changes never replay it", async () => {
	let elected = false;
	const h = harness([transcript], { canSpeak: () => elected });
	const entries = [transcript, reply("answer", 2)];
	h.controller.update(snapshot(entries));
	elected = true;
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toEqual([]);
});

test("a bot waiting for another bot speaks its follow-up in the same call", async () => {
	const h = harness();
	const first = entry(
		"peer-answer",
		2,
		"message",
		{
			text: "The first bot's answer",
			reply_to: "spoken",
		},
		"peer",
	);
	const followUp = reply("follow-up", 3, "My follow-up", {
		reply_to: first.message_id,
	});
	const entries = [transcript, first, followUp];
	const voices = new Map([
		["bot", voice],
		["peer", voice],
	]);
	h.controller.update(snapshot(entries, voices));
	await settle();
	h.controller.update(snapshot(entries, voices));
	await settle();
	expect(h.spoken).toEqual(["The first bot's answer", "My follow-up"]);
	expect(h.played).toEqual(h.spoken);
	h.controller.dispose();
});

test("reply chains outside this call and cyclic references never start speech", async () => {
	const h = harness();
	h.controller.update(
		snapshot([
			transcript,
			reply("typed-answer", 2, "Typed", { reply_to: "typed" }),
			reply("typed-follow-up", 3, "Still typed", { reply_to: "typed-answer" }),
			reply("cycle-a", 4, "Cycle A", { reply_to: "cycle-b" }),
			reply("cycle-b", 5, "Cycle B", { reply_to: "cycle-a" }),
		]),
	);
	await settle();
	expect(h.spoken).toEqual([]);
	h.controller.dispose();
});

test("voice lookup may finish after a new reply without replaying the join baseline", async () => {
	const old = reply("old", 2);
	const h = harness([transcript, old]);
	const entries = [transcript, old, reply("new", 3)];
	h.controller.update(snapshot(entries, new Map()));
	await settle();
	expect(h.spoken).toEqual([]);
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toEqual(["new"]);
});

test("calls prefetch one short chunk while playing and finish the reply before starting another", async () => {
	const first = deferred<void>();
	const starts: string[] = [];
	const played: string[] = [];
	const h = harness([transcript], {
		play: async (blob) => {
			played.push(await blob.text());
			if (played.length === 1) await first.promise;
		},
		synthesize: async (_voice, text) => {
			starts.push(text);
			return new Blob([text]);
		},
	});
	h.controller.update(
		snapshot([transcript, reply("long", 2, "a".repeat(640)), reply("next", 3)]),
	);
	await settle();
	expect(starts).toEqual(["a".repeat(80), "a".repeat(240)]);
	expect(played).toEqual(["a".repeat(80)]);
	first.resolve();
	await settle();
	expect(starts).toEqual([
		"a".repeat(80),
		"a".repeat(240),
		"a".repeat(240),
		"a".repeat(80),
		"next",
	]);
	expect(played).toEqual(starts);
});

test("call chunks prefer phrase boundaries and keep the opening shorter than later chunks", () => {
	const opening = "The opening sentence has enough context to read naturally.";
	const text = `${opening} ${opening}\n${"More detail follows in a longer explanation. ".repeat(12).trim()}`;
	const chunks = callSpeechChunks(text);
	expect(chunks[0]).toBe(opening);
	expect(Array.from(chunks[0]).length).toBeLessThanOrEqual(80);
	expect(
		chunks.slice(1).every((chunk) => Array.from(chunk).length <= 240),
	).toBe(true);
	expect(chunks.join(" ").replace(/\s+/g, " ")).toBe(text.replace(/\s+/g, " "));
	expect(callSpeechChunks("A short reply.")).toEqual(["A short reply."]);
	const longSentence = `${"A ".repeat(79)}x.`;
	const split = callSpeechChunks(`${longSentence} Next sentence.`);
	expect(Array.from(split[0]).length).toBeLessThanOrEqual(80);
	expect(split.join(" ")).toBe(`${longSentence} Next sentence.`);
	expect(
		callSpeechChunks("Hi. This is the rest of the answer, in full."),
	).toEqual(["Hi. This is the rest of the answer, in full."]);
	expect(
		callSpeechChunks("😀".repeat(500)).map((chunk) => Array.from(chunk).length),
	).toEqual([80, 240, 180]);
	expect(callSpeechChunks("😀a", 1)).toEqual(["😀", "a"]);
});

test("pending prefetch is discarded when the call or reply is cancelled", async () => {
	for (const invalidation of [
		"dispose",
		"edit",
		"delete",
		"election",
	] as const) {
		const playback = deferred<void>();
		const prefetch = deferred<Blob>();
		let elected = true;
		let synthesized = 0;
		const played: string[] = [];
		const h = harness([transcript], {
			canSpeak: () => elected,
			synthesize: async (_voice, text) =>
				++synthesized === 1 ? new Blob([text]) : prefetch.promise,
			play: async (blob) => {
				played.push(await blob.text());
				await playback.promise;
			},
			stop: () => playback.resolve(),
		});
		const entries = [transcript, reply("long", 2, "a".repeat(640))];
		h.controller.update(snapshot(entries));
		await settle();
		expect(synthesized).toBe(2);
		expect(played).toEqual(["a".repeat(80)]);
		if (invalidation === "dispose") h.controller.dispose();
		else if (invalidation === "election") {
			elected = false;
			h.controller.update(snapshot(entries));
			elected = true;
			h.controller.update(snapshot(entries));
		} else {
			h.controller.update(
				snapshot([
					...entries,
					entry("changed", 3, invalidation, {
						target: "long",
						text: "Revised",
					}),
				]),
			);
		}
		prefetch.resolve(new Blob(["late"]));
		await settle();
		expect(played).toEqual(["a".repeat(80)]);
		expect(synthesized).toBe(2);
		expect(h.errors).toEqual([]);
		h.controller.dispose();
	}
});

test("prefetch failures are handled while playback is pending and later replies still speak", async () => {
	const playback = deferred<void>();
	const prefetch = deferred<Blob>();
	const failure = new Error("Synthesis failed");
	let synthesized = 0;
	const played: string[] = [];
	const h = harness([transcript], {
		synthesize: async (_voice, text) =>
			++synthesized === 2 ? prefetch.promise : new Blob([text]),
		play: async (blob) => {
			played.push(await blob.text());
			if (played.length === 1) await playback.promise;
		},
	});
	h.controller.update(
		snapshot([transcript, reply("long", 2, "a".repeat(640)), reply("next", 3)]),
	);
	await settle();
	prefetch.reject(failure);
	await new Promise((resolve) => setTimeout(resolve, 0));
	expect(played).toEqual(["a".repeat(80)]);
	expect(h.errors).toEqual([]);
	playback.resolve();
	await settle();
	expect(played).toEqual(["a".repeat(80), "next"]);
	expect(h.errors).toEqual([failure]);
	h.controller.dispose();
});

test("a prefetch rejected after disposal cannot play or report a stale error", async () => {
	const playback = deferred<void>();
	const prefetch = deferred<Blob>();
	let synthesized = 0;
	const h = harness([transcript], {
		synthesize: async (_voice, text) =>
			++synthesized === 1 ? new Blob([text]) : prefetch.promise,
		play: () => playback.promise,
		stop: () => playback.resolve(),
	});
	h.controller.update(
		snapshot([transcript, reply("long", 2, "a".repeat(640))]),
	);
	await settle();
	expect(synthesized).toBe(2);
	h.controller.dispose();
	prefetch.reject(new Error("Late failure"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	expect(synthesized).toBe(2);
	expect(h.errors).toEqual([]);
});

test("failed playback waits for its prefetch before synthesizing the next reply", async () => {
	for (const outcome of ["resolve", "reject"] as const) {
		const playback = deferred<void>();
		const prefetch = deferred<Blob>();
		const failure = new Error("Playback failed");
		const starts: string[] = [];
		const played: string[] = [];
		let active = 0;
		let overlap = 0;
		const h = harness([transcript], {
			synthesize: async (_voice, text) => {
				starts.push(text);
				active++;
				overlap = Math.max(overlap, active);
				try {
					return starts.length === 2
						? await prefetch.promise
						: new Blob([text]);
				} finally {
					active--;
				}
			},
			play: async (blob) => {
				played.push(await blob.text());
				if (played.length === 1) await playback.promise;
			},
		});
		h.controller.update(
			snapshot([
				transcript,
				reply("long", 2, "a".repeat(640)),
				reply("next", 3),
			]),
		);
		await settle();
		playback.reject(failure);
		await settle();
		expect(starts).toEqual(["a".repeat(80), "a".repeat(240)]);
		expect(active).toBe(1);
		if (outcome === "resolve") prefetch.resolve(new Blob(["discarded"]));
		else prefetch.reject(new Error("Discarded synthesis failed"));
		await settle();
		expect(starts).toEqual(["a".repeat(80), "a".repeat(240), "next"]);
		expect(played).toEqual(["a".repeat(80), "next"]);
		expect(overlap).toBe(1);
		expect(active).toBe(0);
		expect(h.errors).toEqual([failure]);
		h.controller.dispose();
	}
});

test("leaving during synthesis discards the result and pending replies", async () => {
	const pending = deferred<Blob>();
	const h = harness([transcript], { synthesize: () => pending.promise });
	h.controller.update(snapshot([transcript, reply("a", 2), reply("b", 3)]));
	h.controller.dispose();
	pending.resolve(new Blob(["late"]));
	await settle();
	expect(h.played).toEqual([]);
	expect(h.stops()).toBe(1);
	expect(h.errors).toEqual([]);
});

test("bot replies use the selected voice's lower character limit", async () => {
	const h = harness();
	const text = "😀".repeat(410);
	h.controller.update(
		snapshot(
			[transcript, reply("long", 2, text)],
			new Map([["bot", { ...voice, maxInputCharacters: 100 }]]),
		),
	);
	await settle();
	expect(h.spoken.map((chunk) => Array.from(chunk).length)).toEqual([
		80, 100, 100, 100, 30,
	]);
	expect(h.spoken.join("")).toBe(text);
	expect(h.played).toEqual(h.spoken);
});

test("voice configuration changes, removal and lost election cancel active work even if eligibility returns", async () => {
	for (const invalidate of ["pin", "voice", "limit", "removed", "election"]) {
		const pending = deferred<Blob>();
		let elected = true;
		const h = harness([transcript], {
			synthesize: () => pending.promise,
			canSpeak: () => elected,
		});
		const entries = [transcript, reply("a", 2)];
		h.controller.update(snapshot(entries));
		if (invalidate === "election") elected = false;
		h.controller.update(
			snapshot(
				entries,
				invalidate === "pin"
					? new Map([["bot", { bitId: "other", local: false }]])
					: invalidate === "voice"
						? new Map([["bot", { ...voice, voice: "af_bella" }]])
						: invalidate === "limit"
							? new Map([["bot", { ...voice, maxInputCharacters: 2000 }]])
							: invalidate === "removed"
								? new Map()
								: undefined,
			),
		);
		elected = true;
		h.controller.update(snapshot(entries));
		pending.resolve(new Blob(["late"]));
		await settle();
		expect(h.played).toEqual([]);
		expect(h.stops()).toBeGreaterThan(0);
	}
});

test("failed voice playback keeps later replies eligible and reports the error", async () => {
	let attempt = 0;
	const h = harness([transcript], {
		play: async () => {
			if (++attempt === 1) throw new Error("Autoplay blocked");
		},
	});
	h.controller.update(snapshot([transcript, reply("a", 2), reply("b", 3)]));
	await settle();
	expect(attempt).toBe(2);
	expect(h.errors).toHaveLength(1);
});

test("speech chunks preserve unicode and do not exceed the speech API character limit", () => {
	const text = "😀".repeat(4100);
	const chunks = speechChunks(text);
	expect(chunks.map((chunk) => Array.from(chunk).length)).toEqual([4096, 4]);
	expect(chunks.join("")).toBe(text);
});

test("invalid character limits cannot stall chunking or bypass the gateway limit", () => {
	const text = "😀".repeat(4100);
	for (const maximum of [
		0,
		-1,
		0.5,
		Number.NaN,
		Number.POSITIVE_INFINITY,
		5000,
	]) {
		const chunks = speechChunks(text, maximum);
		expect(chunks.map((chunk) => Array.from(chunk).length)).toEqual([4096, 4]);
		expect(chunks.join("")).toBe(text);
	}
	for (const value of [undefined, null, "2000", {}, true]) {
		expect(speechCharacterLimit(value)).toBe(4096);
	}
	expect(speechCharacterLimit(2000)).toBe(2000);
	expect(speechChunks("😀a", 1)).toEqual(["😀", "a"]);
});

test("election changes during convergence never start synthesis", async () => {
	let elected = true;
	const h = harness([transcript], {
		canSpeak: () => elected,
		electionDelayMs: 15,
	});
	const entries = [transcript, reply("answer", 2)];
	h.controller.update(snapshot(entries));
	elected = false;
	await new Promise((resolve) => setTimeout(resolve, 25));
	expect(h.spoken).toEqual([]);
	h.controller.dispose();
});

test("speech reads prose without markdown syntax and skips code and table-only replies", async () => {
	const h = harness();
	const text =
		"# Hello\n**Read** the [guide](https://example.test).\n```js\nalert('skip')\n```\n| Skip | Table |\nThanks.";
	h.controller.update(
		snapshot([
			transcript,
			reply("prose", 2, text),
			reply("code", 3, "```js\nalert('skip')\n```"),
			reply("table", 4, "| A | B |\n| - | - |"),
		]),
	);
	await settle();
	expect(h.spoken).toEqual(["Hello\nRead the guide.", "Thanks."]);
});

test("voice pins resolve plain IDs and qualified hubs without losing URL scheme or port", () => {
	expect(speechBitReference("voice")).toEqual({ id: "voice" });
	expect(speechBitReference("api.example:voice")).toEqual({
		id: "voice",
		hub: "api.example",
	});
	expect(speechBitReference("https://api.example:8443:voice")).toEqual({
		id: "voice",
		hub: "https://api.example:8443",
	});
});

test("interrupting cuts the reply, drops the queue and prefetch, and reports what was heard", async () => {
	const playback = deferred<void>();
	const prefetch = deferred<Blob>();
	let synthesized = 0;
	let position = { elapsedMs: 500, durationMs: 1000 };
	const played: string[] = [];
	const h = harness([transcript], {
		synthesize: async (_voice, text) =>
			++synthesized === 2 ? prefetch.promise : new Blob([text]),
		play: async (blob) => {
			played.push(await blob.text());
			await playback.promise;
		},
		stop: () => playback.resolve(),
		progress: () => position,
	});
	const text = `${"a".repeat(79)} ${"b".repeat(200)}`;
	h.controller.update(
		snapshot([transcript, reply("long", 2, text), reply("next", 3)]),
	);
	await settle();
	expect(played).toHaveLength(1);
	expect(h.controller.playingText()).toBe("a".repeat(79));
	expect(h.controller.interrupt()).toEqual({
		message_id: "long",
		heard_chars: 39,
		spoken_chars: 280,
	});
	position = { elapsedMs: 0, durationMs: 0 };
	prefetch.resolve(new Blob(["late"]));
	await settle();
	expect(played).toEqual(["a".repeat(79)]);
	expect(synthesized).toBe(2);
	expect(h.errors).toEqual([]);
	expect(h.controller.playingText()).toBeNull();
	expect(h.controller.interrupt()).toBeNull();
	h.controller.dispose();
});

test("later chunks add the fully played ones to the heard share", async () => {
	const gates = [deferred<void>(), deferred<void>()];
	let plays = 0;
	const h = harness([transcript], {
		play: async () => {
			await gates[plays++]?.promise;
		},
		stop: () => gates[plays - 1]?.resolve(),
		progress: () => ({ elapsedMs: 1000, durationMs: 4000 }),
	});
	const text = `${"a".repeat(79)} ${"b".repeat(200)}`;
	h.controller.update(snapshot([transcript, reply("long", 2, text)]));
	await settle();
	gates[0].resolve();
	await settle();
	expect(plays).toBe(2);
	expect(h.controller.interrupt()).toEqual({
		message_id: "long",
		heard_chars: 79 + 50,
		spoken_chars: 280,
	});
	h.controller.dispose();
});

test("interrupting during the election delay or synthesis cancels without a record", async () => {
	const pending = deferred<Blob>();
	const h = harness([transcript], {
		electionDelayMs: () => 1000,
		synthesize: () => pending.promise,
	});
	h.controller.update(snapshot([transcript, reply("a", 2), reply("b", 3)]));
	await settle();
	expect(h.controller.interrupt()).toBeNull();
	await settle();
	expect(h.spoken).toEqual([]);
	h.controller.dispose();
});

test("an interrupt request older than the reply, or for another bot, is ignored", async () => {
	const playback = deferred<void>();
	let clock = 1000;
	const h = harness([transcript], {
		now: () => clock,
		play: () => playback.promise,
		stop: () => playback.resolve(),
	});
	h.controller.update(snapshot([transcript, reply("a", 2)]));
	await settle();
	clock = 2000;
	expect(h.controller.interrupt({ since: 999 })).toBeNull();
	expect(h.controller.interrupt({ botId: "other" })).toBeNull();
	expect(h.controller.interrupt({ botId: "bot", since: 1500 })).toEqual({
		message_id: "a",
		heard_chars: 0,
		spoken_chars: 1,
	});
	h.controller.dispose();
});

test("a function election delay is read per reply", async () => {
	let delay = 0;
	const h = harness([transcript], { electionDelayMs: () => delay });
	h.controller.update(snapshot([transcript, reply("a", 2)]));
	await settle();
	expect(h.played).toEqual(["a"]);
	delay = 60_000;
	h.controller.update(snapshot([transcript, reply("a", 2), reply("b", 3)]));
	await settle();
	expect(h.played).toEqual(["a"]);
	h.controller.dispose();
});

test("latency trace logs each reply once, when playback starts or when it is not voiced", async () => {
	const lines: Record<string, unknown>[] = [];
	let clock = 0;
	const trace = createCallLatencyTrace({
		now: () => clock,
		log: (line) => lines.push(line),
	});
	trace.transcriptPosted("spoken", {
		speechEndAt: -400,
		sttStartAt: -300,
		sttDoneAt: -100,
		postedAt: 0,
	});
	let speakable = true;
	const h = harness([transcript], {
		trace,
		canSpeak: () => speakable,
		synthesize: async (_voice, text) => {
			clock += 200;
			return new Blob([text]);
		},
	});
	clock = 300;
	h.controller.update(snapshot([transcript, reply("a", 2)]));
	await settle();
	expect(lines).toEqual([
		expect.objectContaining({
			reply: "a",
			outcome: "played",
			speechToSttMs: 100,
			sttMs: 200,
			postMs: 100,
			botMs: 300,
			synthesisMs: 200,
			playMs: 0,
			totalMs: 900,
		}),
	]);
	speakable = false;
	h.controller.update(snapshot([transcript, reply("a", 2), reply("b", 3)]));
	await settle();
	expect(lines).toHaveLength(2);
	expect(lines[1]).toMatchObject({ reply: "b", outcome: "not voiced here" });
	h.controller.dispose();
});

test("latency traces expire", () => {
	const lines: Record<string, unknown>[] = [];
	let clock = 0;
	const trace = createCallLatencyTrace({
		now: () => clock,
		log: (line) => lines.push(line),
	});
	trace.replySeen("old", "reply-old");
	clock = 200_000;
	for (let index = 0; index < 70; index++)
		trace.replySeen(`t${index}`, `r${index}`);
	trace.playbackStarted("reply-old");
	expect(lines).toEqual([]);
});

test("echo detection compares normalized words with what the bot is saying", () => {
	const spoken = "Sure, the meeting is on Tuesday at noon.";
	expect(isBotEcho("the meeting is on tuesday", spoken)).toBe(true);
	expect(isBotEcho("Meeting... on TUESDAY!", spoken)).toBe(true);
	expect(isBotEcho("stop talking please", spoken)).toBe(false);
	expect(isBotEcho("tuesday is wrong, stop", spoken)).toBe(false);
	expect(isBotEcho("", spoken)).toBe(false);
	expect(isBotEcho("...", spoken)).toBe(false);
});

test("the first piece is the first sentence of at least 12 characters, in English and German", () => {
	const english = "Sure, I can do that. Then I will look at the";
	const first = nextSpeechPiece(english, 0);
	expect(first).toEqual({ text: "Sure, I can do that.", handed: 20 });
	expect(nextSpeechPiece(english, 20)).toBeNull();
	const german =
		"Das mache ich gern. Ich schaue kurz nach, was morgen ansteht.";
	expect(nextSpeechPiece(german, 0)?.text).toBe("Das mache ich gern.");
	expect(nextSpeechPiece("Ja. Das ist gut. Und", 0)?.text).toBe(
		"Ja. Das ist gut.",
	);
	expect(nextSpeechPiece("Hello wor", 0)).toBeNull();
	expect(nextSpeechPiece("Version 3.5 is out", 0)).toBeNull();
});

test("a lead-in without an end mark waits until flush or end hands over the remainder", () => {
	const text = "Moment, ich schaue nach:";
	expect(nextSpeechPiece(text, 0)).toBeNull();
	expect(nextSpeechPiece(text, 0, { final: true })).toEqual({
		text,
		handed: text.length,
	});
	expect(
		nextSpeechPiece(`${text}   `, text.length, { final: true }),
	).toBeNull();
});

test("120 characters without a sentence end cut at a clause after 40, never inside a word", () => {
	const words = Array.from({ length: 30 }, (_, i) => `word${i}`);
	words[10] = "word10,";
	words[20] = "word20;";
	const text = words.join(" ");
	expect(text.length).toBeGreaterThan(120);
	const piece = nextSpeechPiece(text, 0);
	expect(piece?.text.endsWith("word10,")).toBe(true);
	expect(nextSpeechPiece(text.slice(0, 100), 0)).toBeNull();
	const plain = Array.from({ length: 80 }, (_, i) => `w${i}`).join(" ");
	const cut = nextSpeechPiece(plain, 0, { limit: 240 });
	expect(cut?.text.length).toBeLessThanOrEqual(240);
	expect(plain.startsWith(cut?.text ?? "x")).toBe(true);
	expect(plain[cut?.handed ?? 0]).toBe(" ");
});

test("later pieces merge whole sentences up to the limit", () => {
	const text =
		"Opening line is here. Second one is here. Third one follows. Fourth";
	const first = nextSpeechPiece(text, 0);
	expect(first?.text).toBe("Opening line is here.");
	const second = nextSpeechPiece(text, first?.handed ?? 0);
	expect(second?.text).toBe("Second one is here. Third one follows.");
	const small = nextSpeechPiece(text, first?.handed ?? 0, { limit: 25 });
	expect(small?.text).toBe("Second one is here.");
});

test("sentence marks cover CJK, newlines and quoted endings", () => {
	const cjk = "今天天气非常好，我们可以出去散步，也可以在家看书。然后";
	expect(nextSpeechPiece(cjk, 0)?.text).toBe(
		"今天天气非常好，我们可以出去散步，也可以在家看书。",
	);
	expect(nextSpeechPiece("# Heading line here\nNext", 0)?.text).toBe(
		"# Heading line here",
	);
	expect(nextSpeechPiece('He said "go home." Then left', 0)?.text).toBe(
		'He said "go home."',
	);
});

test("a shrinking update speaks nothing until it grows past what was handed over", () => {
	const text = "Sure, I can do that. Then more";
	expect(nextSpeechPiece("Sure, I can", 20)).toBeNull();
	expect(nextSpeechPiece(text, 20)).toBeNull();
	expect(nextSpeechPiece(`${text} is said. And`, 20)?.text).toBe(
		"Then more is said.",
	);
});

function streamHarness(overrides: Parameters<typeof harness>[1] = {}) {
	const h = harness([transcript], overrides);
	h.controller.update(snapshot([transcript]));
	return h;
}

test("a streamed reply speaks piece by piece in order while its text grows", async () => {
	const h = streamHarness();
	const stream = h.controller.stream("spoken", "bot");
	expect(stream).not.toBeNull();
	stream?.update("**Sure**, I can do that. Then I will");
	await settle();
	expect(h.spoken).toEqual(["Sure, I can do that."]);
	stream?.update("**Sure**, I can do that. Then I will look at it. And done");
	await settle();
	expect(h.spoken).toEqual(["Sure, I can do that.", "Then I will look at it."]);
	stream?.end("**Sure**, I can do that. Then I will look at it. And done");
	await settle();
	expect(h.played).toEqual([
		"Sure, I can do that.",
		"Then I will look at it.",
		"And done",
	]);
	h.controller.dispose();
});

test("the next piece is synthesized while the previous one plays", async () => {
	const playing = deferred<void>();
	const h = streamHarness({ play: () => playing.promise });
	const stream = h.controller.stream("spoken", "bot");
	stream?.update("First sentence is here. Second sentence follows. Tail");
	await settle();
	expect(h.spoken).toEqual([
		"First sentence is here.",
		"Second sentence follows.",
	]);
	expect(h.controller.playingText("bot")).toBe("First sentence is here.");
	playing.resolve();
	stream?.cancel();
	await settle();
	h.controller.dispose();
});

test("flush speaks a lead-in at once and unknown bots or silent devices get no stream", async () => {
	const h = streamHarness({ canSpeak: (botId) => botId === "bot" });
	expect(h.controller.stream("spoken", "other")).toBeNull();
	const stream = h.controller.stream("spoken", "bot");
	stream?.update("Moment, ich schaue nach:");
	await settle();
	expect(h.spoken).toEqual([]);
	stream?.flush();
	await settle();
	expect(h.spoken).toEqual(["Moment, ich schaue nach:"]);
	stream?.end();
	await settle();
	const quiet = harness([transcript], { canSpeak: () => false });
	quiet.controller.update(snapshot([transcript]));
	expect(quiet.controller.stream("spoken", "bot")).toBeNull();
	h.controller.dispose();
});

test("the stored reply that arrives later is not spoken a second time", async () => {
	const h = streamHarness();
	const stream = h.controller.stream("spoken", "bot");
	stream?.end("Sure, I can do that. Done here.");
	await settle();
	const entries = [
		transcript,
		reply("r1", 2, "Sure, I can do that. Done here."),
	];
	h.controller.update(snapshot(entries));
	await settle();
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toEqual(["Sure, I can do that.", "Done here."]);
	h.controller.dispose();
});

test("a stream cancelled before any audio lets the inbox reply speak normally", async () => {
	const h = streamHarness();
	const stream = h.controller.stream("spoken", "bot");
	stream?.update("Hello th");
	stream?.cancel();
	await settle();
	expect(h.spoken).toEqual([]);
	h.controller.update(snapshot([transcript, reply("r1", 2, "stored answer")]));
	await settle();
	expect(h.spoken).toEqual(["stored answer"]);

	const early = streamHarness();
	const second = early.controller.stream("spoken", "bot");
	early.controller.update(
		snapshot([transcript, reply("r1", 2, "host answer")]),
	);
	await settle();
	expect(early.spoken).toEqual([]);
	second?.cancel();
	await settle();
	expect(early.spoken).toEqual(["host answer"]);
	h.controller.dispose();
	early.controller.dispose();
});

test("a stream cancelled after audio began keeps the stored reply silent", async () => {
	const playing = deferred<void>();
	const h = streamHarness({ play: () => playing.promise });
	const stream = h.controller.stream("spoken", "bot");
	stream?.update("First sentence is here. Second sentence follows. Tail");
	await settle();
	stream?.cancel();
	h.controller.update(snapshot([transcript, reply("r1", 2, "whole")]));
	playing.resolve();
	await settle();
	expect(h.spoken).toEqual([
		"First sentence is here.",
		"Second sentence follows.",
	]);
	expect(h.played).toEqual([]);
	h.controller.dispose();
});

test("interrupting a stream stops at once, and reports what was heard once the reply is stored", async () => {
	const playing = deferred<void>();
	const text = "First sentence is here.";
	const h = streamHarness({
		play: () => playing.promise,
		progress: () => ({ elapsedMs: 500, durationMs: 1000 }),
	});
	const stream = h.controller.stream("spoken", "bot");
	stream?.update(`${text} More to come`);
	await settle();
	expect(h.controller.interrupt()).toBeNull();
	expect(h.stops()).toBe(1);
	stream?.update(`${text} More to come now`);
	await settle();
	expect(h.spoken).toEqual([text]);

	const done = streamHarness({
		play: () => playing.promise,
		progress: () => ({ elapsedMs: 500, durationMs: 1000 }),
	});
	const second = done.controller.stream("spoken", "bot");
	second?.end(text);
	await settle();
	done.controller.update(snapshot([transcript, reply("r1", 2, text)]));
	const record = done.controller.interrupt();
	const chars = Array.from(text).length;
	expect(record).toEqual({
		message_id: "r1",
		heard_chars: Math.floor(chars / 2),
		spoken_chars: chars,
	});
	playing.resolve();
	await settle();
	h.controller.dispose();
	done.controller.dispose();
});

test("pause and resume reach the output only while speech plays", async () => {
	const playing = deferred<void>();
	const calls: string[] = [];
	const h = streamHarness({
		play: () => playing.promise,
		pause: () => {
			calls.push("pause");
			return true;
		},
		resume: () => calls.push("resume"),
	});
	expect(h.controller.pause()).toBe(false);
	const stream = h.controller.stream("spoken", "bot");
	stream?.end("First sentence is here.");
	await settle();
	expect(h.controller.pause()).toBe(true);
	h.controller.resume();
	expect(calls).toEqual(["pause", "resume"]);
	playing.resolve();
	await settle();
	expect(h.controller.pause()).toBe(false);
	h.controller.dispose();
});

test("a failing stream falls back to a complete file and is not tried again for that voice", async () => {
	const attempts: string[] = [];
	const streamed: string[] = [];
	const h = streamHarness({
		synthesizeStream: async (_voice, text) => {
			attempts.push(text);
			if (attempts.length === 1) throw new Error("unsupported");
			return { sampleRate: 24000, chunks: (async function* () {})() };
		},
		playStream: async () => {
			streamed.push("stream");
		},
	});
	const stream = h.controller.stream("spoken", "bot");
	stream?.end(
		"First sentence is here. Second sentence follows. Third one now.",
	);
	await settle();
	expect(attempts).toEqual(["First sentence is here."]);
	expect(h.played[0]).toBe("First sentence is here.");
	expect(h.played).toEqual([
		"First sentence is here.",
		"Second sentence follows. Third one now.",
	]);
	expect(streamed).toEqual([]);
	h.controller.dispose();
});

test("hosted voices play through streams and local voices keep complete files", async () => {
	const h = streamHarness({
		synthesizeStream: async () => ({
			sampleRate: 24000,
			chunks: (async function* () {})(),
		}),
		playStream: async () => {
			h.played.push("streamed");
		},
	});
	h.controller.stream("spoken", "bot")?.end("First sentence is here.");
	await settle();
	expect(h.played).toEqual(["streamed"]);
	expect(h.spoken).toEqual([]);
	h.controller.dispose();

	const local = harness([transcript], {
		synthesizeStream: async () => {
			throw new Error("never");
		},
		playStream: async () => {},
	});
	const localVoice: BotSpeechVoice = { bitId: "local", local: true };
	local.controller.update(
		snapshot([transcript], new Map([["bot", localVoice]])),
	);
	local.controller.stream("spoken", "bot")?.end("First sentence is here.");
	await settle();
	expect(local.played).toEqual(["First sentence is here."]);
	local.controller.dispose();
});

function manualTimers() {
	const pending = new Map<number, { callback: () => void; ms: number }>();
	let next = 0;
	return {
		timers: {
			set: (callback: () => void, ms: number) => {
				pending.set(++next, { callback, ms });
				return next;
			},
			clear: (handle: unknown) => {
				pending.delete(handle as number);
			},
		},
		pending,
		fire(ms: number) {
			for (const [id, timer] of [...pending]) {
				if (timer.ms !== ms) continue;
				pending.delete(id);
				timer.callback();
			}
		},
	};
}

test("the working cue starts after 900 ms of silence and stops when audio starts", async () => {
	const clock = manualTimers();
	const events: string[] = [];
	const playing = deferred<void>();
	const h = streamHarness({
		timers: clock.timers,
		cue: () => ({
			start: () => events.push("start"),
			stop: () => events.push("stop"),
		}),
		play: () => playing.promise,
	});
	const stream = h.controller.stream("spoken", "bot");
	expect([...clock.pending.values()].map((timer) => timer.ms)).toEqual([900]);
	clock.fire(900);
	expect(events).toEqual(["start"]);
	stream?.update("First sentence is here. Tail");
	await settle();
	expect(events).toEqual(["start", "stop"]);
	stream?.end();
	playing.resolve();
	await settle();
	expect(events).toEqual(["start", "stop"]);
	h.controller.dispose();
});

test("after a flush the cue waits for pending speech and 600 ms without new text", async () => {
	const clock = manualTimers();
	const events: string[] = [];
	const playing = deferred<void>();
	const h = streamHarness({
		timers: clock.timers,
		cue: () => ({
			start: () => events.push("start"),
			stop: () => events.push("stop"),
		}),
		play: () => playing.promise,
	});
	const stream = h.controller.stream("spoken", "bot");
	stream?.update("Moment, ich schaue nach:");
	clock.fire(900);
	expect(events).toEqual([]);
	stream?.flush();
	await settle();
	clock.fire(600);
	expect(events).toEqual([]);
	playing.resolve();
	await settle();
	clock.fire(600);
	expect(events).toEqual(["start"]);
	stream?.update("Moment, ich schaue nach: und weiter geht es");
	expect(events).toEqual(["start", "stop"]);
	h.controller.dispose();
});

test("repeated flushes of one tool round do not push the cue back", () => {
	const clock = manualTimers();
	const events: string[] = [];
	const h = streamHarness({
		timers: clock.timers,
		cue: () => ({
			start: () => events.push("start"),
			stop: () => events.push("stop"),
		}),
	});
	const stream = h.controller.stream("spoken", "bot");
	stream?.flush();
	const armed = [...clock.pending.keys()];
	stream?.flush();
	stream?.flush();
	expect([...clock.pending.keys()]).toEqual(armed);
	clock.fire(600);
	expect(events).toEqual(["start"]);
	h.controller.dispose();
});

test("end, cancel and interrupt silence a running cue", async () => {
	for (const stopWith of ["end", "cancel", "interrupt", "dispose"] as const) {
		const clock = manualTimers();
		const events: string[] = [];
		const h = streamHarness({
			timers: clock.timers,
			cue: () => ({
				start: () => events.push("start"),
				stop: () => events.push("stop"),
			}),
		});
		const stream = h.controller.stream("spoken", "bot");
		clock.fire(900);
		if (stopWith === "end") stream?.end();
		if (stopWith === "cancel") stream?.cancel();
		if (stopWith === "interrupt") h.controller.interrupt();
		if (stopWith === "dispose") h.controller.dispose();
		await settle();
		expect(events).toEqual(["start", "stop"]);
		expect(clock.pending.size).toBe(0);
		h.controller.dispose();
	}
});

test("the latency trace tells the stream route from the queued one and times the first text", async () => {
	const lines: Record<string, unknown>[] = [];
	let clock = 0;
	const trace = createCallLatencyTrace({
		now: () => clock,
		log: (line) => lines.push(line),
	});
	const timing = {
		speechEndAt: 0,
		sttStartAt: 10,
		sttDoneAt: 110,
		postedAt: 120,
	};
	trace.transcriptPosted("spoken", {
		...timing,
		route: "stream",
		turnWaitMs: 300,
		turnSource: "model",
	});
	const h = streamHarness({ trace });
	const stream = h.controller.stream("spoken", "bot");
	clock = 420;
	stream?.update("First sentence is here. Tail");
	await settle();
	expect(lines).toHaveLength(1);
	expect(lines[0]).toMatchObject({
		transcript: "spoken",
		reply: "stream:spoken",
		route: "stream",
		turnWaitMs: 300,
		turnSource: "model",
		firstTextMs: 300,
		outcome: "played",
	});
	trace.transcriptPosted("queued", timing);
	trace.replySeen("queued", "r");
	trace.playbackStarted("r");
	expect(lines[1]).toMatchObject({ route: "queued", firstTextMs: undefined });
	h.controller.dispose();
});

const FIRST = "First sentence is here.";

test("a pushed stream is refused for a message already seen or a transcript that is not this call's", async () => {
	const old = reply("r1", 2, FIRST);
	const history = harness([transcript, old]);
	history.controller.update(snapshot([transcript, old]));
	expect(
		history.controller.stream("spoken", "bot", { messageId: "r1" }),
	).toBeNull();

	const stored = streamHarness();
	expect(
		stored.controller.stream("elsewhere", "bot", { messageId: "r9" }),
	).toBeNull();
	expect(
		stored.controller.stream("spoken", "bot", { messageId: "r9" }),
	).not.toBeNull();

	const first = streamHarness();
	first.controller.update(snapshot([transcript, reply("r1", 2, FIRST)]));
	await settle();
	expect(
		first.controller.stream("spoken", "bot", { messageId: "r1" }),
	).toBeNull();
	for (const h of [history, stored, first]) h.controller.dispose();
});

test("a pushed stream speaks as text grows and the stored entry is not spoken again", async () => {
	const h = streamHarness();
	const stream = h.controller.stream("spoken", "bot", { messageId: "r1" });
	stream?.update(`${FIRST} Second sentence follows. Tail`);
	await settle();
	expect(h.spoken).toEqual([FIRST, "Second sentence follows."]);
	stream?.end();
	await settle();
	const entries = [
		transcript,
		reply("r1", 2, `${FIRST} Second sentence follows. Tail`),
	];
	h.controller.update(snapshot(entries));
	await settle();
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.spoken).toEqual([FIRST, "Second sentence follows.", "Tail"]);
	expect(h.played).toEqual(h.spoken);
	h.controller.dispose();
});

test("a stored entry that arrives first speaks once and a later pushed stream is refused", async () => {
	const h = streamHarness();
	h.controller.update(snapshot([transcript, reply("r1", 2, "stored answer")]));
	await settle();
	expect(h.controller.stream("spoken", "bot", { messageId: "r1" })).toBeNull();
	await settle();
	expect(h.spoken).toEqual(["stored answer"]);
	h.controller.dispose();
});

test("an open pushed stream ends when its stored entry arrives and does not hold the queue", async () => {
	const second = entry(
		"spoken2",
		3,
		"transcript",
		{ call: "call", text: "Again", offset_ms: 0 },
		"person",
	);
	const h = streamHarness();
	h.controller.update(snapshot([transcript, second]));
	const stream = h.controller.stream("spoken", "bot", { messageId: "r1" });
	stream?.update(`${FIRST} Tail`);
	await settle();
	const entries = [
		transcript,
		reply("r1", 2, `${FIRST} Tail`),
		second,
		entry("r2", 4, "message", { text: "next answer", reply_to: "spoken2" }),
	];
	h.controller.update(snapshot(entries));
	await settle();
	expect(h.played).toEqual([FIRST, "Tail", "next answer"]);
	h.controller.dispose();
});

test("the election delay applies to a pushed stream but not to a dock stream", async () => {
	const pushed = streamHarness({ electionDelayMs: 40 });
	pushed.controller
		.stream("spoken", "bot", { messageId: "r1" })
		?.update(`${FIRST} Tail`);
	await settle();
	expect(pushed.spoken).toEqual([]);
	await new Promise((resolve) => setTimeout(resolve, 80));
	await settle();
	expect(pushed.spoken).toEqual([FIRST]);

	const dock = streamHarness({ electionDelayMs: 40 });
	dock.controller.stream("spoken", "bot")?.update(`${FIRST} Tail`);
	await settle();
	expect(dock.spoken).toEqual([FIRST]);
	pushed.controller.dispose();
	dock.controller.dispose();
});

test("interrupting a pushed stream that ended reports the pushed message id", async () => {
	const playing = deferred<void>();
	const h = streamHarness({
		play: () => playing.promise,
		progress: () => ({ elapsedMs: 500, durationMs: 1000 }),
	});
	h.controller.stream("spoken", "bot", { messageId: "r1" })?.end(FIRST);
	await settle();
	const chars = Array.from(FIRST).length;
	expect(h.controller.interrupt()).toEqual({
		message_id: "r1",
		heard_chars: Math.floor(chars / 2),
		spoken_chars: chars,
	});
	playing.resolve();
	await settle();
	h.controller.dispose();
});

test("a pushed stream dropped after audio began lets the stored reply speak whole", async () => {
	const playing = deferred<void>();
	const h = streamHarness({ play: () => playing.promise });
	const stream = h.controller.stream("spoken", "bot", { messageId: "r1" });
	stream?.update(`${FIRST} Second sentence follows. Tail`);
	await settle();
	expect(h.controller.playingText("bot")).toBe(FIRST);
	stream?.cancel();
	expect(
		h.controller.stream("spoken", "bot", { messageId: "r1" }),
	).not.toBeNull();
	h.controller.dispose();

	const retried = streamHarness({ play: () => playing.promise });
	const dropped = retried.controller.stream("spoken", "bot", {
		messageId: "r1",
	});
	dropped?.update(`${FIRST} Second sentence follows. Tail`);
	await settle();
	dropped?.cancel();
	retried.controller.update(
		snapshot([transcript, reply("r1", 2, "The whole answer.")]),
	);
	playing.resolve();
	await settle();
	expect(retried.spoken).toEqual([
		FIRST,
		"Second sentence follows.",
		"The whole answer.",
	]);
	retried.controller.dispose();
});
