import type {
	ConversationItem,
	ConversationMessage,
} from "../../components/bots-tab/fold-entries";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { compareInboxCursors } from "../inbox-cursor";
import { plainMarkdownLines } from "../markdown-text";

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

interface Reply {
	id: string;
	botId: string;
	text: string;
	voice: BotSpeechVoice;
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

/** Start calls with a short phrase, then prepare speech in bounded chunks. */
export function callSpeechChunks(text: string, maximum?: number): string[] {
	const modelLimit = speechCharacterLimit(maximum);
	const characters = Array.from(text.trim());
	const chunks: string[] = [];
	for (let from = 0; from < characters.length; ) {
		const limit = Math.min(modelLimit, chunks.length === 0 ? 160 : 240);
		let end = Math.min(from + limit, characters.length);
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
		const chunk = characters.slice(from, end).join("").trim();
		if (chunk) chunks.push(chunk);
		from = end;
		while (from < characters.length && /\s/.test(characters[from])) from++;
	}
	return chunks;
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

/** One call's final replies. Existing history and older pages never start audio. */
export function createBotCallSpeech({
	callId,
	initialEntries,
	synthesize,
	play,
	stop,
	canSpeak,
	onError,
	electionDelayMs = 0,
}: {
	callId: string;
	initialEntries: readonly IInboxEntry[];
	synthesize: (voice: BotSpeechVoice, text: string) => Promise<Blob>;
	play: (blob: Blob, botId: string) => Promise<void>;
	stop: () => void;
	canSpeak: (botId: string) => boolean;
	onError: (error: unknown) => void;
	electionDelayMs?: number;
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
	let closed = false;
	let finishDelay: (() => void) | null = null;

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
		Boolean(currentMessage(reply)) &&
		canSpeak(reply.botId);

	const drain = async () => {
		if (active || closed) return;
		while (queue.length && !closed) {
			const reply = queue.shift();
			if (!reply) break;
			active = reply;
			try {
				if (electionDelayMs > 0)
					await new Promise<void>((resolve) => {
						const timer = setTimeout(() => {
							finishDelay = null;
							resolve();
						}, electionDelayMs);
						finishDelay = () => {
							clearTimeout(timer);
							finishDelay = null;
							resolve();
						};
					});
				if (!valid(reply)) continue;
				const chunks = callSpeechChunks(
					[...plainMarkdownLines(reply.text)].join("\n"),
					reply.voice.maxInputCharacters,
				);
				const prepare = async (text: string) => {
					try {
						return {
							ok: true as const,
							blob: await synthesize(reply.voice, text),
						};
					} catch (error) {
						// Prefetch may fail before playback finishes or after the call ends.
						return { ok: false as const, error };
					}
				};
				let pending = chunks.length ? prepare(chunks[0]) : undefined;
				for (let index = 0; pending; index++) {
					const result = await pending;
					pending = undefined;
					if (!valid(reply)) break;
					if (!result.ok) throw result.error;
					const playback = play(result.blob, reply.botId);
					if (index + 1 < chunks.length && valid(reply))
						pending = prepare(chunks[index + 1]);
					try {
						await playback;
					} catch (error) {
						// Finish the in-flight synthesis before another reply uses the model.
						await pending;
						throw error;
					}
				}
			} catch (error) {
				if (valid(reply)) onError(error);
			} finally {
				active = null;
			}
		}
	};

	return {
		update(next: BotCallSpeechSnapshot) {
			if (closed) return;
			snapshot = next;
			if (active && !valid(active)) {
				cancelled.add(active.id);
				finishDelay?.();
				stop();
			}
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
			const answersCall = (replyTo: string | undefined) => {
				let target = replyTo;
				const visited = new Set<string>();
				while (target && !visited.has(target)) {
					if (transcripts.has(target)) return true;
					visited.add(target);
					target = replyTargets.get(target);
				}
				return false;
			};
			for (const item of replies) {
				if (seen.has(item.id) || !item.replyTo || !answersCall(item.replyTo))
					continue;
				const cursor = cursors.get(item.id);
				if (
					!cursor ||
					(watermark !== null && compareInboxCursors(cursor, watermark) <= 0)
				)
					continue;
				if (!next.voices.has(item.senderId)) continue;
				seen.add(item.id);
				const voice = next.voices.get(item.senderId);
				if (
					!voice ||
					item.approval ||
					item.errorCode ||
					item.content.kind !== "text" ||
					!item.content.text.trim() ||
					!canSpeak(item.senderId)
				)
					continue;
				queue.push({
					id: item.id,
					botId: item.senderId,
					text: item.content.text.trim(),
					voice,
				});
			}
			void drain();
		},
		dispose() {
			closed = true;
			finishDelay?.();
			queue = [];
			stop();
		},
	};
}
