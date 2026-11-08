import { expect, test } from "bun:test";
import { foldInboxEntries } from "../../components/bots-tab/fold-entries";
import type {
	IInboxEntry,
	InboxEntryKind,
} from "../../state/backend-state/inbox-state";
import {
	type BotSpeechVoice,
	createBotCallSpeech,
	speechBitReference,
	speechCharacterLimit,
	speechChunks,
} from "./bot-call-speech";

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
	for (let i = 0; i < 12; i++) await Promise.resolve();
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

test("synthesis and playback remain serial and all long-reply chunks finish before the next reply", async () => {
	const first = deferred<void>();
	const starts: string[] = [];
	let plays = 0;
	const h = harness([transcript], {
		play: async () => {
			plays++;
			if (plays === 1) await first.promise;
		},
		synthesize: async (_voice, text) => {
			starts.push(text);
			return new Blob([text]);
		},
	});
	h.controller.update(
		snapshot([
			transcript,
			reply("long", 2, "a".repeat(4100)),
			reply("next", 3),
		]),
	);
	await settle();
	expect(starts).toEqual(["a".repeat(4096)]);
	first.resolve();
	await settle();
	expect(starts).toEqual(["a".repeat(4096), "aaaa", "next"]);
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
	const text = "😀".repeat(4100);
	h.controller.update(
		snapshot(
			[transcript, reply("long", 2, text)],
			new Map([["bot", { ...voice, maxInputCharacters: 2000 }]]),
		),
	);
	await settle();
	expect(h.spoken.map((chunk) => Array.from(chunk).length)).toEqual([
		2000, 2000, 100,
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
	expect(h.spoken).toEqual(["Hello\nRead the guide.\nThanks."]);
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
