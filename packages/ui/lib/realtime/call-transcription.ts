import { MAX_QUEUED_UTTERANCES, type Utterance } from "./call-listening";

interface Work<T> {
	utterance: T;
	generation: number;
	final: boolean;
}

/** Serializes speech model calls, keeping final utterances ahead of replaceable previews. */
export function createCallTranscription<T extends Utterance>({
	transcribe,
	onPartial,
	onFinal,
	onError,
	limit = MAX_QUEUED_UTTERANCES,
}: {
	transcribe: (utterance: T) => Promise<string>;
	onPartial: (text: string) => void;
	onFinal: (text: string, utterance: T) => Promise<void>;
	onError: (error: unknown) => void;
	limit?: number;
}) {
	const maximum =
		Number.isSafeInteger(limit) && limit > 0 ? limit : MAX_QUEUED_UTTERANCES;
	const finals: Work<T>[] = [];
	let preview: Work<T> | null = null;
	let draft: { offsetMs: number; generation: number } | null = null;
	let draftText = "";
	let generation = 0;
	let latestOffset = Number.NEGATIVE_INFINITY;
	let finalOffset = Number.NEGATIVE_INFINITY;
	let busy = false;
	let closed = false;

	const show = (text: string) => {
		if (text === draftText) return;
		draftText = text;
		onPartial(text);
	};
	const current = (work: Work<T>) =>
		!closed &&
		draft?.offsetMs === work.utterance.offsetMs &&
		draft.generation === work.generation;
	const clear = (work: Work<T>) => {
		if (!current(work)) return;
		draft = null;
		show("");
	};

	const drain = async () => {
		if (busy || closed) return;
		busy = true;
		try {
			while (!closed) {
				const work = finals.shift() ?? preview;
				if (!work) break;
				if (preview === work) preview = null;
				try {
					const text = (await transcribe(work.utterance)).trim();
					if (closed) return;
					if (work.final) {
						if (text) await onFinal(text, work.utterance);
					} else if (text && current(work)) {
						show(text);
					}
				} catch (error) {
					if (!closed && (work.final || current(work))) onError(error);
				} finally {
					if (work.final) clear(work);
				}
			}
		} finally {
			busy = false;
		}
	};

	return {
		partial(utterance: T) {
			if (
				closed ||
				utterance.offsetMs <= finalOffset ||
				utterance.offsetMs < latestOffset
			)
				return;
			latestOffset = utterance.offsetMs;
			if (draft?.offsetMs !== utterance.offsetMs) {
				draft = { offsetMs: utterance.offsetMs, generation };
				show("");
			}
			preview = { utterance, generation, final: false };
			void drain();
		},
		final(utterance: T) {
			if (closed || utterance.offsetMs <= finalOffset) return;
			finalOffset = utterance.offsetMs;
			if (preview && preview.utterance.offsetMs <= utterance.offsetMs)
				preview = null;
			finals.push({ utterance, generation, final: true });
			while (finals.length > maximum) {
				const dropped = finals.shift();
				if (dropped) clear(dropped);
			}
			void drain();
		},
		resetPartial() {
			if (closed) return;
			generation++;
			preview = null;
			draft = null;
			show("");
		},
		close() {
			if (closed) return;
			closed = true;
			finals.length = 0;
			preview = null;
			draft = null;
			show("");
		},
	};
}
