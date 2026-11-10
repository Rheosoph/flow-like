import type {
	ConversationItem,
	ConversationMessage,
} from "../../components/bots-tab/fold-entries";
import type { ISpeechStream } from "../../state/backend-state/audio-state";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { compareInboxCursors } from "../inbox-cursor";
import { plainMarkdownLines } from "../markdown-text";
import { type CallLatencyTrace, callLatency } from "./call-latency";

export interface BotSpeechVoice {
	bitId: string;
	local: boolean;
	voice?: string;
	maxInputCharacters?: number;
}

export interface BotCallSpeechSnapshot {
	entries: readonly IInboxEntry[];
	items: readonly ConversationItem[];
	/** Null means the bot has no voice this participant can serve. Missing means still resolving. */
	voices: ReadonlyMap<string, BotSpeechVoice | null>;
}

export function speechBitReference(reference: string): {
	id: string;
	hub?: string;
} {
	const separator = reference.lastIndexOf(":");
	return separator > 0 && separator < reference.length - 1
		? { id: reference.slice(separator + 1), hub: reference.slice(0, separator) }
		: { id: reference };
}

interface Piece {
	text: string;
	audio?: Promise<PieceResult>;
	abort?: AbortController;
}

type PieceAudio =
	| { kind: "blob"; blob: Blob }
	| { kind: "stream"; stream: ISpeechStream };

type PieceResult =
	| { ok: true; audio: PieceAudio }
	| { ok: false; error: unknown };

interface SpeechSource {
	pieces: Piece[];
	ended: boolean;
	completed: number;
	spokenChars: number;
	wake: (() => void) | null;
}

interface LiveReply {
	key: string;
	transcriptId: string;
	reply: Reply;
	source: SpeechSource;
	state: "open" | "ended" | "dropped";
	text: string;
	handed: number;
	flushed: boolean;
	textSeen: boolean;
	audioStarted: boolean;
	cueOn: boolean;
	timer: unknown;
	messageId?: string;
	stored?: Reply;
}

interface Reply {
	id: string;
	botId: string;
	text: string;
	voice: BotSpeechVoice;
	live?: LiveReply;
}

/** What the person heard of a reply before it was cut off. Sizes count characters of the spoken text. */
export interface InterruptedSpeech {
	message_id: string;
	heard_chars: number;
	spoken_chars: number;
}

export interface SpeechPlaybackProgress {
	elapsedMs: number;
	durationMs: number;
}

/** A reply spoken while its text still streams in from the turn that answers one utterance. */
export interface StreamedReply {
	/** The reply's whole text so far; each call carries everything said before plus what is new. */
	update(text: string): void;
	/** The turn is busy without writing (a tool runs): speak the pending text now. */
	flush(): void;
	/** No more text follows. `text` replaces the last update when given. */
	end(text?: string): void;
	/** The turn failed or was superseded: drop what has not played. */
	cancel(): void;
}

export interface SpeechCue {
	start(): void;
	stop(): void;
}

export interface SpeechTimers {
	set(callback: () => void, ms: number): unknown;
	clear(handle: unknown): void;
}

