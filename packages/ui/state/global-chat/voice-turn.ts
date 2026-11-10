import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import type {
	InterruptedSpeech,
	StreamedReply,
} from "../../lib/realtime/bot-call-speech";

export interface VoiceTranscript {
	call: string;
	offset_ms: number;
	duration_ms: number;
	mentions: string[];
	interrupted?: InterruptedSpeech;
}

export interface VoiceTurnRequest {
	messageId: string;
	text: string;
	transcript: VoiceTranscript;
}

export type VoiceTurnOutcome = "completed" | "failed" | "cancelled";

export interface VoiceTurnHandle {
	runId: string;
	done: Promise<VoiceTurnOutcome>;
}

/** Runs a spoken utterance as the bot chat's interactive turn; null when the chat cannot take it now. */
export type VoiceTurnSender = (
	request: VoiceTurnRequest,
) => Promise<VoiceTurnHandle | null>;

const senders = new Map<string, VoiceTurnSender>();

export function registerVoiceTurnSender(
	botId: string,
	sender: VoiceTurnSender,
): () => void {
	senders.set(botId, sender);
	return () => {
		if (senders.get(botId) === sender) senders.delete(botId);
	};
}

export function voiceTurnSender(botId: string): VoiceTurnSender | null {
	return senders.get(botId) ?? null;
}

export interface VoiceSend {
	messageId: string;
	transcript: VoiceTranscript;
	onRun: (runId: string) => void;
}

function outcomeOf(message: IMessage | undefined): VoiceTurnOutcome {
	if (!message?.error) return "completed";
	return message.error.kind === "cancelled" ? "cancelled" : "failed";
}

/**
 * Runs a spoken turn through the chat's send pipeline. Resolves with the run once it starts, or
 * null when the pipeline ended before starting one (nothing was taken).
 */
export function runVoiceTurn(
	request: VoiceTurnRequest,
	canTake: () => boolean,
	send: (voice: VoiceSend) => Promise<IMessage | undefined>,
): Promise<VoiceTurnHandle | null> {
	if (!canTake()) return Promise.resolve(null);
	return new Promise((resolve) => {
		let started = false;
		const finished = send({
			messageId: request.messageId,
			transcript: request.transcript,
			onRun: (runId) => {
				started = true;
				resolve({
					runId,
					done: finished.then(outcomeOf, () => "failed" as const),
				});
			},
		});
		finished.then(
			() => {
				if (!started) resolve(null);
			},
			() => {
				if (!started) resolve(null);
			},
		);
	});
}

export interface VoiceRunSnapshot {
	text: string;
	toolRunning: boolean;
}

export interface VoiceRunSource {
	subscribe: (listener: () => void) => () => void;
	read: () => VoiceRunSnapshot | null;
}

export interface VoiceReplyFollow {
	cancel: () => void;
}

/**
 * Feeds a streaming run's text to the spoken reply as it grows; a running tool step with no new
 * text flushes what is buffered so the caller hears it before the tool finishes.
 */
export function followVoiceReply(
	reply: StreamedReply | null,
	source: VoiceRunSource,
	done: Promise<VoiceTurnOutcome>,
	onFirstText?: () => void,
): VoiceReplyFollow {
	let settled = false;
	let lastText = "";
	let seenText = false;
	const check = () => {
		const snapshot = source.read();
		if (settled || !snapshot) return;
		if (snapshot.text !== lastText) {
			lastText = snapshot.text;
			if (!seenText && lastText.trim() !== "") {
				seenText = true;
				onFirstText?.();
			}
			reply?.update(lastText);
		} else if (snapshot.toolRunning) reply?.flush();
	};
	const unsubscribe = source.subscribe(check);
	const settle = (outcome: VoiceTurnOutcome) => {
		if (settled) return;
		check();
		settled = true;
		unsubscribe();
		if (outcome === "completed") reply?.end(lastText);
		else reply?.cancel();
	};
	check();
	done.then(settle, () => settle("failed"));
	return {
		cancel: () => {
			if (settled) return;
			settled = true;
			unsubscribe();
			reply?.cancel();
		},
	};
}