const defaultTimers: SpeechTimers = {
	set: (callback, ms) => setTimeout(callback, ms),
	clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

const FIRST_CHUNK_LIMIT = 80;
const FIRST_CHUNK_MINIMUM = 20;
const ECHO_SHARE = 0.6;
const FIRST_PIECE_MINIMUM = 12;
const CLAUSE_AFTER = 120;
const CLAUSE_MINIMUM = 40;
const PIECE_LIMIT = 240;
const CUE_AFTER_OPEN_MS = 900;
const CUE_AFTER_FLUSH_MS = 600;

function words(text: string): string[] {
	return text.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
}

/** Whether most of what a microphone heard is words the bot is itself saying. */
export function isBotEcho(heard: string, spoken: string) {
	const heardWords = words(heard);
	if (heardWords.length === 0) return false;
	const known = new Set(words(spoken));
	const shared = heardWords.filter((word) => known.has(word)).length;
	return shared / heardWords.length >= ECHO_SHARE;
}

function firstChunkEnd(characters: readonly string[], limit: number): number {
	const cap = Math.min(limit, FIRST_CHUNK_LIMIT, characters.length);
	for (let at = Math.min(FIRST_CHUNK_MINIMUM, cap); at <= cap; at++) {
		const next = characters[at];
		if (
			next === undefined ||
			/[。！？\n]/.test(characters[at - 1]) ||
			(/[.!?…]/.test(characters[at - 1]) && /\s/.test(next))
		)
			return at;
	}
	for (let at = cap; at > Math.floor(cap / 2); at--) {
		if (/\s/.test(characters[at - 1])) return at;
	}
	return cap;
}

export function speechCharacterLimit(value: unknown): number {
	return typeof value === "number" && Number.isSafeInteger(value) && value > 0
		? Math.min(value, 4096)
		: 4096;
}

/** Preserve the complete reply within both the model and gateway character limits. */
export function speechChunks(text: string, maximum?: number): string[] {
	const limit = speechCharacterLimit(maximum);
	const characters = Array.from(text.trim());
	const chunks: string[] = [];
	for (let from = 0; from < characters.length; ) {
		let end = Math.min(from + limit, characters.length);
		if (end < characters.length) {
			for (let at = end; at > from + Math.floor(limit / 2); at--) {
				if (/\s/.test(characters[at - 1])) {
					end = at;
					break;
				}
			}
		}
		const chunk = characters.slice(from, end).join("").trim();
		if (chunk) chunks.push(chunk);
		from = end;
	}
	return chunks;
}

/** Start calls with the first sentence, then prepare speech in bounded chunks. */
export function callSpeechChunks(text: string, maximum?: number): string[] {
	const modelLimit = speechCharacterLimit(maximum);
	const characters = Array.from(text.trim());
	const chunks: string[] = [];
	for (let from = 0; from < characters.length; ) {
		let end: number;
		if (chunks.length === 0) {
			end = from + firstChunkEnd(characters.slice(from), modelLimit);
		} else {
			const limit = Math.min(modelLimit, 240);
			end = Math.min(from + limit, characters.length);
			if (end < characters.length) {
				let wordEnd = end;
				let foundWord = false;
				for (let at = end; at > from + Math.floor(limit / 2); at--) {
					const previous = characters[at - 1];
					const next = characters[at];
					if (
						/[。！？\n]/.test(previous) ||
						(/[.!?;:,…]/.test(previous) && /\s/.test(next))
					) {
						end = at;
						break;
					}
					if (!foundWord && /\s/.test(previous)) {
						wordEnd = at;
						foundWord = true;
					}
				}
				if (end === from + limit) end = wordEnd;
			}
		}
		const chunk = characters.slice(from, end).join("").trim();
		if (chunk) chunks.push(chunk);
		from = end;
		while (from < characters.length && /\s/.test(characters[from])) from++;
	}
	return chunks;
}

function sentenceEnds(body: string, final: boolean): number[] {
	const ends = Array.from(
		body.matchAll(/[.!?…]+["'”’»)\]]*(?=\s)|[。！？]+|\n/g),
		(match) => (match.index ?? 0) + match[0].length,
	);
	if (final) ends.push(body.length);
	return ends;
}

function clauseEnd(body: string, limit: number): number | undefined {
	for (const match of body.matchAll(/(?:[,;:]|[-–—])(?=\s)/g)) {
		const end = (match.index ?? 0) + match[0].length;
		const length = body.slice(0, end).trim().length;
		if (length > limit) return undefined;
		if (length >= CLAUSE_MINIMUM) return end;
	}
	return undefined;
}

function wordEnd(body: string, limit: number): number | undefined {
	for (let at = Math.min(limit, body.length - 1); at > 0; at--) {
		if (/\s/.test(body[at])) return at;
	}
	return undefined;
}

/**
 * Decides what a streamed reply says next from its whole text so far and how many characters of it
 * were already handed to synthesis. Returns null to wait for more text. Never cuts inside a word
 * unless `final` forces the remainder out.
 */
export function nextSpeechPiece(
	text: string,
	handed: number,
	{
		first = handed === 0,
		final = false,
		limit = PIECE_LIMIT,
	}: { first?: boolean; final?: boolean; limit?: number } = {},
): { text: string; handed: number } | null {
	if (text.length <= handed) return null;
	const rest = text.slice(handed);
	const base = handed + (rest.length - rest.trimStart().length);
	const body = text.slice(base);
	if (!body.trim()) return null;
	const size = (end: number) => body.slice(0, end).trim().length;
	const ends = sentenceEnds(body, final);
	let cut = first
		? ends.find((end) => size(end) >= FIRST_PIECE_MINIMUM && size(end) <= limit)
		: ends.findLast((end) => size(end) <= limit);
	if (cut === undefined && final)
		cut = body.length <= limit ? body.length : (wordEnd(body, limit) ?? limit);
	if (cut === undefined && body.trim().length >= CLAUSE_AFTER)
		cut = clauseEnd(body, limit);
	if (cut === undefined && body.length > limit) cut = wordEnd(body, limit);
	if (cut === undefined || cut <= 0) return null;
	return { text: body.slice(0, cut).trim(), handed: base + cut };
}

function sameVoice(
	a: BotSpeechVoice | null | undefined,
	b: BotSpeechVoice,
): boolean {
	return (
		a?.bitId === b.bitId &&
		a.local === b.local &&
		a.voice === b.voice &&
		speechCharacterLimit(a.maxInputCharacters) ===
			speechCharacterLimit(b.maxInputCharacters)
	);
}

const charCount = (text: string) => Array.from(text).length;

function wakeSource(source: SpeechSource) {
	const wake = source.wake;
	source.wake = null;
	wake?.();
}

function abortPieces(source: SpeechSource) {
	for (const piece of source.pieces) piece.abort?.abort();
}

/** One call's final replies. Existing history and older pages never start audio. */
export function createBotCallSpeech({
	callId,
	initialEntries,
	synthesize,
	synthesizeStream,
	play,
	playStream,
	stop,
	pause,
	resume,
	canSpeak,
	canStream,
	cue,
	onError,
	electionDelayMs = 0,
	progress,
	trace = callLatency,
	now = Date.now,
	timers = defaultTimers,
}: {
	callId: string;
	initialEntries: readonly IInboxEntry[];
	synthesize: (voice: BotSpeechVoice, text: string) => Promise<Blob>;
	synthesizeStream?: (
		voice: BotSpeechVoice,
		text: string,
		signal: AbortSignal,
	) => Promise<ISpeechStream>;
	play: (blob: Blob, botId: string) => Promise<void>;
	playStream?: (stream: ISpeechStream, botId: string) => Promise<void>;
	stop: () => void;
	pause?: () => boolean;
	resume?: () => void;
	canSpeak: (botId: string) => boolean;
	canStream?: () => boolean;
	cue?: () => SpeechCue | null | undefined;
	onError: (error: unknown) => void;
	electionDelayMs?: number | (() => number);
	progress?: () => SpeechPlaybackProgress | null;
	trace?: CallLatencyTrace;
	now?: () => number;
	timers?: SpeechTimers;
}) {
	const cancelled = new Set<string>();
	const seen = new Set(initialEntries.map((entry) => entry.message_id));
	const watermark = initialEntries.reduce<string | null>(
		(last, entry) =>
			last === null || compareInboxCursors(entry.cursor, last) > 0
				? entry.cursor
				: last,
		null,
	);
	let snapshot: BotCallSpeechSnapshot = {
		entries: [],
		items: [],
		voices: new Map(),
	};
	let queue: Reply[] = [];
	let active: Reply | null = null;
	let current: {
		reply: Reply;
		source: SpeechSource;
		startedAt: number;
		index: number;
		playedChars: number;
		playing: boolean;
		ahead: (() => void) | null;
	} | null = null;
	let closed = false;
	let finishDelay: (() => void) | null = null;
	const streamed = new Map<string, LiveReply>();
	const lives = new Set<LiveReply>();
	const streamFailed = new Set<string>();

	const currentMessage = (reply: Reply): ConversationMessage | undefined =>
		snapshot.items.find(
			(item): item is ConversationMessage =>
				item.type === "message" &&
				item.id === reply.id &&
				!item.approval &&
				!item.errorCode &&
				item.content.kind === "text" &&
				item.content.text.trim() === reply.text,
		);
	const valid = (reply: Reply) =>
		!closed &&
		!cancelled.has(reply.id) &&
		sameVoice(snapshot.voices.get(reply.botId), reply.voice) &&
		(reply.live !== undefined || Boolean(currentMessage(reply))) &&
		canSpeak(reply.botId);

	const delayOf = () =>
		typeof electionDelayMs === "function" ? electionDelayMs() : electionDelayMs;

	const stopCue = (live: LiveReply) => {
		if (live.timer !== null) timers.clear(live.timer);
		live.timer = null;
		if (!live.cueOn) return;
		live.cueOn = false;
		cue?.()?.stop();
	};
	const cueIdle = (live: LiveReply) =>
		live.state !== "dropped" &&
		active === live.reply &&
		!current?.playing &&
		live.source.completed >= live.source.pieces.length &&
		!live.text.slice(live.handed).trim();
	const armCue = (live: LiveReply, delay: number) => {
		if (!cue || live.state === "dropped") return;
		if (live.timer !== null) timers.clear(live.timer);
		live.timer = timers.set(() => {
			live.timer = null;
			if (!cueIdle(live) || live.cueOn) return;
			live.cueOn = true;
			cue()?.start();
		}, delay);
	};
	const dropLive = (live: LiveReply) => {
		live.state = "dropped";
		stopCue(live);
		lives.delete(live);
	};

	const startsStream = (voice: BotSpeechVoice) =>
		Boolean(synthesizeStream && playStream) &&
		!voice.local &&
		!streamFailed.has(voice.bitId) &&
		(canStream?.() ?? true);

	const prepare = async (
		voice: BotSpeechVoice,
		piece: Piece,
	): Promise<PieceResult> => {
		try {
			if (synthesizeStream && startsStream(voice)) {
				const abort = new AbortController();
				piece.abort = abort;
				try {
					const stream = await synthesizeStream(
						voice,
						piece.text,
						abort.signal,
					);
					return { ok: true, audio: { kind: "stream", stream } };
				} catch (error) {
					if (abort.signal.aborted) return { ok: false, error };
					streamFailed.add(voice.bitId);
				}
			}
			return {
				ok: true,
				audio: { kind: "blob", blob: await synthesize(voice, piece.text) },
			};
		} catch (error) {
			// Prefetch may fail before playback finishes or after the call ends.
			return { ok: false, error };
		}
	};

	const playPiece = (audio: PieceAudio, botId: string) =>
		audio.kind === "blob"
			? play(audio.blob, botId)
			: (playStream?.(audio.stream, botId) ?? Promise.resolve());

	const pieceAt = async (
		reply: Reply,
		source: SpeechSource,
		index: number,
	): Promise<Piece | null> => {
		while (valid(reply)) {
			const piece = source.pieces[index];
			if (piece) return piece;
			if (source.ended) return null;
			await new Promise<void>((resolve) => {
				source.wake = resolve;
			});
		}
		return null;
	};

	const speak = async (reply: Reply, state: NonNullable<typeof current>) => {
		const { source } = state;
		const audioOf = (piece: Piece) => {
			piece.audio ??= prepare(reply.voice, piece);
			return piece.audio;
		};
		state.ahead = () => {
			const next = source.pieces[state.index + 1];
			if (next && state.playing && valid(reply)) void audioOf(next);
		};
		try {
			for (let index = 0; ; index++) {
				const piece = await pieceAt(reply, source, index);
				if (!piece) break;
				const result = await audioOf(piece);
				if (!valid(reply)) break;
				if (!result.ok) throw result.error;
				trace.speechReady(reply.id);
				if (index === 0) trace.playbackStarted(reply.id);
				state.index = index;
				state.playing = true;
				if (reply.live) {
					reply.live.audioStarted = true;
					stopCue(reply.live);
				}
				const playback = playPiece(result.audio, reply.botId);
				state.ahead();
				try {
					await playback;
				} catch (error) {
					// Finish the in-flight synthesis before another reply uses the model.
					await source.pieces[index + 1]?.audio;
					throw error;
				} finally {
					state.playing = false;
					state.playedChars += charCount(piece.text);
					source.completed++;
					if (reply.live?.flushed && reply.live.state === "open")
						armCue(reply.live, CUE_AFTER_FLUSH_MS);
				}
				if (!valid(reply)) break;
			}
			if (!valid(reply)) trace.notVoiced(reply.id, "dropped");
		} finally {
			abortPieces(source);
		}
	};

	const electionDelay = async (reply: Reply) => {
		const delay = delayOf();
		if (delay > 0 && valid(reply))
			await new Promise<void>((resolve) => {
				const timer = setTimeout(() => {
					finishDelay = null;
					resolve();
				}, delay);
				finishDelay = () => {
					clearTimeout(timer);
					finishDelay = null;
					resolve();
				};
			});
	};

	const drain = async () => {
		if (active || closed) return;
		while (queue.length && !closed) {
			const reply = queue.shift();
			if (!reply) break;
			active = reply;
			const source: SpeechSource = reply.live?.source ?? {
				pieces: [],
				ended: true,
				completed: 0,
				spokenChars: 0,
				wake: null,
			};
			const state = {
				reply,
				source,
				startedAt: now(),
				index: 0,
				playedChars: 0,
				playing: false,
				ahead: null,
			};
			current = state;
			try {
				if (!reply.live) await electionDelay(reply);
				if (!valid(reply)) {
					trace.notVoiced(reply.id, "dropped");
					continue;
				}
				if (!reply.live) {
					const spoken = [...plainMarkdownLines(reply.text)].join("\n").trim();
					source.pieces = callSpeechChunks(
						spoken,
						reply.voice.maxInputCharacters,
					).map((text) => ({ text }));
					source.spokenChars = charCount(spoken);
				}
				await speak(reply, state);
			} catch (error) {
				if (valid(reply)) {
					trace.notVoiced(reply.id, "failed");
					onError(error);
				}
			} finally {
				active = null;
				current = null;
				if (reply.live) dropLive(reply.live);
			}
		}
	};

	const abandon = (reply: Reply) => {
		cancelled.add(reply.id);
		if (reply.live) {
			dropLive(reply.live);
			reply.live.source.ended = true;
			wakeSource(reply.live.source);
		}
		finishDelay?.();
		stop();
		if (current?.reply === reply) abortPieces(current.source);
	};

	const dropQueue = () => {
		for (const reply of queue) if (reply.live) dropLive(reply.live);
		queue = [];
	};

	const pump = (live: LiveReply, final: boolean) => {
		const { source, reply } = live;
		const limit = Math.min(
			speechCharacterLimit(reply.voice.maxInputCharacters),
			PIECE_LIMIT,
		);
		for (;;) {
			const next = nextSpeechPiece(live.text, live.handed, {
				first: source.pieces.length === 0,
				final,
				limit,
			});
			if (!next) break;
			live.handed = next.handed;
			const spoken = [...plainMarkdownLines(next.text)].join("\n").trim();
			if (!spoken) continue;
			source.pieces.push({ text: spoken });
			source.spokenChars += charCount(spoken);
			if (current?.source === source) current.ahead?.();
		}
		wakeSource(source);
	};

	const noteText = (live: LiveReply) => {
		if (live.textSeen || !live.text.trim()) return;
		live.textSeen = true;
		trace.firstText(live.transcriptId);
		trace.replySeen(live.transcriptId, live.reply.id);
	};

	const openStream = (
		transcriptId: string,
		botId: string,
		voice: BotSpeechVoice,
	) => {
		const key = `${transcriptId}\u0000${botId}`;
		const reply: Reply = {
			id: `stream:${transcriptId}`,
			botId,
			text: "",
			voice,
		};
		const live: LiveReply = {
			key,
			transcriptId,
			reply,
			source: {
				pieces: [],
				ended: false,
				completed: 0,
				spokenChars: 0,
				wake: null,
			},
			state: "open",
			text: "",
			handed: 0,
			flushed: false,
			textSeen: false,
			audioStarted: false,
			cueOn: false,
			timer: null,
		};
		reply.live = live;
		streamed.set(key, live);
		lives.add(live);
		queue.push(reply);
		armCue(live, CUE_AFTER_OPEN_MS);
		void drain();
		return live;
	};

	const handleOf = (live: LiveReply): StreamedReply => ({
		update(text) {
			if (live.state !== "open") return;
			live.text = text;
			noteText(live);
			if (live.flushed && text.slice(live.handed).trim()) {
				live.flushed = false;
				stopCue(live);
			}
			pump(live, false);
		},
		flush() {
			if (live.state !== "open") return;
			pump(live, true);
			// Repeated flushes of one tool round must not keep pushing the cue back.
			if (live.flushed) return;
			live.flushed = true;
			armCue(live, CUE_AFTER_FLUSH_MS);
		},
		end(text) {
			if (live.state !== "open") return;
			if (text !== undefined) live.text = text;
			noteText(live);
			live.state = "ended";
			stopCue(live);
			pump(live, true);
			live.source.ended = true;
			wakeSource(live.source);
		},
		cancel() {
			if (live.state === "dropped") return;
			dropLive(live);
			const { source } = live;
			const keep =
				current?.source === source && current.playing
					? current.index + 1
					: source.completed;
			for (const piece of source.pieces.splice(keep)) piece.abort?.abort();
			source.ended = true;
			wakeSource(source);
			trace.notVoiced(live.reply.id, "cancelled");
			if (!live.audioStarted) {
				streamed.delete(live.key);
				if (live.stored) queue.push(live.stored);
			}
			void drain();
		},
	});

	return {
		update(next: BotCallSpeechSnapshot) {
			if (closed) return;
			snapshot = next;
			if (active && !valid(active)) abandon(active);
			queue = queue.filter(valid);
			const transcripts = new Set(
				next.items
					.filter((item) => item.type === "transcript" && item.call === callId)
					.map((item) => item.id),
			);
			const cursors = new Map(
				next.entries.map((entry) => [entry.message_id, entry.cursor]),
			);
			const replies = next.items
				.filter((item): item is ConversationMessage => item.type === "message")
				.sort((a, b) =>
					compareInboxCursors(cursors.get(a.id) ?? "", cursors.get(b.id) ?? ""),
				);
			const replyTargets = new Map(
				replies.map((item) => [item.id, item.replyTo]),
			);
			// A bot that waited for another bot answers its reply, so follow the chain
			// back to this call's utterance before deciding whether to play it.
			const answeredTranscript = (replyTo: string | undefined) => {
				let target = replyTo;
				const visited = new Set<string>();
				while (target && !visited.has(target)) {
					if (transcripts.has(target)) return target;
					visited.add(target);
					target = replyTargets.get(target);
				}
				return undefined;
			};
			for (const item of replies) {
				if (
					seen.has(item.id) ||
					!item.replyTo ||
					!answeredTranscript(item.replyTo)
				)
					continue;
				const cursor = cursors.get(item.id);
				if (
					!cursor ||
					(watermark !== null && compareInboxCursors(cursor, watermark) <= 0)
				)
					continue;
				if (!next.voices.has(item.senderId)) continue;
				seen.add(item.id);
				const transcript = answeredTranscript(item.replyTo);
				const live = transcript
					? streamed.get(`${transcript}\u0000${item.senderId}`)
					: undefined;
				const voice = next.voices.get(item.senderId);
				const speakable =
					voice &&
					!item.approval &&
					!item.errorCode &&
					item.content.kind === "text" &&
					item.content.text.trim() &&
					canSpeak(item.senderId);
				const stored: Reply | undefined = speakable
					? {
							id: item.id,
							botId: item.senderId,
							text:
								item.content.kind === "text" ? item.content.text.trim() : "",
							voice,
						}
					: undefined;
				if (live) {
					live.messageId ??= item.id;
					live.stored = stored;
					continue;
				}
				if (transcript) trace.replySeen(transcript, item.id);
				if (!stored) {
					trace.notVoiced(item.id, "not voiced here");
					continue;
				}
				queue.push(stored);
			}
			void drain();
		},
		/** Speaks a reply as its text grows; null when the bot has no voice on this device. */
		stream(transcriptId: string, botId: string): StreamedReply | null {
			const voice = snapshot.voices.get(botId);
			if (closed || !voice || !canSpeak(botId)) return null;
			if (streamed.has(`${transcriptId}\u0000${botId}`)) return null;
			return handleOf(openStream(transcriptId, botId, voice));
		},
		/** The text of the chunk being played, for telling the bot's own voice from a person. */
		playingText(botId?: string): string | null {
			if (!current?.playing) return null;
			if (botId !== undefined && current.reply.botId !== botId) return null;
			return current.source.pieces[current.index]?.text ?? null;
		},
		/** Holds the playing speech in place for a barge-in check; false when nothing was playing. */
		pause(): boolean {
			if (closed || !current?.playing) return false;
			return pause?.() ?? false;
		},
		resume() {
			if (!closed) resume?.();
		},
		/**
		 * Cuts the active reply off: the rest of it, the queue, a pending election delay and any prepared
		 * speech are dropped. Returns what was heard, or null when nothing was playing.
		 * `since` ignores a request made before the active reply began.
		 */
		interrupt(
			filter: { botId?: string; since?: number } = {},
		): InterruptedSpeech | null {
			const target = current;
			if (!target || closed) return null;
			if (filter.botId !== undefined && target.reply.botId !== filter.botId)
				return null;
			if (filter.since !== undefined && filter.since < target.startedAt)
				return null;
			const { live } = target.reply;
			const spokenChars = target.source.spokenChars;
			const messageId = live ? live.messageId : target.reply.id;
			let record: InterruptedSpeech | null = null;
			if (
				(target.playing || (live?.audioStarted && target.playedChars > 0)) &&
				spokenChars > 0 &&
				target.source.ended &&
				messageId
			) {
				const position = target.playing ? progress?.() : null;
				const share =
					position && position.durationMs > 0
						? Math.min(1, Math.max(0, position.elapsedMs / position.durationMs))
						: 0;
				const chunk = charCount(target.source.pieces[target.index]?.text ?? "");
				record = {
					message_id: messageId,
					heard_chars: Math.min(
						spokenChars,
						Math.floor(
							target.playedChars + (target.playing ? share * chunk : 0),
						),
					),
					spoken_chars: spokenChars,
				};
			}
			dropQueue();
			abandon(target.reply);
			return record;
		},
		dispose() {
			closed = true;
			for (const live of lives) {
				dropLive(live);
				live.source.ended = true;
				wakeSource(live.source);
			}
			finishDelay?.();
			queue = [];
			stop();
		},
	};
}
